//! Durable source-to-outcome worker shared by the desktop and real-device checks.
//! No UI thread I/O, phone-agent fallback, or manual adoption of model prose.
use crate::{adb::Adb, amoo_usage, companion_link, context_ledger::{self, outcomes::{self, OutcomeBundle, Target}}, device::Device, home_ai};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}};

const MAX_ATTEMPTS: usize = 3;
fn retry_empty_reply(job: &Job, local_preflight_failures: usize) -> bool {
    job.attempts.last().is_some_and(|attempt| attempt.empty_reply)
        && job.attempts.iter().skip(job.round_start).filter(|attempt| attempt.empty_reply).count() == 1
        && job.attempts.len().saturating_sub(job.round_start).saturating_sub(local_preflight_failures) < MAX_ATTEMPTS
}
fn empty_reply_backoff(cancel: &AtomicBool) -> Result<()> {
    for _ in 0..80 {
        check_cancel(cancel)?;
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    check_cancel(cancel)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub kind: String,
    pub transfer_id: String,
    pub sha256: String,
    pub path: String,
    pub verified_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub bundle: OutcomeBundle,
    pub deliveries: Vec<Delivery>,
    pub reused: bool,
    /// Documents remain available when the phone goes away. Retrying sends the
    /// same content-addressed transfer without requesting another model draft.
    pub delivery_error: Option<String>,
    #[serde(default)]
    pub delivery_attempt_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Attempt {
    #[serde(default, skip_serializing_if = "is_false")]
    automatic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<crate::model_source::Source>,
    started_ms: i64,
    raw_path: Option<PathBuf>,
    model: Option<String>,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    empty_reply: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inference: Option<home_ai::InferenceMeta>,
}
fn is_false(value: &bool) -> bool { !*value }
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Retry { started_ms: i64, attempt_start: usize, reason: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Revalidation {
    checked_ms: i64,
    attempt: usize,
    raw_sha256: String,
    accepted: bool,
    error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RetiredOutcome {
    retired_ms: i64,
    reason: String,
    bundle: OutcomeBundle,
    deliveries: Vec<Delivery>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Job {
    schema: u32,
    scope_id: String,
    target: Target,
    input_revision: String,
    attempts: Vec<Attempt>,
    bundle: Option<OutcomeBundle>,
    deliveries: Vec<Delivery>,
    #[serde(default)]
    last_delivery_error: Option<String>,
    #[serde(default)]
    last_delivery_attempt_ms: i64,
    #[serde(default)]
    round_start: usize,
    #[serde(default)]
    retries: Vec<Retry>,
    #[serde(default)]
    revalidations: Vec<Revalidation>,
    /// Only an explicit regeneration advances this boundary. Old raw replies
    /// remain on disk but can never satisfy the newly requested content round.
    #[serde(default)]
    revalidation_floor: usize,
    #[serde(default)]
    retired_outcomes: Vec<RetiredOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_inference: Option<home_ai::InferenceMeta>,
}
// Leave room for JSON escaping in the connector's 256 KiB request limit.
const PROMPT_BUDGET: usize = 224 * 1024;
fn append_bounded(prompt: &mut String, text: &str, limit: usize) {
    let available = PROMPT_BUDGET.saturating_sub(prompt.len()).min(limit).min(text.len());
    let mut end = available;
    while !text.is_char_boundary(end) { end -= 1; }
    prompt.push_str(&text[..end]);
}
fn now() -> i64 { chrono::Utc::now().timestamp_millis() }
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "已停止生成成果；原始材料仍保留"); Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(meta.is_dir() && !meta.file_type().is_symlink(), "成果目录不能是链接");
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(path, fs::Permissions::from_mode(0o700))?; }
    Ok(())
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().context("成果路径无效")?)?;
    tmp.write_all(bytes)?; tmp.as_file().sync_all()?; tmp.persist(path)?; Ok(())
}
fn save(path: &Path, job: &Job) -> Result<()> {
    let raw=serde_json::to_vec(job)?;
    ensure!(raw.len()<2*1024*1024,"生成记录超过原有容量，未覆盖上次状态；已有成果和原始稿件保留");
    crate::data_store::write_json(path,&raw)?; Ok(())
}
fn state_bytes(path: &Path) -> Result<Vec<u8>> { crate::data_store::read_bytes(path, 2 * 1024 * 1024) }
fn state_valid(path: &Path) -> bool { !path.is_symlink() && state_bytes(path).is_ok_and(|b| b.len() < 2 * 1024 * 1024) }
fn scoped_dir(root: &Path, scope: &str) -> Result<PathBuf> {
    private_dir(root)?;
    let scope_dir = root.join(scope);
    private_dir(&scope_dir)?;
    let dir = scope_dir.join("pipeline"); private_dir(&dir)?;
    ensure!(fs::canonicalize(&dir)?.starts_with(fs::canonicalize(root)?), "成果目录已离开本地资料范围");
    Ok(dir)
}
fn check_capacity(dir: &Path) -> Result<()> {
    ensure!(crate::storage_expansion::free_space(dir)? > 2 * 1024 * 1024, "本机磁盘空间不足，原始成果和生成记录已保留"); Ok(())
}
fn job_key(target: &Target, revision: &str) -> String {
    // Non-security filename key; every read validates the full target/revision.
    context_ledger::scope_id(&serde_json::to_string(target).expect("target serializes"), Some(revision))
}
fn transfer_id(scope: &str, desktop: &str, hash: &str, name: &str) -> String {
    let key = context_ledger::scope_id(&format!("{scope}/{desktop}/{name}"), Some(hash));
    format!("{}-{}-{}-{}-{}", &key[..8], &key[8..12], &key[12..16], &key[16..20], &key[20..])
}
fn confined_file_in_scope(canonical_scope: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(!fs::symlink_metadata(path)?.file_type().is_symlink(), "成果文件不能是链接");
    let real = fs::canonicalize(path)?;
    ensure!(real.starts_with(canonical_scope) && real.is_file(), "成果文件不属于当前设备与账号");
    Ok(real)
}
fn confined_file(root: &Path, scope: &str, path: &Path) -> Result<PathBuf> {
    let canonical_scope = fs::canonicalize(root.join(scope))?;
    confined_file_in_scope(&canonical_scope, path)
}
fn verify_bundle(root: &Path, bundle: &OutcomeBundle) -> Result<()> {
    ensure!(!bundle.files.is_empty(), "成果没有可打开的文件");
    let canonical_scope = fs::canonicalize(root.join(&bundle.scope_id))?;
    for file in &bundle.files {
        let path = confined_file_in_scope(&canonical_scope, &file.path)?;
        ensure!(crate::device::sha256_file(&path)? == file.sha256, "成果文件已被编辑，保留你的修改；请从预览中选择发送文件");
    }
    Ok(())
}
fn check_account(adb: &Adb, device: &Device, account: Option<&str>) -> Result<()> {
    let transport = device.resolve_active_transport(adb)?;
    ensure!(adb.shell(&transport, &["getprop", "ro.serialno"])?.trim() == device.serial, "手机传输连接已变化，停止自动回传");
    ensure!(account.is_some() && amoo_usage::account_scope_fresh(adb, device)?.as_deref() == account,
        "手机账号已变化或未确认，已停止读取与自动回传");
    Ok(())
}

// A validator upgrade can recover an unchanged original response without a new
// model request. All current source, scope and semantic checks still apply.
fn revalidate_saved(root: &Path, device: &str, account: Option<&str>, request: &outcomes::GenerationRequest,
    dir: &Path, key: &str, job: &mut Job, accepted: Option<&OutcomeBundle>) -> Result<()> {
    // A later correction may introduce a new error. Recheck the last bounded
    // round newest first, accepting only an entire response that passes all
    // current checks. Never combine drafts or edit their content.
    let canonical_scope = fs::canonicalize(root.join(&request.scope_id))?;
    ensure!(job.revalidation_floor <= job.attempts.len(), "原稿重验边界无效");
    for index in (job.revalidation_floor..job.attempts.len()).rev().take(MAX_ATTEMPTS) {
    let Some(path) = job.attempts[index].raw_path.as_ref() else { continue; };
    ensure!(path == &dir.join(format!("{key}-attempt-{}.json.txt", index + 1)), "原始稿件路径与生成记录不一致");
    let path = confined_file_in_scope(&canonical_scope, path)?;
    ensure!(fs::metadata(&path)?.len() <= 512 * 1024, "原始稿件超过校验容量");
    let raw = fs::read_to_string(&path)?;
    let hash = crate::device::sha256_reader(raw.as_bytes())?;
    // A layout upgrade must use the exact accepted original. A later rejected
    // correction or a different older draft cannot silently replace its content.
    let accepted_check = accepted.map(|bundle| outcomes::sync_saved_actions(root, device, account, bundle, &raw));
    if matches!(accepted_check, Some(Ok(false))) { continue; }
    let saved = match accepted_check {
        Some(Err(error)) => Err(error),
        _ => outcomes::save(root, device, account, request, &raw),
    };
    let error = match saved {
        Ok(bundle) => { job.bundle = Some(bundle); job.accepted_inference=job.attempts[index].inference.clone(); None }
        Err(error) if error.downcast_ref::<outcomes::ModelContentError>().is_some() => Some(format!("{error:#}")),
        Err(error) => return Err(error.context("重新校验原稿失败；未调用模型")),
    };
    if !job.revalidations.iter().any(|r| r.attempt == index + 1 && r.raw_sha256 == hash && r.error == error) {
        job.revalidations.push(Revalidation { checked_ms: now(), attempt: index + 1, raw_sha256: hash,
            accepted: error.is_none(), error });
    }
    if job.bundle.is_some() { break; }
    }
    Ok(())
}

/// Locally upgrade an accepted bundle while its per-job lock is held. No
/// directory creation, source collection, retry-budget reset or model request.
/// Edited documents and all previous bytes/manifests remain untouched.
fn revalidate_cached_bundle(root: &Path, device: &str, account: Option<&str>,
    request: &outcomes::GenerationRequest, dir: &Path, key: &str, job: &mut Job) -> Result<()> {
    let previous=job.bundle.as_ref().context("未找到已保存的成果")?.clone();
    ensure!(job.schema==1 && job.scope_id==request.scope_id && job.target==request.target
        && job.input_revision==request.input_revision && job.round_start<=job.attempts.len()
        && job.revalidation_floor<=job.attempts.len(),"生成记录与当前资料不一致");
    ensure!(previous.scope_id==request.scope_id && previous.target==request.target
        && previous.input_revision==request.input_revision,"已有成果范围已变化");
    if previous.render_version>=outcomes::RENDER_VERSION {return Ok(());}
    verify_bundle(root,&previous)?;
    confined_file(root,&request.scope_id,&previous.manifest_path)?;
    let current=outcomes::prepare(root,device,account,&request.target)?;
    ensure!(current.input_revision==request.input_revision,"资料已更新，未重排旧版本");
    let state_path=dir.join(format!("{key}.json"));
    ensure!(crate::data_store::exists(&state_path)? && !state_path.is_symlink(),"生成记录已清理或变化");
    job.bundle=None;
    let checked=revalidate_saved(root,device,account,&current,dir,key,job,Some(&previous));
    if let Err(error)=checked {job.bundle=Some(previous);return Err(error);}
    if job.bundle.is_none() {
        job.bundle=Some(previous);
        save(&state_path,job)?;
        bail!("已有原稿未通过当前内容校验，旧文件保留；未请求模型，也未重置生成次数");
    }
    let bundle=job.bundle.as_ref().unwrap();
    verify_bundle(root,bundle)?;
    ensure!(outcomes::prepare(root,device,account,&request.target)?.input_revision==request.input_revision,
        "重排期间资料变化，未将旧成果列为当前版本");
    // Receipts acknowledge bytes, not a document title or an old render version.
    let hashes:Vec<_>=bundle.files.iter().map(|file|file.sha256.as_str()).collect();
    job.deliveries.retain(|delivery|hashes.contains(&delivery.sha256.as_str()));
    if job.deliveries.is_empty() {job.last_delivery_error=None;job.last_delivery_attempt_ms=0;}
    ensure!(crate::data_store::exists(&state_path)? && !state_path.is_symlink(),"生成记录已清理或变化");
    save(&state_path,job)?;
    Ok(())
}

/// Model work is bounded to three durable attempts per target/revision, including
/// a request interrupted by a crash. Calling again reuses a validated bundle.
pub fn run(root: &Path, adb: &Adb, device: &Device, account: Option<&str>, target: &Target,
    cancel: &AtomicBool, deliver: bool) -> Result<RunResult> {
    run_inner(root, adb, device, account, target, cancel, deliver, false, None)
}

pub fn run_automatic(root: &Path, adb: &Adb, device: &Device, account: Option<&str>, target: &Target,
    cancel: &AtomicBool, deliver: bool) -> Result<RunResult> {
    run_inner(root, adb, device, account, target, cancel, deliver, true, None)
}

/// User-clicked material generation shares the exact source validation,
/// candidate ingestion, durable retry budget and optional delivery pipeline.
/// The material's real matter also supplies its other linked App discussions.
pub fn run_material(root:&Path,adb:&Adb,device:&Device,account:Option<&str>,material_id:&str,
    cancel:&AtomicBool,deliver:bool)->Result<RunResult> {
    check_cancel(cancel)?;check_account(adb,device,account)?;check_cancel(cancel)?;
    let item=context_ledger::request_material(root,&device.serial,account,material_id,now(),chrono::Local::now().offset().local_minus_utc())?;
    check_cancel(cancel)?;
    let target=Target::Item(item.id);
    regenerate(root,&device.serial,account,&target,"用户明确从资料收件箱重新生成方案与待办")?;
    run_inner(root,adb,device,account,&target,cancel,deliver,false,None)
}

/// Explicit retry only: starts a fresh bounded round while retaining every
/// previous intent, rejected response and error. Never call from a timer.
pub fn retry_failed(root: &Path, device: &str, account: Option<&str>, target: &Target, reason: &str) -> Result<bool> {
    ensure!(!reason.trim().is_empty(), "重新尝试需要明确发起原因");
    let request = outcomes::prepare(root, device, account, target)?;
    let dir = scoped_dir(root, &request.scope_id)?;
    let key = job_key(target, &request.input_revision);
    let state_path = dir.join(format!("{key}.json"));
    if !crate::data_store::exists(&state_path)? { return Ok(false); }
    let lock_path = dir.join(format!("{key}.lock"));
    ensure!(!lock_path.is_symlink() && !state_path.is_symlink() && state_valid(&state_path), "生成记录无效");
    let lock = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path)?;
    lock.try_lock().context("这份成果仍在生成，暂不能重新发起")?;
    let mut job: Job = serde_json::from_slice(&state_bytes(&state_path)?)?;
    ensure!(job.schema == 1 && job.scope_id == request.scope_id && job.input_revision == request.input_revision
        && job.target == *target && job.round_start <= job.attempts.len()
        && job.revalidation_floor <= job.attempts.len(), "生成记录已变化");
    let outdated = if let Some(bundle) = &job.bundle {
        ensure!(bundle.scope_id == request.scope_id && bundle.target == *target && bundle.input_revision == request.input_revision,
            "已缓存成果不属于当前设备、账号、事项或来源版本");
        bundle.render_version < outcomes::RENDER_VERSION
    } else { false };
    if (!outdated && job.bundle.is_some()) || (!outdated && job.attempts.len() == job.round_start) { return Ok(false); }
    if outdated {
        // Only this explicitly invoked operation can retire an obsolete
        // accepted bundle and open a new billable round. Its files, manifests,
        // raw replies and rejection history remain intact on disk.
        job.bundle = None;
        job.accepted_inference = None;
        job.deliveries.clear();
        job.last_delivery_error = None;
        job.last_delivery_attempt_ms = 0;
    }
    job.round_start = job.attempts.len();
    job.retries.push(Retry { started_ms: now(), attempt_start: job.round_start, reason: reason.chars().take(300).collect() });
    save(&state_path, &job)?; Ok(true)
}

/// Explicit user regeneration, including an already accepted current bundle.
/// This does not call a model or delete files. The next run starts a new bounded
/// round and cannot recover any raw reply preceding this user's request.
pub fn regenerate(root:&Path,device:&str,account:Option<&str>,target:&Target,reason:&str)->Result<bool> {
    ensure!(!reason.trim().is_empty(), "重新生成需要明确发起原因");
    let request=outcomes::prepare(root,device,account,target)?;
    let dir=scoped_dir(root,&request.scope_id)?;
    let key=job_key(target,&request.input_revision);
    let state_path=dir.join(format!("{key}.json"));
    if !crate::data_store::exists(&state_path)? {return Ok(false);}
    let lock_path=dir.join(format!("{key}.lock"));
    ensure!(!lock_path.is_symlink() && !state_path.is_symlink()
        && state_valid(&state_path),"生成记录无效");
    let lock=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path)?;
    lock.try_lock().context("这份成果仍在生成，暂不能重新发起")?;
    let mut job:Job=serde_json::from_slice(&state_bytes(&state_path)?)?;
    ensure!(job.schema==1 && job.scope_id==request.scope_id && job.target==*target
        && job.input_revision==request.input_revision && job.round_start<=job.attempts.len()
        && job.revalidation_floor<=job.attempts.len(),"生成记录已变化");
    if job.bundle.is_none() && job.attempts.len()==job.round_start
        && job.revalidation_floor==job.attempts.len() {return Ok(false);}
    let reason:String=reason.chars().take(300).collect();
    if let Some(bundle)=job.bundle.take() {
        ensure!(bundle.scope_id==request.scope_id && bundle.target==*target
            && bundle.input_revision==request.input_revision,"已缓存成果范围已变化");
        ensure!(job.retired_outcomes.len()<128,"保留的旧成果版本过多，请先另存整理，原件未删除");
        job.retired_outcomes.push(RetiredOutcome {retired_ms:now(),reason:reason.clone(),bundle,
            deliveries:std::mem::take(&mut job.deliveries)});
    }
    job.accepted_inference=None;
    job.deliveries.clear();job.last_delivery_error=None;job.last_delivery_attempt_ms=0;
    job.round_start=job.attempts.len();job.revalidation_floor=job.attempts.len();
    job.retries.push(Retry {started_ms:now(),attempt_start:job.round_start,reason});
    save(&state_path,&job)?;Ok(true)
}

/// Delivery is an exact revision operation. It can never spend model credits,
/// including when sources change while the command waits in the UI queue.
pub fn deliver_existing(root: &Path, adb: &Adb, device: &Device, account: Option<&str>, target: &Target,
    expected_revision: &str, cancel: &AtomicBool) -> Result<RunResult> {
    run_inner(root, adb, device, account, target, cancel, true, false, Some(expected_revision))
}

fn run_inner(root: &Path, adb: &Adb, device: &Device, account: Option<&str>, target: &Target,
    cancel: &AtomicBool, deliver: bool, automatic: bool, delivery_revision: Option<&str>) -> Result<RunResult> {
    check_cancel(cancel)?;
    let selected_source = crate::model_source::load();
    let request = outcomes::prepare(root, &device.serial, account, target)?;
    ensure!(delivery_revision.is_none_or(|revision| revision == request.input_revision), "资料已更新，未回传旧版本，也没有重新调用模型；请先生成新版成果");
    ensure!(!automatic || request.eligible_for_automation, "当前材料没有明确任务或安排，已跳过自动生成");
    let dir = scoped_dir(root, &request.scope_id)?;
    let key = job_key(target, &request.input_revision);
    let lock_path = dir.join(format!("{key}.lock"));
    ensure!(!lock_path.is_symlink(), "生成记录不能是链接");
    let lock = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&lock_path)?;
    lock.try_lock().context("同一份成果正在生成，请稍后查看")?;
    let state_path = dir.join(format!("{key}.json"));
    let mut job = if crate::data_store::exists(&state_path)? {
        ensure!(!state_path.is_symlink() && state_valid(&state_path), "生成记录无效");
        serde_json::from_slice::<Job>(&state_bytes(&state_path)?)?
    } else { Job { schema: 1, scope_id: request.scope_id.clone(), target: target.clone(),
        input_revision: request.input_revision.clone(), attempts: vec![], bundle: None, deliveries: vec![], last_delivery_error: None, last_delivery_attempt_ms: 0, round_start: 0, retries: vec![], revalidations: vec![], revalidation_floor: 0, retired_outcomes: vec![], accepted_inference: None } };
    ensure!(job.schema == 1 && job.scope_id == request.scope_id && job.input_revision == request.input_revision
        && job.round_start <= job.attempts.len()
        && job.revalidation_floor <= job.attempts.len()
        && job.target == *target, "生成记录与当前资料不一致");
    if let Some(bundle) = &job.bundle {
        ensure!(bundle.scope_id == request.scope_id && bundle.target == *target
            && bundle.input_revision == request.input_revision,
            "已缓存成果不属于当前设备、账号、事项或来源版本；已保留原文件并停止读取与回传");
    }
    ensure!(delivery_revision.is_none() || job.bundle.is_some(), "未找到已生成的这版成果，回传操作不会重新调用模型");
    // Both ordinary reads and exact-revision delivery can upgrade locally. This
    // must happen before delivery's current-layout guard and any phone access.
    if job.bundle.as_ref().is_some_and(|bundle|bundle.render_version<outcomes::RENDER_VERSION) {
        revalidate_cached_bundle(root,&device.serial,account,&request,&dir,&key,&mut job)?;
    }
    let mut reused = job.bundle.is_some();
    if job.bundle.is_none() {
        check_account(adb, device, account)?;
        check_cancel(cancel)?;
        outcomes::check_storage(root, &device.serial, account)?;
        check_capacity(&dir)?;
        revalidate_saved(root, &device.serial, account, &request, &dir, &key, &mut job, None)?;
        check_cancel(cancel)?;
        reused = job.bundle.is_some();
        save(&state_path, &job)?;
    }
    if job.bundle.is_none() {
        let source=selected_source?.source;
        ensure!(!automatic || job.attempts.last().is_none_or(|attempt|
            attempt.error.is_none() && attempt.raw_path.is_some() && attempt.model.is_some()),
            "上次模型请求已发出但结果未确认或失败；后台不会自动重发，请先核对后手动重试");
        if automatic { home_ai::ensure_automatic_text_source(adb, device, source)?; }
        ensure!(home_ai::text_source_available(source),
            "已选择独立 API，请先配置地址、模型和密钥；缺失配置不会请求模型，也未改用手机账号");
        ensure!(request.prompt.len() <= PROMPT_BUDGET, "来源材料超过单次生成容量，请缩小事项范围后重试；尚未调用模型");
        let mut correction = job.revalidations.iter().rev().find(|r| r.attempt > job.revalidation_floor && r.attempt == job.attempts.len()).and_then(|r| r.error.clone())
            .or_else(|| job.attempts.iter().skip(job.revalidation_floor).filter(|a| !a.empty_reply).last().and_then(|a| a.error.clone()));
        // Older clients recorded a local missing-config error before any HTTP
        // request. Retain that history without consuming a paid-request slot.
        let local_preflight_failures = job.attempts.iter().skip(job.round_start).filter(|a| a.raw_path.is_none() && a.model.is_none()
            && a.error.as_deref() == Some("请先在模型设置配置纯文本 API，再生成方案与待办")).count();
        while job.attempts.len().saturating_sub(job.round_start).saturating_sub(local_preflight_failures) < MAX_ATTEMPTS {
            check_cancel(cancel)?;
            let current = outcomes::prepare(root, &device.serial, account, target)?;
            ensure!(current.input_revision == request.input_revision, "资料已更新，已停止生成旧版本，请整理新资料后重试");
            check_account(adb, device, account)?;
            check_cancel(cancel)?;
            let mut prompt = request.prompt.clone();
            if let Some(error) = &correction {
                append_bounded(&mut prompt, "\n\n上一稿未通过保存前校验。请重新检查全部原始证据并生成完整 JSON，修正以下错误，不要删除必要结论来躲避校验：\n", 1024);
                append_bounded(&mut prompt, error, 32 * 1024);
                if let Some(path) = job.attempts.iter().skip(job.revalidation_floor).rev().find_map(|a| a.raw_path.as_ref()) {
                    if let Ok(raw) = fs::read_to_string(path) {
                        append_bounded(&mut prompt, "\n上一稿（仅用于纠错，不是事实来源）：\n", 1024);
                        append_bounded(&mut prompt, &raw, 48 * 1024);
                    }
                }
            }
            if automatic {
                ensure!(crate::model_source::load()?.source == source,
                    "模型来源已变化，停止原来源的后台请求；未切换到其他账号或 API");
                home_ai::ensure_automatic_text_source(adb, device, source)?;
                crate::automatic_generation::record_intent(root, &device.serial, account, source,
                    &target.key(), &request.input_revision, job.attempts.len() + 1,
                    now())?;
            }
            job.attempts.push(Attempt { automatic, source: automatic.then_some(source),
                inference: None, empty_reply: false, started_ms: now(), raw_path: None, model: None, error: None });
            save(&state_path, &job)?; // Durable intent before every billable request.
            let mut record = |inference: &home_ai::InferenceMeta| {
                ensure!(inference.account_scope.as_deref()==account,"手机 AI 用量不属于当前账号，停止保存");
                job.attempts.last_mut().unwrap().inference=Some(inference.clone());
                save(&state_path,&job)
            };
            let model = if automatic {
                home_ai::generate_selected_json_automatic_recorded_for_account(adb,device,source,account,&prompt,cancel,&mut record)
            } else {
                home_ai::generate_selected_json_recorded(adb,device,source,&prompt,cancel,&mut record)
            };
            match model {
                Ok(generated) => {
                    let brief=generated.brief;
                    let raw = dir.join(format!("{key}-attempt-{}.json.txt", job.attempts.len()));
                    write(&raw, brief.markdown.as_bytes())?;
                    let attempt = job.attempts.last_mut().unwrap();
                    attempt.raw_path = Some(raw); attempt.model = Some(brief.model); attempt.inference=generated.inference;
                    save(&state_path, &job)?;
                    check_cancel(cancel)?;
                    check_account(adb, device, account)?;
                    check_cancel(cancel)?;
                    ensure!(!automatic || crate::model_source::load()?.source == source,
                        "模型来源已变化，未采用原来源的后台结果；原始回复与实际用量已保留");
                    match outcomes::save(root, &device.serial, account, &request, &brief.markdown) {
                        Ok(bundle) => { check_cancel(cancel)?; job.bundle = Some(bundle); job.accepted_inference=job.attempts.last().and_then(|attempt|attempt.inference.clone()); save(&state_path, &job)?; break; }
                        Err(error) => {
                            let repairable = error.downcast_ref::<outcomes::ModelContentError>().is_some();
                            correction = Some(format!("{error:#}"));
                            job.attempts.last_mut().unwrap().error = correction.clone(); save(&state_path, &job)?;
                            if !repairable { return Err(error.context("保存成果失败，原始模型返回已保留；未重复调用模型")); }
                        }
                    }
                }
                Err(error) => {
                    let is_empty = error.downcast_ref::<home_ai::EmptyModelReply>().is_some();
                    let error = format!("{error:#}");
                    let attempt = job.attempts.last_mut().unwrap();
                    attempt.error = Some(error.clone()); attempt.empty_reply = is_empty;
                    save(&state_path, &job)?;
                    if is_empty && retry_empty_reply(&job, local_preflight_failures) {
                        // One durable retry at most, inside the original request
                        // budget. Keep the semantic correction and source draft.
                        empty_reply_backoff(cancel)?;
                        continue;
                    }
                    // All other failures require an explicit user retry.
                    bail!("模型尚未生成成果：{error}");
                }
            }
        }
    }
    let bundle = job.bundle.clone().with_context(|| format!("成果未通过来源校验（本轮最多 {MAX_ATTEMPTS} 次生成请求）；原始返回已保留。{}",
        job.revalidations.iter().rev().find(|r| r.attempt == job.attempts.len()).and_then(|r| r.error.as_deref())
            .or_else(|| job.attempts.last().and_then(|a| a.error.as_deref())).unwrap_or("请检查材料后重试")))?;
    verify_bundle(root, &bundle)?;
    check_cancel(cancel)?;
    if deliver {
        job.last_delivery_attempt_ms = now();
        save(&state_path, &job)?;
        let result = (|| -> Result<()> {
            let desktop = companion_link::desktop_id()?;
            for file in &bundle.files {
                if file.path.extension().and_then(|s| s.to_str()) != Some("md") { continue; }
                check_cancel(cancel)?;
                check_account(adb, device, account)?;
                check_cancel(cancel)?;
                let current = outcomes::prepare(root, &device.serial, account, target)?;
                ensure!(current.input_revision == bundle.input_revision, "资料已更新，停止回传旧版成果");
                let path = confined_file(root, &bundle.scope_id, &file.path)?;
                let name = path.file_name().and_then(|s| s.to_str()).context("成果文件名无效")?;
                let id = transfer_id(&bundle.scope_id, &desktop, &file.sha256, name);
                check_cancel(cancel)?;
                companion_link::send_staged(adb, device, &path, &id, &file.sha256, fs::metadata(&path)?.len())?;
                check_cancel(cancel)?;
                check_account(adb, device, account)?;
                let remote = format!("/sdcard/Android/data/com.stepx.companion.app/files/transfers/inbox/{id}/{name}");
                job.deliveries.retain(|d| d.transfer_id != id);
                job.deliveries.push(Delivery { kind: file.kind.clone(), transfer_id: id, sha256: file.sha256.clone(), path: remote, verified_ms: now() });
                save(&state_path, &job)?;
            }
            Ok(())
        })();
        job.last_delivery_error = result.err().map(|error| format!("{error:#}"));
        save(&state_path, &job)?;
    }
    Ok(RunResult { bundle, deliveries: job.deliveries, reused, delivery_error: job.last_delivery_error, delivery_attempt_ms: job.last_delivery_attempt_ms })
}

/// Loads only current source revisions. Runs in a background worker, never paint.
pub fn list_current(root: &Path, device: &str, account: Option<&str>) -> Result<Vec<RunResult>> {
    let scope = context_ledger::scope_id(device, account);
    let dir = root.join(&scope).join("pipeline");
    if !dir.exists() { return Ok(vec![]); }
    ensure!(!dir.is_symlink() && fs::canonicalize(&dir)?.starts_with(fs::canonicalize(root)?), "成果目录已变化");
    let overview = context_ledger::load_overview(root, device, account, false)?;
    let current: Vec<_> = overview.items.iter().map(|item| (Target::Item(item.id.clone()), item.input_revision.clone()))
        .chain(overview.periods.iter().map(|period| (Target::Period(period.kind), period.evidence_revision.clone()))).collect();
    let mut result = vec![];
    for (target, revision) in current {
        let path = dir.join(format!("{}.json", job_key(&target, &revision)));
        if !crate::data_store::exists(&path)? || path.is_symlink() { continue; }
        if !state_valid(&path) { continue; }
        let Ok(mut job) = serde_json::from_slice::<Job>(&state_bytes(&path)?) else { continue; };
        if job.schema!=1 || job.scope_id != scope || job.target != target || job.input_revision != revision
            || job.round_start>job.attempts.len() || job.revalidation_floor>job.attempts.len() { continue; }
        let Some(bundle) = &job.bundle else { continue; };
        if bundle.target != target || revision != bundle.input_revision || bundle.scope_id != scope { continue; }
        if verify_bundle(root, bundle).is_err() { continue; }
        if bundle.render_version<outcomes::RENDER_VERSION {
            let key=job_key(&target,&revision);
            let lock_path=dir.join(format!("{key}.lock"));
            if lock_path.is_symlink() {continue;}
            let Ok(lock)=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&lock_path) else {continue;};
            if lock.try_lock().is_err() {continue;}
            // The first read may race with a writer. Re-read only after owning
            // its lock, and do not recreate a cleared pipeline or scope.
            if !crate::data_store::exists(&path)? || !state_valid(&path) {continue;}
            let Ok(locked_job)=serde_json::from_slice::<Job>(&state_bytes(&path)?) else {continue;};
            job=locked_job;
            let Ok(request)=outcomes::prepare(root,device,account,&target) else {continue;};
            if request.input_revision!=revision {continue;}
            if revalidate_cached_bundle(root,device,account,&request,&dir,&key,&mut job).is_err() {continue;}
        }
        let Some(bundle)=job.bundle else {continue;};
        if bundle.render_version<outcomes::RENDER_VERSION || verify_bundle(root,&bundle).is_err() {continue;}
        result.push(RunResult { bundle, deliveries: job.deliveries, reused: true, delivery_error: job.last_delivery_error, delivery_attempt_ms: job.last_delivery_attempt_ms });
    }
    result.sort_by_key(|r| std::cmp::Reverse(r.bundle.created_ms));
    result.truncate(32);
    Ok(result)
}

/// Read-only history for the UI. Older drafts remain accessible when live
/// collection advances; this never promotes them to current or imports actions.
pub fn list_for_reading(root: &Path, device: &str, account: Option<&str>) -> Result<Vec<RunResult>> {
    let mut result = list_current(root, device, account)?;
    let scope = context_ledger::scope_id(device, account);
    let dir = root.join(&scope).join("pipeline");
    if !dir.exists() { return Ok(result); }
    let overview = context_ledger::load_overview(root, device, account, false)?;
    let mut historical = vec![];
    // The bounded history window is selected by SQLite, without loading or
    // querying timestamps one at a time for every historical pipeline snapshot.
    for path in crate::data_store::snapshot_paths_page(&dir, 0, 128)? {
        let Ok(bytes) = state_bytes(&path) else { continue; };
        if bytes.len() > 2 * 1024 * 1024 { continue; }
        let Ok(job) = serde_json::from_slice::<Job>(&bytes) else { continue; };
        if job.schema != 1 || job.scope_id != scope || job.round_start > job.attempts.len()
            || job.revalidation_floor > job.attempts.len()
            || path != dir.join(format!("{}.json", job_key(&job.target, &job.input_revision))) { continue; }
        let Some(bundle) = job.bundle else { continue; };
        if bundle.scope_id != scope || bundle.target != job.target || bundle.input_revision != job.input_revision
            || bundle.render_version < outcomes::RENDER_VERSION || verify_bundle(root, &bundle).is_err() { continue; }
        let relevant = match &bundle.target {
            Target::Item(id) => overview.items.iter().any(|item| &item.id == id),
            Target::Period(kind) => overview.periods.iter().any(|period| &period.kind == kind
                && bundle.created_ms >= period.begin_ms && bundle.created_ms < period.end_ms),
        };
        if relevant { historical.push(RunResult { bundle, deliveries: job.deliveries, reused: true,
            delivery_error: job.last_delivery_error, delivery_attempt_ms: job.last_delivery_attempt_ms }); }
    }
    historical.sort_by_key(|r| std::cmp::Reverse(r.bundle.created_ms));
    for previous in historical {
        if result.len() >= 32 { break; }
        if !result.iter().any(|r| r.bundle.target == previous.bundle.target) { result.push(previous); }
    }
    Ok(result)
}

/// Import candidates from current, byte-verified saved drafts after an upgrade.
/// Read the accepted inference receipt without starting a model, creating a
/// scope directory, or exposing credentials. Legacy outcomes have no receipt.
pub fn load_inference_meta(root:&Path, device:&str, account:Option<&str>, target:&Target, revision:&str) -> Result<Option<home_ai::InferenceMeta>> {
    let scope=context_ledger::scope_id(device,account);
    let path=root.join(&scope).join("pipeline").join(format!("{}.json",job_key(target,revision)));
    if !crate::data_store::exists(&path)? {return Ok(None);}
    ensure!(!path.is_symlink() && fs::canonicalize(path.parent().context("记录目录无效")?)?.starts_with(fs::canonicalize(root.join(&scope))?),"生成记录目录越界");
    ensure!(state_valid(&path),"生成记录过大，未读取用量");
    let job:Job=serde_json::from_slice(&state_bytes(&path)?)?;
    ensure!(job.schema==1 && job.scope_id==scope && job.target==*target && job.input_revision==revision,"用量记录不属于当前成果");
    let Some(bundle)=job.bundle else {return Ok(None);};
    ensure!(bundle.scope_id==scope && bundle.target==*target && bundle.input_revision==revision,"用量成果范围已变化");
    verify_bundle(root,&bundle)?;
    if let Some(meta)=job.accepted_inference.as_ref() {
        ensure!(meta.source!=crate::model_source::Source::PhoneAccount || meta.account_scope.as_deref()==account,"手机用量账号已变化");
    }
    Ok(job.accepted_inference)
}

/// This performs local validation only; it never invokes a model or a phone.
pub fn sync_action_candidates(root:&Path,device:&str,account:Option<&str>)->Result<crate::action_ledger::Snapshot> {
    for result in list_current(root,device,account)? {
        let bundle=&result.bundle;
        let dir=scoped_dir(root,&bundle.scope_id)?;
        let key=job_key(&bundle.target,&bundle.input_revision);
        let state_path=dir.join(format!("{key}.json"));
        ensure!(!state_path.is_symlink() && state_valid(&state_path),"成果生成记录无效");
        let job:Job=serde_json::from_slice(&state_bytes(&state_path)?)?;
        ensure!(job.scope_id==bundle.scope_id && job.target==bundle.target && job.input_revision==bundle.input_revision,
            "候选行动的成果范围已变化");
        let canonical_scope=fs::canonicalize(root.join(&bundle.scope_id))?;
        for (index,attempt) in job.attempts.iter().enumerate().rev() {
            let Some(path)=&attempt.raw_path else {continue;};
            ensure!(path==&dir.join(format!("{key}-attempt-{}.json.txt",index+1)),"候选行动原稿路径无效");
            let path=confined_file_in_scope(&canonical_scope,path)?;
            ensure!(fs::metadata(&path)?.len()<=512*1024,"候选行动原稿超过容量");
            if outcomes::sync_saved_actions(root,device,account,bundle,&fs::read_to_string(path)?)? {break;}
        }
    }
    crate::action_ledger::load(root,device,account)
}

/// Inspect durable jobs when a scope is loaded after startup or a scope change.
/// This only releases stale UI scheduling intents; it never resets a request
/// budget, retries an explicit failure, or treats an edited bundle as absent.
pub fn recoverable_outcomes(root: &Path, device: &str, account: Option<&str>) -> Result<Vec<(Target, String)>> {
    let overview = context_ledger::load_overview(root, device, account, false)?;
    let current: Vec<_> = overview.items.iter().map(|item| (Target::Item(item.id.clone()), item.input_revision.clone()))
        .chain(overview.periods.iter().map(|period| (Target::Period(period.kind), period.evidence_revision.clone()))).collect();
    if current.is_empty() { return Ok(vec![]); }
    let scope = context_ledger::scope_id(device, account);
    let scope_dir = root.join(&scope);
    ensure!(!root.is_symlink() && !scope_dir.is_symlink()
        && fs::canonicalize(&scope_dir)?.starts_with(fs::canonicalize(root)?), "恢复记录不属于当前设备与账号");
    let dir = scope_dir.join("pipeline");
    match fs::symlink_metadata(&dir) {
        Ok(meta) => ensure!(meta.is_dir() && !meta.file_type().is_symlink(), "恢复记录目录无效"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(current),
        Err(error) => return Err(error.into()),
    }
    let mut recoverable = vec![];
    for (target, revision) in current {
        let path = dir.join(format!("{}.json", job_key(&target, &revision)));
        if !crate::data_store::exists(&path)? {recoverable.push((target, revision)); continue;}
        if !state_valid(&path) {continue;}
        let Ok(job) = serde_json::from_slice::<Job>(&state_bytes(&path)?) else { continue; };
        let saved_reply = job.attempts.last().is_some_and(|attempt| {
            let expected = dir.join(format!("{}-attempt-{}.json.txt", job_key(&target, &revision), job.attempts.len()));
            attempt.error.is_none() && attempt.model.is_some() && attempt.raw_path.as_ref() == Some(&expected)
                && fs::symlink_metadata(&expected).is_ok_and(|m| m.is_file()
                    && !m.file_type().is_symlink() && m.len() <= 512 * 1024)
        });
        if job.schema == 1 && job.scope_id == scope && job.target == target && job.input_revision == revision
            && job.round_start <= job.attempts.len() && job.bundle.is_none()
            && (job.attempts.is_empty() || saved_reply) {
            recoverable.push((target, revision));
        }
    }
    Ok(recoverable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_native_usage_is_scope_bound_and_read_without_rewriting_history() {
        let f=accepted_legacy_fixture();
        let mut job:Job=serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        let meta=home_ai::InferenceMeta {source:crate::model_source::Source::PhoneAccount,model:"actual-reply-model".into(),
            provider:"native".into(),usage:None,account_scope:Some("account".into()),observed_ms:100,session_cleaned:true,temporary_session_key:None,
            channel:None,isolation:None,session_cleanup:None};
        job.accepted_inference=Some(meta.clone());save(&f.state_path,&job).unwrap();
        let before=state_bytes(&f.state_path).unwrap();
        assert_eq!(load_inference_meta(f.root.path(),"layout-phone",Some("account"),&f.request.target,&f.request.input_revision).unwrap(),Some(meta));
        assert_eq!(state_bytes(&f.state_path).unwrap(),before);
        assert!(load_inference_meta(f.root.path(),"layout-phone",Some("other"),&f.request.target,&f.request.input_revision).unwrap().is_none());
        job.accepted_inference.as_mut().unwrap().account_scope=Some("other".into());save(&f.state_path,&job).unwrap();
        assert!(load_inference_meta(f.root.path(),"layout-phone",Some("account"),&f.request.target,&f.request.input_revision).is_err());
    }

    #[test]
    fn oversized_job_save_never_replaces_a_readable_generation_state() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join("job.json");
        let mut job:Job=serde_json::from_value(serde_json::json!({"schema":1,"scope_id":"scope","target":Target::Item("item".into()),"input_revision":"revision","attempts":[],"bundle":null,"deliveries":[]})).unwrap();
        save(&path,&job).unwrap();
        let before=state_bytes(&path).unwrap();
        let revision=crate::data_store::read_head(&path).unwrap().unwrap().revision;
        job.last_delivery_error=Some("🙂".repeat(2*1024*1024/4));
        assert!(save(&path,&job).is_err());
        assert!(state_valid(&path));
        assert_eq!(state_bytes(&path).unwrap(),before);
        assert_eq!(crate::data_store::read_head(&path).unwrap().unwrap().revision,revision);
        assert_eq!(job.last_delivery_error.as_ref().unwrap().len(),2*1024*1024);
    }
    #[test]
    fn empty_stream_retry_is_durable_bounded_and_never_applies_to_other_errors() {
        use serde_json::json;
        let mut job: Job = serde_json::from_value(json!({"schema":1,"scope_id":"scope","target":Target::Item("item".into()),
            "input_revision":"revision","attempts":[],"bundle":null,"deliveries":[]})).unwrap();
        let empty = Attempt { automatic: false, source: None, inference: None, empty_reply: true, started_ms: 100, raw_path: None, model: None, error: Some("empty".into()) };
        assert!(!retry_empty_reply(&job, 0));
        job.attempts.push(empty.clone());
        assert!(retry_empty_reply(&job, 0));
        // A restart reads the classification from durable history, rather than
        // resetting a local retry counter or comparing a human error string.
        let mut loaded: Job = serde_json::from_slice(&serde_json::to_vec(&job).unwrap()).unwrap();
        loaded.attempts.push(empty.clone());
        assert!(!retry_empty_reply(&loaded, 0), "a second empty stream never retries");
        job.attempts[0].empty_reply = false;
        assert!(!retry_empty_reply(&job, 0), "network/JSON/tool failures cannot retry by their message");
        job.attempts.push(Attempt {automatic: false, source: None, inference:None,empty_reply:false,started_ms:200,raw_path:None,model:None,error:Some("semantic repair".into())});
        job.attempts.push(empty);
        assert!(!retry_empty_reply(&job, 0), "an empty reply cannot exceed MAX_ATTEMPTS");
        let at = std::time::Instant::now();
        assert!(empty_reply_backoff(&AtomicBool::new(true)).is_err());
        assert!(at.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn explicit_regeneration_retires_current_bundle_and_cannot_recover_previous_raw() {
        use serde_json::json;
        let root=tempfile::tempdir().unwrap();
        let text="请整理会议资料并列出可供讨论的方案。";
        let capture=context_ledger::Capture {device:"regen-phone".into(),account:Some("account".into()),now_ms:1_793_318_400_000,utc_offset_seconds:8*3600,
            phone_discussions:vec![crate::phone_context::PhoneMessage {device:"regen-phone".into(),account:"account".into(),session_key:"session".into(),message_id:"message".into(),role:"user".into(),title:"会议资料".into(),text:text.into(),timestamp_ms:Some(1_793_318_399_000),..Default::default()}],..Default::default()};
        let overview=context_ledger::process(root.path(),&capture).unwrap();
        let target=Target::Item(overview.items[0].id.clone());
        let request=outcomes::prepare(root.path(),"regen-phone",Some("account"),&target).unwrap();
        let reference=json!([{"id":request.sources[0].id,"quote":text,"version":"current"}]);
        let raw=json!({"schema":1,"conclusion":{"text":"用户请求整理会议资料并列出讨论方案","evidence":reference},
            "key_facts":[{"text":"用户请求整理会议资料","evidence":reference}],"plan":[],"todos":[],"unknowns":[],"reuse":[],"next_period":[]}).to_string();
        let bundle=outcomes::save(root.path(),"regen-phone",Some("account"),&request,&raw).unwrap();
        let old_files:Vec<_>=bundle.files.iter().map(|file|(file.path.clone(),fs::read(&file.path).unwrap())).collect();
        let old_manifest=(bundle.manifest_path.clone(),fs::read(&bundle.manifest_path).unwrap());
        let dir=scoped_dir(root.path(),&request.scope_id).unwrap();let key=job_key(&target,&request.input_revision);
        let raw_path=dir.join(format!("{key}-attempt-1.json.txt"));fs::write(&raw_path,&raw).unwrap();
        let path=dir.join(format!("{key}.json"));
        let job:Job=serde_json::from_value(json!({"schema":1,"scope_id":request.scope_id,"target":target,"input_revision":request.input_revision,
            "attempts":[{"started_ms":100,"raw_path":raw_path,"model":"synthetic","error":null}],"bundle":bundle,
            "deliveries":[{"kind":"plan","transfer_id":"old","sha256":"old-hash","path":"old-file","verified_ms":100}]})).unwrap();
        save(&path,&job).unwrap();
        assert!(!retry_failed(root.path(),"regen-phone",Some("account"),&target,"重试保持当前缓存").unwrap());
        assert!(regenerate(root.path(),"regen-phone",Some("account"),&target,"用户明确要求改写当前稿").unwrap());
        let mut job:Job=serde_json::from_slice(&state_bytes(&path).unwrap()).unwrap();
        assert!(job.bundle.is_none()&&job.deliveries.is_empty());assert_eq!(job.revalidation_floor,1);assert_eq!(job.round_start,1);
        assert_eq!(job.retired_outcomes.len(),1);assert_eq!(job.retired_outcomes[0].deliveries.len(),1);
        assert!(!regenerate(root.path(),"regen-phone",Some("account"),&target,"重复点击").unwrap());
        revalidate_saved(root.path(),"regen-phone",Some("account"),&request,&dir,&key,&mut job, None).unwrap();
        assert!(job.bundle.is_none()&&job.revalidations.is_empty(),"retired original must not satisfy the new content request");
        assert_eq!(fs::read_to_string(&raw_path).unwrap(),raw);
        assert_eq!(fs::read(old_manifest.0).unwrap(),old_manifest.1);
        for (path,bytes) in old_files {assert_eq!(fs::read(path).unwrap(),bytes);}
    }
    #[test]
    fn recovery_releases_unstarted_and_saved_replies_but_never_replays_unknown_paid_requests() {
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let capture = context_ledger::Capture { device: "recovery-phone".into(), account: Some("account".into()),
            now_ms: 1_793_318_400_000, utc_offset_seconds: 8 * 3600,
            phone_discussions: vec![crate::phone_context::PhoneMessage { device: "recovery-phone".into(), account: "account".into(),
                session_key: "session".into(), message_id: "message".into(), role: "user".into(), title: "图书交流会".into(),
                text: "项目：图书交流会。场地报价仍待确认，需要整理方案与待办。".into(), timestamp_ms: Some(1_793_318_399_000), ..Default::default() }], ..Default::default() };
        let view = context_ledger::process(root.path(), &capture).unwrap();
        let target = Target::Item(view.items[0].id.clone());let revision = view.items[0].input_revision.clone();
        let scope = context_ledger::scope_id("recovery-phone", Some("account"));
        let recover = || recoverable_outcomes(root.path(), "recovery-phone", Some("account")).unwrap().contains(&(target.clone(), revision.clone()));
        assert!(recover(), "A persisted UI intent with no pipeline directory must resume");
        let dir = scoped_dir(root.path(), &scope).unwrap();let path = dir.join(format!("{}.json", job_key(&target, &revision)));
        assert!(recover(), "An absent job must resume even when other pipeline records exist");
        let mut job: Job = serde_json::from_value(json!({"schema":1,"scope_id":scope,"target":target,
            "input_revision":revision,"attempts":[],"bundle":null,"deliveries":[]})).unwrap();
        save(&path,&job).unwrap();assert!(recover(), "Crash after empty job save and before request intent must resume");
        job.attempts.push(Attempt {automatic: false, source: None, inference:None,empty_reply: false, started_ms:100,raw_path:None,model:None,error:None});save(&path,&job).unwrap();
        let before = state_bytes(&path).unwrap();assert!(!recover(),"A paid intent without a completed reply requires explicit review");assert_eq!(state_bytes(&path).unwrap(),before);
        let loaded:Job=serde_json::from_slice(&before).unwrap();assert_eq!(loaded.attempts.len(),1);assert_eq!(loaded.round_start,0);
        job.attempts[0].error=Some("模型返回未通过校验".into());save(&path,&job).unwrap();assert!(!recover(),"Explicit failure is not a crash");
        job.attempts=vec![job.attempts[0].clone();MAX_ATTEMPTS];save(&path,&job).unwrap();
        let before=state_bytes(&path).unwrap();for _ in 0..3 {assert!(!recover());}assert_eq!(state_bytes(&path).unwrap(),before);
        job.attempts.last_mut().unwrap().error=None;save(&path,&job).unwrap();
        assert!(!recover(),"Unknown interrupted requests must not be replayed after restart");
        let raw = dir.join(format!("{}-attempt-{}.json.txt",job_key(&target,&revision),MAX_ATTEMPTS));
        fs::write(&raw,r#"{"schema":1}"#).unwrap();
        job.attempts.last_mut().unwrap().raw_path=Some(raw);
        job.attempts.last_mut().unwrap().model=Some("fixture-no-network".into());save(&path,&job).unwrap();
        assert!(recover(),"A saved terminal reply may be revalidated locally, within the old request budget");
        let loaded:Job=serde_json::from_slice(&state_bytes(&path).unwrap()).unwrap();assert_eq!(loaded.attempts.len(),MAX_ATTEMPTS);assert_eq!(loaded.round_start,0);
        // A bundle remains completed even if its file is edited or missing.
        // Recovery must not infer absence from list_current's display filtering.
        let document=dir.join("edited.md");fs::write(&document,"original").unwrap();
        let original_hash=crate::device::sha256_file(&document).unwrap();fs::write(&document,"user edit preserved").unwrap();
        job.bundle=Some(OutcomeBundle {render_version:outcomes::RENDER_VERSION,scope_id:scope.clone(),target:target.clone(),input_revision:revision.clone(),
            title:"活动".into(),conclusion:"待核对".into(),files:vec![outcomes::OutcomeFile {kind:"plan".into(),path:document.clone(),sha256:original_hash}],
            manifest_path:dir.join("manifest.json"),todo_count:1,created_ms:100});
        save(&path,&job).unwrap();assert!(!recover());assert!(list_current(root.path(),"recovery-phone",Some("account")).unwrap().is_empty());
        assert_eq!(fs::read_to_string(&document).unwrap(),"user edit preserved");fs::remove_file(&document).unwrap();assert!(!recover());
        job.bundle=None;job.scope_id="another-account".into();save(&path,&job).unwrap();assert!(!recover());
        job.scope_id=scope;job.input_revision="old-revision".into();save(&path,&job).unwrap();assert!(!recover());
        fs::write(&path,"corrupt job preserved").unwrap();assert!(!recover());assert_eq!(fs::read_to_string(&path).unwrap(),"corrupt job preserved");
    }
    #[test]
    fn rejected_legacy_bundle_can_explicitly_retry_without_losing_old_files_or_history() {
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let text = "项目：代码审阅。请整理当前代码问题与改进方案。";
        let capture = context_ledger::Capture { device: "legacy-phone".into(), account: Some("account".into()),
            now_ms: 1_793_318_400_000, phone_discussions: vec![crate::phone_context::PhoneMessage {
                device: "legacy-phone".into(), account: "account".into(), session_key: "review".into(), message_id: "message".into(),
                role: "user".into(), title: "代码审阅".into(), text: text.into(), timestamp_ms: Some(1_793_318_399_000),
                ..Default::default() }], ..Default::default() };
        let overview = context_ledger::process(root.path(), &capture).unwrap();
        let target = Target::Item(overview.items[0].id.clone());
        let request = outcomes::prepare(root.path(), "legacy-phone", Some("account"), &target).unwrap();
        let reference = json!([{"id":request.sources[0].id,"quote":text,"version":"current"}]);
        let good = json!({"schema":1,"conclusion":{"text":"当前明确请求是审阅代码并整理改进方案","evidence":reference},
            "key_facts":[{"text":"用户请求整理代码问题与改进方案","evidence":reference}],
            "plan":[],"todos":[],"unknowns":[],"reuse":[],"next_period":[]}).to_string();
        let mut legacy = outcomes::save(root.path(), "legacy-phone", Some("account"), &request, &good).unwrap();
        legacy.render_version = outcomes::RENDER_VERSION - 1;
        let old_files: Vec<_> = legacy.files.iter().map(|file| (file.path.clone(), fs::read(&file.path).unwrap())).collect();
        let mut bad: serde_json::Value = serde_json::from_str(&good).unwrap();
        bad["conclusion"]["text"] = json!("用户本期未向任何会话发送新消息。");
        let bad = bad.to_string();
        let dir = scoped_dir(root.path(), &request.scope_id).unwrap();
        let key = job_key(&target, &request.input_revision);
        let raw_path = dir.join(format!("{key}-attempt-1.json.txt"));
        fs::write(&raw_path, &bad).unwrap();
        let state_path = dir.join(format!("{key}.json"));
        let mut job: Job = serde_json::from_value(json!({"schema":1,"scope_id":request.scope_id,"target":target,
            "input_revision":request.input_revision,"attempts":[{"started_ms":100,"raw_path":raw_path,"model":"synthetic","error":null}],
            "bundle":legacy,"deliveries":[{"kind":"plan","transfer_id":"old","sha256":"old-hash","path":"old-path","verified_ms":100}]})).unwrap();
        // Follow the production upgrade's rejection/restore path without ADB,
        // any model request, or editing an original response to make it pass.
        let previous = job.bundle.take().unwrap();
        revalidate_saved(root.path(), "legacy-phone", Some("account"), &request, &dir, &key, &mut job, None).unwrap();
        assert!(job.bundle.is_none());
        assert!(job.revalidations.iter().any(|check| !check.accepted && check.error.as_deref().is_some_and(|error| error.contains("有限采集"))));
        job.bundle = Some(previous);
        save(&state_path, &job).unwrap();
        let old_attempts = serde_json::to_value(&job.attempts).unwrap();
        let old_checks = serde_json::to_value(&job.revalidations).unwrap();
        assert!(retry_failed(root.path(), "legacy-phone", Some("account"), &target, "用户明确要求修正旧稿").unwrap());
        job = serde_json::from_slice(&state_bytes(&state_path).unwrap()).unwrap();
        assert!(job.bundle.is_none() && job.deliveries.is_empty());
        assert_eq!(job.round_start, 1);
        assert_eq!(job.retries.len(), 1);
        assert_eq!(serde_json::to_value(&job.attempts).unwrap(), old_attempts);
        assert_eq!(serde_json::to_value(&job.revalidations).unwrap(), old_checks);
        assert_eq!(fs::read_to_string(&raw_path).unwrap(), bad);
        for (path, bytes) in old_files { assert_eq!(fs::read(path).unwrap(), bytes); }
        assert!(!retry_failed(root.path(), "legacy-phone", Some("account"), &target, "重复点击").unwrap());
        let corrected_path = dir.join(format!("{key}-attempt-2.json.txt"));
        fs::write(&corrected_path, &good).unwrap();
        job.attempts.push(Attempt { automatic: false, source: None, inference: None, empty_reply: false, started_ms: 200, raw_path: Some(corrected_path), model: Some("synthetic".into()), error: None });
        revalidate_saved(root.path(), "legacy-phone", Some("account"), &request, &dir, &key, &mut job, None).unwrap();
        assert_eq!(job.bundle.as_ref().unwrap().render_version, outcomes::RENDER_VERSION);
        assert_eq!(job.attempts.len(), 2);
        assert!(job.revalidations.iter().any(|check| !check.accepted));
    }
    #[test]
    fn revalidation_recovers_original_without_changing_history_and_rejects_stale_sources() {
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let text = "项目：图书交流会。场地报价仍待确认，需要整理方案与待办。";
        let mut capture = context_ledger::Capture { device: "test-phone".into(), account: Some("account".into()),
            now_ms: 1_793_318_400_000, utc_offset_seconds: 8 * 3600, connected: true,
            phone_discussions: vec![crate::phone_context::PhoneMessage { device: "test-phone".into(), account: "account".into(),
                session_key: "session".into(), message_id: "message".into(), role: "user".into(), title: "图书交流会".into(),
                text: text.into(), timestamp_ms: Some(1_793_318_399_000), ..Default::default() }], ..Default::default() };
        let view = context_ledger::process(root.path(), &capture).unwrap();
        let request = outcomes::prepare(root.path(), "test-phone", Some("account"), &Target::Item(view.items[0].id.clone())).unwrap();
        let reference = json!([{"id":request.sources[0].id,"quote":text,"version":"current"}]);
        let row = |text: &str| json!({"text":text,"evidence":reference});
        let raw = json!({"schema":1,"conclusion":row("需要先核对报价再确定方案"),"key_facts":[row("场地报价仍待确认")],
            "plan":[row("先联系场地取得报价"),row("根据报价整理执行方案")],
            "todos":[{"action":"询问场地报价并记录结果","kind":"suggested","owner":null,"due":null,"evidence":reference}],
            "unknowns":[row("场地报价仍待确认")],"reuse":[],"next_period":[]}).to_string();
        let dir = scoped_dir(root.path(), &request.scope_id).unwrap();
        let key = job_key(&request.target, &request.input_revision);
        let path = dir.join(format!("{key}-attempt-1.json.txt"));
        fs::write(&path, &raw).unwrap();
        let mut job: Job = serde_json::from_value(json!({"schema":1,"scope_id":request.scope_id,"target":request.target,
            "input_revision":request.input_revision,"attempts":[{"started_ms":100,"raw_path":path,
                "model":"synthetic-no-network","error":"original rejection retained"}],"bundle":null,"deliveries":[]})).unwrap();
        revalidate_saved(root.path(), "test-phone", Some("account"), &request, &dir, &key, &mut job, None).unwrap();
        assert!(job.bundle.is_some());
        assert_eq!(job.attempts.len(), 1);
        assert_eq!(job.attempts[0].error.as_deref(), Some("original rejection retained"));
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
        assert!(job.revalidations[0].accepted);
        assert_eq!(job.revalidations[0].raw_sha256, crate::device::sha256_file(&path).unwrap());
        revalidate_saved(root.path(), "test-phone", Some("account"), &request, &dir, &key, &mut job, None).unwrap();
        assert_eq!(job.revalidations.len(), 1);
        job.bundle = None;
        let rejected = dir.join(format!("{key}-attempt-2.json.txt"));
        fs::write(&rejected, "invalid model response preserved").unwrap();
        job.attempts.push(Attempt { automatic: false, source: None, inference: None, empty_reply: false, started_ms: 200, raw_path: Some(rejected.clone()), model: Some("synthetic-no-network".into()), error: Some("invalid JSON".into()) });
        revalidate_saved(root.path(), "test-phone", Some("account"), &request, &dir, &key, &mut job, None).unwrap();
        assert!(job.bundle.is_some(), "A whole earlier valid draft can recover after a later correction introduces a new error");
        assert_eq!(job.attempts.len(), 2);
        assert_eq!(fs::read_to_string(&rejected).unwrap(), "invalid model response preserved");
        assert!(job.revalidations.iter().any(|r| r.attempt == 2 && !r.accepted));
        job.bundle = None;
        capture.phone_discussions[0].text = "项目：图书交流会。活动取消，暂不询价。".into();
        context_ledger::process(root.path(), &capture).unwrap();
        assert!(revalidate_saved(root.path(), "test-phone", Some("account"), &request, &dir, &key, &mut job, None).is_err());
        assert!(job.bundle.is_none());
        job.attempts[0].raw_path = Some(root.path().join("outside.json"));
        assert!(revalidate_saved(root.path(), "test-phone", Some("account"), &request, &dir, &key, &mut job, None).is_err());
    }
    struct LegacyFixture {
        root:tempfile::TempDir, request:outcomes::GenerationRequest, raw:String,
        dir:PathBuf, key:String, state_path:PathBuf,
    }
    fn accepted_legacy_fixture()->LegacyFixture {
        use serde_json::json;
        let root=tempfile::tempdir().unwrap();let text="请整理会议资料并列出可讨论的方案。";
        let capture=context_ledger::Capture {device:"layout-phone".into(),account:Some("account".into()),
            now_ms:1_793_318_400_000,utc_offset_seconds:8*3600,
            phone_discussions:vec![crate::phone_context::PhoneMessage {device:"layout-phone".into(),account:"account".into(),
                session_key:"session".into(),message_id:"message".into(),role:"user".into(),title:"会议资料".into(),
                text:text.into(),timestamp_ms:Some(1_793_318_399_000),..Default::default()}],..Default::default()};
        let overview=context_ledger::process(root.path(),&capture).unwrap();
        let target=Target::Item(overview.items[0].id.clone());
        let request=outcomes::prepare(root.path(),"layout-phone",Some("account"),&target).unwrap();
        let evidence=json!([{"id":request.sources[0].id,"quote":text,"version":"current"}]);
        let raw=json!({"schema":1,"conclusion":{"text":"用户请求整理会议资料并列出讨论方案","evidence":evidence},
            "key_facts":[{"text":text,"evidence":evidence}],"plan":[],"todos":[],"unknowns":[],"reuse":[],"next_period":[]}).to_string();
        let mut bundle=outcomes::save(root.path(),"layout-phone",Some("account"),&request,&raw).unwrap();
        bundle.render_version=outcomes::RENDER_VERSION-1;
        // Retain actual bytes from an older layout; the upgrade must create a
        // separate version rather than overwrite even unedited original files.
        fs::write(&bundle.files[0].path,"# Earlier layout\n\nOriginal accepted content.\n").unwrap();
        bundle.files[0].sha256=crate::device::sha256_file(&bundle.files[0].path).unwrap();
        let dir=scoped_dir(root.path(),&request.scope_id).unwrap();let key=job_key(&target,&request.input_revision);
        let raw_path=dir.join(format!("{key}-attempt-1.json.txt"));fs::write(&raw_path,&raw).unwrap();
        let state_path=dir.join(format!("{key}.json"));
        let job:Job=serde_json::from_value(json!({"schema":1,"scope_id":request.scope_id,"target":target,
            "input_revision":request.input_revision,"attempts":[{"started_ms":100,"raw_path":raw_path,"model":"local-fixture","error":null}],
            "bundle":bundle,"deliveries":[{"kind":"plan","transfer_id":"previous","sha256":"previous-layout-hash","path":"previous-phone-path","verified_ms":100}],
            "round_start":0,"revalidation_floor":0,"retries":[{"started_ms":90,"attempt_start":0,"reason":"existing intent"}]})).unwrap();
        save(&state_path,&job).unwrap();LegacyFixture {root,request,raw,dir,key,state_path}
    }
    #[test]
    fn historical_drafts_stay_readable_without_becoming_current_or_crossing_scopes() {
        let f = accepted_legacy_fixture();
        assert_eq!(list_current(f.root.path(), "layout-phone", Some("account")).unwrap().len(), 1);
        let mut job: Job = serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        job.input_revision = "previous-source-revision".into();
        job.bundle.as_mut().unwrap().input_revision = job.input_revision.clone();
        let previous_path = f.dir.join(format!("{}.json", job_key(&job.target, &job.input_revision)));
        save(&previous_path, &job).unwrap(); crate::data_store::remove_snapshot(&f.state_path).unwrap();
        assert!(list_current(f.root.path(), "layout-phone", Some("account")).unwrap().is_empty());
        let readable = list_for_reading(f.root.path(), "layout-phone", Some("account")).unwrap();
        assert_eq!(readable.len(), 1);
        assert_eq!(readable[0].bundle.input_revision, "previous-source-revision");
        assert!(list_current(f.root.path(), "layout-phone", Some("account")).unwrap().is_empty());
        assert!(list_for_reading(f.root.path(), "layout-phone", Some("other-account")).unwrap().is_empty());
        fs::write(&readable[0].bundle.files[0].path, "edited document").unwrap();
        assert!(list_for_reading(f.root.path(), "layout-phone", Some("account")).unwrap().is_empty(), "edited files must not inherit old verification");
    }
    #[test]
    fn loading_current_upgrades_exact_accepted_raw_without_model_or_budget_reset() {
        let f=accepted_legacy_fixture();
        let mut before:Job=serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        let previous=before.bundle.as_ref().unwrap().clone();
        let old_files:Vec<_>=previous.files.iter().map(|file|(file.path.clone(),fs::read(&file.path).unwrap())).collect();
        let old_manifest=fs::read(&previous.manifest_path).unwrap();
        // A newer valid but different reply must not replace accepted content.
        let other_path=f.dir.join(format!("{}-attempt-2.json.txt",f.key));
        let mut other:serde_json::Value=serde_json::from_str(&f.raw).unwrap();other["conclusion"]["text"]=serde_json::json!("另一份不同的讨论稿");
        fs::write(&other_path,other.to_string()).unwrap();
        before.attempts.push(Attempt {automatic: false, source: None, inference:None,empty_reply: false, started_ms:200,raw_path:Some(other_path),model:Some("local-fixture".into()),error:None});save(&f.state_path,&before).unwrap();
        let attempts=serde_json::to_value(&before.attempts).unwrap();let retries=serde_json::to_value(&before.retries).unwrap();
        let result=list_current(f.root.path(),"layout-phone",Some("account")).unwrap();
        assert_eq!(result.len(),1);assert_eq!(result[0].bundle.render_version,outcomes::RENDER_VERSION);
        assert_eq!(result[0].bundle.conclusion,previous.conclusion);assert!(result[0].deliveries.is_empty());
        let after:Job=serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&after.attempts).unwrap(),attempts);assert_eq!(serde_json::to_value(&after.retries).unwrap(),retries);
        assert_eq!(after.round_start,before.round_start);assert_eq!(after.revalidation_floor,before.revalidation_floor);
        assert!(after.revalidations.iter().any(|check|check.attempt==1&&check.accepted));
        assert!(after.revalidations.iter().all(|check|check.attempt!=2));
        for(path,bytes)in old_files {assert_eq!(fs::read(path).unwrap(),bytes);}
        assert_eq!(fs::read(&previous.manifest_path).unwrap(),old_manifest);
        let unchanged=state_bytes(&f.state_path).unwrap();
        assert_eq!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().len(),1);
        assert_eq!(state_bytes(&f.state_path).unwrap(),unchanged);
    }
    #[test]
    fn local_layout_upgrade_preserves_edits_and_respects_job_lock_and_revalidation_floor() {
        let f=accepted_legacy_fixture();let before=state_bytes(&f.state_path).unwrap();
        let lock=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
            .open(f.dir.join(format!("{}.lock",f.key))).unwrap();lock.try_lock().unwrap();
        assert!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().is_empty());
        assert_eq!(state_bytes(&f.state_path).unwrap(),before);drop(lock);
        let mut job:Job=serde_json::from_slice(&before).unwrap();job.revalidation_floor=job.attempts.len();save(&f.state_path,&job).unwrap();
        assert!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().is_empty());
        let saved:Job=serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        assert_eq!(saved.bundle.unwrap().render_version,outcomes::RENDER_VERSION-1);assert_eq!(saved.attempts.len(),1);
        job.revalidation_floor=0;let edited=job.bundle.as_ref().unwrap().files[0].path.clone();
        fs::write(&edited,"My handwritten changes must remain").unwrap();save(&f.state_path,&job).unwrap();
        let before=state_bytes(&f.state_path).unwrap();
        assert!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().is_empty());
        assert_eq!(fs::read_to_string(edited).unwrap(),"My handwritten changes must remain");
        assert_eq!(state_bytes(&f.state_path).unwrap(),before);
    }
    #[test]
    fn local_layout_upgrade_rejects_changed_raw_scope_and_cleared_context() {
        let f=accepted_legacy_fixture();
        let mut job:Job=serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        fs::write(job.attempts[0].raw_path.as_ref().unwrap(),"different invalid raw preserved").unwrap();
        assert!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().is_empty());
        let after:Job=serde_json::from_slice(&state_bytes(&f.state_path).unwrap()).unwrap();
        assert_eq!(after.attempts.len(),1);assert!(after.bundle.is_some());
        fs::write(job.attempts[0].raw_path.as_ref().unwrap(),&f.raw).unwrap();
        job.scope_id="other-account".into();save(&f.state_path,&job).unwrap();
        assert!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().is_empty());
        job.scope_id=f.request.scope_id.clone();save(&f.state_path,&job).unwrap();
        context_ledger::clear(f.root.path(),"layout-phone",Some("account")).unwrap();
        assert!(list_current(f.root.path(),"layout-phone",Some("account")).unwrap().is_empty());
        assert!(!f.root.path().join(&f.request.scope_id).exists());
    }
    #[test]
    fn retry_has_stable_delivery_identity_without_cross_account_collisions() {
        let id = transfer_id("account-one", "desktop", "content", "方案.md");
        assert!(companion_link::valid_id(&id));
        assert_eq!(id, transfer_id("account-one", "desktop", "content", "方案.md"));
        assert_ne!(id, transfer_id("account-two", "desktop", "content", "方案.md"));
        assert_ne!(id, transfer_id("account-one", "desktop", "updated", "方案.md"));
    }
    #[test]
    fn cancellation_and_exclusive_lock_prevent_duplicate_work() {
        assert!(check_cancel(&AtomicBool::new(true)).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.lock");
        let first = fs::File::create(&path).unwrap(); first.try_lock().unwrap();
        let second = fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert!(second.try_lock().is_err());
        drop(first); assert!(second.try_lock().is_ok());
    }
    #[test]
    fn correction_budget_preserves_sources_and_utf8_boundaries() {
        let source = "事实".repeat(PROMPT_BUDGET / 6 - 2);
        let mut prompt = source.clone();
        append_bounded(&mut prompt, "修复负责人和期限".repeat(5000).as_str(), 32 * 1024);
        assert!(prompt.starts_with(&source));
        assert!(prompt.len() <= PROMPT_BUDGET);
        assert!(prompt.is_char_boundary(prompt.len()));
    }

}

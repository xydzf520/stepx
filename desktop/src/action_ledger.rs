//! Private, account-scoped action state. Model proposals are candidates; only
//! explicit user transitions establish progress. No phone/network side effects.
use crate::context_ledger::{self, outcomes::{Draft, GenerationRequest, Reference, SourcedValue}};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs,  path::{Path, PathBuf}};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ACTIONS: usize = 2048;
const MAX_HISTORY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State { Candidate, Accepted, InProgress, Completed, Deferred, Dismissed }
impl State {
    pub fn label(self) -> &'static str { match self {
        Self::Candidate => "待核对候选", Self::Accepted => "已采纳", Self::InProgress => "进行中",
        Self::Completed => "已完成", Self::Deferred => "已延期", Self::Dismissed => "已忽略",
    } }
    pub fn confirmed(self) -> bool { !matches!(self, Self::Candidate | Self::Dismissed) }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateChange {
    pub id: u64, pub at_ms: i64, pub from: State, pub to: State, pub note: String,
    pub undo_of: Option<u64>, pub source_revision: String,
    #[serde(default)] pub from_needs_review: bool,
    /// A user progress note preserves the state and is never a completion event.
    #[serde(default)] pub progress_note: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Origin { pub target: String, pub input_revision: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceVersion {
    pub revision: String, pub observed_ms: i64, pub owner: Option<SourcedValue>,
    pub due: Option<SourcedValue>, pub evidence: Vec<Reference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: String, pub matter_ids: Vec<String>, pub action: String, pub kind: String,
    pub owner: Option<SourcedValue>, pub due: Option<SourcedValue>, pub evidence: Vec<Reference>,
    pub source_revision: String, pub first_seen_ms: i64, pub last_seen_ms: i64,
    pub state: State, pub needs_review: bool, pub history: Vec<StateChange>, pub origins: Vec<Origin>,
    #[serde(default)] pub source_history: Vec<SourceVersion>,
    #[serde(default)] pub source_check_revision:Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: u32, pub scope_id: String, pub revision: u64, pub progress_revision: u64,
    pub updated_ms: i64, pub actions: Vec<ActionRecord>,
}
impl Snapshot {
    fn empty(scope_id: String) -> Self {
        Self { schema: 1, scope_id, revision: 0, progress_revision: 0, updated_ms: 0, actions: vec![] }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgressFact {
    pub action_id: String, pub event_id: u64, pub at_ms: i64, pub state: State,
    pub matter_ids: Vec<String>, pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeriodProgress {
    pub scope_id: String, pub begin_ms: i64, pub end_ms: i64, pub progress_revision: u64,
    pub confirmed_total: usize, pub completed_in_period: usize, pub pending_confirmed: usize,
    pub candidates: usize, pub events: Vec<ProgressFact>, pub markdown: String,
}

fn scope(device: &str, account: Option<&str>) -> Result<String> {
    ensure!(!device.trim().is_empty(), "行动账本缺少设备范围");
    ensure!(account.is_none_or(|account| !account.trim().is_empty()), "行动账本账号范围无效");
    Ok(context_ledger::scope_id(device, account))
}
fn normalize(action: &str) -> String {
    action.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}
fn action_id(scope: &str, matters: &[String], action: &str) -> String {
    context_ledger::scope_id(&normalize(action), Some(&format!("{scope}:{}", matters.join("/"))))
}
fn checked_dir(root: &Path, scope: &str, create: bool) -> Result<Option<PathBuf>> {
    let scope_dir = root.join(scope);
    let dir = scope_dir.join("actions");
    if !create && !dir.exists() {
        for path in [root, &scope_dir, &dir] { if let Ok(meta) = fs::symlink_metadata(path) {
            ensure!(!meta.file_type().is_symlink() && meta.is_dir(), "行动账本目录不能是链接或文件");
        } }
        return Ok(None);
    }
    for path in [root, &scope_dir, &dir] {
        if create { fs::create_dir_all(path)?; }
        let meta = fs::symlink_metadata(path)?;
        ensure!(!meta.file_type().is_symlink() && meta.is_dir(), "行动账本目录不能是链接或文件");
        #[cfg(unix)] if create { use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    ensure!(fs::canonicalize(&dir)?.starts_with(fs::canonicalize(root)?), "行动账本已离开当前资料范围");
    Ok(Some(dir))
}
fn checked_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => { ensure!(meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= MAX_BYTES,
            "行动账本文件无效或超过容量，原记录未覆盖"); Ok(true) }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
fn read(dir: &Path, scope: &str) -> Result<Snapshot> {
    let path = dir.join("ledger.json");
    checked_file(&path)?;
    let Some(value): Option<Snapshot> = crate::data_store::read(&path, MAX_BYTES)? else { return Ok(Snapshot::empty(scope.into())); };
    ensure!(value.schema == 1 && value.scope_id == scope && value.actions.len() <= MAX_ACTIONS,
        "行动账本不属于当前设备和账号，或版本不受支持");
    let mut ids = BTreeSet::new();
    for action in &value.actions {
        ensure!(ids.insert(&action.id) && action.id == action_id(scope, &action.matter_ids, &action.action)
            && !action.action.trim().is_empty() && action.history.len() <= MAX_HISTORY,
            "行动账本标识或历史无效，原记录未覆盖");
        let mut state = State::Candidate; let mut previous_ms = 0;
        for (index, change) in action.history.iter().enumerate() {
            ensure!(change.id == index as u64 + 1 && change.from == state && change.at_ms >= previous_ms,
                "行动状态历史不连续，原记录未覆盖");
            if let Some(reverted) = change.undo_of {
                ensure!(reverted > 0 && reverted < change.id
                    && action.history[reverted as usize - 1].undo_of.is_none()
                    && !action.history[..index].iter().any(|c| c.undo_of == Some(reverted)), "行动撤销记录无效");
            }
            if change.progress_note {
                ensure!(change.from == change.to && change.from.confirmed() && !change.note.trim().is_empty(),
                    "行动进展记录无效，原记录未覆盖");
            }
            if let Some(reverted) = change.undo_of {
                ensure!(change.progress_note == action.history[reverted as usize - 1].progress_note,
                    "行动撤销类型无效，原记录未覆盖");
            }
            state = change.to; previous_ms = change.at_ms;
        }
        ensure!(state == action.state, "行动状态与历史不一致，原记录未覆盖");
    }
    Ok(value)
}
fn write(dir: &Path, snapshot: &Snapshot) -> Result<()> {
    let path = dir.join("ledger.json"); checked_file(&path)?;
    let bytes = serde_json::to_vec_pretty(snapshot)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES && snapshot.actions.len() <= MAX_ACTIONS, "行动账本达到容量上限，原记录未覆盖");
    crate::data_store::write(&path, snapshot)?;
    Ok(())
}
/// Persist the actual changed actions and scalar revisions in one transaction.
/// Complete source/history validation still precedes this write boundary.
fn write_actions(dir: &Path, snapshot: &Snapshot, ids: &BTreeSet<String>) -> Result<()> {
    let path = dir.join("ledger.json"); checked_file(&path)?;
    let Some(mut head) = crate::data_store::read_head(&path)? else {return write(dir,snapshot);};
    ensure!(head.value.get("revision").and_then(serde_json::Value::as_u64) == snapshot.revision.checked_sub(1), "行动状态已变化，请刷新后再操作；未覆盖新状态");
    let mut changes = Vec::new();
    let mut added_bytes = 0usize;
    for action in snapshot.actions.iter().filter(|action|ids.contains(&action.id)) {
        let value=serde_json::to_value(action)?;
        let old_size=crate::data_store::record(&path,"actions",&action.id)?.map(|old|serde_json::to_vec(&old).map(|bytes|bytes.len())).transpose()?.unwrap_or(0);
        added_bytes=added_bytes.saturating_add(serde_json::to_vec(&value)?.len().saturating_sub(old_size));
        changes.push(crate::data_store::RecordChange {section:"actions",id:&action.id,position:None,value});
    }
    ensure!(changes.len()==ids.len() && snapshot.actions.len()<=MAX_ACTIONS,"行动变更集合无效或达到容量上限，原记录未覆盖");
    let name = if path.is_absolute() {path.clone()} else {std::env::current_dir()?.join(&path)}.to_string_lossy().into_owned();
    let stored_bytes: i64 = crate::data_store::with_connection(&crate::data_store::database_for(&path), |c| Ok(c.query_row(
        "SELECT COALESCE(SUM(length(CAST(body AS BLOB))),0)+count(*) FROM records WHERE snapshot=?",[name],|r|r.get(0))?))?;
    ensure!(stored_bytes as u64 + added_bytes as u64 + serde_json::to_vec(&head.value)?.len() as u64 + 1024 <= MAX_BYTES, "行动账本达到容量上限，原记录未覆盖");
    head.value["revision"] = snapshot.revision.into();
    head.value["progress_revision"] = snapshot.progress_revision.into();
    head.value["updated_ms"] = snapshot.updated_ms.into();
    crate::data_store::apply_delta(&path,head.revision,Some(&head.value),&changes,&[])?;
    Ok(())
}
fn write_action(dir:&Path,snapshot:&Snapshot,id:&str)->Result<()> {
    write_actions(dir,snapshot,&BTreeSet::from([id.to_owned()]))
}
struct LedgerLock(fs::File);
impl Drop for LedgerLock {
    fn drop(&mut self) {
        // Do not rely solely on close: a concurrent process spawn may briefly
        // inherit a duplicate of this open file description until exec.
        let _=self.0.unlock();
    }
}
fn lock(dir: &Path) -> Result<LedgerLock> {
    let path = dir.join("ledger.lock"); checked_file(&path)?;
    let file = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.try_lock().context("行动账本正在更新，请刷新后重试")?; Ok(LedgerLock(file))
}
/// A missing ledger is empty. Corrupt, linked or copied scopes are errors.
pub fn load(root: &Path, device: &str, account: Option<&str>) -> Result<Snapshot> {
    let scope = scope(device, account)?;
    let Some(dir)=checked_dir(root,&scope,false)? else{return Ok(Snapshot::empty(scope));};
    let mut snapshot=read(&dir,&scope)?;
    let mut sources=context_ledger::action_source_view(root,device,account)?;
    if has_changed_sources(&sources,&snapshot)? {
        let _lock=lock(&dir)?;snapshot=read(&dir,&scope)?;
        sources=context_ledger::action_source_view(root,device,account)?;
        let changed = reconcile_sources(&sources,&mut snapshot)?;
        if !changed.is_empty() {write_actions(&dir,&snapshot,&changed)?;}
    }
    let excluded=sources.excluded;
    snapshot.actions.retain(|action|!action.evidence.iter().any(|reference|excluded.contains(&reference.id)));
    Ok(snapshot)
}
fn changed_source(sources:&context_ledger::ActionSourceView,action:&ActionRecord)->Result<bool> {
    let current=sources.revision_for(&action.matter_ids)?;
    Ok(current!=action.source_check_revision && (current.is_some() || action.source_check_revision.is_some()))
}
fn has_changed_sources(sources:&context_ledger::ActionSourceView,snapshot:&Snapshot)->Result<bool> {
    let excluded=&sources.excluded;
    for action in &snapshot.actions {if !action.needs_review && !action.evidence.iter().any(|r|excluded.contains(&r.id))
        && changed_source(sources,action)? {return Ok(true);}}Ok(false)
}
fn reconcile_sources(sources:&context_ledger::ActionSourceView,snapshot:&mut Snapshot)->Result<BTreeSet<String>> {
    let excluded=&sources.excluded;
    let mut changed=BTreeSet::new();let mut progress_changed=false;
    for action in &mut snapshot.actions {
        if action.needs_review || action.evidence.iter().any(|r|excluded.contains(&r.id)) {continue;}
        if changed_source(sources,action)? {action.needs_review=true;changed.insert(action.id.clone());progress_changed|=action.state.confirmed();}
    }
    if !changed.is_empty() {snapshot.revision+=1;snapshot.updated_ms=snapshot.updated_ms.max(chrono::Utc::now().timestamp_millis());
        if progress_changed {snapshot.progress_revision+=1;}}
    Ok(changed)
}

/// Called only after source revalidation and schema/citation validation in outcomes.
/// Repeated model drafts never adopt an action or reset an explicit user state.
pub(crate) fn sync_validated_draft(root: &Path, device: &str, account: Option<&str>, request: &GenerationRequest,
    draft: &Draft, at_ms: i64) -> Result<Snapshot> {
    let scope = scope(device, account)?;
    ensure!(request.scope_id == scope, "候选行动不属于当前设备和账号");
    let dir = checked_dir(root, &scope, true)?.expect("created"); let _lock = lock(&dir)?;
    let mut snapshot = read(&dir, &scope)?; let mut changed = BTreeSet::new(); let mut progress_changed = false;
    let source_view=context_ledger::action_source_view(root,device,account)?;
    for todo in &draft.todos {
        let mut matters: Vec<String> = todo.evidence.iter().filter_map(|reference|
            request.sources.iter().find(|source| source.id == reference.id)
                .filter(|source| source.source != "用户维护的行动账本").map(|source| source.matter_id.clone())).collect();
        matters.sort(); matters.dedup();
        // A previous generated action/status cannot propose another action by itself.
        if matters.is_empty() { continue; }
        let id = action_id(&scope, &matters, &todo.action);
        let mut references = todo.evidence.iter().map(|r|(&r.id,&r.quote,&r.version)).collect::<Vec<_>>(); references.sort();
        let mut sources = todo.evidence.iter().filter_map(|reference| request.sources.iter().find(|s|s.id==reference.id))
            .map(|source|(&source.id,&source.text)).collect::<Vec<_>>(); sources.sort(); sources.dedup();
        let fingerprint = context_ledger::scope_id(&serde_json::to_string(&(
            &todo.action, &todo.kind, &todo.owner, &todo.due, references, sources,
        ))?, None);
        if let Some(existing) = snapshot.actions.iter_mut().find(|a| a.id == id) {
            ensure!(normalize(&existing.action) == normalize(&todo.action) && existing.matter_ids == matters,
                "候选行动标识冲突，原状态未覆盖");
            // A period draft may summarize an already tracked action with a
            // different citation subset. It must not rewrite the item source
            // or invalidate the period that is currently being saved.
            if existing.source_revision != fingerprint && matches!(request.target, crate::context_ledger::outcomes::Target::Item(_)) {
                ensure!(existing.source_history.len() < 256, "行动来源修订达到容量上限，原记录保留");
                existing.source_history.push(SourceVersion { revision: existing.source_revision.clone(), observed_ms:existing.last_seen_ms,
                    owner:existing.owner.clone(),due:existing.due.clone(),evidence:existing.evidence.clone() });
                if existing.state.confirmed() { existing.needs_review = true; progress_changed = true; }
                existing.action = todo.action.clone(); existing.kind = todo.kind.clone();
                existing.owner = todo.owner.clone(); existing.due = todo.due.clone(); existing.evidence = todo.evidence.clone();
                existing.source_revision = fingerprint; existing.last_seen_ms = at_ms; changed.insert(id.clone());
            }
            let target = request.target.key();
            if let Some(origin) = existing.origins.iter_mut().find(|o|o.target == target) {
                if origin.input_revision != request.input_revision { origin.input_revision = request.input_revision.clone(); changed.insert(id.clone()); }
            } else { existing.origins.push(Origin { target, input_revision: request.input_revision.clone() }); changed.insert(id.clone()); }
        } else {
            ensure!(snapshot.actions.len() < MAX_ACTIONS, "候选行动达到容量上限，已有记录保留");
            let source_check_revision=source_view.revision_for(&matters)?;
            snapshot.actions.push(ActionRecord { id:id.clone(), matter_ids: matters, action: todo.action.clone(), kind: todo.kind.clone(),
                owner: todo.owner.clone(), due: todo.due.clone(), evidence: todo.evidence.clone(), source_revision: fingerprint,
                first_seen_ms: at_ms, last_seen_ms: at_ms, state: State::Candidate, needs_review: false, history: vec![],
                source_check_revision,
                origins: vec![Origin { target: request.target.key(), input_revision: request.input_revision.clone() }], source_history:vec![] });
            changed.insert(id.clone());
        }
    }
    if !changed.is_empty() {
        snapshot.revision += 1;
        if progress_changed {
            snapshot.progress_revision += 1;
            snapshot.updated_ms = snapshot.updated_ms.max(at_ms);
        }
        write_actions(&dir, &snapshot, &changed)?;
    }
    Ok(snapshot)
}

fn allowed(from: State, to: State) -> bool {
    match from { State::Candidate => matches!(to, State::Accepted | State::Dismissed),
        State::Dismissed => to == State::Candidate,
        _ => to != State::Candidate }
}
/// Optimistic revision checks prevent a stale click overwriting a concurrent edit.
pub fn set_state(root: &Path, device: &str, account: Option<&str>, id: &str, expected_revision: u64,
    next: State, note: &str, at_ms: i64) -> Result<Snapshot> {
    mutate(root, device, account, id, expected_revision, Mutation::State(next, note), at_ms)
}
pub fn undo_state(root: &Path, device: &str, account: Option<&str>, id: &str, expected_revision: u64,
    at_ms: i64) -> Result<Snapshot> {
    mutate(root, device, account, id, expected_revision, Mutation::Undo, at_ms)
}
/// Record user-reported progress without changing state, ownership, deadlines or
/// source verification. It participates in period evidence and can be undone.
pub fn record_progress(root: &Path, device: &str, account: Option<&str>, id: &str,
    expected_revision: u64, note: &str, at_ms: i64) -> Result<Snapshot> {
    mutate(root, device, account, id, expected_revision, Mutation::Progress(note), at_ms)
}
enum Mutation<'a> { State(State, &'a str), Progress(&'a str), Undo }
fn mutate(root: &Path, device: &str, account: Option<&str>, id: &str, expected_revision: u64,
    change: Mutation<'_>, at_ms: i64) -> Result<Snapshot> {
    let scope = scope(device, account)?;
    let dir = checked_dir(root, &scope, false)?.context("行动记录已清理或不存在，请刷新后再操作")?; let _lock = lock(&dir)?;
    let mut snapshot = read(&dir, &scope)?;
    let sources=context_ledger::action_source_view(root,device,account)?;
    let changed = reconcile_sources(&sources,&mut snapshot)?;
    if !changed.is_empty() {write_actions(&dir,&snapshot,&changed)?;}
    ensure!(snapshot.revision == expected_revision, "行动状态已变化，请刷新后再操作；未覆盖新状态");
    let action = snapshot.actions.iter_mut().find(|a|a.id == id).context("当前范围没有这项行动")?;
    let excluded=&sources.excluded;
    ensure!(!action.evidence.iter().any(|reference|excluded.contains(&reference.id)),"这是已隔离的旧验收资料，未改写个人行动状态");
    ensure!(at_ms > 0 && action.history.last().is_none_or(|c|at_ms >= c.at_ms), "行动更新时间无效");
    ensure!(action.history.len() < MAX_HISTORY, "该行动历史达到容量上限，已有状态保留");
    let (next, note, undo_of, needs_review, progress_note, verifies_sources) = match change {
        Mutation::State(next, note) => {
            if next == action.state && !action.needs_review { return Ok(snapshot); }
            ensure!(allowed(action.state, next) || (next == action.state && action.needs_review), "候选需先采纳；已忽略行动需先恢复候选");
            ensure!(note.chars().count() <= 500, "状态备注请控制在500字内");
            (next, note.trim().to_owned(), None, false, false, true)
        }
        Mutation::Progress(note) => {
            ensure!(action.state.confirmed(), "请先核对并采纳行动，再记录进展");
            let note = note.trim();
            ensure!(!note.is_empty(), "请填写这项行动的进展或阻塞原因");
            ensure!(note.chars().count() <= 500, "进展记录请控制在500字内");
            (action.state, note.to_owned(), None, action.needs_review, true, false)
        }
        Mutation::Undo => {
            let reverted: BTreeSet<_> = action.history.iter().filter_map(|c|c.undo_of).collect();
            let previous = action.history.iter().rev().find(|c|c.undo_of.is_none() && !reverted.contains(&c.id))
                .context("没有可撤销的状态变更")?;
            (previous.from, format!("用户撤销{} {}", if previous.progress_note {"进展记录"} else {"状态变更"}, previous.id), Some(previous.id),
                previous.from_needs_review || previous.source_revision != action.source_revision || changed_source(&sources,action)?,
                previous.progress_note, false)
        }
    };
    let previous_state = action.state; let previous_review = action.needs_review;
    action.history.push(StateChange { id: action.history.len() as u64 + 1, at_ms, from: previous_state, to: next,
        note, undo_of, source_revision: action.source_revision.clone(), from_needs_review: previous_review, progress_note });
    action.state = next; action.needs_review = needs_review;
    if verifies_sources {action.source_check_revision=sources.revision_for(&action.matter_ids)?;}
    snapshot.revision += 1; snapshot.progress_revision += 1; snapshot.updated_ms = at_ms;
    write_action(&dir, &snapshot, id)?; Ok(snapshot)
}

fn effective_change(action: &ActionRecord) -> Option<&StateChange> {
    let reverted: BTreeSet<_> = action.history.iter().filter_map(|c|c.undo_of).collect();
    action.history.iter().rev().find(|c|c.undo_of.is_none() && !reverted.contains(&c.id))
}
fn effective_state_change(action: &ActionRecord) -> Option<&StateChange> {
    let reverted: BTreeSet<_> = action.history.iter().filter_map(|c|c.undo_of).collect();
    action.history.iter().rev().find(|c|!c.progress_note && c.undo_of.is_none() && !reverted.contains(&c.id))
}
fn plain(value: &str) -> String { value.chars().map(|c|if c.is_control() {' '} else {c}).collect() }
pub fn summarize_period(root: &Path, device: &str, account: Option<&str>, begin_ms: i64, end_ms: i64) -> Result<PeriodProgress> {
    ensure!(begin_ms < end_ms, "复盘期间无效");
    let snapshot = load(root, device, account)?;
    Ok(progress(&snapshot, begin_ms, end_ms))
}
fn progress(snapshot: &Snapshot, begin_ms: i64, end_ms: i64) -> PeriodProgress {
    let in_period = |at| at >= begin_ms && at < end_ms;
    let confirmed: Vec<_> = snapshot.actions.iter().filter(|a|a.state.confirmed()).collect();
    let completed_in_period = confirmed.iter().filter(|a| a.state == State::Completed && !a.needs_review
        && effective_state_change(a).is_some_and(|c|c.to == State::Completed && in_period(c.at_ms))).count();
    let pending: Vec<_> = confirmed.iter().copied().filter(|a|a.state != State::Completed || a.needs_review).collect();
    let mut events = vec![];
    for action in &snapshot.actions {
        for change in action.history.iter().filter(|c|in_period(c.at_ms)) {
            // Withdrawn notes stay in the private history, but must not remain
            // independently quotable period evidence after the user retracts them.
            if change.progress_note && change.undo_of.is_none()
                && action.history.iter().any(|c|c.undo_of == Some(change.id)) { continue; }
            let text = if let Some(undone) = change.undo_of {
                format!("用户撤销行动「{}」的{}{undone}，状态为{}；原约定负责人和期限未改写。", plain(&action.action),
                    if change.progress_note {"进展记录"} else {"状态变更"}, change.to.label())
            } else if change.progress_note {
                format!("用户为行动「{}」记录进展：{}；状态保持{}，未确认新的完成。这是用户自述，未独立核验现实执行。",
                    plain(&action.action), plain(&change.note), change.to.label())
            } else { format!("用户将行动「{}」从{}标记为{}{}；这是用户维护的状态声明，未独立核验现实执行。",
                plain(&action.action), change.from.label(), change.to.label(),
                if change.note.is_empty() {String::new()} else {format!("，备注：{}",plain(&change.note))}) };
            events.push(ProgressFact { action_id: action.id.clone(), event_id: change.id, at_ms: change.at_ms,
                state: change.to, matter_ids: action.matter_ids.clone(), text });
        }
    }
    // Include current confirmed backlog even if adopted before this period.
    for action in &pending {
        if let Some(change) = effective_change(action) {
            let text = format!("当前用户确认行动「{}」状态为{}{}；来源约定负责人：{}；来源约定期限：{}。未核验现实执行，未把延期备注改写为来源期限。",
                plain(&action.action), action.state.label(), if action.needs_review {"，来源已变化需重新核对"} else {""},
                action.owner.as_ref().map(|v|plain(&v.value)).unwrap_or_else(||"未约定".into()),
                action.due.as_ref().map(|v|plain(&v.value)).unwrap_or_else(||"未约定".into()));
            events.push(ProgressFact { action_id: action.id.clone(), event_id: change.id, at_ms: change.at_ms,
                state: action.state, matter_ids: action.matter_ids.clone(), text });
        }
    }
    events.sort_by_key(|event|(event.at_ms,event.action_id.clone(),event.event_id));
    let mut markdown = format!("# 用户确认的行动进展\n\n本期用户标记完成 {} 项；当前已采纳待推进或待重新核对 {} 项。仅反映本地已确认状态，不代表手机全部工作，也不代表独立验收。候选与已忽略项不计完成量，不计算完成率。\n\n## 本期状态变更与当前待推进\n\n", completed_in_period,pending.len());
    if events.is_empty() {markdown.push_str("本期没有已保存的用户状态变更；不能据此判断现实没有推进。\n");}
    for event in &events { markdown.push_str(&format!("- {}\n",event.text)); }
    PeriodProgress {scope_id:snapshot.scope_id.clone(),begin_ms,end_ms,progress_revision:snapshot.progress_revision,
        confirmed_total:confirmed.len(),completed_in_period,pending_confirmed:pending.len(),
        candidates:snapshot.actions.iter().filter(|a|a.state == State::Candidate).count(),events,markdown}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_ledger::outcomes::{SourceEvidence,SupportedText,Target,Todo};
    fn packet() -> (GenerationRequest,Draft) {
        let quote="我负责在周五前整理会议资料。";
        let reference=Reference{id:"source-a".into(),quote:quote.into(),version:"current".into()};
        let request=GenerationRequest{scope_id:context_ledger::scope_id("phone",Some("account")),target:Target::Item("matter".into()),input_revision:"one".into(),title:"会议资料".into(),prompt:String::new(),
            sources:vec![SourceEvidence{id:reference.id.clone(),matter_id:"matter".into(),title:"会议资料".into(),text:quote.into(),author:"我".into(),source:"讨论".into(),locator:"会话".into(),occurred_ms:Some(10),observed_ms:10,changed_ms:10,kind:"承诺候选，需核对".into(),authoritative:true,user_authored:true,in_period:true,baseline:None}],eligible_for_automation:true,coverage:"测试来源".into(),begin_ms:None,end_ms:None,history_baseline_available:false,utc_offset_seconds:0};
        let fact=SupportedText{text:quote.into(),evidence:vec![reference.clone()]};
        let draft=Draft{schema:1,conclusion:fact.clone(),key_facts:vec![fact],plan:vec![],todos:vec![Todo{action:"整理会议资料".into(),kind:"commitment".into(),owner:Some(SourcedValue{value:"我".into(),evidence:reference.clone()}),due:Some(SourcedValue{value:"周五前".into(),evidence:reference.clone()}),evidence:vec![reference]}],unknowns:vec![],reuse:vec![],next_period:vec![]};
        (request,draft)
    }
    #[test]
    fn candidate_refresh_updates_only_the_changed_action_and_revisions() {
        let root=tempfile::tempdir().unwrap(); let(mut request,mut draft)=packet();
        let todo=draft.todos[0].clone();
        draft.todos=(0..20).map(|index|{let mut item=todo.clone();item.action=format!("整理第{index}项会议资料");item}).collect();
        let before=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();
        draft.todos.truncate(1); request.input_revision="next-origin".into();
        let path=checked_dir(root.path(),&before.scope_id,false).unwrap().unwrap().join("ledger.json");
        let db=crate::data_store::database_for(&path);
        let old_changes=crate::data_store::with_connection(&db,|c|Ok(c.total_changes())).unwrap();
        let after=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,20).unwrap();
        let new_changes=crate::data_store::with_connection(&db,|c|Ok(c.total_changes())).unwrap();
        assert_eq!(new_changes-old_changes,3,"one candidate, logical revision head, storage revision");
        assert_eq!(after.actions.len(),20);
        assert_eq!(after.actions[0].origins[0].input_revision,"next-origin");
        assert!(after.actions.iter().skip(1).all(|action|action.origins[0].input_revision=="one"));
    }
    #[test]
    fn proposals_are_not_progress_and_stale_clicks_do_not_overwrite_user_edits() {
        let root=tempfile::tempdir().unwrap();let(request,draft)=packet();
        let initial=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();
        let id=&initial.actions[0].id;
        assert_eq!(initial.actions[0].state,State::Candidate);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),0,100).unwrap().confirmed_total,0);
        assert!(set_state(root.path(),"phone",Some("account"),id,initial.revision,State::Completed,"",20).is_err());
        let accepted=set_state(root.path(),"phone",Some("account"),id,initial.revision,State::Accepted,"确认推进",20).unwrap();
        assert!(set_state(root.path(),"phone",Some("account"),id,initial.revision,State::Dismissed,"旧页面点击",21).is_err());
        assert_eq!(load(root.path(),"phone",Some("account")).unwrap().revision,accepted.revision);
        let repeated=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,30).unwrap();
        assert_eq!(repeated.revision,accepted.revision);assert_eq!(repeated.actions[0].state,State::Accepted);
        assert!(load(root.path(),"phone",Some("another-account")).unwrap().actions.is_empty());
        assert!(load(root.path(),"other-phone",Some("account")).unwrap().actions.is_empty());
    }
    #[test]
    fn completion_undo_and_period_boundaries_are_factual_and_reversible() {
        let root=tempfile::tempdir().unwrap();let(request,draft)=packet();
        let mut snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();let id=snapshot.actions[0].id.clone();
        for (state,at) in [(State::Accepted,20),(State::InProgress,100),(State::Completed,200)] {
            snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,state,"",at).unwrap();
        }
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),100,200).unwrap().completed_in_period,0);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),100,201).unwrap().completed_in_period,1);
        snapshot=undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,210).unwrap();
        assert_eq!(snapshot.actions[0].state,State::InProgress);
        let progress=summarize_period(root.path(),"phone",Some("account"),100,300).unwrap();
        assert_eq!(progress.completed_in_period,0);assert_eq!(progress.pending_confirmed,1);assert!(progress.markdown.contains("撤销"));
        snapshot=undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,220).unwrap();
        assert_eq!(snapshot.actions[0].state,State::Accepted);
        snapshot=undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,230).unwrap();
        assert_eq!(snapshot.actions[0].state,State::Candidate);
        assert!(undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,240).is_err());
        assert_eq!(load(root.path(),"phone",Some("account")).unwrap().actions[0].history.len(),6);
    }
    #[test]
    fn progress_notes_preserve_state_and_completion_period_and_are_reversible() {
        let root=tempfile::tempdir().unwrap();let(request,draft)=packet();
        let mut snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();
        let id=snapshot.actions[0].id.clone();
        assert!(record_progress(root.path(),"phone",Some("account"),&id,snapshot.revision,"尚未采纳",15).is_err());
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Accepted,"",20).unwrap();
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::InProgress,"",30).unwrap();
        let before=snapshot.clone();
        assert!(record_progress(root.path(),"phone",Some("account"),&id,snapshot.revision,"  ",40).is_err());
        snapshot=record_progress(root.path(),"phone",Some("account"),&id,snapshot.revision,"已整理目录，等待原始数据",40).unwrap();
        assert_eq!(snapshot.actions[0].state,State::InProgress);
        assert!(snapshot.actions[0].history.last().unwrap().progress_note);
        assert_eq!(snapshot.progress_revision,before.progress_revision+1);
        assert!(record_progress(root.path(),"phone",Some("account"),&id,before.revision,"旧页面备注",41).is_err());
        assert!(record_progress(root.path(),"phone",Some("other"),&id,snapshot.revision,"错误账号",41).is_err());
        let progress=summarize_period(root.path(),"phone",Some("account"),35,45).unwrap();
        assert_eq!(progress.completed_in_period,0);
        assert!(progress.events.iter().any(|e|e.text.contains("已整理目录，等待原始数据")));
        snapshot=undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,50).unwrap();
        assert_eq!(snapshot.actions[0].state,State::InProgress);
        let withdrawn=summarize_period(root.path(),"phone",Some("account"),35,55).unwrap();
        assert!(withdrawn.markdown.contains("撤销") && withdrawn.markdown.contains("进展记录"));
        assert!(!withdrawn.markdown.contains("已整理目录，等待原始数据"));
        assert!(snapshot.actions[0].history.iter().any(|c|c.note=="已整理目录，等待原始数据"));
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Completed,"",60).unwrap();
        snapshot=record_progress(root.path(),"phone",Some("account"),&id,snapshot.revision,"补充成果归档位置",160).unwrap();
        assert_eq!(snapshot.actions[0].state,State::Completed);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),50,100).unwrap().completed_in_period,1);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),100,200).unwrap().completed_in_period,0);
        snapshot=undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,170).unwrap();
        assert_eq!(snapshot.actions[0].state,State::Completed);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),50,100).unwrap().completed_in_period,1);
        snapshot=undo_state(root.path(),"phone",Some("account"),&id,snapshot.revision,180).unwrap();
        assert_eq!(snapshot.actions[0].state,State::InProgress);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),50,100).unwrap().completed_in_period,0);
    }
    #[test]
    fn progress_notes_do_not_reconfirm_changed_sources_and_legacy_history_still_loads() {
        let root=tempfile::tempdir().unwrap();let(mut request,mut draft)=packet();
        let mut snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();
        let id=snapshot.actions[0].id.clone();
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Accepted,"",20).unwrap();
        let path=root.path().join(&snapshot.scope_id).join("actions/ledger.json");
        let mut legacy=serde_json::to_value(&snapshot).unwrap();
        for change in legacy["actions"][0]["history"].as_array_mut().unwrap() {change.as_object_mut().unwrap().remove("progress_note");}
        crate::data_store::remove_scope(path.parent().unwrap()).unwrap();fs::write(&path,serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(load(root.path(),"phone",Some("account")).unwrap().revision,snapshot.revision);
        request.input_revision="changed".into();request.sources[0].text="我负责在周六前整理会议资料。".into();
        draft.todos[0].due.as_mut().unwrap().value="周六前".into();
        draft.todos[0].due.as_mut().unwrap().evidence.quote=request.sources[0].text.clone();
        draft.todos[0].evidence[0].quote=request.sources[0].text.clone();
        snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,30).unwrap();
        assert!(snapshot.actions[0].needs_review);
        let source_check=snapshot.actions[0].source_check_revision.clone();
        snapshot=record_progress(root.path(),"phone",Some("account"),&id,snapshot.revision,"新期限尚待核对",40).unwrap();
        assert!(snapshot.actions[0].needs_review);
        assert_eq!(snapshot.actions[0].source_check_revision,source_check);
        assert_eq!(snapshot.actions[0].state,State::Accepted);
    }
    #[test]
    fn changed_sources_preserve_user_status_original_terms_and_require_review() {
        let root=tempfile::tempdir().unwrap();let(mut request,mut draft)=packet();
        let mut snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();let id=snapshot.actions[0].id.clone();
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Accepted,"",20).unwrap();
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Completed,"",30).unwrap();
        request.input_revision="two".into();request.sources[0].text="我负责在周六前整理会议资料。".into();
        draft.todos[0].due.as_mut().unwrap().value="周六前".into();draft.todos[0].due.as_mut().unwrap().evidence.quote=request.sources[0].text.clone();
        draft.todos[0].evidence[0].quote=request.sources[0].text.clone();
        let updated=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,40).unwrap();
        assert_eq!(updated.actions[0].id,id);assert_eq!(updated.actions[0].state,State::Completed);assert!(updated.actions[0].needs_review);
        assert_eq!(updated.actions[0].source_history[0].due.as_ref().unwrap().value,"周五前");
        assert_eq!(updated.actions[0].due.as_ref().unwrap().value,"周六前");assert!(updated.progress_revision>snapshot.progress_revision);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),0,100).unwrap().completed_in_period,0);
    }
    #[test]
    fn damaged_or_copied_scopes_are_not_silently_replaced() {
        let root=tempfile::tempdir().unwrap();let(request,draft)=packet();
        let snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();
        let path=root.path().join(&snapshot.scope_id).join("actions/ledger.json");
        crate::data_store::remove_scope(path.parent().unwrap()).unwrap();
        fs::write(&path,b"broken original").unwrap();
        assert!(load(root.path(),"phone",Some("account")).is_err());
        assert!(set_state(root.path(),"phone",Some("account"),&snapshot.actions[0].id,snapshot.revision,State::Accepted,"",20).is_err());
        assert_eq!(fs::read(&path).unwrap(),b"broken original");
        fs::write(&path,serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let other=root.path().join(context_ledger::scope_id("phone",Some("other"))).join("actions");fs::create_dir_all(&other).unwrap();
        fs::copy(&path,other.join("ledger.json")).unwrap();assert!(load(root.path(),"phone",Some("other")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linked_action_files_cannot_redirect_status_writes() {
        let root=tempfile::tempdir().unwrap();let(request,draft)=packet();
        let snapshot=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,10).unwrap();
        let path=root.path().join(&snapshot.scope_id).join("actions/ledger.json");let outside=root.path().join("outside.json");
        fs::write(&outside,b"keep me").unwrap();let _=fs::remove_file(&path);std::os::unix::fs::symlink(&outside,&path).unwrap();
        assert!(set_state(root.path(),"phone",Some("account"),&snapshot.actions[0].id,snapshot.revision,State::Accepted,"",20).is_err());
        assert_eq!(fs::read(outside).unwrap(),b"keep me");
    }
    #[cfg(unix)]
    #[test]
    fn released_guard_unlocks_even_when_an_open_descriptor_was_duplicated() {
        let root=tempfile::tempdir().unwrap();let scope=scope("phone",Some("account")).unwrap();
        let dir=checked_dir(root.path(),&scope,true).unwrap().unwrap();
        let guard=lock(&dir).unwrap();let duplicate=guard.0.try_clone().unwrap();
        drop(guard);
        let next=lock(&dir).expect("returning from an update must release its lock, not wait for inherited handles");
        drop(duplicate);drop(next);
    }
    #[test]
    fn accepted_outcome_seeds_candidates_and_user_progress_invalidates_actual_period_prompt() {
        use crate::context_ledger::{Capture,Period};
        let root=tempfile::tempdir().unwrap();let at=chrono::DateTime::parse_from_rfc3339("2026-11-02T10:00:00Z").unwrap().timestamp_millis();
        let capture=Capture{device:"phone".into(),account:Some("account".into()),now_ms:at,utc_offset_seconds:8*3600,
            phone_discussions:vec![crate::phone_context::PhoneMessage{device:"phone".into(),account:"account".into(),session_key:"human-discussion".into(),message_id:"human-message".into(),title:"会议方案".into(),role:"user".into(),text:"我负责在周五前整理会议资料。".into(),withdrawn:false,timestamp_ms:Some(at)}],..Default::default()};
        let overview=context_ledger::process(root.path(),&capture).unwrap();
        let request=crate::context_ledger::outcomes::prepare(root.path(),"phone",Some("account"),&Target::Item(overview.items[0].id.clone())).unwrap();
        let(_,mut draft)=packet();
        let reference=Reference{id:request.sources[0].id.clone(),quote:request.sources[0].text.clone(),version:"current".into()};
        draft.conclusion.evidence=vec![reference.clone()];draft.key_facts[0].evidence=vec![reference.clone()];
        draft.todos[0].evidence=vec![reference.clone()];draft.todos[0].owner.as_mut().unwrap().evidence=reference.clone();draft.todos[0].due.as_mut().unwrap().evidence=reference;
        crate::context_ledger::outcomes::save(root.path(),"phone",Some("account"),&request,&serde_json::to_string(&draft).unwrap()).unwrap();
        let before=crate::context_ledger::outcomes::prepare(root.path(),"phone",Some("account"),&Target::Period(Period::Day)).unwrap();
        let mut snapshot=load(root.path(),"phone",Some("account")).unwrap();assert_eq!(snapshot.actions.len(),1);let id=snapshot.actions[0].id.clone();
        snapshot=set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Accepted,"",at+1000).unwrap();
        let adopted=crate::context_ledger::outcomes::prepare(root.path(),"phone",Some("account"),&Target::Period(Period::Day)).unwrap();
        snapshot=record_progress(root.path(),"phone",Some("account"),&id,snapshot.revision,"目录已整理，等待附件",at+1500).unwrap();
        let noted=crate::context_ledger::outcomes::prepare(root.path(),"phone",Some("account"),&Target::Period(Period::Day)).unwrap();
        assert_ne!(adopted.input_revision,noted.input_revision);
        assert!(noted.sources.iter().any(|s|s.source=="用户维护的行动账本" && s.text.contains("目录已整理，等待附件")));
        set_state(root.path(),"phone",Some("account"),&id,snapshot.revision,State::Completed,"用户确认整理完成",at+2000).unwrap();
        let after=crate::context_ledger::outcomes::prepare(root.path(),"phone",Some("account"),&Target::Period(Period::Day)).unwrap();
        assert_ne!(before.input_revision,after.input_revision);
        assert!(after.sources.iter().any(|s|s.source=="用户维护的行动账本" && s.text.contains("标记为已完成")));
        assert!(after.prompt.contains("未独立核验现实执行"));
        let loaded=context_ledger::load(root.path(),"phone",Some("account")).unwrap();
        assert_eq!(loaded.periods.iter().find(|p|p.kind==Period::Day).unwrap().evidence_revision,after.input_revision);
    }
    #[test]
    fn source_change_without_regeneration_requires_review_and_does_not_rewrite_user_history() {
        let root=tempfile::tempdir().unwrap();let at=1_793_577_600_000;
        let mut capture=context_ledger::Capture{device:"phone".into(),account:Some("account".into()),now_ms:at,utc_offset_seconds:8*3600,
            phone_discussions:vec![crate::phone_context::PhoneMessage{device:"phone".into(),account:"account".into(),session_key:"source-change".into(),message_id:"original".into(),title:"会议资料".into(),role:"user".into(),text:"我负责在周五前整理会议资料。".into(),withdrawn:false,timestamp_ms:Some(at)}],..Default::default()};
        let overview=context_ledger::process(root.path(),&capture).unwrap();
        let request=crate::context_ledger::outcomes::prepare(root.path(),"phone",Some("account"),&Target::Item(overview.items[0].id.clone())).unwrap();
        let(_,mut draft)=packet();let reference=Reference{id:request.sources[0].id.clone(),quote:request.sources[0].text.clone(),version:"current".into()};
        draft.todos[0].evidence=vec![reference.clone()];draft.todos[0].owner.as_mut().unwrap().evidence=reference.clone();draft.todos[0].due.as_mut().unwrap().evidence=reference;
        let mut state=sync_validated_draft(root.path(),"phone",Some("account"),&request,&draft,at).unwrap();let id=state.actions[0].id.clone();
        state=set_state(root.path(),"phone",Some("account"),&id,state.revision,State::Accepted,"",at+1000).unwrap();
        state=set_state(root.path(),"phone",Some("account"),&id,state.revision,State::Completed,"",at+2000).unwrap();
        let history=serde_json::to_value(&state.actions[0].history).unwrap();
        capture.now_ms+=60_000;capture.phone_discussions[0].text="我负责在周六前整理会议资料。".into();
        context_ledger::process(root.path(),&capture).unwrap();
        let changed=load(root.path(),"phone",Some("account")).unwrap();
        assert!(changed.actions[0].needs_review);assert_eq!(changed.actions[0].state,State::Completed);
        assert_eq!(serde_json::to_value(&changed.actions[0].history).unwrap(),history);
        assert!(changed.progress_revision>state.progress_revision);
        assert_eq!(summarize_period(root.path(),"phone",Some("account"),at,at+86_400_000).unwrap().completed_in_period,0);
        assert_eq!(load(root.path(),"phone",Some("account")).unwrap().revision,changed.revision,"unchanged source failure must not churn every refresh");
        let confirmed=set_state(root.path(),"phone",Some("account"),&id,changed.revision,State::Completed,"核对更新后的原文，再确认完成",capture.now_ms+1000).unwrap();
        assert!(!load(root.path(),"phone",Some("account")).unwrap().actions[0].needs_review);
        assert_eq!(load(root.path(),"phone",Some("account")).unwrap().revision,confirmed.revision);
    }
}

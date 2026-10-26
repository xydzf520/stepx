//! Read-only checks of the connected phone's official updater metadata endpoints.
//! A metadata candidate is not proof of device eligibility, nor of every unpublished build.
use crate::{
    adb::{shell_quote, Adb},
    device::Device,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
    time::Duration,
};

const PROBE: &[u8] = include_bytes!("../assets/stepx-ota.jar");
const REMOTE_DIR: &str = "/data/local/tmp/stepx-companion";
const REMOTE_OTA_JAR: &str = "/data/local/tmp/stepx-companion/stepx-ota.jar";

static VERIFIED_OTA_CACHE: OnceLock<Mutex<HashSet<(String, usize)>>> = OnceLock::new();

fn ota_cache() -> &'static Mutex<HashSet<(String, usize)>> {
    VERIFIED_OTA_CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_ota_jar_cached(transport: &str) -> bool {
    let key = transport.trim();
    if key.is_empty() {
        return false;
    }
    ota_cache()
        .lock()
        .map(|guard| guard.contains(&(key.to_string(), PROBE.len())))
        .unwrap_or(false)
}

fn mark_ota_jar_cached(transport: &str) {
    let key = transport.trim();
    if key.is_empty() {
        return;
    }
    if let Ok(mut guard) = ota_cache().lock() {
        guard.insert((key.to_string(), PROBE.len()));
    }
}

fn invalidate_ota_cache(transport: &str) {
    let key = transport.trim();
    if let Ok(mut guard) = ota_cache().lock() {
        guard.retain(|(t, _)| t != key);
    }
}

fn ensure_ota_jar(adb: &Adb, device: &Device) -> Result<()> {
    if is_ota_jar_cached(&device.transport) {
        return Ok(());
    }
    let expected_len = PROBE.len();
    let check_cmd = format!(
        "mkdir -p {dir} && chmod 700 {dir} && [ -f {jar} ] && [ \"$(stat -c %s {jar} 2>/dev/null)\" = \"{expected_len}\" ] && printf READY",
        dir = shell_quote(REMOTE_DIR),
        jar = shell_quote(REMOTE_OTA_JAR),
    );
    if let Ok(out) = adb.script(&device.transport, &check_cmd) {
        if out.trim() == "READY" {
            mark_ota_jar_cached(&device.transport);
            return Ok(());
        }
    }
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(PROBE)?;
    file.as_file().sync_all()?;
    adb.transfer(
        &device.transport,
        "push",
        file.path(),
        Path::new(REMOTE_OTA_JAR),
    )?;
    adb.script(
        &device.transport,
        &format!("chmod 600 {}", shell_quote(REMOTE_OTA_JAR)),
    )?;
    mark_ota_jar_cached(&device.transport);
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OtaStatus {
    NotChecked,
    Available,
    Metadata,
    NoNewerReturned,
    DeviceContextMissing,
    AccessDenied,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VersionRelation {
    Current,
    Newer,
    Older,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Firmware {
    pub slot: String,
    pub version: String,
    pub system_version: String,
    pub release_date: String,
    pub channel_label: String,
    pub release_notes: String,
    pub size_bytes: u64,
    pub need_update: bool,
    pub exists_update: bool,
    pub mandatory_clear: bool,
    pub optional_clear: bool,
    pub mandatory_upgrade: bool,
    pub package_type: Option<i64>,
    pub installation: Option<i64>,
    pub download_condition: Option<i64>,
    pub upgrade_condition: Option<i64>,
    pub check_type: Option<i64>,
    pub relation: VersionRelation,
    /// HTTPS URL returned by the official response. Excluded from exported diagnostics.
    #[serde(default, skip_serializing)]
    pub(crate) download_url: Option<String>,
    pub sha256: Option<String>,
    pub md5: Option<String>,
}
impl Firmware {
    pub fn has_download(&self) -> bool {
        self.download_url.is_some()
    }
}
impl std::fmt::Debug for Firmware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Firmware")
            .field("version", &self.version)
            .field("slot", &self.slot)
            .field("relation", &self.relation)
            .field("size_bytes", &self.size_bytes)
            .field("has_download", &self.download_url.is_some())
            .finish()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleasePlan {
    pub status: i64,
    pub publish_time_ms: Option<i64>,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelReport {
    pub channel: String,
    pub status: OtaStatus,
    pub endpoint: String,
    pub http_status: Option<u16>,
    pub server_code: Option<i64>,
    pub message: String,
    pub releases: Vec<Firmware>,
    pub plan: Option<ReleasePlan>,
}
impl ChannelReport {
    pub fn label(&self) -> &str {
        match self.channel.as_str() {
            "current" => "当前渠道",
            "stable" => "稳定版",
            "beta" => "测试版",
            "daily" => "每日构建",
            _ => &self.channel,
        }
    }
    pub fn status_label(&self) -> &'static str {
        match self.status {
            OtaStatus::NotChecked => "尚未检查",
            OtaStatus::Available => "服务器返回可升级候选",
            OtaStatus::Metadata => "发现版本元数据",
            OtaStatus::NoNewerReturned => "本次未返回更高版本",
            OtaStatus::DeviceContextMissing => "查询权限受限",
            OtaStatus::AccessDenied => "无访问资格",
            OtaStatus::Failed => "检查失败",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstallStatus {
    pub internal_permission: Option<bool>,
    pub shop_demo: Option<bool>,
    pub status: Option<i64>,
    pub channel: Option<String>,
    pub mask_id: Option<String>,
    pub state_readable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OtaReport {
    pub serial: String,
    pub current_version: String,
    pub model: String,
    pub environment: String,
    pub checked_at: String,
    pub identity_complete: bool,
    pub identity_fields: BTreeMap<String, bool>,
    /// The official updater itself returned a matching newer candidate for this phone.
    /// This does not bypass the updater's package, data-loss or installation checks.
    pub eligibility_verified: bool,
    pub root_parameter: Option<i64>,
    pub device_state: Option<i64>,
    pub install_status: InstallStatus,
    pub channels: Vec<ChannelReport>,
    pub limitation: String,
}

pub fn check(adb: &Adb, device: &Device) -> Result<OtaReport> {
    check_with_progress(adb, device, |_| {})
}
pub fn check_with_progress(
    adb: &Adb,
    device: &Device,
    mut progress: impl FnMut(&str),
) -> Result<OtaReport> {
    let payload = run_probe(adb, device, &mut progress, false)?;
    parse_report(&payload, &device.serial)
}
/// User-triggered fallback: opens the official phone updater, then reads only fresh
/// logs from that package's PID. It never presses controls, changes a channel or installs.
pub fn check_official(
    adb: &Adb,
    device: &Device,
    mut progress: impl FnMut(&str),
) -> Result<OtaReport> {
    let payload = run_probe(adb, device, &mut progress, true)?;
    let mut context: Value = serde_json::from_str(&payload)?;
    ensure!(context["error"].is_null(), "无法读取手机更新组件");
    let stamp = adb.shell(&device.transport, &["date", "+%m-%d %H:%M:%S.%3N"])?;
    let stamp = stamp.trim();
    ensure!(
        stamp.len() == 18
            && stamp
                .chars()
                .all(|c| c.is_ascii_digit() || "- :.".contains(c)),
        "手机时间格式不受支持"
    );
    progress("正在打开手机官方更新界面，由系统更新器检查…");
    let launched = adb.shell(
        &device.transport,
        &[
            "am",
            "start",
            "-W",
            "-f",
            "0x14000000",
            "-a",
            "stepos.settings.SYSTEM_UPDATE_SETTINGS",
            "-n",
            "com.stepos.update/.SystemUpgradeListActivity",
        ],
    )?;
    ensure!(launched.contains("Status: ok"), "手机官方更新界面未能启动");
    let deadline = std::time::Instant::now() + Duration::from_secs(35);
    let stamp_q = shell_quote(stamp);
    let poll_cmd = format!(
        "set -- $(pidof com.stepos.update 2>/dev/null); pid=$1; \
         if [ -z \"$pid\" ]; then printf 'STEPX_OTA_NO_PID\\n'; exit 0; fi; \
         case \"$pid\" in *[!0-9]*) printf 'STEPX_OTA_BAD_PID\\n'; exit 0 ;; esac; \
         logcat -d -v brief -T {stamp_q} --pid=\"$pid\" StepOSUpdate:V *:S"
    );
    loop {
        let logs = adb
            .run_timeout(
                &["-s", &device.transport, "shell", &poll_cmd],
                Duration::from_secs(8),
            )
            .map_err(|_| anyhow::anyhow!("无法读取官方更新器本次检查结果"))?;
        if logs.lines().any(|line| line.trim() == "STEPX_OTA_NO_PID") {
            anyhow::bail!("官方更新进程没有启动");
        }
        if logs.lines().any(|line| line.trim() == "STEPX_OTA_BAD_PID") {
            anyhow::bail!("官方更新进程标识无效");
        }
        if let Some(channel) = official_log_result(&logs)? {
            let mut channels = vec![channel.clone()];
            for name in ["stable", "beta", "daily"] {
                channels
                    .push(serde_json::json!({"channel":name,"error":"not_checked","releases":[]}));
            }
            context["channels"] = Value::Array(channels);
            let mut report = parse_report(&context.to_string(), &device.serial)?;
            for channel in report.channels.iter_mut().skip(1) {
                channel.message =
                    "此渠道尚未通过官方更新器检查；可在手机更新器查看可申请的版本".into();
            }
            let current = &mut report.channels[0];
            if let Some(state) = channel["official_summary"].as_str() {
                current.status = if state == "no_new" {
                    OtaStatus::NoNewerReturned
                } else {
                    OtaStatus::Metadata
                };
                current.message=match state {
                    "no_new"=>"手机官方更新器已完成本次检查，当前渠道未返回升级候选；不代表不存在未发布版本",
                    "candidate"=>"手机官方更新器返回了候选，但未开放完整版本与下载元数据，请在手机官方界面查看",
                    _=>"手机官方更新器返回了发布计划，但未开放详细信息，请在手机官方界面查看",
                }.into();
            } else if current.server_code == Some(200)
                && current.releases.iter().any(|r| {
                    r.slot == "new"
                        && r.exists_update
                        && r.need_update
                        && r.relation == VersionRelation::Newer
                })
            {
                report.eligibility_verified = true;
                current.status = OtaStatus::Available;
                current.message =
                    "手机官方更新器为本机返回了更高版本；安装前仍由系统核验兼容性与安全条件".into();
            }
            report.limitation="结果仅来自本次打开手机官方更新器后、该进程的新鲜检查日志。未读取受保护设备标识，未改渠道报名或权限。安装仍由手机系统完成；本次未返回新版本不能证明不存在未发布构建。".into();
            progress("已读取手机官方更新器的本次检查结果");
            return Ok(report);
        }
        ensure!(std::time::Instant::now()<deadline,"官方更新器未在35秒内返回可读取的新鲜结果；请在手机确认网络、隐私许可或点检查更新后重试。没有使用历史缓存");
        std::thread::sleep(Duration::from_millis(650));
    }
}
fn json_object(value: &Value) -> Option<Value> {
    if value.is_object() {
        Some(value.clone())
    } else {
        value
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .filter(Value::is_object)
    }
}
fn official_log_result(logs: &str) -> Result<Option<Value>> {
    // A successful unrelated notice/marketing check is not an OTA check.
    let Some(start) = logs.rfind("OtaUpdateChecker: startCheck type=fw_upgrade_check") else {
        return Ok(None);
    };
    let logs = &logs[start..];
    ensure!(
        !logs.contains("OtaUpdateChecker: startCheck but privacy disallowed"),
        "手机更新器尚未获得隐私许可，请在手机官方界面确认后重试"
    );
    ensure!(
        !logs.contains("OtaUpdateChecker: onErrorResponse:"),
        "手机官方更新器本次请求失败，请检查手机网络后重试"
    );
    let finish = "onCheckEnd : checkType = fw_upgrade_check state = 1 errorCode = 0";
    if !logs.contains(finish) {
        ensure!(
            !logs.contains("onCheckEnd : checkType = fw_upgrade_check state = 0"),
            "手机官方更新器检查失败，尚未确认可用版本"
        );
        return Ok(None);
    }
    for line in logs.lines() {
        if let Some((_, body)) = line.split_once("OtaUpdateChecker: response: ") {
            let Ok(envelope) = serde_json::from_str::<Value>(body) else {
                continue;
            };
            let reply = json_object(&envelope["reply"]).unwrap_or(envelope);
            if reply["code"] != 200 {
                continue;
            }
            let Some(value) = json_object(&reply["value"]) else {
                continue;
            };
            let mut releases = Vec::new();
            for slot in ["new", "cur"] {
                if let Some(mut item) = json_object(&value[slot]) {
                    item["slot"] = slot.into();
                    item["download_url"] = item["updateUrl"].clone();
                    releases.push(item);
                }
            }
            if value.get("new").is_none() && value.get("cur").is_none() {
                continue;
            }
            return Ok(Some(
                serde_json::json!({"channel":"current","server_code":200,"releases":releases,"plan":json_object(&value["plan"])}),
            ));
        }
    }
    // Release builds suppress response JSON. Null/non-null flags are still explicit
    // evidence; never interpret the updater's misleading "have new version" log.
    if logs.contains("OtaUpdateChecker: response code = 200,") {
        let state = if logs.contains("BaseOtaUpdateChecker: upgradeFirmware: is null")
            && logs.contains("BaseOtaUpdateChecker: firmwarePlan: is null")
        {
            Some("no_new")
        } else if logs.contains("BaseOtaUpdateChecker: upgradeFirmware: not null") {
            Some("candidate")
        } else if logs.contains("BaseOtaUpdateChecker: firmwarePlan: not null") {
            Some("plan")
        } else {
            None
        };
        if let Some(state) = state {
            return Ok(Some(
                serde_json::json!({"channel":"current","server_code":200,"releases":[],"official_summary":state}),
            ));
        }
    }
    Ok(None)
}

/// Read only official permission and update state. Never opens the phone UI.
pub fn install_status(adb: &Adb, device: &Device) -> Result<InstallStatus> {
    let payload = run_probe(adb, device, &mut |_| {}, true)?;
    let data: Value = serde_json::from_str(&payload)?;
    ensure!(data["error"].is_null(), "手机更新状态读取失败");
    Ok(parse_install_status(&data["install_status"]))
}
fn run_probe_once(adb: &Adb, device: &Device, context_only: bool) -> Result<String> {
    let remote = format!("/data/local/tmp/stepx-ota-{:032x}", rand::random::<u128>());
    let remote_q = shell_quote(&remote);
    let jar_q = shell_quote(REMOTE_OTA_JAR);
    let ctx_arg = if context_only { " --context" } else { "" };
    let command = format!(
        "apk=$(pm path com.stepos.update | sed -n 's/^package://p' | head -n 1); \
         if [ -z \"$apk\" ]; then printf 'STEPX_OTA_NO_UPDATER\\n'; exit 0; fi; \
         case \"$apk\" in /system/*|/product/*|/system_ext/*|/data/app/*) ;; *) printf 'STEPX_OTA_BAD_PATH\\n'; exit 0 ;; esac; \
         env=$(settings get secure net_env 2>/dev/null | tr -d '\\r\\n'); \
         case \"$env\" in dev|stg|test) ;; *) env=prod ;; esac; \
         mkdir -m 700 {remote_q} || exit 1; \
         CLASSPATH={jar_q} app_process / OtaProbe \"$apk\" {remote_q} \"$env\"{ctx_arg}; \
         rm -rf {remote_q}"
    );
    let output = adb
        .run_timeout(
            &["-s", &device.transport, "shell", &command],
            Duration::from_secs(135),
        )
        .context("官方 OTA 检查超时或手机连接中断，请重试")?;
    if output
        .lines()
        .any(|line| line.trim() == "STEPX_OTA_NO_UPDATER")
    {
        anyhow::bail!("手机未安装受支持的官方更新组件");
    }
    if output
        .lines()
        .any(|line| line.trim() == "STEPX_OTA_BAD_PATH")
    {
        anyhow::bail!("更新组件路径不受支持");
    }
    let payload = output
        .lines()
        .find_map(|line| line.strip_prefix("STEPX_OTA_RESULT:"))
        .context("手机更新检测组件没有返回结果")?;
    Ok(payload.to_owned())
}

fn run_probe(
    adb: &Adb,
    device: &Device,
    progress: &mut impl FnMut(&str),
    context_only: bool,
) -> Result<String> {
    device.verify(adb)?;
    progress("正在读取手机系统版本与官方更新组件…");
    let was_cached = is_ota_jar_cached(&device.transport);
    ensure_ota_jar(adb, device)?;
    progress(if context_only {
        "正在读取官方更新组件权限与状态…"
    } else {
        "正在查询官方当前渠道、稳定版、测试版和每日构建…"
    });
    let payload = match run_probe_once(adb, device, context_only) {
        Ok(payload) => payload,
        Err(err)
            if was_cached
                && err.to_string() != "手机未安装受支持的官方更新组件"
                && err.to_string() != "更新组件路径不受支持" =>
        {
            invalidate_ota_cache(&device.transport);
            ensure_ota_jar(adb, device)?;
            run_probe_once(adb, device, context_only)?
        }
        Err(err) => return Err(err),
    };
    progress("正在核对返回版本与本机系统…");
    Ok(payload)
}

fn safe_text(text: &str, limit: usize) -> String {
    let mut plain = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' && in_tag {
            in_tag = false;
            plain.push(' ');
            continue;
        }
        if !in_tag && (!c.is_control() || c == '\n' || c == '\t') {
            plain.push(c)
        }
    }
    plain
        .split_whitespace()
        .map(|word| {
            if word.contains("https://") || word.contains("http://") {
                "[链接]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}
fn string(value: &Value, key: &str) -> String {
    safe_text(value[key].as_str().unwrap_or(""), 1024)
}
fn hex_digest(value: &Value, key: &str, len: usize) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|s| s.to_ascii_lowercase())
}
fn download_url(value: &Value) -> Option<String> {
    value["download_url"]
        .as_str()
        .filter(|s| {
            s.starts_with("https://")
                && !s.chars().any(|c| c.is_control() || c.is_whitespace())
                && s.len() <= 8192
        })
        .map(str::to_owned)
}
fn build_key(version: &str) -> Option<(Vec<u32>, u64, u64)> {
    let prefix = version
        .strip_prefix("Step AOS ")
        .or_else(|| version.strip_prefix("StepAOS "))?;
    let parts: Vec<_> = prefix.split('_').collect();
    if parts.len() < 3 {
        return None;
    }
    let numeric = parts[0]
        .split('.')
        .take(4)
        .map(str::parse)
        .collect::<std::result::Result<Vec<u32>, _>>()
        .ok()?;
    if numeric.len() != 4 {
        return None;
    }
    let date = parts[parts.len() - 2];
    let time = parts[parts.len() - 1];
    if date.len() != 8
        || time.len() != 6
        || !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    chrono::NaiveDateTime::parse_from_str(&format!("{date}{time}"), "%Y%m%d%H%M%S").ok()?;
    Some((numeric, date.parse().ok()?, time.parse().ok()?))
}
pub fn version_relation(remote: &str, current: &str) -> VersionRelation {
    if remote == current && !remote.is_empty() {
        return VersionRelation::Current;
    }
    match (build_key(remote), build_key(current)) {
        (Some(a), Some(b)) => match a.cmp(&b) {
            Ordering::Greater => VersionRelation::Newer,
            Ordering::Less => VersionRelation::Older,
            Ordering::Equal => VersionRelation::Unknown,
        },
        _ => VersionRelation::Unknown,
    }
}
fn parse_install_status(data: &Value) -> InstallStatus {
    InstallStatus {
        internal_permission: data["internal_permission"].as_bool(),
        shop_demo: data["shop_demo"].as_bool(),
        status: data["system_update"]["status"].as_i64(),
        channel: data["system_update"]["channel"]
            .as_str()
            .map(|s| safe_text(s, 80)),
        mask_id: data["system_update"]["mask_id"]
            .as_str()
            .map(|s| safe_text(s, 256)),
        state_readable: data["system_update"].is_object(),
    }
}
fn endpoint(raw: &Value) -> String {
    let s = raw["endpoint"].as_str().unwrap_or("");
    for domain in [
        "stepaos.com",
        "d-stepos.com",
        "s-stepos.com",
        "t-stepos.com",
    ] {
        for host in ["upush", "u.in"] {
            for path in ["/v4/firmware/check", "/v4/firmware/switchover"] {
                if s == format!("https://{host}.{domain}{path}") {
                    return s.into();
                }
            }
        }
    }
    String::new()
}
fn parse_report(payload: &str, serial: &str) -> Result<OtaReport> {
    let data: Value = serde_json::from_str(payload).context("官方 OTA 结果格式无效")?;
    ensure!(data["error"].is_null(), "手机官方更新组件无法完成后台查询");
    ensure!(data["protocol"] == 1, "OTA 查询组件版本不兼容");
    let current = string(&data, "current_version");
    ensure!(!current.is_empty(), "手机没有返回当前系统版本");
    let identity_complete = data["identity_complete"].as_bool() == Some(true);
    let mut channels = Vec::new();
    for raw in data["channels"]
        .as_array()
        .context("OTA 结果没有渠道信息")?
    {
        let channel = string(raw, "channel");
        ensure!(
            ["current", "stable", "beta", "daily"].contains(&channel.as_str()),
            "OTA 返回未知渠道"
        );
        let server_code = raw["server_code"].as_i64();
        let http_status = raw["http_status"]
            .as_u64()
            .and_then(|s| u16::try_from(s).ok());
        let mut releases = Vec::new();
        for r in raw["releases"].as_array().into_iter().flatten() {
            let version = string(r, "latestVersion");
            if version.is_empty() {
                continue;
            }
            let notes = r["markDownText"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| r["releaseNote"].as_str())
                .unwrap_or("");
            releases.push(Firmware {
                slot: string(r, "slot"),
                relation: version_relation(&version, &current),
                version,
                system_version: string(r, "systemVersion"),
                release_date: string(r, "releaseDate"),
                channel_label: string(r, "verType"),
                release_notes: safe_text(notes, 8192),
                size_bytes: r["size"].as_u64().unwrap_or(0),
                need_update: r["needUpdate"] == true,
                exists_update: r["existsUpdate"] == true,
                mandatory_clear: r["mandatoryClear"] == true,
                optional_clear: r["optionalClear"] == true,
                mandatory_upgrade: r["mandatoryUpgrade"] == true,
                package_type: r["packageType"].as_i64(),
                installation: r["installation"].as_i64(),
                download_condition: r["downloadCondition"].as_i64(),
                upgrade_condition: r["upgradeCondition"].as_i64(),
                check_type: r["checkType"].as_i64(),
                download_url: download_url(r),
                sha256: hex_digest(r, "digestV2", 64),
                md5: hex_digest(r, "digest", 32),
            });
        }
        let plan = raw
            .get("plan")
            .filter(|p| p.is_object())
            .map(|p| ReleasePlan {
                status: p["status"].as_i64().unwrap_or(-1),
                publish_time_ms: p["planPublishTime"].as_i64().filter(|t| *t > 0),
                note: safe_text(p["planPublicTimeoutTips"].as_str().unwrap_or(""), 1024),
            });
        let (status, message) = if raw["error"] == "not_checked" {
            (OtaStatus::NotChecked, "此渠道尚未检查".into())
        } else if matches!(http_status, Some(401 | 403)) {
            (OtaStatus::AccessDenied, "官方服务拒绝访问此渠道".into())
        } else if server_code == Some(110001) && !identity_complete {
            (
                OtaStatus::DeviceContextMissing,
                "官方服务要求完整设备标识；当前系统没有向后台查询组件提供 DSID，未确认升级资格"
                    .into(),
            )
        } else if raw["error"].is_string() {
            (
                OtaStatus::Failed,
                match raw["error"].as_str().unwrap() {
                    "timeout" => "官方服务响应超时",
                    "malformed_response" => "官方服务返回的数据不完整或格式无效",
                    "http_error" => "官方服务返回 HTTP 错误",
                    _ => "无法连接官方服务",
                }
                .into(),
            )
        } else if server_code != Some(200) {
            (
                OtaStatus::Failed,
                format!(
                    "官方服务返回业务状态 {}：{}",
                    server_code.map(|v| v.to_string()).unwrap_or("未知".into()),
                    string(raw, "server_message")
                ),
            )
        } else if releases.iter().any(|r| {
            r.slot == "new"
                && r.exists_update
                && r.need_update
                && r.relation == VersionRelation::Newer
        }) && identity_complete
        {
            (
                OtaStatus::Available,
                "官方服务返回比本机更新的候选版本，安装资格仍由手机更新程序核验".into(),
            )
        } else if releases
            .iter()
            .any(|r| r.relation != VersionRelation::Current)
            || plan.is_some()
        {
            (
                OtaStatus::Metadata,
                "已读取官方版本或发布计划元数据，不代表当前手机可安装".into(),
            )
        } else {
            (
                OtaStatus::NoNewerReturned,
                "官方服务本次未返回更高版本；这不能证明不存在尚未公开的构建".into(),
            )
        };
        channels.push(ChannelReport {
            channel,
            status,
            endpoint: endpoint(raw),
            http_status,
            server_code,
            message,
            releases,
            plan,
        });
    }
    ensure!(channels.len() == 4, "OTA 渠道结果不完整");
    ensure!(
        channels
            .iter()
            .map(|c| c.channel.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == 4,
        "OTA 返回重复渠道"
    );
    Ok(OtaReport{serial:serial.into(),current_version:current,model:string(&data,"model"),environment:string(&data,"environment"),checked_at:chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),identity_complete,identity_fields:serde_json::from_value(data["identity_fields"].clone()).unwrap_or_default(),eligibility_verified:false,root_parameter:data["root_parameter"].as_i64(),device_state:data["device_state"].as_i64(),install_status:parse_install_status(&data["install_status"]),channels,limitation:"仅查询官方客户端已有的当前环境与已知渠道；未改设备标识、系统版本或渠道报名。系统完整性和最终升级资格需由手机官方更新程序核验。服务未返回候选不能证明不存在未发布版本。".into()})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_actual_build_identifiers() {
        let current = "Step AOS 1.0.0.2.26001_stable_user_20261004_131555";
        assert_eq!(version_relation(current, current), VersionRelation::Current);
        assert_eq!(
            version_relation(
                "Step AOS 1.0.0.3.26001_stable_user_20261020_131555",
                current
            ),
            VersionRelation::Newer
        );
        assert_eq!(
            version_relation(
                "Step AOS 1.0.0.1.26001_stable_user_20260904_131555",
                current
            ),
            VersionRelation::Older
        );
        assert_eq!(
            version_relation("internal-experiment", current),
            VersionRelation::Unknown
        );
    }
    #[test]
    fn preserves_unavailable_channels_without_claiming_latest() {
        let value = serde_json::json!({"protocol":1,"current_version":"current","identity_complete":false,"identity_fields":{"oaid":true,"dsid":false},"channels":(["current","stable","beta","daily"].map(|channel|serde_json::json!({"channel":channel,"http_status":200,"server_code":110001,"releases":[]})))});
        let report = parse_report(&value.to_string(), "device").unwrap();
        assert!(report
            .channels
            .iter()
            .all(|c| c.status == OtaStatus::DeviceContextMissing));
        assert!(!report.eligibility_verified);
    }
    #[test]
    fn official_check_requires_fresh_correct_success_sequence() {
        let response="OtaUpdateChecker: response code = 200, msg = \nBaseOtaUpdateChecker: upgradeFirmware: is null\nBaseOtaUpdateChecker: firmwarePlan: is null\nOtaUpdateChecker: have new version\nSystemUpdatePresenter: onCheckEnd : checkType = fw_upgrade_check state = 1 errorCode = 0";
        assert!(official_log_result(response).unwrap().is_none());
        let fresh = format!("OtaUpdateChecker: startCheck type=fw_upgrade_check\n{response}");
        assert_eq!(
            official_log_result(&fresh).unwrap().unwrap()["official_summary"],
            "no_new"
        );
        assert!(official_log_result(&fresh.replace(
            "checkType = fw_upgrade_check state",
            "checkType = notice_check_list state"
        ))
        .unwrap()
        .is_none());
        assert!(official_log_result("OtaUpdateChecker: startCheck type=fw_upgrade_check\nOtaUpdateChecker: have new version").unwrap().is_none());
    }
    #[test]
    fn official_check_preserves_candidates_and_blocks_failures() {
        let source = serde_json::json!({"code":200,"value":{"new":{"latestVersion":"Step AOS 1.0.0.3.26001_stable_user_20261004_131555","updateUrl":"https://ota-firmware.stepaos.com/firmware.zip?secret=hidden","existsUpdate":true,"needUpdate":true},"cur":null,"plan":null}});
        let log=format!("OtaUpdateChecker: startCheck type=fw_upgrade_check\nOtaUpdateChecker: response: {source}\nonCheckEnd : checkType = fw_upgrade_check state = 1 errorCode = 0");
        let result = official_log_result(&log).unwrap().unwrap();
        assert_eq!(result["releases"][0]["slot"], "new");
        assert!(result["releases"][0]["download_url"]
            .as_str()
            .unwrap()
            .contains("hidden"));
        assert!(official_log_result("OtaUpdateChecker: startCheck type=fw_upgrade_check\nOtaUpdateChecker: onErrorResponse: private-detail").unwrap_err().to_string().find("private-detail").is_none());
    }
    #[test]
    fn compares_same_product_version_by_real_build_date() {
        let current = "Step AOS 1.0.0.2.26001_stable_user_20261004_131555";
        assert_eq!(
            version_relation(
                "Step AOS 1.0.0.2.26002_stable_user_20260904_131555",
                current
            ),
            VersionRelation::Older
        );
        assert_eq!(
            version_relation(
                "Step AOS 1.0.0.3.26001_stable_user_20261344_131555",
                current
            ),
            VersionRelation::Unknown
        );
    }
    #[test]
    fn metadata_flags_and_download_secrets_are_preserved_privately() {
        let release = serde_json::json!({"slot":"new","latestVersion":"Step AOS 1.0.0.3.26001_stable_user_20261004_131555","existsUpdate":false,"needUpdate":true,"mandatoryClear":true,"download_url":"https://ota-firmware.stepaos.com/file.zip?secret=hidden","digestV2":"a".repeat(64)});
        let data = serde_json::json!({"protocol":1,"current_version":"Step AOS 1.0.0.2.26001_stable_user_20261004_131555","identity_complete":true,"channels":(["current","stable","beta","daily"].map(|channel|serde_json::json!({"channel":channel,"server_code":200,"releases":[release.clone()]})))});
        let report = parse_report(&data.to_string(), "test").unwrap();
        assert_eq!(report.channels[0].status, OtaStatus::Metadata);
        assert!(report.channels[0].releases[0].mandatory_clear);
        assert!(report.channels[0].releases[0].has_download());
        assert!(!serde_json::to_string(&report).unwrap().contains("hidden"));
        assert!(!format!("{report:?}").contains("hidden"));
    }
    #[test]
    fn strips_html_and_urls_from_release_notes() {
        assert_eq!(
            safe_text("<p>Hello</p> https://example.com/?token=secret", 100),
            "Hello [链接]"
        );
    }
}

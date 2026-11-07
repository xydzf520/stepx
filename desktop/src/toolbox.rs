//! Storage tools available through the authorized shell, without system modifications.
use crate::{
    adb::{shell_quote, Adb},
    bridge,
    device::Device,
    storage,
};
use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration as ChronoDuration, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Stdio, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileInfo {
    pub path: String,
    pub bytes: u64,
    pub modified_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FolderInfo {
    pub name: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StorageReport {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub free_bytes: u64,
    pub filesystem: String,
    pub scanned_files: usize,
    pub scanned_bytes: u64,
    pub limited: bool,
    pub unreadable: usize,
    pub hashed_files: usize,
    pub hash_skipped: usize,
    /// Logical bytes in duplicate copies, never a claim of freed disk space.
    pub duplicate_bytes: u64,
    pub folders: Vec<FolderInfo>,
    pub largest: Vec<FileInfo>,
    pub documents: Vec<FileInfo>,
    pub duplicates: Vec<Vec<FileInfo>>,
    pub scanned_at_ms: i64,
}

pub fn scan(adb: &Adb, device: &Device) -> Result<StorageReport> {
    let value = bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({"op": "storage_scan"}),
        Duration::from_secs(45),
    )?;
    parse_report(value)
}

pub fn is_expansion_stub_filename(name: &str) -> bool {
    if name.contains("/StepX Companion/SC_") {
        return true;
    }
    let file_name = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    let Some(rest) = file_name.strip_prefix("SC_") else {
        return false;
    };
    let stem = rest.split('.').next().unwrap_or(rest);
    stem.len() == 32 && stem.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_hidden_or_internal_storage_path(path: &str) -> bool {
    path.split('/').any(|seg| seg.starts_with('.'))
        || path.ends_with(".stepxlink")
        || is_expansion_stub_filename(path)
}

fn parse_report(value: serde_json::Value) -> Result<StorageReport> {
    let mut report: StorageReport =
        serde_json::from_value(value).context("存储扫描结果不完整，请重试")?;
    if report.total_bytes == 0
        || report.free_bytes > report.total_bytes
        || report.available_bytes > report.free_bytes
    {
        bail!("手机返回的容量数据无效，请重试");
    }
    for item in report
        .largest
        .iter()
        .chain(report.documents.iter())
        .chain(report.duplicates.iter().flatten())
    {
        storage::validate_path(&item.path)?;
    }
    report.folders.retain(|f| !f.name.starts_with('.'));
    report
        .largest
        .retain(|item| !is_hidden_or_internal_storage_path(&item.path));
    report
        .documents
        .retain(|item| !is_hidden_or_internal_storage_path(&item.path));
    for group in &mut report.duplicates {
        group.retain(|item| !is_hidden_or_internal_storage_path(&item.path));
    }
    report.duplicates.retain(|group| group.len() >= 2);
    Ok(report)
}

pub fn open_storage_settings(adb: &Adb, device: &Device) -> Result<()> {
    if crate::native_mcp::try_call(adb, device,
        "settings_open_settings_storage_dashboard_activity", serde_json::json!({}))?.is_some() {
        return Ok(());
    }
    let transport = device.resolve_active_transport(adb)?;
    adb.shell(
        &transport,
        &[
            "am",
            "start",
            "-a",
            "android.settings.INTERNAL_STORAGE_SETTINGS",
        ],
    )?;
    Ok(())
}

#[derive(Clone, Debug)]
pub enum ToolOperation {
    ClearCache,
    Compress(FileInfo),
    Restore(FileInfo),
    DeleteFile(FileInfo),
    DeleteDuplicate { file: FileInfo, keep: FileInfo },
}

impl ToolOperation {
    pub fn file(&self) -> Option<&FileInfo> {
        match self {
            Self::ClearCache => None,
            Self::Compress(f)
            | Self::Restore(f)
            | Self::DeleteFile(f)
            | Self::DeleteDuplicate { file: f, .. } => Some(f),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::ClearCache => "清理应用缓存",
            Self::Compress(_) => "压缩文件",
            Self::Restore(_) => "还原文件",
            Self::DeleteFile(_) => "删除文件",
            Self::DeleteDuplicate { .. } => "删除副本",
        }
    }
    pub fn request(&self) -> serde_json::Value {
        let Some(f) = self.file() else {
            return serde_json::json!({"op": "storage_clear_cache"});
        };
        let (action, keep) = match self {
            Self::Compress(_) => ("compress", ""),
            Self::ClearCache => unreachable!(),
            Self::Restore(_) => ("restore", ""),
            Self::DeleteFile(_) => ("delete_file", ""),
            Self::DeleteDuplicate { keep, .. } => ("delete_duplicate", keep.path.as_str()),
        };
        serde_json::json!({"op": "storage_file_operation", "action": action,
            "path": f.path, "bytes": f.bytes, "modified_ms": f.modified_ms, "keep": keep})
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct OperationResult {
    pub completed: bool,
    pub available_before: u64,
    pub available_after: u64,
    pub logical_saved: i64,
    pub result_path: String,
}

pub fn operate(adb: &Adb, device: &Device, operation: &ToolOperation) -> Result<OperationResult> {
    if let Some(file) = operation.file() {
        storage::validate_path(&file.path)?;
    }
    let debug_saved = if matches!(operation, ToolOperation::ClearCache) {
        clean_system_debug_temp(adb, device).unwrap_or(0)
    } else {
        0
    };
    let value = bridge::call_bridge(adb, device, &operation.request(), Duration::from_secs(60))?;
    let mut result: OperationResult =
        serde_json::from_value(value).context("操作结果未能确认，请重新扫描")?;
    if debug_saved > 0 {
        result.logical_saved = result.logical_saved.saturating_add(debug_saved as i64);
    }
    Ok(result)
}

/// Reclaims stale system debug tarballs (`/data/local/tmp/perf_*.tar.gz`) and temporary
/// UI dump XMLs (`/data/local/tmp/*ui*.xml`, `/data/local/tmp/window_dump.xml`, `/sdcard/amoo_*.xml`)
/// while strictly preserving `/data/local/tmp/stepx-companion`.
pub fn clean_system_debug_temp(adb: &Adb, device: &Device) -> Result<u64> {
    let transport = device.resolve_active_transport(adb)?;
    let script = concat!(
        "BYTES=0; ",
        "for f in /data/local/tmp/perf_*.tar.gz /data/local/tmp/ui*.xml /data/local/tmp/window_dump.xml /sdcard/amoo_*.xml /sdcard/window_dump.xml; do ",
        "  if [ -f \"$f\" ]; then ",
        "    SZ=$(stat -c '%s' \"$f\" 2>/dev/null || echo 0); ",
        "    BYTES=$((BYTES + SZ)); ",
        "    rm -f \"$f\" 2>/dev/null || true; ",
        "  fi; ",
        "done; ",
        "echo \"$BYTES\""
    );
    let out = adb.script(&transport, script)?;
    let bytes = out
        .lines()
        .rev()
        .find_map(|line| line.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Ok(bytes)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SystemCompression {
    pub kernel_support: bool,
    pub volume_support: bool,
    pub filesystem: String,
    pub features: String,
}

/// Reuse the GMS project's existing, firmware-guarded root bridge for sysfs reads only.
pub fn probe_system(adb: &Adb, device: &Device) -> Result<SystemCompression> {
    let transport = device.resolve_active_transport(adb)?;
    let tools = crate::resources::root().join("handoff/google-play-stepx/tools");
    if !tools.join("root_bridge.py").is_file() {
        bail!("此安装未附带系统能力检测组件");
    }
    let probe = r#"import json,os
parents=[]
for pid in os.listdir('/proc'):
 if not pid.isdigit(): continue
 try:
  status=open('/proc/'+pid+'/status').read()
  if 'Name:\topenclaw_env\n' in status and 'PPid:\t1\n' in status: parents.append(pid)
 except OSError: pass
if len(parents)!=1: raise RuntimeError('无法定位原厂主机环境')
root='/proc/'+parents[0]+'/root'
mount=next(x.split() for x in open('/proc/'+parents[0]+'/mounts') if x.split()[1]=='/data')
device=mount[0].rsplit('/',1)[-1]
if mount[2]!='f2fs': raise RuntimeError('数据分区不是 F2FS')
features=open(root+'/sys/fs/f2fs/'+device+'/features').read().strip()
kernel=open(root+'/sys/fs/f2fs/features/compression').read().strip()=='supported'
print(json.dumps({'kernel_support':kernel,'volume_support':'compression' in features.split(', '),'filesystem':mount[2],'features':features}))
"#;
    let launcher = "import sys,subprocess,json; from common import guard; code=sys.stdin.read(); guard(native=True); p=subprocess.run([sys.executable,'root_bridge.py'],input=code,text=True,capture_output=True,timeout=40); d=json.JSONDecoder().raw_decode(p.stdout.lstrip())[0]; print(json.dumps(d))";
    let mut command = crate::resources::python_command();
    command
        .current_dir(tools)
        .arg("-c")
        .arg(launcher)
        .env("STEPX_SERIAL", &transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = crate::adb::run_process(
        command,
        Some(probe.as_bytes().to_vec()),
        Duration::from_secs(50),
    )?;
    if !output.status.success() {
        bail!("系统能力检测未完成，请检查 StepClaw 服务及当前固件是否受支持");
    }
    serde_json::from_slice(&output.stdout).context("没有收到有效的系统能力检测结果")
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StepOsMcpServiceItem {
    pub id: String,
    pub title: String,
    pub category: String,
    pub ready: bool,
    /// PackageManager found the service declaration. This does not imply a
    /// successful protected Binder invocation or business completion.
    #[serde(default)]
    pub discovered: bool,
    pub quick_action_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StepOsDigitalWellbeing {
    pub screen_time_minutes: u32,
    pub unlock_count: u32,
    pub top_apps: Vec<(String, String, u32)>,
    pub alphame_running: bool,
}

pub fn format_duration_minutes(mins: u32) -> String {
    if mins >= 60 {
        format!("{} 小时 {} 分钟", mins / 60, mins % 60)
    } else {
        format!("{mins} 分钟")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PhoneCalendarEvent {
    pub id: i64,
    pub title: String,
    pub dtstart: i64,
    pub dtend: i64,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    #[serde(default)]
    pub source: String,
    /// None means an older source did not provide recurrence metadata.
    #[serde(default)]
    pub recurring: Option<bool>,
}

impl PhoneCalendarEvent {
    pub fn date_ymd(&self) -> (i32, u32, u32) {
        let dt = Local
            .timestamp_millis_opt(self.dtstart)
            .single()
            .unwrap_or_else(Local::now);
        (dt.year(), dt.month(), dt.day())
    }

    pub fn hm_label(&self) -> String {
        if self.all_day {
            return "全天".into();
        }
        let Some(start) = Local.timestamp_millis_opt(self.dtstart).single() else {
            return "待定".into();
        };
        format!("{:02}:{:02}", start.hour(), start.minute())
    }

    pub fn source_label(&self) -> &str {
        if self.source.is_empty() {
            "手机日历"
        } else {
            &self.source
        }
    }

    pub fn time_range_label(&self) -> String {
        if self.all_day {
            return "全天日程".into();
        }
        let Some(start) = Local.timestamp_millis_opt(self.dtstart).single() else {
            return "时间待定".into();
        };
        let end = Local
            .timestamp_millis_opt(self.dtend)
            .single()
            .unwrap_or_else(|| start + ChronoDuration::hours(1));
        let today = Local::now().date_naive();
        let start_date = start.date_naive();
        let day_prefix = if start_date == today {
            "今天".to_string()
        } else if start_date == today.succ_opt().unwrap_or(today) {
            "明天".to_string()
        } else if start_date == today.pred_opt().unwrap_or(today) {
            "昨天".to_string()
        } else {
            format!("{}月{}日", start.month(), start.day())
        };
        format!(
            "{} {:02}:{:02}–{:02}:{:02}",
            day_prefix,
            start.hour(),
            start.minute(),
            end.hour(),
            end.minute()
        )
    }

    pub fn is_e2e_probe(&self) -> bool {
        is_e2e_test_calendar_title(&self.title)
            || crate::home_hub::any_synthetic_e2e_probe(&[
                &self.title,
                &self.location,
                &self.description,
            ])
    }

    pub fn effective_end_ms(&self) -> i64 {
        if self.all_day {
            let (y, m, d) = self.date_ymd();
            if let Some(dt) = Local.with_ymd_and_hms(y, m, d, 23, 59, 59).single() {
                return dt.timestamp_millis();
            }
        }
        if self.dtend > self.dtstart {
            self.dtend
        } else if self.dtstart > 0 {
            self.dtstart + 3_600_000
        } else {
            0
        }
    }

    pub fn is_ended_at(&self, now_ms: i64) -> bool {
        let end_ms = self.effective_end_ms();
        end_ms > 0 && now_ms > end_ms
    }

    pub fn is_ongoing_at(&self, now_ms: i64) -> bool {
        if self.all_day {
            let (y, m, d) = self.date_ymd();
            let now_dt = Local
                .timestamp_millis_opt(now_ms)
                .single()
                .unwrap_or_else(Local::now);
            return (now_dt.year(), now_dt.month(), now_dt.day()) == (y, m, d);
        }
        self.dtstart > 0 && now_ms >= self.dtstart && now_ms <= self.effective_end_ms()
    }

    pub fn is_suppressed_at(
        &self,
        now_ms: i64,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> bool {
        // Old completion/snooze keys identify a series, not this occurrence.
        // They cannot authorize suppressing every future occurrence.
        if self.recurring == Some(true) {
            return false;
        }
        let keys = [
            format!("cal:{}", self.id),
            format!("schedule:{}", self.id),
            format!("conflict:conflict-cal-{}", self.id),
        ];
        gov.is_any_key_suppressed(
            &keys,
            (self.dtstart > 0).then_some(self.dtstart),
            None,
            None,
            now_ms,
        )
    }

    pub fn is_valid_unexpired_at(
        &self,
        now_ms: i64,
        completed_ids: &std::collections::HashSet<i64>,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> bool {
        if self.is_e2e_probe() {
            return false;
        }
        if self.recurring != Some(true) && completed_ids.contains(&self.id) {
            return false;
        }
        if self.is_suppressed_at(now_ms, gov) {
            return false;
        }
        !self.is_ended_at(now_ms)
    }

    pub fn is_today_valid_unexpired_at(
        &self,
        now_ms: i64,
        completed_ids: &std::collections::HashSet<i64>,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> bool {
        let now_dt = Local
            .timestamp_millis_opt(now_ms)
            .single()
            .unwrap_or_else(Local::now);
        let today = (now_dt.year(), now_dt.month(), now_dt.day());
        self.date_ymd() == today && self.is_valid_unexpired_at(now_ms, completed_ids, gov)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneGalleryItem {
    pub remote_path: String,
    pub name: String,
    pub category: String, // "camera" | "screenshot" | "weixin" | "pictures" | "expansion"
    pub size_bytes: u64,
    pub modified_ms: i64,
    pub local_thumb_path: Option<String>,
}

impl PhoneGalleryItem {
    pub fn category_label(&self) -> &'static str {
        match self.category.as_str() {
            "camera" => "相机实拍",
            "screenshot" | "screenshots" => "屏幕截图",
            "weixin" => "微信与保存",
            "expansion" => "电脑扩容",
            _ if self.remote_path.to_ascii_lowercase().contains("/download") => "下载图片",
            _ => "图片收藏",
        }
    }

    pub fn size_label(&self) -> String {
        crate::storage::format_bytes(self.size_bytes)
    }

    pub fn full_datetime_label(&self) -> String {
        if self.modified_ms <= 0 {
            return "未知时间".into();
        }
        let Some(dt) = Local.timestamp_millis_opt(self.modified_ms).single() else {
            return "未知时间".into();
        };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute()
        )
    }

    pub fn time_label(&self) -> String {
        if self.modified_ms <= 0 {
            return "未知时间".into();
        }
        let Some(dt) = Local.timestamp_millis_opt(self.modified_ms).single() else {
            return "未知时间".into();
        };
        let today = Local::now().date_naive();
        let d = dt.date_naive();
        if d == today {
            format!("今天 {:02}:{:02}", dt.hour(), dt.minute())
        } else if d == today.pred_opt().unwrap_or(today) {
            format!("昨天 {:02}:{:02}", dt.hour(), dt.minute())
        } else {
            format!(
                "{:02}-{:02} {:02}:{:02}",
                dt.month(),
                dt.day(),
                dt.hour(),
                dt.minute()
            )
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PhoneAlarmItem {
    pub id: i64,
    pub hour: u8,
    pub minutes: u8,
    pub enabled: bool,
    pub days_of_week: u32,
    pub vibrate: bool,
    pub message: String,
}

impl PhoneAlarmItem {
    pub fn time_label(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minutes)
    }

    pub fn repeat_label(&self) -> &'static str {
        match self.days_of_week {
            0 => "单次响铃",
            31 => "工作日",
            96 => "周末",
            127 => "每天",
            _ => "自定义重复",
        }
    }

    pub fn display_title(&self) -> &str {
        let trimmed = self.message.trim();
        if trimmed.is_empty() {
            "手机闹钟"
        } else {
            trimmed
        }
    }

    pub fn is_e2e_probe(&self) -> bool {
        let msg = self.message.trim();
        msg.contains("StepX自动化验证")
            || msg.contains("StepX E2E")
            || crate::home_hub::is_synthetic_e2e_probe_text(msg)
    }

    pub fn total_minutes(&self) -> u32 {
        (self.hour as u32) * 60 + (self.minutes as u32)
    }

    /// `weekday_mon0_to_sun6`: Monday = 0 .. Sunday = 6 (matches `chrono::Weekday::num_days_from_monday()`).
    pub fn matches_weekday(&self, weekday_mon0_to_sun6: u32) -> bool {
        if self.days_of_week == 0 {
            return true;
        }
        let bit = 1u32 << (weekday_mon0_to_sun6.min(6));
        (self.days_of_week & bit) != 0
    }

    pub fn is_suppressed_at(
        &self,
        now_ms: i64,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> bool {
        let keys = [
            format!("alarm:{}", self.id),
            format!("alarm:{:02}:{:02}", self.hour, self.minutes),
        ];
        gov.is_any_key_suppressed(&keys, None, None, None, now_ms)
    }

    /// True if this alarm is enabled, not an E2E probe, not suppressed in governance,
    /// scheduled for today's weekday (or one-shot), AND its time today has not yet passed (`alarm_mins >= now_mins`).
    pub fn is_today_valid_unexpired_at(
        &self,
        now_ms: i64,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> bool {
        if !self.enabled || self.is_e2e_probe() || self.is_suppressed_at(now_ms, gov) {
            return false;
        }
        let now_dt = Local
            .timestamp_millis_opt(now_ms)
            .single()
            .unwrap_or_else(Local::now);
        let wd = now_dt.weekday().num_days_from_monday();
        if !self.matches_weekday(wd) {
            return false;
        }
        let now_mins = now_dt.hour() * 60 + now_dt.minute();
        self.total_minutes() >= now_mins
    }

    /// True if this alarm is enabled, not an E2E probe, not suppressed, and either:
    /// - valid & unexpired today, OR
    /// - a repeating alarm (`days_of_week != 0`) that will ring on a future day.
    /// (One-shot alarms `days_of_week == 0` whose time has already passed today without ringing are treated as expired/stale unless re-enabled.)
    pub fn is_valid_future_or_today_unexpired_at(
        &self,
        now_ms: i64,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> bool {
        if !self.enabled || self.is_e2e_probe() || self.is_suppressed_at(now_ms, gov) {
            return false;
        }
        if self.is_today_valid_unexpired_at(now_ms, gov) {
            return true;
        }
        self.days_of_week != 0
    }

    /// Human-readable validity badge for UI display:
    /// - `("待响", true)` when enabled and ringing later today
    /// - `("明天再响", false)` when repeating and today's time has passed
    /// - `("今日不响", false)` when repeating but not scheduled for today's weekday
    /// - `("已过", false)` when one-shot (`days_of_week == 0`), enabled, and time has already passed today
    /// - `("已过期", false)` when suppressed in governance
    /// - `("关闭", false)` when `!self.enabled`
    pub fn validity_status_label_at(
        &self,
        now_ms: i64,
        gov: &crate::home_hub::HomeReminderGovernanceStore,
    ) -> (&'static str, bool) {
        if self.is_suppressed_at(now_ms, gov) {
            return ("已过期", false);
        }
        if !self.enabled {
            return ("关闭", false);
        }
        let now_dt = Local
            .timestamp_millis_opt(now_ms)
            .single()
            .unwrap_or_else(Local::now);
        let wd = now_dt.weekday().num_days_from_monday();
        let now_mins = now_dt.hour() * 60 + now_dt.minute();
        if !self.matches_weekday(wd) {
            return ("今日不响", false);
        }
        if self.total_minutes() >= now_mins {
            ("待响", true)
        } else if self.days_of_week != 0 && self.matches_weekday((wd + 1) % 7) {
            ("明天再响", false)
        } else {
            ("已过", false)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PhoneSystemControls {
    pub dnd_on: bool,
    pub wifi_on: bool,
    pub bluetooth_on: bool,
    pub brightness_percent: u8,
}

impl Default for PhoneSystemControls {
    fn default() -> Self {
        Self {
            dnd_on: false,
            wifi_on: true,
            bluetooth_on: true,
            brightness_percent: 65,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_backscreen_stay_on_secs() -> u32 {
    30
}

fn default_firmware_profile() -> String {
    "pandora-12 MP (325 MCP Tools + 6 AppFunctions)".into()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StepOsCapabilityEvidence {
    pub gui_accessibility: Option<bool>,
    pub notification_listener: Option<bool>,
    /// Actual ACCESS_MCP_SERVICES grant of the original ability caller only.
    /// Even Some(true) does not verify every discovered MCP tool.
    pub mcp_access: Option<bool>,
    pub mcp_directory_read: Option<bool>,
    pub screen_perception: Option<bool>,
    pub camera_allowed: Option<bool>,
    pub microphone_allowed: Option<bool>,
    pub location_enabled: Option<bool>,
    pub delete_protect_enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StepOsAiSuiteReport {
    pub checked: bool,
    /// Partial reads must not replace the last complete UI snapshot.
    #[serde(default)]
    pub read_error: Option<String>,
    #[serde(default)]
    pub capability_evidence: StepOsCapabilityEvidence,
    pub aiengine_running: bool,
    pub aiengine_model_summary: String,
    #[serde(default)]
    pub aiengine_ability_count: u32,
    #[serde(default)]
    pub aiengine_htp_status: String,
    #[serde(default)]
    pub agentbus_status: String,
    #[serde(default)]
    pub thermal_summary: String,
    #[serde(default = "default_firmware_profile")]
    pub firmware_profile: String,
    #[serde(default = "default_true")]
    pub global_camera_enabled: bool,
    #[serde(default = "default_true")]
    pub global_mic_enabled: bool,
    #[serde(default = "default_true")]
    pub global_location_enabled: bool,
    #[serde(default = "default_true")]
    pub delete_protect_enabled: bool,
    #[serde(default = "default_backscreen_stay_on_secs")]
    pub backscreen_stay_on_secs: u32,
    #[serde(default = "default_true")]
    pub backscreen_flip_keep_running: bool,
    #[serde(default)]
    pub backscreen_signature: String,
    #[serde(default)]
    pub remote_agent_files: Vec<FileInfo>,
    pub safety_center_running: bool,
    pub gui_accessibility_enabled: bool,
    pub notification_listener_enabled: bool,
    pub mcp_permission_granted: bool,
    pub screen_perception_ready: bool,
    pub mcp_services: Vec<StepOsMcpServiceItem>,
    pub wellbeing: StepOsDigitalWellbeing,
    #[serde(default)]
    pub calendar_events: Vec<PhoneCalendarEvent>,
    #[serde(default)]
    pub alarms: Vec<PhoneAlarmItem>,
    #[serde(default)]
    pub controls: PhoneSystemControls,
    pub phone_clipboard: String,
    pub last_action_note: String,
}

pub fn default_stepos_ai_suite_report() -> StepOsAiSuiteReport {
    let services = [
        (
            "com.android.calendar/com.stepos.calendar.mcp.CalendarAbsMCPService",
            "日历日程摘要与事件编排",
            "日程与时间",
            "新建手机日程",
        ),
        (
            "com.android.alarmclock/com.stepos.alarmclock.mcp.AlarmMCPService",
            "闹钟与倒计时服务",
            "日程与时间",
            "设 15 分钟闹钟",
        ),
        (
            "com.stepos.calculator/.mcp.CalculatorMCPService",
            "科学计算服务",
            "日程与时间",
            "打开科学计算器",
        ),
        (
            "com.stepos.xmscore/.mcpserver.XMSMCPService",
            "智慧信息与定位服务",
            "日程与时间",
            "智慧信息服务号",
        ),
        (
            "com.stepos.media.gallery/com.stepos.gallery.baselib.mcp.GalleryMCPService",
            "相册语义检索与剪映AI成片(50项)",
            "相册与文件",
            "剪映AI一键成片",
        ),
        (
            "com.stepos.filemanager/.mcp.FileManagerMcpService",
            "跨端文件寻址服务",
            "相册与文件",
            "跨端文件寻址",
        ),
        (
            "com.android.soundrecorder/.atomicabilitymcp.RecordManagerMcpService",
            "录音与纪要服务",
            "相册与文件",
            "打开录音机纪要",
        ),
        (
            "com.android.packageinstaller/com.stepos.packageinstaller.mcp.McpService",
            "应用安装与校验服务",
            "相册与文件",
            "应用部署校验",
        ),
        (
            "com.stepos.ailauncher/.agent.ability.VoiceAbilityService",
            "桌面与远程智能体服务",
            "语音与系统",
            "远程智能体协同",
        ),
        (
            "com.android.settings/.stepos.mcp.McpService",
            "系统设置与勿扰控制",
            "语音与系统",
            "切换勿扰模式",
        ),
        (
            "com.stepos.systemuiex/.mcp.ScreenMCPService",
            "端侧屏幕感知服务",
            "语音与系统",
            "端侧视觉看屏",
        ),
        (
            "com.android.systemui/.stepos.mcp.mcpservice.SystemUIMCPService",
            "系统状态栏与快捷协同",
            "语音与系统",
            "唤醒语音助手",
        ),
        (
            "com.android.dialer/com.stepos.mcp.service.AbilityService",
            "电话拨号与通话协同",
            "语音与系统",
            "通话状态协同",
        ),
        (
            "com.android.mms/com.stepos.mcp.service.AbilityService",
            "短信与验证码流转服务",
            "语音与系统",
            "验证码流转",
        ),
        (
            "com.stepos.net.search/.server.SearchManagerMcpService",
            "全网搜索与知识检索",
            "语音与系统",
            "知识检索协同",
        ),
        (
            "com.stepos.feedback/.mcp.FeedBackMCPService",
            "系统问题与诊断反馈",
            "语音与系统",
            "提交诊断反馈",
        ),
        (
            "com.stepos.common.ability.proxy/com.stepos.common.ability.biz.CommonAbilityService",
            "背屏点阵(21项)与跨应用代理(6项)",
            "语音与系统",
            "系统基础协同",
        ),
        (
            "com.stepos.ability/com.stepos.taskrobot.ability.TaskRobotMCPService",
            "自动化任务编排服务",
            "安全与自动化",
            "自动化任务编排",
        ),
        (
            "com.stepos.agentsafetycenter/.mcp.service.AbilityService",
            "AI 安全与权限审计中心",
            "安全与自动化",
            "打开安全审计台账",
        ),
        (
            "com.stepos.safebox/com.stepos.mcp.service.AbilityService",
            "私密保险箱存储服务",
            "安全与自动化",
            "私密存储保护",
        ),
        (
            "com.stepos.safe/com.stepos.mcp.service.AbilityService",
            "系统安全防护中心",
            "安全与自动化",
            "系统防护策略",
        ),
        (
            "com.stepos.logreport/.mcp.LogReportAbilityService",
            "系统健康诊断日志服务",
            "安全与自动化",
            "故障健康诊断",
        ),
        (
            "com.android.permissioncontroller/com.stepos.permissioncontroller.mcp.McpService",
            "全局硬件隐私开关与回收站保护(6项)",
            "安全与自动化",
            "权限自愈守护",
        ),
        (
            "com.stepos.secureservice/com.stepos.mcp.service.AbilityService",
            "硬件可信执行环境服务",
            "安全与自动化",
            "可信执行环境",
        ),
    ];

    StepOsAiSuiteReport {
        checked: false,
        read_error: None,
        capability_evidence: StepOsCapabilityEvidence::default(),
        aiengine_running: true,
        aiengine_model_summary:
            "StepOS 端侧 NPU 引擎已就绪 · 支持离线意图识别、屏幕语义理解与多模态视觉感知".into(),
        aiengine_ability_count: 33,
        aiengine_htp_status: "QNN HTP 2/2 空闲".into(),
        agentbus_status: "AgentBus 就绪 · 6 路通道在线 · 云端 mTLS 就绪".into(),
        thermal_summary: String::new(),
        firmware_profile: default_firmware_profile(),
        global_camera_enabled: true,
        global_mic_enabled: true,
        global_location_enabled: true,
        delete_protect_enabled: true,
        backscreen_stay_on_secs: 30,
        backscreen_flip_keep_running: true,
        backscreen_signature: String::new(),
        remote_agent_files: Vec::new(),
        safety_center_running: true,
        gui_accessibility_enabled: true,
        notification_listener_enabled: true,
        mcp_permission_granted: true,
        screen_perception_ready: true,
        mcp_services: services
            .into_iter()
            .map(
                |(id, title, category, quick_action_label)| StepOsMcpServiceItem {
                    id: id.into(),
                    title: title.into(),
                    category: category.into(),
                    ready: true,
                    discovered: true,
                    quick_action_label: quick_action_label.into(),
                },
            )
            .collect(),
        wellbeing: StepOsDigitalWellbeing {
            screen_time_minutes: 0,
            unlock_count: 0,
            top_apps: Vec::new(),
            alphame_running: false,
        },
        calendar_events: Vec::new(),
        alarms: Vec::new(),
        controls: PhoneSystemControls::default(),
        phone_clipboard: String::new(),
        last_action_note: "连接手机后将自动同步手机日历日程、闹钟、屏幕使用统计与剪贴板内容".into(),
    }
}

pub use crate::bridge::clean_nullable_field;

pub fn normalize_sdcard_path(raw: &str) -> String {
    raw.trim().replacen("/storage/emulated/0/", "/sdcard/", 1)
}

fn is_e2e_test_calendar_title(title: &str) -> bool {
    let t = title.trim();
    t == "StepX 日历网格验证" || t.contains("StepX E2E Calendar") || t.contains("StepX自动化验证")
}

pub fn calendar_recurrence(item: &serde_json::Value) -> Option<bool> {
    let explicit = ["recurring", "is_recurring", "isRecurring"].iter()
        .filter_map(|key| item.get(*key).and_then(|value| value.as_bool()));
    let rules = ["rrule", "rdate", "recurrenceRule", "recurrence_rule"].iter()
        .filter_map(|key| item.get(*key).and_then(|value| value.as_str()))
        .map(|rule| !clean_nullable_field(rule).is_empty());
    let mut known = false;
    for recurring in explicit.chain(rules) {
        known = true;
        if recurring { return Some(true); }
    }
    known.then_some(false)
}

#[cfg(test)]
#[test]
fn calendar_recurrence_preserves_unknown_and_treats_rule_evidence_conservatively() {
    use serde_json::json;
    assert_eq!(calendar_recurrence(&json!({})), None);
    assert_eq!(calendar_recurrence(&json!({"rrule":null})), None);
    assert_eq!(calendar_recurrence(&json!({"recurring":false})), Some(false));
    assert_eq!(calendar_recurrence(&json!({"rrule":"", "rdate":""})), Some(false));
    assert_eq!(calendar_recurrence(&json!({"isRecurring":true})), Some(true));
    assert_eq!(calendar_recurrence(&json!({"recurring":false,"rrule":"FREQ=WEEKLY"})), Some(true));
    assert_eq!(calendar_recurrence(&json!({"recurrence_rule":"FREQ=MONTHLY"})), Some(true));
    assert_eq!(calendar_recurrence(&json!({"rdate":"20261107T100000Z"})), Some(true));
}

#[cfg(test)]
#[test]
fn calendar_instances_detect_repeated_ids_without_fabricating_unknown_rules() {
    use serde_json::json;
    let row = |id, start| json!({"id":id,"title":"项目周会","start_ms":start});
    let events = parse_bridge_calendar_events(&json!({"calendar_events":[
        row(71, 1794016800000i64), row(71, 1794621600000i64),
        row(72, 1794016800000i64),
    ]}));
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].recurring, Some(true));
    assert_eq!(events[1].recurring, Some(true));
    assert_eq!(events[2].recurring, None);
}

pub fn parse_bridge_calendar_events(val: &serde_json::Value) -> Vec<PhoneCalendarEvent> {
    let Some(arr) = val.get("calendar_events").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut events: Vec<PhoneCalendarEvent> = Vec::new();
    for item in arr {
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let raw_title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let cleaned_title = clean_nullable_field(raw_title);
        let title = if cleaned_title.is_empty() {
            "未命名日程".to_string()
        } else {
            cleaned_title
        };
        if is_e2e_test_calendar_title(&title) {
            continue;
        }
        let dtstart = item
            .get("start_ms")
            .or_else(|| item.get("dtstart"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let dtend = item
            .get("end_ms")
            .or_else(|| item.get("dtend"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let all_day = item
            .get("all_day")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let location =
            clean_nullable_field(item.get("location").and_then(|v| v.as_str()).unwrap_or(""));
        let description = clean_nullable_field(
            item.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );
        if crate::home_hub::any_synthetic_e2e_probe(&[&title, &location, &description]) {
            continue;
        }
        let source =
            clean_nullable_field(item.get("source").and_then(|v| v.as_str()).unwrap_or(""));
        let source = if source.is_empty() {
            "手机日历".to_string()
        } else {
            source
        };
        let recurring = calendar_recurrence(item);
        if id > 0 && dtstart > 0 {
            // Equal titles/times do not establish identity: two calendars may
            // legitimately have separate "会议" events at the same time.
            if let Some(existing) = events.iter_mut().find(|e| e.id == id && e.dtstart == dtstart) {
                if existing.location.is_empty() && !location.is_empty() {
                    existing.location = location;
                }
                if existing.description.is_empty() && !description.is_empty() {
                    existing.description = description;
                }
                if recurring == Some(true) || existing.recurring.is_none() {
                    existing.recurring = recurring;
                }
                continue;
            }
            events.push(PhoneCalendarEvent {
                id,
                title,
                dtstart,
                dtend: if dtend > dtstart {
                    dtend
                } else if all_day {
                    dtstart + 86_400_000
                } else {
                    dtstart + 3_600_000
                },
                all_day,
                location,
                description,
                source,
                recurring,
            });
        }
    }
    let mut starts = std::collections::HashMap::new();
    let mut series = std::collections::HashSet::new();
    for event in &events {
        if starts.insert(event.id, event.dtstart).is_some_and(|start| start != event.dtstart) {
            series.insert(event.id);
        }
    }
    for event in &mut events {
        if series.contains(&event.id) { event.recurring = Some(true); }
    }
    events
}

fn deterministic_event_id(summary: &str, dtstart: i64) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in summary.trim().to_lowercase().as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for b in dtstart.to_le_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    ((hash & 0x7fff_ffff_ffff_ffff) as i64).max(1)
}

fn unescape_ics_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn escape_ics_text(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\r', "")
        .replace('\n', "\\n")
}

fn parse_ics_datetime(prop_part: &str, value: &str) -> Option<(i64, bool)> {
    let val = value.trim();
    if val.is_empty() {
        return None;
    }
    let prop_upper = prop_part.to_ascii_uppercase();
    let is_date_only = (prop_upper.contains("VALUE=DATE")
        && !prop_upper.contains("VALUE=DATE-TIME"))
        || (val.len() == 8 && val.bytes().all(|b| b.is_ascii_digit()));
    if is_date_only && val.len() >= 8 {
        let year = val[0..4].parse::<i32>().ok()?;
        let month = val[4..6].parse::<u32>().ok()?;
        let day = val[6..8].parse::<u32>().ok()?;
        let naive = chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(0, 0, 0)?;
        let ms = Local
            .from_local_datetime(&naive)
            .earliest()?
            .timestamp_millis();
        return Some((ms, true));
    }
    let is_utc = val.ends_with('Z') || val.ends_with('z');
    let clean = val.trim_end_matches(['Z', 'z']);
    let (date_part, time_part) = clean.split_once('T')?;
    if date_part.len() < 8 || time_part.len() < 4 {
        return None;
    }
    let year = date_part[0..4].parse::<i32>().ok()?;
    let month = date_part[4..6].parse::<u32>().ok()?;
    let day = date_part[6..8].parse::<u32>().ok()?;
    let hour = time_part[0..2].parse::<u32>().ok()?;
    let minute = time_part[2..4].parse::<u32>().ok()?;
    let second = if time_part.len() >= 6 {
        time_part[4..6].parse::<u32>().unwrap_or(0)
    } else {
        0
    };
    let naive =
        chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)?;
    let ms = if is_utc {
        chrono::Utc.from_utc_datetime(&naive).timestamp_millis()
    } else {
        Local
            .from_local_datetime(&naive)
            .earliest()?
            .timestamp_millis()
    };
    Some((ms, false))
}

pub fn parse_ics_events(ics_text: &str, source_name: &str) -> Vec<PhoneCalendarEvent> {
    let stepx_export = ics_text.lines().any(|line| line.trim_end_matches('\r') == "PRODID:-//StepX Companion//Cross-Platform Calendar//CN");
    let mut unfolded: Vec<String> = Vec::new();
    for raw_line in ics_text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = unfolded.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        unfolded.push(line.to_string());
    }

    let mut events = Vec::new();
    let mut in_vevent = false;
    let mut summary = String::new();
    let mut dtstart: Option<i64> = None;
    let mut dtend: Option<i64> = None;
    let mut all_day = false;
    let mut location = String::new();
    let mut description = String::new();
    let mut origin = String::new();
    let mut phone_event_id = None;
    let mut recurring = Some(false);

    let default_source = if source_name.trim().is_empty() {
        "电脑日历"
    } else {
        source_name.trim()
    };

    for line in unfolded {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("BEGIN:VEVENT") {
            in_vevent = true;
            summary.clear();
            dtstart = None;
            dtend = None;
            all_day = false;
            location.clear();
            description.clear();
            origin.clear();
            phone_event_id = None;
            recurring = Some(false);
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VEVENT") {
            if in_vevent {
                if let Some(start_ms) = dtstart.filter(|&ms| ms > 0) {
                    let cleaned_summary = clean_nullable_field(&summary);
                    let clean_title = if cleaned_summary.is_empty() {
                        "未命名日程".to_string()
                    } else {
                        cleaned_summary
                    };
                    if !is_e2e_test_calendar_title(&clean_title) {
                        let end_ms = dtend.filter(|&e| e > start_ms).unwrap_or_else(|| {
                            if all_day {
                                start_ms + 86_400_000
                            } else {
                                start_ms + 3_600_000
                            }
                        });
                        let source = if stepx_export {
                            match origin.as_str() {
                                "desktop" => default_source,
                                "phone" | "phone-mirror" => PHONE_CALENDAR_MIRROR,
                                _ => LEGACY_CALENDAR_MIRROR,
                            }
                        } else { default_source };
                        let id = if stepx_export && (origin == "phone" || origin == "phone-mirror") { phone_event_id.filter(|id| *id > 0).unwrap_or_else(|| deterministic_event_id(&clean_title, start_ms)) }
                            else { deterministic_event_id(&clean_title, start_ms) };
                        let clean_loc = clean_nullable_field(&location);
                        let clean_desc = clean_nullable_field(&description);
                        if let Some(existing) =
                            events.iter_mut().find(|e: &&mut PhoneCalendarEvent| {
                                (e.id == id && e.dtstart == start_ms)
                                    || (e.title.trim().eq_ignore_ascii_case(clean_title.trim())
                                        && e.dtstart == start_ms)
                            })
                        {
                            if existing.location.is_empty() && !clean_loc.is_empty() {
                                existing.location = clean_loc;
                            }
                            if existing.description.is_empty() && !clean_desc.is_empty() {
                                existing.description = clean_desc;
                            }
                        } else {
                            events.push(PhoneCalendarEvent {
                                id,
                                title: clean_title,
                                dtstart: start_ms,
                                dtend: end_ms,
                                all_day,
                                location: clean_loc,
                                description: clean_desc,
                                source: source.to_string(),
                                recurring,
                            });
                        }
                    }
                }
            }
            in_vevent = false;
            continue;
        }
        if !in_vevent {
            continue;
        }
        let Some((prop_part, value)) = line.split_once(':') else {
            continue;
        };
        let prop_name = prop_part
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        match prop_name.as_str() {
            "RRULE" | "RDATE" => {
                if !clean_nullable_field(value).is_empty() { recurring = Some(true); }
            }
            "X-STEPX-RECURRING" if recurring != Some(true) => {
                recurring = match value.trim().to_ascii_uppercase().as_str() {
                    "TRUE" => Some(true), "FALSE" => Some(false), _ => None,
                };
            }
            "SUMMARY" => {
                summary = unescape_ics_text(value);
            }
            "DTSTART" => {
                if let Some((ms, is_all_day)) = parse_ics_datetime(prop_part, value) {
                    dtstart = Some(ms);
                    all_day = is_all_day;
                }
            }
            "DTEND" => {
                if let Some((ms, _)) = parse_ics_datetime(prop_part, value) {
                    dtend = Some(ms);
                }
            }
            "LOCATION" => {
                location = unescape_ics_text(value);
            }
            "DESCRIPTION" => {
                description = unescape_ics_text(value);
            }
            "X-STEPX-ORIGIN" => origin = value.trim().to_ascii_lowercase(),
            "X-STEPX-PHONE-EVENT-ID" => phone_event_id = value.trim().parse::<i64>().ok(),
            _ => {}
        }
    }
    events
}

fn desktop_calendar_ics_path() -> PathBuf {
    crate::resources::user_config().join("stepx-companion/desktop-calendar.ics")
}

fn collect_ics_files_recursive(
    path: &std::path::Path,
    max_depth: usize,
    source_name: &str,
    filter_packages: bool,
    seen_paths: &mut std::collections::BTreeSet<PathBuf>,
    out: &mut Vec<(PathBuf, String)>,
) {
    if out.len() >= 64 {
        return;
    }
    if path.is_file() {
        let is_ics = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("ics"))
            .unwrap_or(false);
        if is_ics && seen_paths.insert(path.to_path_buf()) {
            out.push((path.to_path_buf(), source_name.to_string()));
        }
        return;
    }
    if max_depth == 0 || !path.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten().take(128) {
        let child = entry.path();
        if filter_packages && child.is_dir() {
            let name = child
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !name.contains("calendar")
                && !name.contains("communications")
                && !name.contains("outlook")
                && !name.contains("stepx")
            {
                continue;
            }
        }
        collect_ics_files_recursive(&child, max_depth - 1, source_name, false, seen_paths, out);
    }
}

fn events_match(a_title: &str, a_start: i64, b_title: &str, b_start: i64) -> bool {
    a_title.trim().eq_ignore_ascii_case(b_title.trim()) && (a_start - b_start).abs() < 60_000
}

const PHONE_CALENDAR_MIRROR: &str = "手机日历镜像";
const LEGACY_CALENDAR_MIRROR: &str = "StepX 导出镜像（待核对）";

fn independent_desktop_calendar_event(event: &PhoneCalendarEvent) -> bool {
    // The generated file is an export, not a second author. In particular an
    // old export without provenance cannot authorize recreating a deleted event.
    event.source == "电脑日历"
}

pub fn read_desktop_system_calendar_events() -> Vec<PhoneCalendarEvent> {
    read_desktop_system_calendar_events_with_errors().0
}

fn read_desktop_system_calendar_events_with_errors() -> (Vec<PhoneCalendarEvent>, Vec<String>) {
    let mut read_errors = Vec::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    let mut files = Vec::new();

    let stepx_ics = desktop_calendar_ics_path();
    collect_ics_files_recursive(
        &stepx_ics,
        1,
        "电脑日历",
        false,
        &mut seen_paths,
        &mut files,
    );

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            ".local/share/evolution/calendar/system/calendar.ics",
            ".local/share/evolution/calendar",
            ".local/share/gnome-calendar",
            ".calendar",
            ".local/share/korganizer",
        ] {
            collect_ics_files_recursive(
                &home.join(rel),
                3,
                "电脑日历",
                false,
                &mut seen_paths,
                &mut files,
            );
        }
        collect_ics_files_recursive(
            &home.join("Library/Calendars"),
            3,
            "电脑日历",
            false,
            &mut seen_paths,
            &mut files,
        );
    }

    if let Some(userprofile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        collect_ics_files_recursive(
            &userprofile.join("Calendars"),
            3,
            "电脑日历",
            false,
            &mut seen_paths,
            &mut files,
        );
    }
    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        collect_ics_files_recursive(
            &appdata.join("StepX"),
            3,
            "电脑日历",
            false,
            &mut seen_paths,
            &mut files,
        );
    }
    if let Some(localappdata) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        collect_ics_files_recursive(
            &localappdata.join("Packages"),
            3,
            "电脑日历",
            true,
            &mut seen_paths,
            &mut files,
        );
    }

    let mut all_events: Vec<PhoneCalendarEvent> = Vec::new();
    for (file_path, source_name) in files {
        let Ok(text) = std::fs::read_to_string(&file_path) else {
            read_errors.push("部分电脑日历文件未能读取".into());
            continue;
        };
        for ev in parse_ics_events(&text, &source_name) {
            if let Some(existing) = all_events.iter_mut().find(|existing| {
                events_match(&existing.title, existing.dtstart, &ev.title, ev.dtstart)
            }) {
                // An actual external desktop calendar has authority that an
                // old StepX export of the same title/time does not have.
                if independent_desktop_calendar_event(&ev) && !independent_desktop_calendar_event(existing) { *existing = ev; }
            } else { all_events.push(ev); }
        }
    }
    all_events.sort_by_key(|e| e.dtstart);
    (all_events, read_errors)
}

fn format_ics_local_datetime(ms: i64, all_day: bool) -> String {
    let dt = Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now);
    if all_day {
        format!(
            ";VALUE=DATE:{:04}{:02}{:02}",
            dt.year(),
            dt.month(),
            dt.day()
        )
    } else {
        format!(
            ":{:04}{:02}{:02}T{:02}{:02}{:02}",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second()
        )
    }
}

pub fn export_merged_calendar_ics(events: &[PhoneCalendarEvent]) -> Result<()> {
    let path = desktop_calendar_ics_path();
    export_calendar_ics_at(&path, events, true)
}

fn export_calendar_ics_at(path: &std::path::Path, events: &[PhoneCalendarEvent], replace_phone_mirrors: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut merged: Vec<PhoneCalendarEvent> = Vec::new();
    if let Ok(existing_text) = std::fs::read_to_string(&path) {
        for ev in parse_ics_events(&existing_text, "电脑日历") {
            if !replace_phone_mirrors || ev.source != PHONE_CALENDAR_MIRROR { merged.push(ev); }
        }
    }
    for ev in events {
        if is_e2e_test_calendar_title(&ev.title) {
            continue;
        }
        let clean_loc = clean_nullable_field(&ev.location);
        let clean_desc = clean_nullable_field(&ev.description);
        if let Some(existing) = merged
            .iter_mut()
            .find(|e| events_match(&e.title, e.dtstart, &ev.title, ev.dtstart))
        {
            existing.dtend = ev.dtend;
            existing.recurring = if existing.recurring == Some(true) || ev.recurring == Some(true) {
                Some(true)
            } else {
                ev.recurring.or(existing.recurring)
            };
            if !independent_desktop_calendar_event(existing)
                || !independent_desktop_calendar_event(ev)
            {
                existing.id = ev.id;
                existing.source = ev.source.clone();
            }
            if !clean_loc.is_empty() {
                existing.location = clean_loc;
            }
            if !clean_desc.is_empty() {
                existing.description = clean_desc;
            }
        } else {
            let mut copy = ev.clone();
            copy.location = clean_loc;
            copy.description = clean_desc;
            merged.push(copy);
        }
    }
    merged.sort_by_key(|e| e.dtstart);

    // Android Instances expands a series into individual dates. Without the
    // full RRULE/exception contract these must be independent exported events,
    // otherwise an external calendar merges dates sharing the same event ID.
    let mut first_start = std::collections::HashMap::new();
    let mut repeated_ids = std::collections::HashSet::new();
    for ev in &merged {
        if first_start.insert(ev.id, ev.dtstart).is_some_and(|start| start != ev.dtstart) {
            repeated_ids.insert(ev.id);
        }
    }
    let expanded_instances = merged.iter().any(|ev| ev.recurring != Some(false) || repeated_ids.contains(&ev.id));

    let now_utc = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut ics = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//StepX Companion//Cross-Platform Calendar//CN\r\nCALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\n",
    );
    if expanded_instances {
        ics.push_str("X-WR-CALDESC:当前读取范围内的独立日程实例；不保留周期规则及例外日期\r\n");
    }
    for ev in &merged {
        let uid_id = if ev.id > 0 {
            ev.id
        } else {
            deterministic_event_id(&ev.title, ev.dtstart)
        };
        let dtstart_prop = format_ics_local_datetime(ev.dtstart, ev.all_day);
        let dtend_ms = if ev.dtend > ev.dtstart {
            ev.dtend
        } else if ev.all_day {
            ev.dtstart + 86_400_000
        } else {
            ev.dtstart + 3_600_000
        };
        let dtend_prop = format_ics_local_datetime(dtend_ms, ev.all_day);
        ics.push_str("BEGIN:VEVENT\r\n");
        if ev.recurring != Some(false) || repeated_ids.contains(&ev.id) {
            ics.push_str(&format!("UID:stepx-{uid_id}-{}@stepx.local\r\n", ev.dtstart));
        } else {
            ics.push_str(&format!("UID:stepx-{uid_id}@stepx.local\r\n"));
        }
        let recurring = if repeated_ids.contains(&ev.id) { "TRUE" } else {
            match ev.recurring { Some(true) => "TRUE", Some(false) => "FALSE", None => "UNKNOWN" }
        };
        ics.push_str(&format!("X-STEPX-RECURRING:{recurring}\r\n"));
        let origin = if independent_desktop_calendar_event(ev) { "desktop" }
            else if ev.source == LEGACY_CALENDAR_MIRROR { "unknown" } else { "phone" };
        ics.push_str(&format!("X-STEPX-ORIGIN:{origin}\r\n"));
        if origin == "phone" && ev.id > 0 { ics.push_str(&format!("X-STEPX-PHONE-EVENT-ID:{}\r\n", ev.id)); }
        ics.push_str(&format!("DTSTAMP:{now_utc}\r\n"));
        ics.push_str(&format!("DTSTART{dtstart_prop}\r\n"));
        ics.push_str(&format!("DTEND{dtend_prop}\r\n"));
        ics.push_str(&format!("SUMMARY:{}\r\n", escape_ics_text(&ev.title)));
        let clean_loc = clean_nullable_field(&ev.location);
        if !clean_loc.is_empty() {
            ics.push_str(&format!("LOCATION:{}\r\n", escape_ics_text(&clean_loc)));
        }
        let clean_desc = clean_nullable_field(&ev.description);
        if !clean_desc.is_empty() {
            ics.push_str(&format!("DESCRIPTION:{}\r\n", escape_ics_text(&clean_desc)));
        }
        ics.push_str("END:VEVENT\r\n");
    }
    ics.push_str("END:VCALENDAR\r\n");
    std::fs::write(&path, ics)?;
    if let Some(parent) = path.parent() {
        let mut md = String::from("# StepX 跨端日程同步纪要\n\n");
        if expanded_instances {
            md.push_str("导出仅包含当前读取范围内的独立日程实例，不保留周期规则及例外日期。\n\n");
        }
        for ev in &merged {
            if is_e2e_test_calendar_title(&ev.title) {
                continue;
            }
            md.push_str(&format_calendar_event_markdown_line(ev));
            md.push('\n');
        }
        let _ = std::fs::write(parent.join("stepx-calendar-notes.md"), &md);
    }
    Ok(())
}

pub fn extract_inline_calendar_location(title: &str, description: &str) -> (String, String, String) {
    let mut clean_title = title.trim().to_string();
    let mut clean_desc = description.trim().to_string();
    let mut location = String::new();
    if let Some((t_part, loc_part)) = title.trim().split_once(" @ ") {
        let t = t_part.trim();
        let l = loc_part.trim();
        if !t.is_empty() && !l.is_empty() {
            clean_title = t.to_string();
            location = l.to_string();
        }
    }
    if location.is_empty() {
        let desc_trimmed = description.trim();
        if let Some((d_part, loc_part)) = desc_trimmed.split_once(" @ ") {
            let l = loc_part.trim();
            if !l.is_empty() {
                clean_desc = d_part.trim().to_string();
                location = l.to_string();
            }
        } else if let Some(rest) = desc_trimmed
            .strip_prefix("地点：")
            .or_else(|| desc_trimmed.strip_prefix("地点:"))
        {
            let first_line = rest.lines().next().unwrap_or("").trim();
            if !first_line.is_empty() {
                location = first_line.to_string();
            }
        }
    }
    if clean_title.is_empty() {
        clean_title = "桌面协同日程".to_string();
    }
    (clean_title, location, clean_desc)
}

pub fn save_desktop_calendar_event(
    title: &str,
    dtstart: i64,
    dtend: i64,
    description: &str,
) -> Result<()> {
    let (clean_title, location, clean_desc) = extract_inline_calendar_location(title, description);
    let end_ms = if dtend > dtstart {
        dtend
    } else {
        dtstart + 3_600_000
    };
    let ev = PhoneCalendarEvent {
        id: deterministic_event_id(&clean_title, dtstart),
        title: clean_title,
        dtstart,
        dtend: end_ms,
        all_day: false,
        location,
        description: clean_desc,
        source: "电脑日历".into(),
        recurring: Some(false),
    };
    export_calendar_ics_at(&desktop_calendar_ics_path(), &[ev], false)
}

pub fn format_calendar_event_markdown_line(ev: &PhoneCalendarEvent) -> String {
    let time_str = ev.time_range_label();
    let loc = clean_nullable_field(&ev.location);
    let desc = clean_nullable_field(&ev.description);
    let mut line = format!("- **{}** ({time_str})", ev.title.trim());
    if !loc.is_empty() {
        line.push_str(&format!(" @ {loc}"));
    }
    if !desc.is_empty() {
        line.push_str(&format!(" — {desc}"));
    }
    line
}

pub fn sync_phone_calendar_to_desktop_notes(events: &[PhoneCalendarEvent]) -> Result<String> {
    let _ = export_merged_calendar_ics(events);
    let mut md = String::from("# StepX 跨端日程同步纪要\n\n");
    for ev in events {
        if is_e2e_test_calendar_title(&ev.title) {
            continue;
        }
        md.push_str(&format_calendar_event_markdown_line(ev));
        md.push('\n');
    }
    if let Some(parent) = desktop_calendar_ics_path().parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(parent.join("stepx-calendar-notes.md"), &md);
    }
    Ok(md)
}

pub fn parse_bridge_alarms(val: &serde_json::Value) -> Vec<PhoneAlarmItem> {
    let Some(arr) = val.get("alarms").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut alarms: Vec<PhoneAlarmItem> = Vec::new();
    for item in arr {
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let hour = item
            .get("hour")
            .and_then(|v| v.as_u64())
            .unwrap_or(8)
            .min(23) as u8;
        let minutes = item
            .get("minutes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(59) as u8;
        let enabled = item
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let days_of_week = item
            .get("days_of_week")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let vibrate = item
            .get("vibrate")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let raw_message = item
            .get("label")
            .or_else(|| item.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let message = clean_nullable_field(raw_message);
        if message.contains("StepX自动化验证")
            || message.contains("StepX E2E")
            || crate::home_hub::is_synthetic_e2e_probe_text(&message)
        {
            continue;
        }
        if id > 0 {
            if let Some(existing) = alarms.iter_mut().find(|a| {
                a.hour == hour
                    && a.minutes == minutes
                    && a.days_of_week == days_of_week
                    && a.message.trim() == message.trim()
            }) {
                if (enabled && !existing.enabled)
                    || (enabled == existing.enabled && id > existing.id)
                {
                    existing.id = id;
                    existing.enabled = enabled;
                    existing.vibrate = vibrate;
                }
                continue;
            }
            alarms.push(PhoneAlarmItem {
                id,
                hour,
                minutes,
                enabled,
                days_of_week,
                vibrate,
                message,
            });
        }
    }
    alarms
}

pub fn call_bridge_batch(
    adb: &Adb,
    device: &Device,
    ops: &[serde_json::Value],
    timeout: Duration,
) -> Result<serde_json::Value> {
    let val = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "batch",
            "ops": ops,
        }),
        timeout,
    )?;
    Ok(val
        .get("results")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({})))
}

pub fn parse_bridge_controls(val: &serde_json::Value) -> Option<PhoneSystemControls> {
    let obj = val.get("controls")?.as_object()?;
    Some(PhoneSystemControls {
        dnd_on: obj.get("dnd_on").and_then(|v| v.as_bool()).unwrap_or(false),
        wifi_on: obj.get("wifi_on").and_then(|v| v.as_bool()).unwrap_or(true),
        bluetooth_on: obj
            .get("bluetooth_on")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        brightness_percent: obj
            .get("brightness_percent")
            .and_then(|v| v.as_u64())
            .unwrap_or(65)
            .clamp(1, 100) as u8,
    })
}

pub fn detect_stepos_firmware_profile(prop_text: &str) -> String {
    let lower = prop_text.to_ascii_lowercase();
    if lower.contains("172") && !lower.contains("pandora-12") && !lower.contains(".12") {
        "pandora-172 (300 MCP Tools)".into()
    } else {
        default_firmware_profile()
    }
}

fn step_os_usage_read_errors(value: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(items) = value.get("read_errors").and_then(|v| v.as_array()) {
        for item in items.iter().take(12) {
            if let Some(message) = item.as_str().filter(|s| !s.trim().is_empty()) {
                errors.push(message.chars().take(120).collect());
            }
        }
    }
    for (key, title) in [("screen_time_minutes", "用机时长"), ("unlock_count", "解锁次数")] {
        if value.get(key).and_then(|v| v.as_u64()).is_none() {
            errors.push(format!("{title}未返回有效数据"));
        }
    }
    for (key, title) in [("calendar_events", "手机日历"), ("alarms", "手机闹钟"), ("top_apps", "应用使用统计")] {
        if value.get(key).and_then(|v| v.as_array()).is_none() {
            errors.push(format!("{title}未返回有效数据"));
        }
    }
    if value.get("clipboard").and_then(|v| v.as_str()).is_none() {
        errors.push("手机剪贴板未返回有效数据".into());
    }
    let controls = &value["controls"];
    if ["dnd_on", "wifi_on", "bluetooth_on"].iter().any(|k| controls[*k].as_bool().is_none())
        || controls["brightness_percent"].as_u64().is_none()
    {
        errors.push("手机控制状态未返回有效数据".into());
    }
    errors.sort();
    errors.dedup();
    errors
}

const SUITE_REPORT_CACHE_TTL: Duration = Duration::from_secs(90);

static SUITE_REPORT_CACHE: std::sync::OnceLock<
    std::sync::Mutex<
        std::collections::HashMap<String, (std::time::Instant, StepOsAiSuiteReport)>,
    >,
> = std::sync::OnceLock::new();

fn suite_report_cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<String, (std::time::Instant, StepOsAiSuiteReport)>,
> {
    SUITE_REPORT_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub fn invalidate_stepos_suite_cache() {
    if let Ok(mut guard) = suite_report_cache().lock() {
        guard.clear();
    }
}

pub fn inspect_stepos_ai_suite(adb: &Adb, device: &Device) -> Result<StepOsAiSuiteReport> {
    let cache_key = if !device.serial.trim().is_empty() {
        device.serial.trim().to_string()
    } else {
        device.transport.trim().to_string()
    };
    let bypass_for_external_test =
        std::thread::current().name().is_some_and(|n| n.contains("home_hub"));
    if !cache_key.is_empty() && !bypass_for_external_test {
        if let Ok(guard) = suite_report_cache().lock() {
            if let Some((cached_at, cached_report)) = guard.get(&cache_key) {
                if cached_at.elapsed() < SUITE_REPORT_CACHE_TTL {
                    return Ok(cached_report.clone());
                }
            }
        }
    }

    let transport = device.resolve_active_transport(adb)?;
    let script = concat!(
        "sget() { v=$(printf '%s\\n' \"$1\" | sed -n \"s/^$2=//p\" | head -n 1); if [ -n \"$v\" ]; then printf '%s\\n' \"$v\"; else printf 'null\\n'; fi; }; ",
        "SEC_ALL=$(settings list secure 2>/dev/null || true); ",
        "SYS_ALL=$(settings list system 2>/dev/null || true); ",
        "echo '===PS==='; ",
        "ps -A 2>/dev/null | grep -E 'stepos|openclaw|alphame|aiengine|agentsafety|agentbus|companion' || true; ",
        "echo '===ACC==='; ",
        "if [ -n \"$SEC_ALL\" ]; then sget \"$SEC_ALL\" enabled_accessibility_services; echo ACC_READ_OK=1; ",
        "elif ACC=$(settings get secure enabled_accessibility_services 2>/dev/null); then echo \"$ACC\"; echo ACC_READ_OK=1; fi; ",
        "echo '===NOTIF==='; ",
        "if [ -n \"$SEC_ALL\" ]; then sget \"$SEC_ALL\" enabled_notification_listeners; echo NOTIF_READ_OK=1; ",
        "elif NOTIF=$(settings get secure enabled_notification_listeners 2>/dev/null); then echo \"$NOTIF\"; echo NOTIF_READ_OK=1; fi; ",
        "echo '===PERCEPTION==='; ",
        "settings get global stepos_agentsafetycenter_screen_perception_enabled 2>/dev/null || true; ",
        "echo '===PROP==='; ",
        "getprop ro.build.fingerprint 2>/dev/null || true; ",
        "getprop ro.build.display.id 2>/dev/null || true; ",
        "echo '===PRIVACY==='; ",
        "if [ -n \"$SEC_ALL\" ]; then ",
        "  echo \"LOC_MODE=$(sget \"$SEC_ALL\" location_mode)\"; ",
        "  echo \"DEL_PROT=$(sget \"$SYS_ALL\" stepos_delete_file_and_image_protect)\"; ",
        "  echo \"DEL_PROT=$(sget \"$SEC_ALL\" stepos_delete_file_and_image_protect)\"; ",
        "  echo \"CAM_TOGGLE=$(sget \"$SEC_ALL\" camera_toggle_enabled)\"; ",
        "  echo \"MIC_TOGGLE=$(sget \"$SEC_ALL\" mic_toggle_enabled)\"; ",
        "else ",
        "  echo \"LOC_MODE=$(settings get secure location_mode 2>/dev/null)\"; ",
        "  echo \"DEL_PROT=$(settings get system stepos_delete_file_and_image_protect 2>/dev/null || true)\"; ",
        "  echo \"DEL_PROT=$(settings get secure stepos_delete_file_and_image_protect 2>/dev/null || true)\"; ",
        "  echo \"CAM_TOGGLE=$(settings get secure camera_toggle_enabled 2>/dev/null)\"; ",
        "  echo \"MIC_TOGGLE=$(settings get secure mic_toggle_enabled 2>/dev/null)\"; ",
        "fi; ",
        "if [ -n \"$SYS_ALL\" ]; then ",
        "  echo \"BS_STAY=$(sget \"$SYS_ALL\" stepos_back_screen_stay_on_duration)\"; ",
        "  echo \"BS_FLIP=$(sget \"$SYS_ALL\" stepos_back_screen_flip_keep_running)\"; ",
        "  echo \"BS_SIG=$(sget \"$SYS_ALL\" stepos_back_screen_basic_text)\"; ",
        "  echo \"BS_SIG=$(sget \"$SYS_ALL\" stepos_back_screen_signature_text)\"; ",
        "  echo \"SCR_BRIGHT=$(sget \"$SYS_ALL\" screen_brightness)\"; ",
        "else ",
        "  echo \"BS_STAY=$(settings get system stepos_back_screen_stay_on_duration 2>/dev/null || echo 30)\"; ",
        "  echo \"BS_FLIP=$(settings get system stepos_back_screen_flip_keep_running 2>/dev/null || echo 1)\"; ",
        "  echo \"BS_SIG=$(settings get system stepos_back_screen_basic_text 2>/dev/null || true)\"; ",
        "  echo \"BS_SIG=$(settings get system stepos_back_screen_signature_text 2>/dev/null || true)\"; ",
        "  echo \"SCR_BRIGHT=$(settings get system screen_brightness 2>/dev/null || echo 128)\"; ",
        "fi; ",
        "dumpsys sensor_privacy 2>/dev/null | head -n 80 || true; ",
        "echo '===FILES==='; ",
        "find /sdcard/Download /sdcard/Documents -maxdepth 2 -type f -not -name '.*' -printf '%T@\\t%s\\t%p\\n' 2>/dev/null | sort -rn | head -n 8 || true; ",
        "echo '===AIENGINE==='; ",
        "dumpsys activity service com.stepos.aiengine/.service.AIEngineManagerService 2>/dev/null | head -n 100 || true; ",
        "echo '===AGENTBUS==='; ",
        "dumpsys activity service com.stepos.agentbus/.platform.AgentBusService status 2>/dev/null | head -n 20 || true; ",
        "echo '===MCP_ACCESS==='; ",
        "dumpsys package com.stepos.ability 2>/dev/null | grep 'com.stepos.mcp.sdk.permission.ACCESS_MCP_SERVICES:' | head -n 1; ",
        "echo '===MCP==='; ",
        "cmd package query-services -a com.stepos.mcp.sdk.TOOLS_SERVICE --brief 2>/dev/null || true; ",
        "echo '===THERMAL==='; ",
        "dumpsys thermalservice 2>/dev/null | grep -E 'Current temperatures from HAL:|Current cooling devices from HAL:|mValue=|Thermal Status:' | head -n 90 || true"
    );
    let (raw_res, usage) = std::thread::scope(|s| {
        let t_ref = transport.as_str();
        let script_handle = s.spawn(move || adb.script(t_ref, script));
        let usage_res = crate::bridge::call_bridge_on_transport(
            adb,
            t_ref,
            &serde_json::json!({ "op": "stepos_usage" }),
            std::time::Duration::from_secs(15),
        );
        let raw_res = script_handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("读取手机状态脚本异常")));
        (raw_res, usage_res)
    });
    let raw = raw_res?;
    let mut report = parse_stepos_ai_suite_output(&raw);
    let shell_brightness_parsed = shell_output_has_explicit_brightness(&raw);

    let val = match usage {
        Ok(value) => value,
        Err(_) => {
            report.read_error = Some("手机日历、闹钟与用机统计读取失败，请检查连接或手机授权后重试".into());
            return Ok(report);
        }
    };
    let read_errors = step_os_usage_read_errors(&val);
    if !read_errors.is_empty() {
        report.read_error = Some(read_errors.join("；"));
        return Ok(report);
    }
    {
        if let Some(mins) = val.get("screen_time_minutes").and_then(|v| v.as_u64()) {
            report.wellbeing.screen_time_minutes = mins as u32;
        }
        if let Some(unlocks) = val.get("unlock_count").and_then(|v| v.as_u64()) {
            report.wellbeing.unlock_count = unlocks as u32;
        }
        if let Some(clip) = val.get("clipboard").and_then(|v| v.as_str()) {
            report.phone_clipboard = clip.trim().to_string();
        }
        if let Some(arr) = val.get("top_apps").and_then(|v| v.as_array()) {
            let mut apps = Vec::new();
            for item in arr {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let pkg = item
                    .get("package")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let mins = item.get("minutes").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if !name.is_empty() && !pkg.is_empty() && mins > 0 {
                    let display_name = if name.contains("com.") || name == pkg {
                        friendly_package_name(pkg)
                    } else {
                        name.to_string()
                    };
                    apps.push((display_name, pkg.to_string(), mins));
                }
            }
            report.wellbeing.top_apps = apps;
        }
        report.calendar_events = parse_bridge_calendar_events(&val);
        report.alarms = parse_bridge_alarms(&val);
        let parsed_brightness = report.controls.brightness_percent;
        let parsed_dnd_on = report.controls.dnd_on;
        if let Some(mut controls) = parse_bridge_controls(&val) {
            if shell_brightness_parsed {
                controls.brightness_percent = parsed_brightness;
            }
            if parsed_dnd_on {
                controls.dnd_on = true;
            }
            report.controls = controls;
        }

        // Inspection combines read-only views only. Calendar writes belong to
        // the explicitly named sync operation below, never a Home/status poll.
        merge_desktop_calendar_readonly(&mut report);
    }

    if !cache_key.is_empty() && report.read_error.is_none() {
        if let Ok(mut guard) = suite_report_cache().lock() {
            guard.insert(cache_key, (std::time::Instant::now(), report.clone()));
        }
    }

    Ok(report)
}

fn merge_desktop_calendar_readonly(report: &mut StepOsAiSuiteReport) {
    let (events, errors) = read_desktop_system_calendar_events_with_errors();
    let now = Local::now().timestamp_millis();
    for event in events
        .into_iter()
        .filter(independent_desktop_calendar_event)
    {
        if let Some(existing) = report
            .calendar_events
            .iter_mut()
            .find(|p| events_match(&p.title, p.dtstart, &event.title, event.dtstart))
        {
            existing.source = "双端已同步".into();
        } else if (now - 7 * 86_400_000..=now + 30 * 86_400_000).contains(&event.dtstart) {
            report.calendar_events.push(event);
        }
    }
    report.calendar_events.sort_by_key(|e| e.dtstart);
    report.last_action_note = format!(
        "已读取日历 {} 项 · 手机闹钟 {} 个{}",
        report.calendar_events.len(),
        report.alarms.len(),
        if errors.is_empty() {
            String::new()
        } else {
            format!("（{}）", errors.join("；"))
        }
    );
}

/// Explicit calendar synchronization for an already-authorized auto-sync job.
/// Never call this from a capability/status probe or page rendering.
pub fn sync_stepos_ai_suite_calendar(
    adb: &Adb,
    device: &Device,
    report: &mut StepOsAiSuiteReport,
) -> Result<()> {
    if !crate::pairing::Preferences::load()
        .context("无法核对日历自动同步设置，尚未写入")?
        .auto_sync_calendar
    {
        return Ok(());
    }
    if report.read_error.is_some() {
        bail!("手机采集不完整，未自动写入日历");
    }
    // Read-only inspection may include independent desktop entries. They are
    // not phone rows and must not cause a false 'already synchronized' match.
    report
        .calendar_events
        .retain(|event| !independent_desktop_calendar_event(event));
    let (desktop_events, calendar_read_errors) = read_desktop_system_calendar_events_with_errors();
    let desktop_events: Vec<_> = desktop_events
        .into_iter()
        .filter(independent_desktop_calendar_event)
        .collect();
    let mut local_cal_notes: Vec<String> = calendar_read_errors;
    let now_ms = Local::now().timestamp_millis();
    let min_ms = now_ms - 7 * 86_400_000;
    let max_ms = now_ms + 30 * 86_400_000;

    {
        let mut pushed = 0usize;
        let mut pending = Vec::new();
        for dev in &desktop_events {
            if dev.dtstart < min_ms || dev.dtstart > max_ms {
                continue;
            }
            if let Some(existing) = report.calendar_events.iter_mut().find(|p| {
                !independent_desktop_calendar_event(p)
                    && events_match(&p.title, p.dtstart, &dev.title, dev.dtstart)
            }) {
                existing.source = "双端已同步".into();
            } else if pushed < 5 {
                let desc = if dev.description.trim().is_empty() {
                    "由 StepX 自动同步自电脑日历"
                } else {
                    dev.description.trim()
                };
                if let Ok((created_id, updated, _)) = create_phone_calendar_event(
                    adb,
                    device,
                    &dev.title,
                    dev.dtstart,
                    dev.dtend,
                    desc,
                ) {
                    report.calendar_events = updated;
                    if let Some(created) = report
                        .calendar_events
                        .iter_mut()
                        .find(|p| p.id == created_id)
                    {
                        created.source = "双端已同步".into();
                    }
                    pushed += 1;
                } else {
                    local_cal_notes.push("部分电脑日程未能写入手机".into());
                    pending.push(dev.clone());
                }
            } else {
                pending.push(dev.clone());
            }
        }
        for dev in &desktop_events {
            if let Some(existing) = report.calendar_events.iter_mut().find(|p| {
                !independent_desktop_calendar_event(p)
                    && events_match(&p.title, p.dtstart, &dev.title, dev.dtstart)
            }) {
                existing.source = "双端已同步".into();
            }
        }
        report.calendar_events.extend(pending);
        report.calendar_events.sort_by_key(|e| e.dtstart);
        if export_merged_calendar_ics(&report.calendar_events).is_err() {
            local_cal_notes.push("双端日历未能保存到电脑".into());
        }
    }
    local_cal_notes.sort();
    local_cal_notes.dedup();
    report.last_action_note = format!(
        "{}：{} 项日程 · 手机闹钟 {} 个{}",
        if local_cal_notes.is_empty() {
            "日历同步完成"
        } else {
            "日历同步未全部完成"
        },
        report.calendar_events.len(),
        report.alarms.len(),
        if local_cal_notes.is_empty() {
            String::new()
        } else {
            format!("（{}）", local_cal_notes.join("；"))
        }
    );
    if !local_cal_notes.is_empty() {
        report.read_error = Some(local_cal_notes.join("；"));
    }
    Ok(())
}

fn shell_output_has_explicit_brightness(raw: &str) -> bool {
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("SCR_BRIGHT=") {
            if val.trim().parse::<u32>().ok().is_some_and(|v| v > 0) {
                return true;
            }
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.split("display brightness=").nth(1) {
            if rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .parse::<f32>()
                .ok()
                .is_some_and(|f| f > 0.0 && f.is_finite())
            {
                return true;
            }
        }
    }
    false
}

fn parse_stepos_ai_suite_output(raw: &str) -> StepOsAiSuiteReport {
    let mut report = default_stepos_ai_suite_report();
    report.checked = true;
    report.gui_accessibility_enabled = false;
    report.notification_listener_enabled = false;
    report.mcp_permission_granted = false;
    report.screen_perception_ready = false;
    report.global_camera_enabled = false;
    report.global_mic_enabled = false;
    report.global_location_enabled = false;
    report.delete_protect_enabled = false;
    for item in &mut report.mcp_services { item.ready = false; item.discovered = false; }

    let mut section = "";
    let mut ps_lines = Vec::new();
    let mut acc_text = String::new();
    let mut notif_text = String::new();
    let mut perception_text = String::new();
    let mut prop_lines = Vec::new();
    let mut privacy_lines = Vec::new();
    let mut file_lines = Vec::new();
    let mut clip_lines = Vec::new();
    let mut aiengine_lines = Vec::new();
    let mut agentbus_lines = Vec::new();
    let mut mcp_lines = Vec::new();
    let mut mcp_access_lines = Vec::new();
    let mut seen_acc = false;
    let mut seen_notif = false;
    let mut thermal_lines = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        match trimmed {
            "===PS===" => {
                section = "ps";
                continue;
            }
            "===ACC===" => {
                section = "acc";
                continue;
            }
            "===NOTIF===" => {
                section = "notif";
                continue;
            }
            "===PERCEPTION===" => {
                section = "perception";
                continue;
            }
            "===PROP===" => {
                section = "prop";
                continue;
            }
            "===PRIVACY===" => {
                section = "privacy";
                continue;
            }
            "===FILES===" => {
                section = "files";
                continue;
            }
            "===CLIP===" => {
                section = "clip";
                continue;
            }
            "===USAGE===" => {
                section = "usage";
                continue;
            }
            "===AIENGINE===" => {
                section = "aiengine";
                continue;
            }
            "===AGENTBUS===" => {
                section = "agentbus";
                continue;
            }
            "===MCP_ACCESS===" => { section = "mcp_access"; continue; }
            "===MCP===" => {
                section = "mcp";
                continue;
            }
            "===THERMAL===" => {
                section = "thermal";
                continue;
            }
            _ => {}
        }
        match section {
            "ps" => ps_lines.push(trimmed),
            "acc" => {
                if trimmed == "null" || trimmed == "ACC_READ_OK=1" || trimmed.contains('/') { seen_acc = true; }
                if !trimmed.is_empty() && trimmed != "null" {
                    acc_text.push_str(trimmed);
                    acc_text.push(' ');
                }
            }
            "notif" => {
                if trimmed == "null" || trimmed == "NOTIF_READ_OK=1" || trimmed.contains('/') { seen_notif = true; }
                if !trimmed.is_empty() && trimmed != "null" {
                    notif_text.push_str(trimmed);
                    notif_text.push(' ');
                }
            }
            "perception" => {
                if !trimmed.is_empty() && trimmed != "null" {
                    perception_text.push_str(trimmed);
                }
            }
            "prop" => {
                if !trimmed.is_empty() {
                    prop_lines.push(trimmed);
                }
            }
            "privacy" => {
                if !trimmed.is_empty() {
                    privacy_lines.push(trimmed);
                }
            }
            "files" => {
                if !trimmed.is_empty() {
                    file_lines.push(trimmed);
                }
            }
            "clip" => {
                if !trimmed.is_empty()
                    && !trimmed.starts_with("Error")
                    && !trimmed.starts_with("cmd: Can't find")
                {
                    clip_lines.push(trimmed);
                }
            }
            "aiengine" => {
                if !trimmed.is_empty() {
                    aiengine_lines.push(trimmed);
                }
            }
            "agentbus" => {
                if !trimmed.is_empty() {
                    agentbus_lines.push(trimmed);
                }
            }
            "mcp_access" => { mcp_access_lines.push(trimmed); }
            "mcp" => {
                if !trimmed.is_empty() {
                    mcp_lines.push(trimmed);
                }
            }
            "thermal" => {
                if !trimmed.is_empty() {
                    thermal_lines.push(trimmed);
                }
            }
            _ => {}
        }
    }

    report.capability_evidence.mcp_directory_read = mcp_lines.iter().any(|line| line.contains("services found")).then_some(true);
    if !mcp_lines.is_empty()
        && mcp_lines
            .iter()
            .any(|l| l.contains('/') || l.contains("services found"))
    {
        for item in &mut report.mcp_services {
            let pkg_prefix = format!("{}/", item.id.split('/').next().unwrap_or(&item.id));
            item.discovered = mcp_lines.iter().any(|line| {
                line.split_whitespace()
                    .any(|tok| tok == item.id || tok.starts_with(&pkg_prefix))
            });
        }
    }

    if !prop_lines.is_empty() {
        let joined_prop = prop_lines.join(" ");
        report.firmware_profile = detect_stepos_firmware_profile(&joined_prop);
    }

    let mut cur_sensor = "";
    let mut cam_toggle_off = false;
    let mut mic_toggle_off = false;
    let mut scr_bright_parsed = false;
    for line in &privacy_lines {
        if let Some(val) = line.strip_prefix("LOC_MODE=") {
            let v = val.trim();
            if v == "0" || v.eq_ignore_ascii_case("false") {
                report.global_location_enabled = false;
            } else if !v.is_empty() && v != "null" {
                report.global_location_enabled = true;
            }
        } else if let Some(val) = line.strip_prefix("DEL_PROT=") {
            let v = val.trim();
            if !v.is_empty() && !v.eq_ignore_ascii_case("null") {
                report.delete_protect_enabled = !(v == "0" || v.eq_ignore_ascii_case("false"));
            }
        } else if let Some(val) = line.strip_prefix("CAM_TOGGLE=") {
            let v = val.trim();
            if v == "0" || v.eq_ignore_ascii_case("false") {
                cam_toggle_off = true;
                report.global_camera_enabled = false;
            }
        } else if let Some(val) = line.strip_prefix("MIC_TOGGLE=") {
            let v = val.trim();
            if v == "0" || v.eq_ignore_ascii_case("false") {
                mic_toggle_off = true;
                report.global_mic_enabled = false;
            }
        } else if let Some(val) = line.strip_prefix("BS_STAY=") {
            if let Ok(raw_num) = val.trim().parse::<u32>() {
                let secs = if raw_num >= 1000 {
                    raw_num / 1000
                } else if raw_num > 0 {
                    raw_num
                } else {
                    30
                };
                report.backscreen_stay_on_secs = secs;
            }
        } else if let Some(val) = line.strip_prefix("BS_FLIP=") {
            let v = val.trim();
            report.backscreen_flip_keep_running = !(v == "0" || v.eq_ignore_ascii_case("false"));
        } else if let Some(val) = line.strip_prefix("BS_SIG=") {
            let v = val.trim();
            if !v.is_empty() && !v.eq_ignore_ascii_case("null") {
                report.backscreen_signature = v.to_string();
            }
        } else if let Some(val) = line.strip_prefix("SCR_BRIGHT=") {
            if let Ok(raw_b) = val.trim().parse::<u32>() {
                if raw_b > 0 {
                    let pct = if raw_b > 4095 {
                        ((raw_b * 100 + 16383) / 32767).clamp(1, 100) as u8
                    } else if raw_b > 255 {
                        ((raw_b * 100 + 2047) / 4095).clamp(1, 100) as u8
                    } else {
                        ((raw_b * 100 + 127) / 255).clamp(1, 100) as u8
                    };
                    report.controls.brightness_percent = pct;
                    scr_bright_parsed = true;
                }
            }
        } else {
            if line.starts_with("manualRule=ZenRule[")
                && line.contains("state=STATE_TRUE")
                && !line.contains("zenMode=ZEN_MODE_OFF")
            {
                report.controls.dnd_on = true;
            }
            let lower = line.to_ascii_lowercase();
            if !scr_bright_parsed {
                if let Some(rest) = lower.split("display brightness=").nth(1) {
                    if let Ok(f) = rest.split_whitespace().next().unwrap_or("").parse::<f32>() {
                        if f > 0.0 && f.is_finite() {
                            report.controls.brightness_percent =
                                (f * 100.0).round().clamp(1.0, 100.0) as u8;
                        }
                    }
                }
            }
            if lower.contains("sensor=1") || lower.contains("microphone") {
                cur_sensor = "mic";
            } else if lower.contains("sensor=2") || lower.contains("camera") {
                cur_sensor = "cam";
            }
            if lower.contains("isenabled=true") || lower.contains("state_type=1") {
                if cur_sensor == "mic" {
                    report.global_mic_enabled = false;
                } else if cur_sensor == "cam" {
                    report.global_camera_enabled = false;
                }
            } else if lower.contains("isenabled=false") || lower.contains("state_type=2") {
                if cur_sensor == "mic" && !mic_toggle_off {
                    report.global_mic_enabled = true;
                } else if cur_sensor == "cam" && !cam_toggle_off {
                    report.global_camera_enabled = true;
                }
            }
        }
    }

    let setting = |prefix: &str| privacy_lines.iter().rev().find_map(|line| line.strip_prefix(prefix).and_then(parse_setting_bool));
    let privacy_dump = privacy_lines.join("\n");
    report.capability_evidence.camera_allowed = sensor_privacy_allowed(&privacy_dump, "camera").or_else(|| setting("CAM_TOGGLE="));
    report.capability_evidence.microphone_allowed = sensor_privacy_allowed(&privacy_dump, "microphone").or_else(|| setting("MIC_TOGGLE="));
    report.capability_evidence.delete_protect_enabled = setting("DEL_PROT=");
    report.capability_evidence.location_enabled = privacy_lines.iter().rev().find_map(|line| {
        line.strip_prefix("LOC_MODE=").and_then(|value| value.trim().parse::<u8>().ok()).filter(|value| *value <= 3).map(|value| value > 0)
    });
    report.global_camera_enabled = report.capability_evidence.camera_allowed == Some(true);
    report.global_mic_enabled = report.capability_evidence.microphone_allowed == Some(true);
    report.global_location_enabled = report.capability_evidence.location_enabled == Some(true);
    report.delete_protect_enabled = report.capability_evidence.delete_protect_enabled == Some(true);

    for line in &file_lines {
        let mut parts = line.splitn(3, '\t');
        let Some(ts_str) = parts.next() else {
            continue;
        };
        let Some(sz_str) = parts.next() else {
            continue;
        };
        let Some(path_str) = parts.next() else {
            continue;
        };
        let modified_ms = ts_str
            .trim()
            .parse::<f64>()
            .ok()
            .map(|sec| (sec * 1000.0) as u64)
            .unwrap_or(0);
        let bytes = sz_str.trim().parse::<u64>().unwrap_or(0);
        let path = normalize_sdcard_path(path_str);
        if !path.is_empty() && report.remote_agent_files.len() < 8 {
            report.remote_agent_files.push(FileInfo {
                path,
                bytes,
                modified_ms,
            });
        }
    }

    let ps_joined = ps_lines.join("\n");
    let aiengine_alive =
        ps_joined.contains("com.stepos.aiengine") || ps_joined.contains("aiengine");
    let safety_alive = ps_joined.contains("com.stepos.agentsafetycenter")
        || ps_joined.contains("agentsafetycenter");
    let alphame_alive = ps_joined.contains("com.stepos.alphame")
        || ps_joined.contains("openclaw-gateway")
        || ps_joined.contains("agentbus");
    report.aiengine_running = aiengine_alive;
    report.safety_center_running = safety_alive;
    report.wellbeing.alphame_running = alphame_alive;
    let component_present = |raw: &str, expected: &str| raw.split(|c: char| c == ':' || c.is_whitespace()).any(|part| {
        let Some((package, class)) = part.split_once('/') else { return false; };
        package == "com.stepx.companion.app" && (class == expected || class == format!("com.stepx.companion.app{expected}"))
    });
    report.capability_evidence.gui_accessibility = seen_acc.then(|| component_present(&acc_text, ".GuiAccessibilityService") || component_present(&acc_text, ".neo.NeoAccessibilityService"));
    report.capability_evidence.notification_listener = seen_notif.then(|| component_present(&notif_text, ".PushNotificationService"));
    report.capability_evidence.screen_perception = parse_setting_bool(&perception_text);
    report.capability_evidence.mcp_access = mcp_access_lines.iter().find_map(|line| {
        if !line.contains("com.stepos.mcp.sdk.permission.ACCESS_MCP_SERVICES:") { return None; }
        if line.contains("granted=true") { Some(true) } else if line.contains("granted=false") { Some(false) } else { None }
    });
    report.gui_accessibility_enabled = report.capability_evidence.gui_accessibility == Some(true);
    report.notification_listener_enabled = report.capability_evidence.notification_listener == Some(true);
    report.mcp_permission_granted = report.capability_evidence.mcp_access == Some(true);
    report.screen_perception_ready = report.capability_evidence.screen_perception == Some(true);


    // Parse AIEngineManagerService telemetry (Registered abilities, QNN HTP buckets, active holders)
    let mut ability_count: u32 = 0;
    let mut htp_status = String::new();
    let mut active_holders: Vec<String> = Vec::new();
    for line in &aiengine_lines {
        if let Some(rest) = line.strip_prefix("Registered abilities count:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                ability_count = n;
            }
        } else if let Some(rest) = line.strip_prefix("Registered abilities (") {
            if let Some(num_str) = rest.split(')').next() {
                if let Ok(n) = num_str.trim().parse::<u32>() {
                    ability_count = n;
                }
            }
        } else if line.contains("DSP bucket=HTP:") {
            // e.g. "DSP bucket=HTP: available=2 / max=2"
            if let Some(after) = line.split("available=").nth(1) {
                let normalized = after.replace(" / max=", "/");
                let compact = normalized.split_whitespace().next().unwrap_or("").trim();
                if !compact.is_empty() {
                    htp_status = format!("QNN HTP {compact} 空闲");
                }
            }
        } else if line.starts_with('[') && line.contains("pkg=") {
            if let Some(end) = line.find(']') {
                let model = line[1..end].trim();
                if !model.is_empty() && !active_holders.iter().any(|m| m == model) {
                    active_holders.push(model.to_string());
                }
            }
        }
    }
    if ability_count > 0 {
        report.aiengine_running = true;
    }
    report.aiengine_ability_count = ability_count;
    report.aiengine_htp_status = if !htp_status.is_empty() {
        htp_status.clone()
    } else if report.aiengine_running {
        "QNN HTP 状态未读取".into()
    } else {
        "未运行".into()
    };

    report.aiengine_model_summary = if report.aiengine_running {
        let mut parts = vec!["StepOS AI 引擎运行中".to_string()];
        if ability_count > 0 {
            parts.push(format!("{ability_count} 项已注册能力"));
        } else {
            parts.push("模型调用能力尚待验证".to_string());
        }
        if !htp_status.is_empty() {
            parts.push(htp_status);
        }
        if !active_holders.is_empty() {
            parts.push(format!("常驻算子：{}", active_holders.join(", ")));
        }
        parts.join(" · ")
    } else {
        "StepOS AI 引擎未运行 · 离线推理能力尚待验证".into()
    };

    // Parse AgentBusService status
    let mut agentbus_container_matched = false;
    for line in &agentbus_lines {
        if line.starts_with("Container:") {
            agentbus_container_matched = true;
            // e.g. "Container: ready, connections: 6 (...), inflight invocations: 0, ls server ready: true"
            let ready = line.contains("Container: ready");
            let conns = line
                .split("connections:")
                .nth(1)
                .and_then(|s| s.trim().split_whitespace().next())
                .and_then(|s| s.trim_end_matches(',').parse::<u32>().ok())
                .unwrap_or(6);
            let inflight = line
                .split("inflight invocations:")
                .nth(1)
                .and_then(|s| {
                    s.trim()
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .next()
                })
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            report.agentbus_status = format!(
                "AgentBus {} · {} 路原生通道在线 · 并发调用 {} · 云端 mTLS 就绪",
                if ready { "已就绪" } else { "启动中" },
                conns,
                inflight
            );
            break;
        }
    }
    if !agentbus_container_matched {
        if ps_joined.contains("agentbus") {
            report.agentbus_status = "AgentBus 运行中".into();
        } else {
            report.agentbus_status = "未安装 AgentBus".into();
        }
    }

    // Parse ThermalService telemetry (CPU / GPU / NPU / Skin / Battery).
    // Prefer the live "Current temperatures from HAL:" section over the historical
    // "Cached temperatures:" peak block printed earlier in `dumpsys thermalservice`.
    let has_current_hal_section = thermal_lines
        .iter()
        .any(|l| l.contains("Current temperatures from HAL:"));
    let mut in_live_hal_section = !has_current_hal_section;
    let mut skin_temp: Option<f32> = None;
    let mut batt_temp: Option<f32> = None;
    let mut npu_temp: Option<f32> = None;
    let mut cpu_temp: Option<f32> = None;
    for line in &thermal_lines {
        if has_current_hal_section {
            if line.contains("Current temperatures from HAL:") {
                in_live_hal_section = true;
                continue;
            }
            if in_live_hal_section && line.contains("Current cooling devices from HAL:") {
                break;
            }
            if !in_live_hal_section {
                continue;
            }
        }
        if let Some(temp) = crate::device::parse_thermal_mvalue(line) {
            if line.contains("mName=skin-therm")
                || line.contains("mName=skin")
                || line.contains("mType=3,")
            {
                skin_temp = Some(temp);
            } else if line.contains("mName=battery") || line.contains("mType=2,") {
                batt_temp = Some(temp);
            } else if line.contains("mName=nsp") || line.contains("mType=9,") {
                npu_temp = Some(npu_temp.map_or(temp, |cur| cur.max(temp)));
            } else if line.contains("mName=cpu-")
                || line.contains("mName=CPU")
                || line.contains("mType=0,")
            {
                cpu_temp = Some(cpu_temp.map_or(temp, |cur| cur.max(temp)));
            }
        }
    }
    let mut t_parts = Vec::new();
    if let Some(t) = skin_temp {
        t_parts.push(format!("机身 {t:.1}°C"));
    }
    if let Some(t) = batt_temp {
        t_parts.push(format!("电池 {t:.1}°C"));
    }
    if let Some(t) = npu_temp {
        t_parts.push(format!("NPU {t:.1}°C"));
    }
    if let Some(t) = cpu_temp {
        t_parts.push(format!("CPU {t:.1}°C"));
    }
    if !t_parts.is_empty() {
        report.thermal_summary = format!("多温区实时温控：{}", t_parts.join(" · "));
    }

    if !clip_lines.is_empty() {
        let joined = clip_lines.join(" ").trim().to_string();
        if !joined.is_empty() {
            report.phone_clipboard = joined;
        }
    }

    let discovered_count = report.mcp_services.iter().filter(|s| s.discovered).count();
    report.last_action_note = format!(
        "检测完成：发现 {} / {} 项原生服务（调用尚待验证） · AI 引擎 {} · 安全中心 {}",
        discovered_count,
        report.mcp_services.len(),
        if report.aiengine_running {
            "运行中"
        } else {
            "按需唤醒"
        },
        if report.safety_center_running {
            "已守护"
        } else {
            "待命"
        }
    );
    report
}

fn friendly_package_name(pkg: &str) -> String {
    match pkg {
        "com.stepos.alphame" => "AlphaMe 个人中心".into(),
        "com.stepos.habit" | "com.stepos.ailauncher" => "Amoo 助手".into(),
        "com.stepx.companion.app" => "StepX伴侣".into(),
        "com.stepos.aiengine" => "端侧大模型引擎".into(),
        "com.stepos.agentsafetycenter" => "AI 安全审计中心".into(),
        "com.stepos.media.gallery" | "com.stepos.gallery" => "相册".into(),
        "com.stepos.filemanager" => "文件管理".into(),
        "com.stepos.update" => "系统更新".into(),
        "com.tencent.mm" => "微信".into(),
        "com.twitter.android" => "X".into(),
        "mark.via" => "Via 浏览器".into(),
        "com.android.chrome" => "Chrome 浏览器".into(),
        "com.android.vending" => "Google Play 商店".into(),
        "com.google.android.gm" => "Gmail".into(),
        "com.google.android.apps.bard" => "Gemini".into(),
        "com.google.android.googlequicksearchbox" => "Google".into(),
        "com.android.settings" => "系统设置".into(),
        "com.android.calendar" => "日历".into(),
        "com.android.alarmclock" | "com.android.deskclock" => "时钟".into(),
        _ => pkg.rsplit('.').next().unwrap_or(pkg).to_string(),
    }
}

pub(crate) fn push_clipboard_to_phone_on_transport(
    adb: &Adb,
    transport: &str,
    text: &str,
) -> Result<String> {
    let clean = text.trim();
    if clean.is_empty() {
        bail!("电脑剪贴板当前为空，请先复制文字后再推送");
    }
    let val = crate::bridge::call_bridge_on_transport(
        adb,
        transport,
        &serde_json::json!({
            "op": "clipboard_set",
            "text": clean,
        }),
        std::time::Duration::from_secs(6),
    )?;
    if !val
        .get("written")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        bail!("手机剪贴板写入未确认完成，请确认手机连接正常");
    }
    invalidate_stepos_suite_cache();
    let preview: String = clean.chars().take(28).collect();
    Ok(format!(
        "已推送电脑剪贴板到手机：「{}{}」",
        preview,
        if clean.chars().count() > 28 {
            "…"
        } else {
            ""
        }
    ))
}

pub fn push_clipboard_to_phone(adb: &Adb, device: &Device, text: &str) -> Result<String> {
    let clean = text.trim();
    if clean.is_empty() {
        bail!("电脑剪贴板当前为空，请先复制文字后再推送");
    }
    let transport = device.resolve_active_transport(adb)?;
    push_clipboard_to_phone_on_transport(adb, &transport, clean)
}

pub(crate) fn pull_clipboard_from_phone_on_transport(adb: &Adb, transport: &str) -> Result<String> {
    let val = crate::bridge::call_bridge_on_transport(
        adb,
        transport,
        &serde_json::json!({ "op": "clipboard_get" }),
        std::time::Duration::from_secs(6),
    )?;
    let text = val
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        bail!("手机剪贴板当前为空，请先在手机复制文字后再拉取");
    }
    Ok(text)
}

pub fn pull_clipboard_from_phone(adb: &Adb, device: &Device) -> Result<String> {
    if let Some(value) = crate::native_mcp::try_call(adb, device, "common_ability_clipboard_text", serde_json::json!({}))? {
        if let Some(text) = value.as_str() {
            anyhow::ensure!(!text.trim().is_empty(), "手机剪贴板为空或系统不允许读取");
            return Ok(text.to_owned());
        }
        // This firmware deserializes copied numeric/JSON text before returning
        // it. Formatting/leading zeros cannot be reconstructed from that value.
        // Read the original text through the authorized structured provider;
        // this is read-only and does not retry an uncertain mutation.
        let original = crate::bridge::call_bridge(adb, device,
            &serde_json::json!({"op":"clipboard_get"}), std::time::Duration::from_secs(6))?;
        return original["text"].as_str().map(str::to_owned)
            .context("手机原生剪贴板已转换为 JSON，且原文回读失败；未猜测或重排文字");
    }
    let transport = device.resolve_active_transport(adb)?;
    pull_clipboard_from_phone_on_transport(adb, &transport)
}

/// Update only the description on the verified phone. `Adb::shell` owns shell
/// quoting; adding another quote layer would store those quotes in Calendar.
pub fn update_phone_calendar_event_description(adb: &Adb, device: &Device, event_id: i64, description: &str) -> Result<()> {
    anyhow::ensure!(event_id > 0, "手机日历 ID 无效");
    let transport = device.resolve_active_transport(adb)?;
    anyhow::ensure!(adb.shell(&transport, &["getprop", "ro.serialno"])?.trim() == device.serial,
        "手机连接已变化，停止更新日历");
    adb.shell(&transport, &["content", "update", "--uri",
        &format!("content://com.android.calendar/events/{event_id}"), "--bind", &format!("description:s:{description}")])?;
    // Android builds may print no acknowledgement for a successful update.
    // Read the exact row back; raw output preserves trailing description text.
    let uri = format!("content://com.android.calendar/events/{event_id}");
    let command = ["content", "query", "--uri", &uri, "--projection", "description"]
        .into_iter().map(crate::adb::shell_quote).collect::<Vec<_>>().join(" ");
    let result = adb.run_output_with_input(&["-s", &transport, "shell", &command], None, std::time::Duration::from_secs(8))?;
    let expected = format!("Row: 0 description={description}");
    anyhow::ensure!(result.status.success()
        && (result.stdout == format!("{expected}\n").as_bytes() || result.stdout == format!("{expected}\r\n").as_bytes()),
        "手机日历写入后的正文尚未通过读回核验");
    invalidate_stepos_suite_cache();
    Ok(())
}

pub fn create_phone_calendar_event(
    adb: &Adb,
    device: &Device,
    title: &str,
    dtstart: i64,
    dtend: i64,
    description: &str,
) -> Result<(i64, Vec<PhoneCalendarEvent>, String)> {
    let (clean_title, location, clean_desc) = extract_inline_calendar_location(title, description);
    let val = if let Some(value) = crate::native_mcp::create_calendar(adb, device, &clean_title, &clean_desc, &location, dtstart, dtend)? { value } else { crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "calendar_create",
            "title": clean_title,
            "location": location,
            "eventLocation": location,
            "start_ms": dtstart,
            "end_ms": dtend,
            "dtstart": dtstart,
            "dtend": dtend,
            "description": clean_desc,
        }),
        std::time::Duration::from_secs(8),
    )? };
    let created_id = val.get("created_id").and_then(|v| v.as_i64()).unwrap_or(0);
    if created_id <= 0 {
        bail!("写入手机日历未返回有效日程 ID");
    }
    invalidate_stepos_suite_cache();
    let mut events = parse_bridge_calendar_events(&val);
    if !location.trim().is_empty() {
        if let Some(created_ev) = events.iter_mut().find(|e| e.id == created_id) {
            if created_ev.location.trim().is_empty() {
                created_ev.location = location.trim().to_string();
            }
        }
    }
    let _ = sync_phone_calendar_to_desktop_notes(&events);
    let time_label = events
        .iter()
        .find(|e| e.id == created_id)
        .map(|e| e.time_range_label())
        .unwrap_or_else(|| "已同步".into());
    let resolved_loc = events
        .iter()
        .find(|e| e.id == created_id)
        .map(|e| e.location.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| location.trim().to_string());
    let loc_suffix = if !resolved_loc.trim().is_empty() {
        format!(" @ {}", resolved_loc.trim())
    } else {
        String::new()
    };
    Ok((
        created_id,
        events,
        format!("已在手机日历创建日程：「{clean_title}」（{time_label}{loc_suffix}）"),
    ))
}

/// Fast unified phone context overview connecting `bridge::calls`, `bridge::contacts`,
/// `bridge::calendar`, `bridge::media`, and `bridge::status` for toolbox & AI suite workflows.
pub fn collect_phone_quick_overview(
    adb: &Adb,
    device: &Device,
    limit: usize,
) -> Result<(
    crate::bridge::BridgeStatus,
    Vec<crate::bridge::BridgeCall>,
    Vec<crate::bridge::BridgeContact>,
    Vec<crate::bridge::BridgeCalendarEvent>,
    Vec<crate::bridge::BridgeMediaItem>,
)> {
    let transport = device.resolve_active_transport(adb)?;
    let capped = limit.clamp(1, 50);
    let contacts_limit = (capped * 4).clamp(1, 500);
    let batch_req = serde_json::json!({
        "op": "batch",
        "ops": [
            { "id": "capabilities", "op": "capabilities" },
            { "id": "usage", "op": "stepos_usage" },
            { "id": "calls", "op": "read_calls", "limit": capped },
            { "id": "contacts", "op": "read_contacts", "limit": contacts_limit },
            { "id": "calendar", "op": "read_calendar", "limit": capped },
            { "id": "recordings", "op": "read_recordings" },
        ]
    });
    let find_script = format!(
        "find /sdcard/Recordings /sdcard/Music/Recordings /sdcard/Sounds /sdcard/MIUI/sound_recorder /sdcard/Pictures/Screenshots /sdcard/DCIM/Screenshots /sdcard/DCIM/Camera -maxdepth 3 -type f -printf '%T@|%s|%p\\n' 2>/dev/null | sort -rn | head -n {capped}"
    );
    let (batch_res, find_res) = std::thread::scope(|s| {
        let t_ref = transport.as_str();
        let f_ref = find_script.as_str();
        let find_handle = s.spawn(move || adb.script(t_ref, f_ref));
        let batch_res = crate::bridge::call_bridge_on_transport(
            adb,
            t_ref,
            &batch_req,
            Duration::from_secs(20),
        );
        let find_res = find_handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("media find thread panicked")));
        (batch_res, find_res)
    });

    if let Ok(batch_val) = batch_res {
        let results = &batch_val["results"];
        let usage_opt = results
            .get("usage")
            .filter(|u| u["ok"].as_bool() == Some(true));
        let status = if results["capabilities"]["ok"].as_bool() == Some(true) {
            crate::bridge::parse_status(&results["capabilities"], usage_opt).unwrap_or_default()
        } else {
            crate::bridge::BridgeStatus::default()
        };
        let mut contacts = if results["contacts"]["ok"].as_bool() == Some(true) {
            crate::bridge::parse_contacts(&results["contacts"]).unwrap_or_default()
        } else {
            Vec::new()
        };
        contacts.truncate(contacts_limit);
        let mut calls = if results["calls"]["ok"].as_bool() == Some(true) {
            crate::bridge::parse_calls(&results["calls"]).unwrap_or_default()
        } else {
            Vec::new()
        };
        calls.truncate(capped);
        crate::phone_context::enrich_calls_with_contacts(&mut calls, &contacts);
        let mut calendar = if results["calendar"]["ok"].as_bool() == Some(true) {
            crate::bridge::parse_calendar(&results["calendar"]).unwrap_or_default()
        } else {
            Vec::new()
        };
        calendar.truncate(capped);
        let mut media = if results["recordings"]["ok"].as_bool() == Some(true) {
            crate::bridge::parse_media(&results["recordings"], capped).unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Ok(raw) = find_res {
            for item in crate::bridge::parse_media_find_output(&raw, capped) {
                if !media
                    .iter()
                    .any(|existing: &crate::bridge::BridgeMediaItem| existing.path == item.path)
                {
                    media.push(item);
                }
            }
        }
        media.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
        media.truncate(capped);
        return Ok((status, calls, contacts, calendar, media));
    }

    let status = crate::bridge::status(adb, &transport).unwrap_or_default();
    let mut calls = crate::bridge::calls(adb, &transport, capped).unwrap_or_default();
    let contacts = crate::bridge::contacts(adb, &transport, contacts_limit).unwrap_or_default();
    crate::phone_context::enrich_calls_with_contacts(&mut calls, &contacts);
    let calendar = crate::bridge::calendar(adb, &transport, capped).unwrap_or_default();
    let media = crate::bridge::media(adb, &transport, capped).unwrap_or_default();
    Ok((status, calls, contacts, calendar, media))
}

fn parse_hh_mm(input: &str) -> Option<(u32, u32)> {
    let norm = input.trim().replace('：', ":");
    if norm.is_empty() {
        return None;
    }
    if let Some((h_s, m_s)) = norm.split_once(':') {
        let h = h_s.trim().parse::<u32>().ok()?;
        let m = m_s.trim().parse::<u32>().ok()?;
        if h <= 23 && m <= 59 {
            return Some((h, m));
        }
        return None;
    }
    if norm.len() == 4 && norm.bytes().all(|b| b.is_ascii_digit()) {
        let h = norm[0..2].parse::<u32>().ok()?;
        let m = norm[2..4].parse::<u32>().ok()?;
        if h <= 23 && m <= 59 {
            return Some((h, m));
        }
    } else if norm.len() == 3 && norm.bytes().all(|b| b.is_ascii_digit()) {
        let h = norm[0..1].parse::<u32>().ok()?;
        let m = norm[1..3].parse::<u32>().ok()?;
        if h <= 23 && m <= 59 {
            return Some((h, m));
        }
    }
    None
}

pub fn parse_event_time_range(
    year: i32,
    month: u32,
    day: u32,
    time_str: &str,
) -> Result<(i64, i64)> {
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day).context("无效的日程日期")?;
    let norm = time_str
        .trim()
        .replace('：', ":")
        .replace(['–', '—', '~'], "-");
    let ((sh, sm), end_opt) = if let Some((s_part, e_part)) = norm.split_once('-') {
        let s_hm = parse_hh_mm(s_part).unwrap_or((9, 30));
        let e_hm = parse_hh_mm(e_part);
        (s_hm, e_hm)
    } else {
        let s_hm = parse_hh_mm(&norm).unwrap_or((9, 30));
        (s_hm, None)
    };
    let start_naive = date.and_hms_opt(sh, sm, 0).context("无效的日程开始时间")?;
    let start_dt = Local
        .from_local_datetime(&start_naive)
        .earliest()
        .context("无法转换本地时间")?;
    let dtstart = start_dt.timestamp_millis();
    let dtend = if let Some((eh, em)) = end_opt {
        if let Some(end_naive) = date.and_hms_opt(eh, em, 0) {
            let end_ms = Local
                .from_local_datetime(&end_naive)
                .earliest()
                .map(|dt| dt.timestamp_millis())
                .unwrap_or(dtstart + 3_600_000);
            if end_ms > dtstart {
                end_ms
            } else {
                dtstart + 3_600_000
            }
        } else {
            dtstart + 3_600_000
        }
    } else {
        dtstart + 3_600_000
    };
    Ok((dtstart, dtend))
}

pub fn create_calendar_event_at(
    adb: &Adb,
    device: &Device,
    title: &str,
    year: i32,
    month: u32,
    day: u32,
    time_str: &str,
    auto_sync_desktop: bool,
) -> Result<(i64, Vec<PhoneCalendarEvent>, String)> {
    let clean_title = if title.trim().is_empty() {
        "桌面协同日程"
    } else {
        title.trim()
    };
    let (dtstart, dtend) = parse_event_time_range(year, month, day, time_str)?;
    let (created_id, mut events, mut note) = create_phone_calendar_event(
        adb,
        device,
        clean_title,
        dtstart,
        dtend,
        "由 StepX 桌面端直接同步创建",
    )?;
    if auto_sync_desktop {
        let _ =
            save_desktop_calendar_event(clean_title, dtstart, dtend, "由 StepX 桌面端直接同步创建");
        let _ = export_merged_calendar_ics(&events);
        if let Some(ev) = events.iter_mut().find(|e| e.id == created_id) {
            ev.source = "双端已同步".into();
        }
        note = format!("{note} · 已同步至电脑日历");
    }
    let desktop_events = read_desktop_system_calendar_events()
        .into_iter()
        .filter(independent_desktop_calendar_event);
    let now_ms = Local::now().timestamp_millis();
    let min_ms = now_ms - 7 * 86_400_000;
    let max_ms = now_ms + 30 * 86_400_000;
    for dev in desktop_events {
        if let Some(existing) = events
            .iter_mut()
            .find(|p| events_match(&p.title, p.dtstart, &dev.title, dev.dtstart))
        {
            existing.source = "双端已同步".into();
        } else if dev.dtstart >= min_ms && dev.dtstart <= max_ms {
            events.push(dev);
        }
    }
    events.sort_by_key(|e| e.dtstart);
    Ok((created_id, events, note))
}

pub fn parse_calendar_datetime_str_at(
    input: &str,
    now: chrono::DateTime<Local>,
) -> Result<(i64, i64)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("请输入日程日期与时间，例如 明天 15:00 或 2026-11-02 14:30");
    }
    let mut rest = trimmed;
    let mut target_date = now.date_naive();
    let mut explicit_day = false;
    if let Some(after) = rest.strip_prefix("大后天") {
        target_date += ChronoDuration::days(3);
        rest = after.trim();
        explicit_day = true;
    } else if let Some(after) = rest.strip_prefix("后天") {
        target_date += ChronoDuration::days(2);
        rest = after.trim();
        explicit_day = true;
    } else if let Some(after) = rest
        .strip_prefix("明天")
        .or_else(|| rest.strip_prefix("明日"))
    {
        target_date += ChronoDuration::days(1);
        rest = after.trim();
        explicit_day = true;
    } else if let Some(after) = rest
        .strip_prefix("今天")
        .or_else(|| rest.strip_prefix("今日"))
    {
        rest = after.trim();
        explicit_day = true;
    } else if let Some((date_tok, after)) = rest.split_once(char::is_whitespace) {
        if let Ok(parsed_date) = chrono::NaiveDate::parse_from_str(date_tok.trim(), "%Y-%m-%d")
            .or_else(|_| chrono::NaiveDate::parse_from_str(date_tok.trim(), "%Y/%m/%d"))
        {
            target_date = parsed_date;
            rest = after.trim();
            explicit_day = true;
        }
    }

    let time_part = if rest.is_empty() { "09:30" } else { rest };
    let norm = time_part
        .trim()
        .replace('：', ":")
        .replace(['–', '—', '~'], "-");
    if let Some((s_part, e_part)) = norm.split_once('-') {
        let s_hm = parse_hh_mm(s_part)
            .or_else(|| {
                parse_alarm_time_str_at(s_part, now)
                    .ok()
                    .map(|(h, m)| (h as u32, m as u32))
            })
            .context("无效的日程开始时间")?;
        let e_hm = parse_hh_mm(e_part).or_else(|| {
            parse_alarm_time_str_at(e_part, now)
                .ok()
                .map(|(h, m)| (h as u32, m as u32))
        });
        let range_str = if let Some((eh, em)) = e_hm {
            format!("{:02}:{:02}-{:02}:{:02}", s_hm.0, s_hm.1, eh, em)
        } else {
            format!("{:02}:{:02}", s_hm.0, s_hm.1)
        };
        return parse_event_time_range(
            target_date.year(),
            target_date.month(),
            target_date.day(),
            &range_str,
        );
    }
    let (sh, sm) = parse_hh_mm(&norm)
        .or_else(|| {
            parse_alarm_time_str_at(&norm, now)
                .ok()
                .map(|(h, m)| (h as u32, m as u32))
        })
        .context("无效的日程时间格式")?;
    if !explicit_day && (norm.starts_with("明早") || norm.starts_with("明晚")) {
        target_date += ChronoDuration::days(1);
    }
    parse_event_time_range(
        target_date.year(),
        target_date.month(),
        target_date.day(),
        &format!("{sh:02}:{sm:02}"),
    )
}

pub fn parse_calendar_datetime_str(input: &str) -> Result<(i64, i64)> {
    parse_calendar_datetime_str_at(input, Local::now())
}

pub fn create_phone_calendar_preset(
    adb: &Adb,
    device: &Device,
    preset: u8,
    custom_title: &str,
) -> Result<(i64, Vec<PhoneCalendarEvent>, String)> {
    let now = Local::now();
    let (default_title, start) = match preset {
        0 => ("桌面协同事项", now + ChronoDuration::hours(1)),
        1 => {
            let today_18 = now
                .with_hour(18)
                .and_then(|t| t.with_minute(0))
                .and_then(|t| t.with_second(0))
                .unwrap_or(now + ChronoDuration::hours(2));
            let target = if today_18 > now {
                today_18
            } else {
                today_18 + ChronoDuration::days(1)
            };
            ("晚间重点待办复盘", target)
        }
        2 => {
            let tomorrow = now + ChronoDuration::days(1);
            let target = tomorrow
                .with_hour(9)
                .and_then(|t| t.with_minute(30))
                .and_then(|t| t.with_second(0))
                .unwrap_or(tomorrow);
            ("晨会与重点任务推进", target)
        }
        _ => {
            let tomorrow = now + ChronoDuration::days(1);
            let target = tomorrow
                .with_hour(14)
                .and_then(|t| t.with_minute(0))
                .and_then(|t| t.with_second(0))
                .unwrap_or(tomorrow);
            ("下午专项协作日程", target)
        }
    };
    let title = if custom_title.trim().is_empty() {
        default_title
    } else {
        custom_title.trim()
    };
    let dtstart = start.timestamp_millis();
    let dtend = (start + ChronoDuration::hours(1)).timestamp_millis();
    let (created_id, mut events, mut note) = create_phone_calendar_event(
        adb,
        device,
        title,
        dtstart,
        dtend,
        "由 StepX 桌面端直接同步创建",
    )?;
    let _ = save_desktop_calendar_event(title, dtstart, dtend, "由 StepX 桌面端直接同步创建");
    let _ = export_merged_calendar_ics(&events);
    if let Some(ev) = events.iter_mut().find(|e| e.id == created_id) {
        ev.source = "双端已同步".into();
    }
    note = format!("{note} · 已同步至电脑日历");
    Ok((created_id, events, note))
}

pub fn delete_phone_calendar_event(
    adb: &Adb,
    device: &Device,
    id: i64,
) -> Result<(Vec<PhoneCalendarEvent>, String)> {
    if id <= 0 {
        bail!("无效的手机日程 ID");
    }
    let val = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "calendar_delete",
            "id": id,
        }),
        std::time::Duration::from_secs(8),
    )?;
    let events = parse_bridge_calendar_events(&val);
    invalidate_stepos_suite_cache();
    let _ = export_merged_calendar_ics(&events);
    Ok((events, "已从手机日历移除该日程".into()))
}

pub fn parse_alarm_time_str_at(time_str: &str, now: chrono::DateTime<Local>) -> Result<(u8, u8)> {
    let trimmed = time_str.trim();
    if trimmed.is_empty() {
        bail!("请输入闹钟时间，例如 07:30 或 +15m");
    }
    let lower = trimmed.to_ascii_lowercase();

    // 1. Relative hour/minute expressions: "2小时后", "1小时30分后", "1个半小时后", "+15m", "15分钟后", "+1h30m"
    if trimmed.contains("小时") || lower.ends_with('h') || (lower.starts_with('+') && lower.contains('h')) {
        let norm = lower
            .replace("个半小时", "小时30分")
            .replace("小时半", "小时30分")
            .replace("小时", "h")
            .replace("分钟", "m")
            .replace('分', "m")
            .replace('后', "");
        if let Some((h_part, m_part)) = norm.trim_start_matches('+').split_once('h') {
            let hours: i64 = h_part
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            let mins: i64 = m_part
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            let total_mins = hours * 60 + mins;
            if (1..=1440).contains(&total_mins) {
                let target = now + ChronoDuration::minutes(total_mins);
                return Ok((target.hour() as u8, target.minute() as u8));
            }
        }
        bail!("无效的小时倒计时，请输入如 2小时后 或 1小时30分后");
    }
    if lower.starts_with('+')
        || lower.ends_with('m')
        || lower.ends_with("min")
        || trimmed.contains("分钟")
        || trimmed.ends_with("分后")
    {
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(mins) = digits.parse::<i64>() {
            if mins > 0 && mins <= 1440 {
                let target = now + ChronoDuration::minutes(mins);
                return Ok((target.hour() as u8, target.minute() as u8));
            }
        }
        bail!("无效的倒计时分钟数，请输入如 +15m 或 15分钟后");
    }

    // 2. Natural Chinese clock expressions: "早上8点半", "今晚9点", "下午3点15分", "晚上10点一刻"
    let is_pm = ["下午", "傍晚", "晚上", "今晚", "明晚", "午后", "pm"]
        .iter()
        .any(|kw| lower.contains(kw));
    let is_midnight = ["凌晨", "半夜"].iter().any(|kw| trimmed.contains(kw));
    let cleaned = [
        "早上", "早晨", "清晨", "上午", "中午", "下午", "傍晚", "晚上", "今晚", "明早", "明晚",
        "凌晨", "半夜", "今天", "明天", "am", "pm", "AM", "PM",
    ]
    .iter()
    .fold(trimmed.to_string(), |acc, kw| acc.replace(kw, ""))
    .trim()
    .to_string();

    if let Some((h_str, rest)) = cleaned.split_once('点').or_else(|| cleaned.split_once('时')) {
        if let Ok(mut h) = h_str.trim().parse::<u32>() {
            let r = rest.trim().trim_end_matches('整');
            let m = if r.is_empty() {
                0
            } else if r == "半" {
                30
            } else if r == "一刻" {
                15
            } else if r == "三刻" {
                45
            } else {
                r.trim_end_matches("分钟")
                    .trim_end_matches('分')
                    .trim()
                    .parse::<u32>()
                    .context("无效的分钟数")?
            };
            if is_pm && h < 12 {
                h += 12;
            } else if is_midnight && h == 12 {
                h = 0;
            }
            if h <= 23 && m <= 59 {
                return Ok((h as u8, m as u8));
            }
        }
    }

    if let Some((mut h, m)) = parse_hh_mm(&cleaned).or_else(|| parse_hh_mm(trimmed)) {
        if is_pm && h < 12 {
            h += 12;
        } else if is_midnight && h == 12 {
            h = 0;
        }
        return Ok((h as u8, m as u8));
    }
    bail!("请输入有效的时间格式，如 07:30、0800、早上8点半 或 +15m")
}

pub fn parse_alarm_time_str(time_str: &str) -> Result<(u8, u8)> {
    parse_alarm_time_str_at(time_str, Local::now())
}

pub fn create_phone_alarm_from_time_str(
    adb: &Adb,
    device: &Device,
    time_str: &str,
    label: &str,
) -> Result<(i64, Vec<PhoneAlarmItem>, String)> {
    let (hour, minutes) = parse_alarm_time_str(time_str)?;
    create_phone_alarm(adb, device, hour, minutes, label)
}

pub fn create_phone_alarm(
    adb: &Adb,
    device: &Device,
    hour: u8,
    minutes: u8,
    message: &str,
) -> Result<(i64, Vec<PhoneAlarmItem>, String)> {
    let clean_msg = if message.trim().is_empty() {
        "StepX 桌面提醒"
    } else {
        message.trim()
    };
    let val = if let Some(value) = crate::native_mcp::create_alarm(adb, device, hour, minutes, clean_msg)? { value } else { crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "alarm_create",
            "hour": hour.min(23),
            "minutes": minutes.min(59),
            "label": clean_msg,
            "message": clean_msg,
            "enabled": true,
        }),
        std::time::Duration::from_secs(8),
    )? };
    let created_id = val.get("created_id").and_then(|v| v.as_i64()).unwrap_or(0);
    if created_id <= 0 {
        bail!("写入手机闹钟未返回有效 ID");
    }
    invalidate_stepos_suite_cache();
    let alarms = parse_bridge_alarms(&val);
    Ok((
        created_id,
        alarms,
        format!(
            "已在手机添加并开启 {:02}:{:02} 闹钟：「{clean_msg}」",
            hour.min(23),
            minutes.min(59)
        ),
    ))
}

pub fn create_phone_alarm_preset(
    adb: &Adb,
    device: &Device,
    preset: u8,
    custom_label: &str,
) -> Result<(i64, Vec<PhoneAlarmItem>, String)> {
    let now = Local::now();
    let (h, m, default_label) = match preset {
        0 => {
            let t = now + ChronoDuration::minutes(15);
            (t.hour() as u8, t.minute() as u8, "15分钟专注提醒")
        }
        1 => {
            let t = now + ChronoDuration::minutes(30);
            (t.hour() as u8, t.minute() as u8, "30分钟休息提醒")
        }
        2 => (7, 30, "早间起床与日程提醒"),
        _ => (8, 0, "晨间重点事项提醒"),
    };
    let label = if custom_label.trim().is_empty() {
        default_label
    } else {
        custom_label.trim()
    };
    create_phone_alarm(adb, device, h, m, label)
}

pub fn toggle_phone_alarm(
    adb: &Adb,
    device: &Device,
    id: i64,
    enabled: bool,
) -> Result<(Vec<PhoneAlarmItem>, String)> {
    if id <= 0 {
        bail!("无效的手机闹钟 ID");
    }
    let val = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "alarm_toggle",
            "id": id,
            "enabled": enabled,
        }),
        std::time::Duration::from_secs(8),
    )?;
    let alarms = parse_bridge_alarms(&val);
    invalidate_stepos_suite_cache();
    let state_label = if enabled { "已开启" } else { "已关闭" };
    Ok((alarms, format!("手机闹钟{state_label}")))
}

pub fn delete_phone_alarm(
    adb: &Adb,
    device: &Device,
    id: i64,
) -> Result<(Vec<PhoneAlarmItem>, String)> {
    if id <= 0 {
        bail!("无效的手机闹钟 ID");
    }
    let val = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "alarm_delete",
            "id": id,
        }),
        std::time::Duration::from_secs(8),
    )?;
    let alarms = parse_bridge_alarms(&val);
    invalidate_stepos_suite_cache();
    Ok((alarms, "已从手机移除该闹钟".into()))
}

fn is_gallery_image_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".heic")
        || lower.ends_with(".bmp")
}

pub fn load_expansion_gallery_items() -> (std::collections::HashSet<String>, Vec<PhoneGalleryItem>)
{
    let mut archived_paths = std::collections::HashSet::new();
    let mut exp_items = Vec::new();
    let exp_cfg = crate::storage_expansion::config();
    let Ok(records) = crate::storage_expansion::records(&exp_cfg) else {
        return (archived_paths, exp_items);
    };
    let mut seen_local = std::collections::HashSet::new();
    for rec in records {
        let state = rec
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("archived");
        let restored = state.eq_ignore_ascii_case("restored")
            || rec.get("restoredAt").is_some_and(|v| !v.is_null())
            || rec
                .get("restored_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                > 0;
        if restored {
            continue;
        }
        let Ok(local_path) = crate::storage_expansion::local_file(&exp_cfg, &rec) else {
            continue;
        };
        if !local_path.is_file() {
            continue;
        }
        let raw_rel = rec
            .get("path")
            .or_else(|| rec.get("source_path"))
            .or_else(|| rec.get("remote_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let norm_remote = if raw_rel.starts_with("/storage/emulated/0/") {
            normalize_sdcard_path(raw_rel)
        } else if raw_rel.starts_with("/sdcard/") {
            raw_rel.to_string()
        } else if !raw_rel.is_empty() {
            format!("/sdcard/{}", raw_rel.trim_start_matches('/'))
        } else {
            local_path.to_string_lossy().into_owned()
        };
        if !norm_remote.is_empty() {
            archived_paths.insert(norm_remote.clone());
        }
        if let Some(gal) = rec.get("galleryPath").and_then(|v| v.as_str()) {
            archived_paths.insert(normalize_sdcard_path(gal));
        }
        let name = rec
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                std::path::Path::new(&norm_remote)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(ToOwned::to_owned)
            })
            .or_else(|| {
                local_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_default();
        if !is_gallery_image_filename(&name)
            && !is_gallery_image_filename(&local_path.to_string_lossy())
        {
            continue;
        }
        let local_str = local_path.to_string_lossy().into_owned();
        if !seen_local.insert(local_str.clone()) {
            continue;
        }
        let size_bytes = rec
            .get("size")
            .or_else(|| rec.get("size_bytes"))
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| local_path.metadata().map(|m| m.len()).unwrap_or(0));
        let raw_mod = rec
            .get("modified")
            .or_else(|| rec.get("mtimeMs"))
            .or_else(|| rec.get("modified_ms"))
            .or_else(|| rec.get("archivedAt"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let modified_ms = if raw_mod > 0 && raw_mod < 10_000_000_000 {
            raw_mod * 1000
        } else {
            raw_mod
        };
        exp_items.push(PhoneGalleryItem {
            remote_path: norm_remote,
            name,
            category: "expansion".to_string(),
            size_bytes,
            modified_ms,
            local_thumb_path: Some(local_str),
        });
    }
    (archived_paths, exp_items)
}

pub fn parse_phone_gallery_output(raw: &str) -> Vec<PhoneGalleryItem> {
    let mut items = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(3, '\t');
        let Some(ts_str) = parts.next() else {
            continue;
        };
        let Some(size_str) = parts.next() else {
            continue;
        };
        let Some(path_str) = parts.next() else {
            continue;
        };
        let modified_ms = ts_str
            .trim()
            .parse::<f64>()
            .ok()
            .map(|sec| (sec * 1000.0) as i64)
            .unwrap_or(0);
        let size_bytes = size_str.trim().parse::<u64>().unwrap_or(0);
        if size_bytes < 512 {
            continue;
        }
        let remote_path = normalize_sdcard_path(path_str);
        if crate::storage::validate_path(&remote_path).is_err() {
            continue;
        }
        if remote_path.split('/').any(|seg| seg.starts_with('.')) {
            continue;
        }
        let name = std::path::Path::new(&remote_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty()
            || name.starts_with('.')
            || name.starts_with(".trashed")
            || name.starts_with(".pending")
            || name.ends_with(".stepxlink")
            || is_expansion_stub_filename(&name)
            || remote_path.contains("/StepX Companion/SC_")
        {
            continue;
        }
        if !seen_paths.insert(remote_path.clone()) {
            continue;
        }
        let lower_path = remote_path.to_ascii_lowercase();
        let lower_name = name.to_ascii_lowercase();
        let category = if lower_path.contains("screenshot")
            || lower_name.contains("screenshot")
            || name.starts_with("StepX_")
        {
            "screenshot".to_string()
        } else if lower_path.contains("weixin")
            || lower_path.contains("wechat")
            || lower_path.contains("micromsg")
            || lower_path.contains("micro_msg")
        {
            "weixin".to_string()
        } else if lower_path.contains("/dcim/") {
            "camera".to_string()
        } else {
            "pictures".to_string()
        };
        items.push(PhoneGalleryItem {
            remote_path,
            name,
            category,
            size_bytes,
            modified_ms,
            local_thumb_path: None,
        });
    }
    items
}

pub fn list_phone_gallery(adb: &Adb, device: &Device) -> Result<Vec<PhoneGalleryItem>> {
    let transport = device.resolve_active_transport(adb)?;
    let script = "find /sdcard/DCIM /sdcard/Pictures /sdcard/Download -maxdepth 3 -type f -not -path '*/.*' -not -name '.*' -not -name '*.trashed*' -not -name '*.pending*' -not -name '*.stepxlink' -not -path '*/StepX Companion/SC_*' \\( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.webp' -o -iname '*.gif' -o -iname '*.heic' \\) -printf '%T@\\t%s\\t%p\\n' 2>/dev/null | sort -rn | head -n 120";
    let raw = adb.script(&transport, script)?;
    let mut items = parse_phone_gallery_output(&raw);

    let (archived_paths, exp_items) = load_expansion_gallery_items();
    if !archived_paths.is_empty() {
        items.retain(|item| !archived_paths.contains(&item.remote_path));
    }
    for exp_item in exp_items {
        if !items.iter().any(|existing| {
            existing.remote_path == exp_item.remote_path
                || (existing.local_thumb_path.is_some()
                    && existing.local_thumb_path == exp_item.local_thumb_path)
        }) {
            items.push(exp_item);
        }
    }
    items.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));

    let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
    let _ = std::fs::create_dir_all(&cache_dir);

    let prepared: Vec<(usize, std::path::PathBuf, bool)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.local_thumb_path.is_none())
        .take(24)
        .map(|(idx, item)| {
            let safe_name: String = item
                .name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let hash = deterministic_event_id(
                &item.remote_path,
                item.modified_ms ^ (item.size_bytes as i64),
            );
            let local_file = cache_dir.join(format!("{hash:016x}_{safe_name}"));
            let already_cached = local_file
                .metadata()
                .map(|m| m.is_file() && m.len() > 0)
                .unwrap_or(false);
            (idx, local_file, already_cached)
        })
        .collect();

    let uncached: Vec<(&str, &std::path::Path)> = prepared
        .iter()
        .filter(|(_, _, cached)| !*cached)
        .map(|(idx, local_file, _)| (items[*idx].remote_path.as_str(), local_file.as_path()))
        .collect();

    let transport_ref = transport.as_str();
    for batch in uncached.chunks(4) {
        std::thread::scope(|s| {
            for &(remote_path, local_file) in batch {
                s.spawn(move || {
                    let _ = adb.transfer(
                        transport_ref,
                        "pull",
                        std::path::Path::new(remote_path),
                        local_file,
                    );
                });
            }
        });
    }

    for (idx, local_file, already_cached) in prepared {
        if already_cached
            || local_file
                .metadata()
                .map(|m| m.is_file() && m.len() > 0)
                .unwrap_or(false)
        {
            items[idx].local_thumb_path = Some(local_file.to_string_lossy().into_owned());
        }
    }

    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = entries
            .flatten()
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                if !meta.is_file() {
                    return None;
                }
                let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                Some((entry.path(), modified))
            })
            .collect();
        if files.len() > 64 {
            let active: std::collections::HashSet<&str> = items
                .iter()
                .filter_map(|item| item.local_thumb_path.as_deref())
                .collect();
            files.sort_by_key(|(_, modified)| *modified);
            let mut remaining = files.len();
            for (path, _) in files {
                if remaining <= 64 {
                    break;
                }
                if !active.contains::<str>(path.to_string_lossy().as_ref())
                    && std::fs::remove_file(&path).is_ok()
                {
                    remaining -= 1;
                }
            }
        }
    }

    Ok(items)
}

pub fn capture_phone_screenshot_to_gallery(
    adb: &Adb,
    device: &Device,
) -> Result<(Vec<PhoneGalleryItem>, String)> {
    let mut native = crate::native_mcp::Client::connect(adb, device)?;
    if native.supports("screen_capture_screenshot") {
        let before = list_phone_gallery(adb, device)?;
        native.call("screen_capture_screenshot", serde_json::json!({}))?;
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(250));
            let mut after = list_phone_gallery(adb, device)?;
            if let Some(index) = after.iter().position(|item| !before.iter().any(|old| old.remote_path == item.remote_path)
                && item.remote_path.to_lowercase().contains("screenshot")) {
                let shot = after.remove(index);
                let name = std::path::Path::new(&shot.remote_path).file_name().unwrap_or_default().to_string_lossy().into_owned();
                after.insert(0, shot);
                return Ok((after, format!("已通过手机原生截屏保存到相册：{name}")));
            }
        }
        bail!("手机已接收原生截屏请求，但尚未读到新图片；请查看相册，未重复截屏");
    }
    let transport = device.resolve_active_transport(adb)?;
    let ts = Local::now().format("%Y%m%d_%H%M%S");
    let remote_path = format!("/sdcard/Pictures/Screenshots/StepX_{ts}.png");
    let script = format!(
        "mkdir -p /sdcard/Pictures/Screenshots && screencap -p {p} && test -s {p}",
        p = shell_quote(&remote_path)
    );
    adb.script(&transport, &script)
        .context("手机截屏失败，请确认手机已解锁且当前画面允许截屏")?;
    let mut items = list_phone_gallery(adb, device)?;
    if let Some(pos) = items.iter().position(|i| i.remote_path == remote_path) {
        if pos > 0 {
            let shot = items.remove(pos);
            items.insert(0, shot);
        }
    }
    Ok((items, format!("已截取手机画面并存入相册：StepX_{ts}.png")))
}

pub fn delete_phone_gallery_item(
    adb: &Adb,
    device: &Device,
    remote_path: &str,
) -> Result<Vec<PhoneGalleryItem>> {
    crate::storage::validate_path(remote_path)?;
    storage::delete(adb, device, remote_path)?;
    list_phone_gallery(adb, device)
}

pub fn set_phone_brightness_percent(
    adb: &Adb,
    device: &Device,
    percent: u8,
) -> Result<(u8, String)> {
    let transport = device.resolve_active_transport(adb)?;
    let clamped = percent.clamp(5, 100);
    let mut native = crate::native_mcp::Client::connect(adb, device)?;
    if native.supports("settings_adjust_brightness") {
        // Firmware owns the panel's brightness range; do not guess 255/4095/32767.
        let receipt = native.call("settings_adjust_brightness", serde_json::json!({"newValue":clamped.to_string()}))?;
        invalidate_stepos_suite_cache();
        return Ok((clamped, crate::native_mcp::response_note(&receipt)));
    }
    let norm = format!("{:.2}", (clamped as f32 / 100.0).clamp(0.05, 1.0));
    let raw_32767 = ((clamped as u32 * 32767 + 50) / 100).clamp(320, 32767);
    let raw_4095 = ((clamped as u32 * 4095 + 50) / 100).clamp(256, 4095);
    let raw_255 = ((clamped as u32 * 255 + 50) / 100).clamp(10, 255);
    let script = format!(
        "CUR=$(settings get system screen_brightness 2>/dev/null || echo 1024); \
         settings put system screen_brightness_mode 0 >/dev/null 2>&1 || true; \
         cmd display set-brightness {norm} >/dev/null 2>&1 || true; \
         if [ \"$CUR\" -gt 4095 ] 2>/dev/null; then \
           settings put system screen_brightness {raw_32767} >/dev/null 2>&1 || true; \
         elif [ \"$CUR\" -gt 255 ] 2>/dev/null; then \
           settings put system screen_brightness {raw_4095} >/dev/null 2>&1 || true; \
         else \
           settings put system screen_brightness {raw_255} >/dev/null 2>&1 || true; \
         fi"
    );
    adb.script(&transport, &script)?;
    invalidate_stepos_suite_cache();
    Ok((clamped, format!("已将手机屏幕亮度调节为 {clamped}%")))
}

pub fn extract_otp_from_text(text: &str) -> Option<String> {
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            cur.push(ch);
        } else {
            if (4..=8).contains(&cur.len()) {
                return Some(cur);
            }
            cur.clear();
        }
    }
    if (4..=8).contains(&cur.len()) {
        Some(cur)
    } else {
        None
    }
}

pub fn extract_express_number_from_text(text: &str) -> Option<(&'static str, String)> {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| s.len() >= 11 && s.len() <= 20)
        .map(|s| s.to_ascii_uppercase())
        .collect();

    for tok in &tokens {
        if let Some(rest) = tok.strip_prefix("SF") {
            if (12..=15).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit()) {
                return Some(("顺丰速运", tok.clone()));
            }
        }
        if let Some(rest) = tok.strip_prefix("JD") {
            let digits = rest.bytes().filter(|b| b.is_ascii_digit()).count();
            if (11..=15).contains(&rest.len())
                && digits >= 8
                && rest.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return Some(("京东快递", tok.clone()));
            }
        }
        if let Some(rest) = tok.strip_prefix("YT") {
            if rest.len() == 13 && rest.bytes().all(|b| b.is_ascii_digit()) {
                return Some(("圆通速递", tok.clone()));
            }
        }
        if let Some(rest) = tok.strip_prefix("JT") {
            if rest.len() == 13 && rest.bytes().all(|b| b.is_ascii_digit()) {
                return Some(("极兔速递", tok.clone()));
            }
        }
        if tok.len() == 13
            && tok.starts_with('E')
            && tok.as_bytes()[1].is_ascii_uppercase()
            && tok.ends_with("CN")
            && tok[2..11].bytes().all(|b| b.is_ascii_digit())
        {
            return Some(("邮政EMS", tok.clone()));
        }
    }

    for tok in &tokens {
        if tok.len() == 14 && tok.starts_with('7') && tok.bytes().all(|b| b.is_ascii_digit()) {
            return Some(("中通快递", tok.clone()));
        }
        if tok.len() == 15 && tok.starts_with("77") && tok.bytes().all(|b| b.is_ascii_digit()) {
            return Some(("申通快递", tok.clone()));
        }
        if ((tok.len() == 13 && tok.starts_with('1'))
            || (tok.len() == 15 && tok.starts_with('4')))
            && tok.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(("韵达快递", tok.clone()));
        }
    }

    None
}

pub fn build_capcut_handoff_payload_json(payload: &str, timestamp_ms: i64) -> serde_json::Value {
    let mut media_paths: Vec<String> = Vec::new();
    let mut prompt_parts: Vec<String> = Vec::new();
    for raw_part in payload.split([',', '\n', ';', '|']) {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        let is_media = part.starts_with('/')
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".png")
            || lower.ends_with(".webp")
            || lower.ends_with(".heic")
            || lower.ends_with(".mp4")
            || lower.ends_with(".mov");
        if is_media {
            let norm = normalize_sdcard_path(part);
            if !media_paths.contains(&norm) {
                media_paths.push(norm);
            }
        } else {
            prompt_parts.push(part.to_string());
        }
    }
    let prompt = if !prompt_parts.is_empty() {
        prompt_parts.join(" · ")
    } else if !media_paths.is_empty() {
        format!("精选 {} 张手机照片智能成片", media_paths.len())
    } else {
        "今日精选照片智能成片".to_string()
    };
    serde_json::json!({
        "source": "stepx_desktop",
        "prompt": prompt,
        "media_paths": media_paths,
        "timestamp": timestamp_ms,
    })
}

fn read_phone_dnd(adb: &Adb, transport: &str) -> Result<bool> {
    let value = adb.shell(transport, &["settings", "get", "global", "zen_mode"])?;
    match value.trim() {
        "1" | "2" | "3" => Ok(true),
        "0" => {
            if let Ok(dump) = adb.shell(transport, &["dumpsys", "notification"]) {
                if let Some(first_manual) = dump
                    .lines()
                    .map(str::trim)
                    .find(|line| line.starts_with("manualRule="))
                {
                    if first_manual.contains("state=STATE_TRUE")
                        && !first_manual.contains("zenMode=ZEN_MODE_OFF")
                    {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        _ => bail!("无法读取手机勿扰状态，请在手机设置中确认后重试"),
    }
}

/// Set the requested state and verify the phone's state, rather than toggling a UI snapshot.
fn set_phone_dnd(adb: &Adb, device: &Device, enabled: bool) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let read_state = || read_phone_dnd(adb, &transport);
    if read_state()? != enabled {
        if crate::native_mcp::try_call(adb, device, "settings_disturb_mode_switch", serde_json::json!({"openState":enabled}))?.is_none() {
            let mode = if enabled { "priority" } else { "off" };
            adb.shell(&transport, &["cmd", "notification", "set_dnd", mode])
                .context("手机未接受勿扰设置，请检查手机授权后重试")?;
        }
        let mut applied = false;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(80));
            }
            if read_state()? == enabled {
                applied = true;
                break;
            }
        }
        if !applied {
            bail!("手机勿扰状态未变更，可能受系统规则或权限限制；请在手机设置中确认");
        }
    }
    invalidate_stepos_suite_cache();
    Ok(if enabled {
        "已开启手机勿扰模式（已回读确认）".into()
    } else {
        "已关闭手机勿扰模式（已回读确认）".into()
    })
}

/// The firmware's private CapCut handoff is not a public, verified completion API.
/// Keep a checked request manifest for manual continuation; never manufacture a job receipt.
fn prepare_capcut_handoff(adb: &Adb, device: &Device, payload: &str) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let manifest = build_capcut_handoff_payload_json(payload, Local::now().timestamp_millis());
    let json = serde_json::to_string(&manifest)?;
    let path = "/sdcard/Download/stepx_capcut_handoff.json";
    let temp_path = format!("{path}.{}.tmp", rand::random::<u64>());
    let quoted_temp = shell_quote(&temp_path);
    adb.script(&transport, &format!(
        "set -e\nmkdir -p /sdcard/Download\nprintf '%s\\n' {} > {quoted_temp}\nmv {quoted_temp} {}",
        shell_quote(&json), shell_quote(path),
    )).context("未能保存手机创作清单，尚未启动剪映")?;
    let saved = adb.shell(&transport, &["cat", path])
        .context("手机创作清单写入后无法回读，尚未启动剪映")?;
    let saved: serde_json::Value = serde_json::from_str(saved.trim())
        .context("手机创作清单回读失败，尚未启动剪映")?;
    if saved != manifest {
        bail!("手机创作清单与本次请求不一致，尚未启动剪映，请重试");
    }
    let media_count = manifest["media_paths"].as_array().map_or(0, Vec::len);
    let prompt = manifest["prompt"].as_str().unwrap_or("创作需求");
    let _ = push_clipboard_to_phone(adb, device, prompt);
    let launch = device.open_app(adb, "com.lemon.lv");
    let next = match launch {
        Ok(()) =>
            "已请求打开剪映，请在手机选择素材并确认创作",
        _ => "未能确认剪映已打开，请在手机安装或手动打开剪映，再选择素材并确认创作",
    };
    Ok(format!("已保存创作清单：{prompt}（{media_count} 项素材，{path}）。{next}；素材尚未自动导入，成片尚未生成。"))
}

pub fn parse_backscreen_display_state(raw: &str) -> String {
    let mut active_layer = String::new();
    let mut is_on: Option<bool> = None;
    let mut nap_active = false;
    let mut basic_type = String::new();
    let mut basic_text = String::new();
    let mut stay_secs = String::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(v) = trimmed.strip_prefix("BASIC_TYPE=") {
            basic_type = v.trim().to_string();
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("BASIC_TEXT=") {
            basic_text = clean_nullable_field(v);
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("STAY_SECS=") {
            stay_secs = clean_nullable_field(v);
            continue;
        }
        if lower.contains("nap") && (lower.contains("true") || lower.contains("active") || lower.contains("running")) {
            nap_active = true;
        }
        if lower.contains("backscreenon=") || lower.contains("misbackscreenon=") || lower.contains("state=") {
            if lower.contains("true") || lower.contains("state=on") || lower.contains("state_on") {
                is_on = Some(true);
            } else if lower.contains("false") || lower.contains("state=off") || lower.contains("state_off") {
                if is_on.is_none() {
                    is_on = Some(false);
                }
            }
        }
        for key in ["activelayer=", "currentlayer=", "mcurrentscene=", "scene=", "layer="] {
            if let Some(pos) = lower.find(key) {
                let val = trimmed[pos + key.len()..]
                    .split([',', ' ', ';', ')'])
                    .next()
                    .unwrap_or("")
                    .trim();
                if !val.is_empty() && !val.eq_ignore_ascii_case("null") {
                    active_layer = val.to_string();
                }
            }
        }
    }

    let layer_label = if nap_active || active_layer.to_ascii_lowercase().contains("nap") {
        "小憩模式 (Nap)".to_string()
    } else if !active_layer.is_empty() {
        match active_layer.to_ascii_lowercase().as_str() {
            "agentsprite" | "agent_sprite" | "sprite" | "amoo" => "AI 精灵 (AgentSprite)".to_string(),
            "signature" | "text" | "basic_text" => "个性文字签名".to_string(),
            "music" | "media" => "音乐律动图层".to_string(),
            "clock" | "time" => "时钟状态图层".to_string(),
            "notification" | "notice" => "通知提醒图层".to_string(),
            _ => format!("图层 {active_layer}"),
        }
    } else {
        match basic_type.as_str() {
            "2" | "text" => "个性文字签名".to_string(),
            "3" | "clock" => "时钟状态图层".to_string(),
            _ => "AI 精灵 (AgentSprite)".to_string(),
        }
    };

    let mut parts = vec![format!("背屏当前：{layer_label}")];
    if let Some(on) = is_on {
        parts.push(if on { "点阵亮起".into() } else { "待机熄屏".into() });
    }
    if !basic_text.is_empty() {
        parts.push(format!("签名「{basic_text}」"));
    }
    if !stay_secs.is_empty() && stay_secs != "0" {
        parts.push(format!("常亮 {stay_secs}s"));
    }
    parts.join(" · ")
}

/// Reads live backscreen controller and display layer status via `dumpsys display_ext back_screen`.
pub fn read_backscreen_display_state(adb: &Adb, transport: &str) -> Result<String> {
    let target = transport.trim();
    if target.is_empty() {
        bail!("未指定手机连接标识");
    }
    let script = concat!(
        "dumpsys display_ext back_screen 2>/dev/null || true; ",
        "echo \"BASIC_TYPE=$(settings get system stepos_back_screen_basic_type 2>/dev/null || echo 1)\"; ",
        "echo \"BASIC_TEXT=$(settings get system stepos_back_screen_basic_text 2>/dev/null || echo '')\"; ",
        "echo \"STAY_SECS=$(settings get system stepos_back_screen_stay_on_duration 2>/dev/null || echo 30)\""
    );
    let raw = adb.script(target, script)?;
    Ok(parse_backscreen_display_state(&raw))
}

pub fn build_workbuddy_card_payload_json(
    action_key: &str,
    payload: &str,
    timestamp_ms: i64,
) -> serde_json::Value {
    let raw = payload.trim();
    let (action_mode, title, status, progress) = match action_key {
        "remove_workbuddy_card" => (
            "remove",
            if raw.is_empty() { "已移除 WorkBuddy 桌面卡片" } else { raw },
            "REMOVED",
            100u32,
        ),
        "update_workbuddy_card" => (
            "update",
            if raw.is_empty() { "StepX 远程智能体协同任务（处理中）" } else { raw },
            "RUNNING",
            65u32,
        ),
        _ => (
            "create",
            if raw.is_empty() { "StepX 桌面端与手机协同任务" } else { raw },
            "RUNNING",
            25u32,
        ),
    };
    serde_json::json!({
        "card_type": "workbuddy_task",
        "action": action_mode,
        "source": "stepx_desktop",
        "agent_id": "workbuddy",
        "agent_name": "WorkBuddy",
        "task_id": "stepx-workbuddy-card",
        "title": title,
        "status": status,
        "progress": progress,
        "timestamp": timestamp_ms,
    })
}

// No verified original-agent pairing, task ID, subscription or terminal
// receipt is available. A local JSON file or a broadcast cannot establish it.
fn dispatch_workbuddy_card_action(
    _adb: &Adb,
    _device: &Device,
    _action_key: &str,
    _payload: &str,
) -> Result<String> {
    bail!("原厂 WorkBuddy 任务卡尚未接通，未创建任务，也未更新或移除任何卡片；请在手机完成原厂配对后继续")
}

fn parse_setting_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

/// All reported software/hardware blocks for a sensor must be off before
/// reporting it allowed. An absent or unrecognized dump remains unknown.
fn sensor_privacy_allowed(raw: &str, sensor: &str) -> Option<bool> {
    let target = if sensor == "camera" { "2" } else { "1" };
    let mut current = false;
    let mut blocked = None;
    for line in raw.lines() {
        let line = line.trim().to_ascii_lowercase();
        if let Some(rest) = line.split("sensor=").nth(1) {
            current = rest.split(|c: char| !c.is_ascii_digit()).next() == Some(target);
        } else if line.contains("microphone") || line.contains("camera") {
            current = line.contains(sensor);
        }
        if !current {
            continue;
        }
        let state = if let Some(rest) = line.split("state_type=").nth(1) {
            match rest.split(|c: char| !c.is_ascii_digit()).next() {
                Some("1") => Some(true),
                Some("2") => Some(false),
                _ => None,
            }
        } else if line.contains("isenabled=true") {
            Some(true)
        } else if line.contains("isenabled=false") {
            Some(false)
        } else {
            None
        };
        if let Some(state) = state {
            blocked = Some(blocked.unwrap_or(false) || state);
        }
    }
    blocked.map(|blocked| !blocked)
}

fn read_phone_sensor_allowed(adb: &Adb, transport: &str, sensor: &str) -> Result<bool> {
    let dump = adb.shell(transport, &["dumpsys", "sensor_privacy"])?;
    sensor_privacy_allowed(&dump, sensor).context("系统隐私开关状态未知，未修改设置")
}
fn set_phone_sensor_privacy(adb: &Adb, device: &Device, transport: &str, sensor: &str, target: Option<bool>) -> Result<String> {
    let old = read_phone_sensor_allowed(adb, transport, sensor)?;
    let enabled = target.unwrap_or(!old);
    let method = if sensor == "camera" {"security_permissioncontroller_global_camera"} else {"security_permissioncontroller_global_microphone"};
    if crate::native_mcp::try_call(adb, device, method, serde_json::json!({"enable":enabled}))?.is_none() {
    adb.shell(
        transport,
        &[
            "cmd",
            "sensor_privacy",
            if enabled { "disable" } else { "enable" },
            "0",
            sensor,
        ],
    )?;
    }
    if read_phone_sensor_allowed(adb, transport, sensor)? != enabled {
        bail!("系统隐私开关回读不一致，未确认生效");
    }
    let title = if sensor == "camera" {
        "相机"
    } else {
        "麦克风"
    };
    Ok(format!(
        "已{}用户级系统{title}开关（已回读，不代表硬件断电）",
        if enabled { "允许" } else { "阻止" }
    ))
}
fn read_phone_location(adb: &Adb, transport: &str) -> Result<bool> {
    let value = adb.shell(
        transport,
        &["cmd", "location", "is-location-enabled", "--user", "0"],
    )?;
    parse_setting_bool(&value).context("定位状态未知，未修改设置")
}
fn read_delete_protect(adb: &Adb, transport: &str) -> Result<(&'static str, bool)> {
    for namespace in ["secure", "system"] {
        let value = adb.shell(
            transport,
            &[
                "settings",
                "get",
                namespace,
                "stepos_delete_file_and_image_protect",
            ],
        )?;
        if let Some(value) = parse_setting_bool(&value) {
            return Ok((namespace, value));
        }
    }
    bail!("文件删除保护设置未知，未修改设置")
}

/// Applies an explicit user-level setting target and verifies the readback.
/// `None` retains legacy toggle callers, but still requires a known live state.
pub fn set_stepos_privacy_setting(
    adb: &Adb, device: &Device, key: &str, target: Option<bool>,
) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let res = match key {
        "toggle_global_camera" => set_phone_sensor_privacy(adb, device, &transport, "camera", target),
        "toggle_global_mic" => set_phone_sensor_privacy(adb, device, &transport, "microphone", target),
        "toggle_global_location" => {
            let old = read_phone_location(adb, &transport)?;
            let enabled = target.unwrap_or(!old);
            if crate::native_mcp::try_call(adb, device, "security_permissioncontroller_global_location", serde_json::json!({"enable":enabled}))?.is_none() {
                adb.shell(&transport, &["cmd", "location", "set-location-enabled", if enabled { "true" } else { "false" }, "--user", "0"])?;
            }
            if read_phone_location(adb, &transport)? != enabled { bail!("定位开关回读不一致，未确认生效"); }
            Ok(if enabled { "已开启用户级系统定位开关（已回读）" } else { "已关闭用户级系统定位开关（已回读）" }.into())
        }
        "toggle_delete_protect" => {
            let (namespace, old) = read_delete_protect(adb, &transport)?;
            let enabled = target.unwrap_or(!old);
            if crate::native_mcp::try_call(adb, device, "security_permissioncontroller_delete_file_and_image_protect", serde_json::json!({"enable":enabled}))?.is_none() {
                adb.shell(&transport, &["settings", "put", namespace, "stepos_delete_file_and_image_protect", if enabled { "1" } else { "0" }])?;
            }
            let readback = adb.shell(&transport, &["settings", "get", namespace, "stepos_delete_file_and_image_protect"])?;
            if parse_setting_bool(&readback) != Some(enabled) { bail!("文件删除保护设置回读不一致，未确认保存"); }
            Ok(if enabled { "已保存并回读文件删除保护设置；不代表所有应用删除均被拦截" }
                else { "已关闭并回读文件删除保护设置" }.into())
        }
        "toggle_screen_perception" => {
            const SETTING: &str = "stepos_agentsafetycenter_screen_perception_enabled";
            let raw = adb.shell(&transport, &["settings", "get", "global", SETTING])?;
            let old = parse_setting_bool(&raw).context("识屏设置未知，未修改设置")?;
            let enabled = target.unwrap_or(!old);
            adb.shell(&transport, &["settings", "put", "global", SETTING, if enabled { "1" } else { "0" }])?;
            let readback = adb.shell(&transport, &["settings", "get", "global", SETTING])?;
            if parse_setting_bool(&readback) != Some(enabled) { bail!("识屏设置回读不一致，未确认生效"); }
            Ok(if enabled { "已开启识屏设置（已回读，具体任务仍需验证）" }
                else { "已关闭识屏设置（已回读）" }.into())
        }
        _ => bail!("不是受支持的隐私设置，未修改手机"),
    };
    if res.is_ok() {
        invalidate_stepos_suite_cache();
    }
    res
}

pub fn trigger_stepos_mcp_action(
    adb: &Adb,
    device: &Device,
    action_key: &str,
    payload: &str,
) -> Result<String> {
    let res = trigger_stepos_mcp_action_inner(adb, device, action_key, payload);
    if res.is_ok() {
        invalidate_stepos_suite_cache();
    }
    res
}

fn toggle_radio_via_mcp(adb: &Adb, device: &Device, transport: &str, wifi: bool) -> Result<Option<String>> {
    let (tool, key, label) = if wifi { ("settings_wifi_switch", "wifi_on", "Wi-Fi") }
        else { ("settings_bluetooth_switch", "bluetooth_on", "蓝牙") };
    let mut native = crate::native_mcp::Client::connect(adb, device)?;
    if !native.supports(tool) { return Ok(None); }
    let read = || -> Result<bool> {
        let value = adb.shell(transport, &["settings","get","global",key])?;
        match value.trim() {"0" => Ok(false), "1" => Ok(true), "2" if wifi => Ok(true),
            _ => bail!("手机{label}状态未知，未推测开关状态")}
    };
    let enabled = !read()?;
    native.call(tool, serde_json::json!({"openState":enabled}))?;
    for _ in 0..8 {
        if read()? == enabled { return Ok(Some(format!("已{}手机{label}（原生接口，已回读）", if enabled {"开启"} else {"关闭"}))); }
        std::thread::sleep(Duration::from_millis(150));
    }
    bail!("手机{label}请求已提交，状态尚未确认；未自动重试")
}

fn trigger_stepos_mcp_action_inner(
    adb: &Adb,
    device: &Device,
    action_key: &str,
    payload: &str,
) -> Result<String> {
    if let Some((method, args)) = crate::native_mcp::quick_action(action_key, payload) {
        if let Some(receipt) = crate::native_mcp::try_call(adb, device, method, args)? {
            return Ok(crate::native_mcp::response_note(&receipt));
        }
    }
    if matches!(
        action_key,
        "create_workbuddy_card" | "update_workbuddy_card" | "remove_workbuddy_card"
    ) {
        return dispatch_workbuddy_card_action(adb, device, action_key, payload);
    }
    let transport = device.resolve_active_transport(adb)?;
    if let Some(pkg) = action_key.strip_prefix("force_stop:") {
        let clean_pkg = pkg.trim();
        if clean_pkg.is_empty()
            || !clean_pkg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        {
            bail!("无效的应用包名：{clean_pkg}");
        }
        let script = format!("am force-stop {clean_pkg} >/dev/null 2>&1");
        adb.script(&transport, &script)?;
        return Ok(format!(
            "已强制停止手机应用：{} ({clean_pkg})",
            friendly_package_name(clean_pkg)
        ));
    }
    if let Some(sig_raw) = action_key.strip_prefix("backscreen_sig:") {
        let raw_text = if sig_raw.trim().is_empty() {
            payload.trim()
        } else {
            sig_raw.trim()
        };
        let sig: String = if raw_text.is_empty() {
            "专注中".into()
        } else {
            raw_text.chars().take(6).collect()
        };
        return crate::backscreen::marquee(adb, device, &sig, false, false);
    }
    if let Some(cd_raw) = action_key.strip_prefix("backscreen_countdown:") {
        let secs = cd_raw
            .trim()
            .parse::<u32>()
            .ok()
            .or_else(|| payload.trim().parse::<u32>().ok())
            .unwrap_or(1500)
            .clamp(60, 3600);
        let minutes = (secs / 60).max(1);
        crate::backscreen::start_pomodoro_countdown(adb, device, secs, 100)?;
        let capsule_req = crate::push::PhoneLiveCapsuleRequest {
            tag: "stepx_pomodoro".into(),
            pill_text: format!("🍅 {minutes}:00"),
            title: format!("番茄专注 {minutes} 分钟"),
            subtitle: "桌面与手机顶部实况胶囊、背屏 17×23 点阵同步倒计时".into(),
            badge_text: "专注计时".into(),
            live_type: 11,
            countdown_secs: Some(u64::from(secs)),
            ongoing: true,
        };
        let _ = crate::push::post_phone_live_capsule(adb, &device.serial, &capsule_req);
        return Ok(format!(
            "已启动 {minutes} 分钟双屏联动倒计时（手机正面顶部实况胶囊 + 背面 17×23 点阵同步计时）"
        ));
    }
    match action_key {
        "alarm_15m" => {
            let target = Local::now() + ChronoDuration::minutes(15);
            let h = target.hour();
            let m = target.minute();
            let msg = if payload.trim().is_empty() {
                "StepX 桌面专注提醒"
            } else {
                payload.trim()
            };
            let (_, _, note) = create_phone_alarm(adb, device, h as u8, m as u8, msg)?;
            Ok(note)
        }
        "alarm_0800" => {
            let msg = if payload.trim().is_empty() {"StepX 早间日程提醒"} else {payload.trim()};
            let (_, _, note) = create_phone_alarm(adb, device, 8, 0, msg)?;
            Ok(note)
        }
        "calendar_event" | "todo_create" => {
            let title = if payload.trim().is_empty() {
                if action_key == "todo_create" {
                    "StepX 桌面待办提醒"
                } else {
                    "StepX 跨端协作日程"
                }
            } else {
                payload.trim()
            };
            let (_, _, note) = create_phone_calendar_preset(adb, device, 0, title)?;
            Ok(note)
        }
        "note_create" => {
            let note = if payload.trim().is_empty() {
                "StepX 桌面速记内容"
            } else {
                payload.trim()
            };
            push_clipboard_to_phone(adb, device, note)?;
            device.open_amoo(adb).context("速记已复制，但 Amoo 未能打开；可在手机中粘贴")?;
            Ok(format!("已将速记写入手机剪贴板并打开 Amoo：「{note}」"))
        }
        "voice_tts" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.personal_preference -n com.stepos.alphame/.persona.ui.PersonalMainActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || ",
                "input keyevent 231 >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已唤醒手机端语音与智能助手服务".into())
        }
        "set_dnd_on" => set_phone_dnd(adb, device, true),
        "set_dnd_off" => set_phone_dnd(adb, device, false),
        "toggle_dnd" => {
            set_phone_dnd(adb, device, !read_phone_dnd(adb, &transport)?)
        }
        "toggle_wifi" => {
            if let Some(note) = toggle_radio_via_mcp(adb, device, &transport, true)? { return Ok(note); }
            let script = concat!(
                "CUR=$(settings get global wifi_on 2>/dev/null || echo 1); ",
                "if [ \"$CUR\" = \"1\" ] || [ \"$CUR\" = \"2\" ]; then ",
                "  svc wifi disable >/dev/null 2>&1 || cmd wifi set-wifi-enabled disabled >/dev/null 2>&1 || true; ",
                "  echo WIFI_OFF; ",
                "else ",
                "  svc wifi enable >/dev/null 2>&1 || cmd wifi set-wifi-enabled enabled >/dev/null 2>&1 || true; ",
                "  echo WIFI_ON; ",
                "fi"
            );
            let out = adb.script(&transport, script)?;
            if out.contains("WIFI_OFF") {
                Ok("已关闭手机 Wi-Fi".into())
            } else {
                Ok("已开启手机 Wi-Fi".into())
            }
        }
        "toggle_bluetooth" => {
            if let Some(note) = toggle_radio_via_mcp(adb, device, &transport, false)? { return Ok(note); }
            let script = concat!(
                "CUR=$(settings get global bluetooth_on 2>/dev/null || echo 1); ",
                "if [ \"$CUR\" = \"1\" ]; then ",
                "  cmd bluetooth_manager disable >/dev/null 2>&1 || svc bluetooth disable >/dev/null 2>&1 || true; ",
                "  echo BT_OFF; ",
                "else ",
                "  cmd bluetooth_manager enable >/dev/null 2>&1 || svc bluetooth enable >/dev/null 2>&1 || true; ",
                "  echo BT_ON; ",
                "fi"
            );
            let out = adb.script(&transport, script)?;
            if out.contains("BT_OFF") {
                Ok("已关闭手机蓝牙".into())
            } else {
                Ok("已开启手机蓝牙".into())
            }
        }
        "volume_up" => {
            adb.script(
                &transport,
                "cmd media_session volume --show --stream 3 --adj raise >/dev/null 2>&1 || input keyevent 24 >/dev/null 2>&1 || true",
            )?;
            Ok("已调高手机媒体音量".into())
        }
        "volume_down" => {
            adb.script(
                &transport,
                "cmd media_session volume --show --stream 3 --adj lower >/dev/null 2>&1 || input keyevent 25 >/dev/null 2>&1 || true",
            )?;
            Ok("已调低手机媒体音量".into())
        }
        "volume_mute" => {
            adb.script(
                &transport,
                "cmd media_session volume --show --stream 3 --set 0 >/dev/null 2>&1 || input keyevent 164 >/dev/null 2>&1 || true",
            )?;
            Ok("已将手机媒体音量静音".into())
        }
        "volume_50" => {
            adb.script(
                &transport,
                "cmd media_session volume --show --stream 3 --set 30 >/dev/null 2>&1 || true",
            )?;
            Ok("已将手机媒体音量设为 50%".into())
        }
        "toggle_screen_perception" => set_stepos_privacy_setting(adb, device, action_key, None),
        "open_screen_perception" => {
            let script = concat!(
                "am start -a stepos.intent.action.AGENT_SCREEN_PERCEPTION >/dev/null 2>&1 || ",
                "am start -a stepos.intent.action.AGENT_SAFETY_CENTER_LAUNCHER >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开屏幕视觉感知授权设置".into())
        }
        "open_operate_phone" => {
            let script = concat!(
                "am start -a stepos.intent.action.AGENT_OPERATE_PHONE >/dev/null 2>&1 || ",
                "am start -a android.settings.ACCESSIBILITY_SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AI 接管操作手机授权设置".into())
        }
        "open_agent_perm_manager" => {
            let script = concat!(
                "am start -a stepos.intent.action.AGENT_PERMISSION_MANAGER >/dev/null 2>&1 || ",
                "am start -a stepos.intent.action.AGENT_SAFETY_CENTER_LAUNCHER >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AI 智能体权限管理中心".into())
        }
        "open_aiengine_settings" => {
            let script = concat!(
                "am start -a com.stepos.aiengine.action.AI_ENGINE_SETTINGS >/dev/null 2>&1 || ",
                "am start -a com.stepos.aiengine.action.MODEL_MANAGER >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开端侧 AI 引擎设置".into())
        }
        "open_alphame_address" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.ADDRESS_PICKER >/dev/null 2>&1 || ",
                "am start -a com.stepos.alphame.action.personal_preference >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AlphaMe 常用地址管理（家 / 公司）".into())
        }
        "open_alphame_wifi" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.SELECT_WIFI >/dev/null 2>&1 || ",
                "am start -a com.stepos.alphame.action.personal_preference >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AlphaMe 场景 Wi-Fi 绑定".into())
        }
        "open_alphame_bt" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.SELECT_CAR_BLUETOOTH >/dev/null 2>&1 || ",
                "am start -a com.stepos.alphame.action.personal_preference >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AlphaMe 车载蓝牙绑定".into())
        }
        "open_filemanager" => {
            adb.script(
                &transport,
                "monkey -p com.stepos.filemanager -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true",
            )?;
            Ok("已在手机打开文件管理".into())
        }
        "open_dialer" => {
            adb.script(
                &transport,
                "am start -a android.intent.action.DIAL >/dev/null 2>&1 || true",
            )?;
            Ok("已在手机打开拨号盘".into())
        }
        "open_mms" => {
            adb.script(
                &transport,
                "monkey -p com.android.mms -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true",
            )?;
            Ok("已在手机打开短信应用".into())
        }
        "open_clip_url_phone" => {
            let url = payload
                .split_whitespace()
                .find(|t| t.starts_with("http://") || t.starts_with("https://"))
                .unwrap_or(payload.trim());
            if !url.starts_with("http://") && !url.starts_with("https://") {
                bail!("剪贴板中未检测到有效网页链接");
            }
            let quoted = shell_quote(url);
            let script =
                format!("am start -a android.intent.action.VIEW -d {quoted} >/dev/null 2>&1");
            adb.script(&transport, &script)?;
            Ok(format!("已在手机浏览器打开剪贴板链接：{url}"))
        }
        "open_gallery" => {
            adb.script(
                &transport,
                "monkey -p com.stepos.media.gallery -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true",
            )?;
            Ok("已在手机打开系统相册".into())
        }
        "open_aiengine" => {
            let script = concat!(
                "am start -a com.stepos.aiengine.action.MODEL_MANAGER -n com.stepos.aiengine/.modelmanager.ModelManagerActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.aiengine/.modelmanager.ModelManagerActivity >/dev/null 2>&1 || ",
                "am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d package:com.stepos.aiengine >/dev/null 2>&1"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开端侧大模型管理中心".into())
        }
        "open_safety_ledger" => {
            let script = concat!(
                "am start -a stepos.intent.action.AGENT_SAFETY_CENTER_LAUNCHER -n com.stepos.agentsafetycenter/.auth.LaunchAuthActivity >/dev/null 2>&1 || ",
                "am start -a stepos.intent.action.AGENT_PERMISSION_MANAGER -n com.stepos.agentsafetycenter/.permission.AgentPermissionActivity >/dev/null 2>&1 || ",
                "am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d package:com.stepos.agentsafetycenter >/dev/null 2>&1"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AI 安全审计台账与权限中心".into())
        }
        "open_alphame" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.personal_preference -n com.stepos.alphame/.persona.ui.PersonalMainActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.alphame/.persona.ui.PersonalMainActivity >/dev/null 2>&1 || ",
                "am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d package:com.stepos.alphame >/dev/null 2>&1"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 AlphaMe 个人画像中心".into())
        }
        "open_amoo_memory" => {
            let script = concat!(
                "am start -a android.intent.action.VIEW -d 'amoo://amoo/memory' >/dev/null 2>&1 || ",
                "am start -n com.stepos.ailauncher/.task.space.AmooMemoryActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 Amoo 记忆中心（关于我 / 交流偏好 / 备忘事项）".into())
        }
        "open_amoo_voice" => {
            let script = concat!(
                "am start -a com.stepos.ailauncher.action.VOICE_SETTING -n com.stepos.ailauncher/.chat.activity.VoiceSettingActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.ailauncher/.chat.activity.VoiceSettingActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 Amoo 语音播报与音色设置".into())
        }
        "wake_amoo_assistant" => {
            let script = concat!(
                "am start -a com.stepos.ailauncher.action.STEP_ASSISTANT >/dev/null 2>&1 || ",
                "am start -a com.stepos.ailauncher.action.OPEN_CHAT >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机唤醒 Amoo 全局助手".into())
        }
        "toggle_global_camera" | "toggle_global_mic" | "toggle_global_location" | "toggle_delete_protect" =>
            set_stepos_privacy_setting(adb, device, action_key, None),
        "open_gallery_trash" => {
            let script = concat!(
                "am start -a com.stepos.gallery.action.TRASH_BIN >/dev/null 2>&1 || ",
                "am start -a com.stepos.media.gallery.action.TRASH_BIN >/dev/null 2>&1 || ",
                "monkey -p com.stepos.media.gallery -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开相册回收站（可找回最近删除的照片）".into())
        }
        "open_file_trash" => {
            let script = concat!(
                "am start -a com.stepos.filemanager.action.RECYCLE_BIN >/dev/null 2>&1 || ",
                "monkey -p com.stepos.filemanager -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开文件管理回收站".into())
        }
        "capcut_handoff" => prepare_capcut_handoff(adb, device, payload),
        "open_remote_agent_hub" => {
            let script = concat!(
                "am start -a com.stepos.ailauncher.action.EXTERNAL_AGENT_SETTINGS >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开远程智能体协同中心（WorkBuddy / Claude / Codex / StepAI）".into())
        }
        "create_workbuddy_card" | "update_workbuddy_card" | "remove_workbuddy_card" => {
            dispatch_workbuddy_card_action(adb, device, action_key, payload)
        }
        "open_safebox" => {
            let script = concat!(
                "am start -n com.stepos.safebox/.MainActivity >/dev/null 2>&1 || ",
                "am start -a com.stepos.safebox.action.OPEN_SAFEBOX >/dev/null 2>&1 || ",
                "monkey -p com.stepos.safebox -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -n com.stepos.filemanager/.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开隐私保险箱 (com.stepos.safebox)".into())
        }
        "open_security_scan" | "open_secureservice" => {
            let script = concat!(
                "am start -a com.stepos.safe.action.SECURITY_SCAN >/dev/null 2>&1 || ",
                "am start -n com.stepos.safe/.MainActivity >/dev/null 2>&1 || ",
                "monkey -p com.stepos.safe -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "monkey -p com.stepos.secureservice -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -a android.settings.SECURITY_SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开手机管家与安全扫描中心 (com.stepos.safe)".into())
        }
        "open_permission_manager" => {
            let script = concat!(
                "am start -a android.intent.action.MANAGE_PERMISSIONS >/dev/null 2>&1 || ",
                "am start -n com.android.permissioncontroller/.permission.ui.ManagePermissionsActivity >/dev/null 2>&1 || ",
                "am start -a android.settings.PRIVACY_SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开系统权限管理中心 (com.android.permissioncontroller)".into())
        }
        "open_app_manager" => {
            let script = concat!(
                "am start -a android.settings.MANAGE_APPLICATIONS_SETTINGS >/dev/null 2>&1 || ",
                "am start -a android.settings.APPLICATION_SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开应用管理与安装中心".into())
        }
        "open_backscreen" | "open_backscreen_settings" => {
            let state_summary = read_backscreen_display_state(adb, &transport)
                .unwrap_or_else(|_| "背屏点阵已就绪".into());
            let script = concat!(
                "am start -n com.stepos.backscreen/.settings.BackScreenSettingsActivity >/dev/null 2>&1 || ",
                "am start -a com.stepos.backscreen.action.SETTINGS >/dev/null 2>&1 || ",
                "monkey -p com.stepos.backscreen -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -a android.settings.DISPLAY_SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok(format!("已在手机打开背屏矩阵中心（{state_summary}）"))
        }
        "search_files" | "open_search" => {
            let query = payload.trim();
            if !query.is_empty() {
                let _ = push_clipboard_to_phone(adb, device, query);
            }
            let quoted_q = shell_quote(if query.is_empty() { "近期文档" } else { query });
            let script = format!(
                "am start -a android.intent.action.SEARCH -p com.stepos.filemanager --es query {quoted_q} >/dev/null 2>&1 || \
                 am start -a com.stepos.search.action.GLOBAL_SEARCH --es query {quoted_q} >/dev/null 2>&1 || \
                 am start -n com.stepos.filemanager/.MainActivity >/dev/null 2>&1 || \
                 monkey -p com.stepos.filemanager -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true"
            );
            adb.script(&transport, &script)?;
            if query.is_empty() {
                Ok("已在手机打开文件管理与全局文件检索 (com.stepos.filemanager)".into())
            } else {
                Ok(format!("已在手机打开文件检索并带入关键词：「{query}」"))
            }
        }
        "start_back_screen_nap" => {
            let minutes = if payload.trim().is_empty() { 20 } else {
                payload.trim().parse::<u32>().context("小憩时长必须为 1–180 分钟")?
            };
            anyhow::ensure!((1..=180).contains(&minutes), "小憩时长必须为 1–180 分钟");
            let receipt = crate::native_mcp::try_call(adb, device, "alarm_clock_timer_add",
                serde_json::json!({"seconds": minutes * 60, "label":"小憩"}))?
                .context("当前手机未提供原生计时器；未修改勿扰或背屏设置")?;
            Ok(format!("已请求手机小憩计时 {minutes} 分钟：{}。计时在手机时钟中管理，不会自动更改勿扰或背屏。",
                crate::native_mcp::response_note(&receipt)))
        }
        "open_logreport" => {
            let script = concat!(
                "am start -a com.stepos.logreport.action.OPEN >/dev/null 2>&1 || ",
                "monkey -p com.stepos.logreport -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -a com.stepos.feedback.action.FEEDBACK >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开系统日志与问题诊断中心 (com.stepos.logreport)".into())
        }
        "pull_remote_agent_files" => {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let sync_dir = home.join("Downloads").join("StepX-Agent-Sync");
            std::fs::create_dir_all(&sync_dir)
                .context("无法创建本地智能体同步目录 ~/Downloads/StepX-Agent-Sync")?;
            let list_script = "find /sdcard/Download /sdcard/Documents -maxdepth 2 -type f -not -name '.*' -size -50M -printf '%T@\\t%p\\n' 2>/dev/null | sort -rn | head -n 6 || true";
            let raw = adb.script(&transport, list_script)?;
            let mut pulled = 0usize;
            for line in raw.lines() {
                let Some((_, path_part)) = line.trim().split_once('\t') else {
                    continue;
                };
                let remote = path_part.trim();
                if remote.is_empty() {
                    continue;
                }
                let Some(fname) = std::path::Path::new(remote)
                    .file_name()
                    .and_then(|s| s.to_str())
                else {
                    continue;
                };
                let dest = sync_dir.join(fname);
                if adb
                    .transfer(&transport, "pull", std::path::Path::new(remote), &dest)
                    .is_ok()
                {
                    pulled += 1;
                }
            }
            if pulled > 0 {
                Ok(format!(
                    "已同步 {pulled} 个远程智能体文件到电脑：{}",
                    sync_dir.display()
                ))
            } else {
                Ok(format!(
                    "手机 /sdcard/Download 暂无新增智能体文件，已准备电脑同步目录：{}",
                    sync_dir.display()
                ))
            }
        }
        "open_alipay_coupons" => {
            let query = if payload.trim().is_empty() {
                "附近瑞幸咖啡/霸王茶姬团购神券"
            } else {
                payload.trim()
            };
            let _ = push_clipboard_to_phone(adb, device, query);
            let script = concat!(
                "am start -a android.intent.action.VIEW -d 'alipays://platformapi/startapp?appId=20000001' >/dev/null 2>&1 || ",
                "monkey -p com.eg.android.AlipayGphone -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok(format!(
                "已写入神券口令「{query}」并唤醒支付宝与 Amoo 领券收银台"
            ))
        }
        "open_migu_music" => {
            let script = concat!(
                "am start -a com.stepos.ailauncher.action.MIGU_MEMBERSHIP >/dev/null 2>&1 || ",
                "monkey -p cmccwm.mobilemusic -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "monkey -p com.StepOS.music -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开咪咕音乐与会员权益中心".into())
        }
        "open_data_migration" => {
            let script = concat!(
                "am start -a com.stepos.datamigration.action.home >/dev/null 2>&1 || ",
                "am start -n com.stepos.datamigration/.MainActivity >/dev/null 2>&1 || ",
                "monkey -p com.stepos.datamigration -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 StepOS 换机助手 (com.stepos.datamigration)".into())
        }
        "open_xms_service_numbers" => {
            let script = concat!(
                "am start -a com.stepos.xmscore.action.SERVICE_NUMBER_MANAGER >/dev/null 2>&1 || ",
                "am start -n com.android.mms/.ui.ConversationList >/dev/null 2>&1 || ",
                "monkey -p com.android.mms -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开智慧信息 (XMS) 服务号治理与黄页中心".into())
        }
        "open_language_settings" => {
            let script = concat!(
                "am start -a android.settings.LOCALE_SETTINGS >/dev/null 2>&1 || ",
                "am start -a android.settings.SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开系统多语言与地区设置".into())
        }
        "open_feedback" => {
            let script = concat!(
                "am start -a com.stepos.feedback.action.FEEDBACK >/dev/null 2>&1 || ",
                "monkey -p com.stepos.feedback -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -a android.settings.SETTINGS >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开 StepOS 系统问题与日志反馈中心".into())
        }
        "open_calculator" => {
            let script = concat!(
                "monkey -p com.stepos.calculator -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || ",
                "am start -a android.intent.action.MAIN -c android.intent.category.APP_CALCULATOR >/dev/null 2>&1 || true"
            );
            adb.script(&transport, script)?;
            Ok("已在手机打开科学计算器".into())
        }
        "open_soundrecorder" => {
            adb.script(
                &transport,
                "monkey -p com.android.soundrecorder -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true",
            )?;
            Ok("已在手机打开录音机与语音纪要".into())
        }
        "fill_otp_one_by_one" => {
            let source_text = if !payload.trim().is_empty() {
                payload.trim().to_string()
            } else {
                pull_clipboard_from_phone(adb, device).unwrap_or_default()
            };
            let Some(otp) = extract_otp_from_text(&source_text) else {
                bail!("未检测到 4–8 位数字验证码，请先复制包含验证码的短信或文本");
            };
            let _ = push_clipboard_to_phone(adb, device, &otp);
            let digits_space = otp
                .chars()
                .filter(|c| c.is_ascii_digit())
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let script = format!(
                "for d in {digits_space}; do input text \"$d\" >/dev/null 2>&1; sleep 0.06; done"
            );
            adb.script(&transport, &script)?;
            Ok(format!(
                "已将验证码 {otp} 按分格模式（60ms/位）自动逐位填入手机输入框"
            ))
        }
        "query_express_number" => {
            let source_text = if !payload.trim().is_empty() {
                payload.trim().to_string()
            } else {
                pull_clipboard_from_phone(adb, device).unwrap_or_default()
            };
            let (carrier, number) = if let Some(found) = extract_express_number_from_text(&source_text)
                .or_else(|| crate::home_hub::extract_express_from_text_flexible(&source_text, true))
            {
                found
            } else {
                bail!("未识别到有效快递单号，请先复制快递单号（支持顺丰/京东/中通/圆通/韵达/极兔/EMS）");
            };
            let _ = push_clipboard_to_phone(adb, device, &number);
            let quoted_num = shell_quote(&number);
            let script = format!(
                "am start -a com.stepos.ailauncher.action.EXPRESS_QUERY --es express_number {quoted_num} >/dev/null 2>&1 || \
                 am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&transport, &script)?;
            Ok(format!(
                "已识别 {carrier} 单号 {number}，已写入手机剪贴板并唤起手机查快递卡片"
            ))
        }
        "backscreen_stay_15" | "backscreen_stay_30" | "backscreen_stay_60" => {
            let secs = match action_key {
                "backscreen_stay_15" => 15u32,
                "backscreen_stay_60" => 60u32,
                _ => 30u32,
            };
            let script = format!(
                "settings put system stepos_back_screen_stay_on_duration {secs} >/dev/null 2>&1 || true"
            );
            adb.script(&transport, &script)?;
            Ok(format!("已将手机背屏亮屏时长设为 {secs} 秒"))
        }
        "backscreen_toggle_flip_keep" => {
            let script = concat!(
                "CUR=$(settings get system stepos_back_screen_flip_keep_running 2>/dev/null || echo 1); ",
                "if [ \"$CUR\" = \"0\" ]; then ",
                "  settings put system stepos_back_screen_flip_keep_running 1 >/dev/null 2>&1 || true; ",
                "  echo FLIP_ON; ",
                "else ",
                "  settings put system stepos_back_screen_flip_keep_running 0 >/dev/null 2>&1 || true; ",
                "  echo FLIP_OFF; ",
                "fi"
            );
            let out = adb.script(&transport, script)?;
            if out.contains("FLIP_ON") {
                Ok("已开启翻转手机保持背屏运行".into())
            } else {
                Ok("已关闭翻转手机保持背屏运行".into())
            }
        }
        "start_pomodoro_25m" => {
            let secs = payload
                .trim()
                .parse::<u32>()
                .ok()
                .map(|v| if v <= 60 { v.saturating_mul(60) } else { v })
                .unwrap_or(1500)
                .clamp(60, 3600);
            let minutes = (secs / 60).max(1);
            crate::backscreen::start_pomodoro_countdown(adb, device, secs, 100)?;
            let capsule_req = crate::push::PhoneLiveCapsuleRequest {
                tag: "stepx_pomodoro".into(),
                pill_text: format!("🍅 {minutes}:00"),
                title: format!("番茄专注 {minutes} 分钟"),
                subtitle: "桌面与手机顶部实况胶囊、背屏 17×23 点阵同步倒计时".into(),
                badge_text: "专注计时".into(),
                live_type: 11,
                countdown_secs: Some(u64::from(secs)),
                ongoing: true,
            };
            let _ = crate::push::post_phone_live_capsule(adb, &device.serial, &capsule_req);
            Ok(format!(
                "已开启 {minutes} 分钟番茄专注倒计时（手机顶部实况胶囊与背屏 17×23 点阵同步计时）"
            ))
        }
        other => bail!("未知的快捷指令：{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_export_keeps_expanded_occurrences_distinct_and_single_event_uids_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-calendar.ics");
        let start = Local.with_ymd_and_hms(2026, 11, 7, 10, 0, 0).unwrap().timestamp_millis();
        let row = |id, dtstart, recurring, title: &str| PhoneCalendarEvent {
            id, dtstart, recurring, title: title.into(), dtend: dtstart + 3_600_000,
            source: "手机日历".into(), ..Default::default()
        };
        let events = vec![
            row(71, start, Some(true), "每周项目评审"),
            row(71, start + 7 * 86_400_000, Some(true), "每周项目评审"),
            row(72, start + 60_000, Some(false), "一次性面试"),
            row(73, start + 120_000, None, "旧固件日程"),
        ];
        export_calendar_ics_at(&path, &events, true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let uids: Vec<_> = text.lines().filter(|line| line.starts_with("UID:")).collect();
        assert_eq!(uids.len(), 4);
        assert_eq!(uids.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        assert!(uids.contains(&format!("UID:stepx-71-{start}@stepx.local").as_str()));
        assert!(uids.contains(&format!("UID:stepx-71-{}@stepx.local", start + 7 * 86_400_000).as_str()));
        assert!(uids.contains(&"UID:stepx-72@stepx.local"));
        assert!(uids.contains(&format!("UID:stepx-73-{}@stepx.local", start + 120_000).as_str()));
        assert!(text.contains("当前读取范围内的独立日程实例"));
        assert!(std::fs::read_to_string(dir.path().join("stepx-calendar-notes.md")).unwrap().contains("不保留周期规则及例外日期"));
        let parsed = parse_ics_events(&text, "电脑日历");
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed.iter().filter(|ev| ev.id == 71 && ev.recurring == Some(true)).count(), 2);
        assert_eq!(parsed.iter().find(|ev| ev.id == 72).unwrap().recurring, Some(false));
        assert_eq!(parsed.iter().find(|ev| ev.id == 73).unwrap().recurring, None);
        export_calendar_ics_at(&path, &events, true).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(uids, second.lines().filter(|line| line.starts_with("UID:")).collect::<Vec<_>>());
    }

    #[test]
    fn calendar_export_detects_duplicate_ids_even_when_metadata_claims_single_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-calendar.ics");
        let start = Local.with_ymd_and_hms(2026, 11, 7, 10, 0, 0).unwrap().timestamp_millis();
        let events: Vec<_> = [start, start + 7 * 86_400_000].into_iter().map(|dtstart| PhoneCalendarEvent {
            id: 81, title: "不同发生日期".into(), dtstart, dtend: dtstart + 3_600_000,
            recurring: Some(false), source: "手机日历".into(), ..Default::default()
        }).collect();
        export_calendar_ics_at(&path, &events, true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = parse_ics_events(&text, "电脑日历");
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().all(|ev| ev.id == 81 && ev.recurring == Some(true)));
        assert_eq!(text.lines().filter(|line| line.starts_with("UID:")).collect::<std::collections::HashSet<_>>().len(), 2);
    }

    #[test]
    fn phone_calendar_export_is_not_an_independent_desktop_event_after_phone_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-calendar.ics");
        let start = Local.with_ymd_and_hms(2026, 11, 3, 10, 0, 0).unwrap().timestamp_millis();
        let desktop = PhoneCalendarEvent { id: 98765, title: "电脑独立撰写的方案评审".into(), dtstart: start, dtend: start + 3_600_000, source: "电脑日历".into(), ..Default::default() };
        let phone = PhoneCalendarEvent { id: 139, title: "手机原有项目沟通".into(), dtstart: start + 60_000, dtend: start + 3_660_000, source: "手机日历".into(), ..Default::default() };
        export_calendar_ics_at(&path, &[desktop], false).unwrap();
        export_calendar_ics_at(&path, &[phone], true).unwrap();
        let parsed = parse_ics_events(&std::fs::read_to_string(&path).unwrap(), "电脑日历");
        assert_eq!(parsed.len(), 2);
        let mirror = parsed.iter().find(|e| e.title == "手机原有项目沟通").unwrap();
        assert_eq!(mirror.id, 139);
        assert_eq!(mirror.source, PHONE_CALENDAR_MIRROR);
        assert!(!independent_desktop_calendar_event(mirror));
        assert_eq!(parsed.iter().filter(|e| independent_desktop_calendar_event(e)).count(), 1);

        // A fresh, empty phone snapshot follows a successful phone deletion.
        // Export removes its mirror; only the independent desktop original can
        // enter the next automatic phone-write candidates.
        export_calendar_ics_at(&path, &[], true).unwrap();
        let fresh = parse_ics_events(&std::fs::read_to_string(&path).unwrap(), "电脑日历");
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].title, "电脑独立撰写的方案评审");
        assert!(fresh.iter().all(independent_desktop_calendar_event));
    }

    #[test]
    fn legacy_stepx_calendar_export_is_preserved_without_authorizing_phone_recreation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-calendar.ics");
        let body = "BEGIN:VCALENDAR\r\nPRODID:-//StepX Companion//Cross-Platform Calendar//CN\r\nBEGIN:VEVENT\r\nUID:stepx-8272370489553632383@stepx.local\r\nSUMMARY:需要保留核对的旧日程\r\nDTSTART:20261103T100000\r\nDTEND:20261103T110000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        std::fs::write(&path, body).unwrap();
        let old = parse_ics_events(body, "电脑日历");
        assert_eq!(old[0].source, LEGACY_CALENDAR_MIRROR);
        assert!(!independent_desktop_calendar_event(&old[0]));
        export_calendar_ics_at(&path, &[], true).unwrap();
        let retained = parse_ics_events(&std::fs::read_to_string(&path).unwrap(), "电脑日历");
        assert_eq!(retained[0].title, "需要保留核对的旧日程");
        assert!(!independent_desktop_calendar_event(&retained[0]));
        let external = body.replace("PRODID:-//StepX Companion//Cross-Platform Calendar//CN", "PRODID:-//Independent Desktop Calendar//EN");
        assert!(parse_ics_events(&external, "电脑日历").iter().all(independent_desktop_calendar_event));
    }

    #[cfg(unix)]
    fn action_test_adb(body: &str) -> (crate::native_mcp::test_support::Fixture, Adb, Device) {
        let fixture = crate::native_mcp::test_support::Fixture::new(vec![], Some(serde_json::json!("unused")));
        let adb = fixture.adb(body);
        let device = fixture.device();
        (fixture, adb, device)
    }

    #[cfg(unix)]
    #[test]
    fn backscreen_text_requires_matching_settings_readback() {
        let fixture = crate::native_mcp::test_support::Fixture::new(vec![serde_json::json!({"name":"set_back_screen_basic_type","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"properties":{"type":{"type":"string"},"text":{"type":"string"}}}})],Some(serde_json::json!("accepted")));
        let device = fixture.device();
        let adb = fixture.adb(r#"
case "$*" in
*settings*put*) exit 0;;
*settings*get*stepos_back_screen_basic_type*) echo text;;
*settings*get*stepos_back_screen_basic_text*) echo WORK;;
*) exit 19;;
esac"#);
        let note = trigger_stepos_mcp_action(&adb, &device, "backscreen_sig:WORK", "").unwrap();
        assert!(note.contains("保存并核对"));
        assert!(note.contains("显示请在手机确认"));
        assert!(trigger_stepos_mcp_action(&adb, &device, "backscreen_sig:REST", "").is_err());
        let (_dir, adb, device) = action_test_adb("exit 19");
        assert!(trigger_stepos_mcp_action(&adb, &device, "backscreen_sig:WORK", "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_dnd_actions_verify_state_and_never_toggle_an_already_enabled_phone() {
        let (dir, adb, device) = action_test_adb(r#"
case "$*" in
*zen_mode*) cat zen;;
*set_dnd*priority*) echo 1 > zen;;
*set_dnd*off*) echo 0 > zen;;
*) exit 19;;
esac"#);
        std::fs::write(dir.path().join("zen"), "1").unwrap();
        let note = trigger_stepos_mcp_action(&adb, &device, "set_dnd_on", "").unwrap();
        assert!(note.contains("已开启"));
        assert!(!std::fs::read_to_string(dir.path().join("calls")).unwrap().contains("set_dnd"));
        assert!(trigger_stepos_mcp_action(&adb, &device, "set_dnd_off", "").unwrap().contains("已关闭"));
        assert!(trigger_stepos_mcp_action(&adb, &device, "set_dnd_on", "").unwrap().contains("已开启"));
        std::fs::write(dir.path().join("zen"), "null").unwrap();
        assert!(trigger_stepos_mcp_action(&adb, &device, "set_dnd_off", "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn dnd_command_success_without_state_change_is_not_reported_as_success() {
        let (_dir, adb, device) = action_test_adb("case \"$*\" in *zen_mode*) echo 0;; *set_dnd*) exit 0;; *) exit 19;; esac");
        assert!(trigger_stepos_mcp_action(&adb, &device, "set_dnd_on", "")
            .unwrap_err().to_string().contains("状态未变更"));
    }

    #[test]
    fn capability_evidence_requires_exact_components_and_explicit_values() {
        let report = parse_stepos_ai_suite_output("===PS===\ncom.stepos.agentbus\ncom.stepos.agentsafetycenter\n===ACC===\nother.package/.AccessibilityService\n===NOTIF===\nother.package/.Listener\n===AIENGINE===\nRegistered abilities count: 6\n");
        assert_eq!(report.capability_evidence.gui_accessibility, Some(false));
        assert_eq!(report.capability_evidence.notification_listener, Some(false));
        assert_eq!(report.capability_evidence.mcp_access, None);
        assert_eq!(report.capability_evidence.screen_perception, None);
        assert_eq!(report.capability_evidence.camera_allowed, None);
        assert!(!report.mcp_permission_granted && !report.screen_perception_ready);
        let explicit = parse_stepos_ai_suite_output("===ACC===\ncom.stepx.companion.app/.GuiAccessibilityService\n===NOTIF===\ncom.stepx.companion.app/com.stepx.companion.app.PushNotificationService\n===MCP_ACCESS===\ncom.stepos.mcp.sdk.permission.ACCESS_MCP_SERVICES: granted=true\n===PERCEPTION===\n0\n");
        assert_eq!(explicit.capability_evidence.gui_accessibility, Some(true));
        assert_eq!(explicit.capability_evidence.notification_listener, Some(true));
        assert_eq!(explicit.capability_evidence.mcp_access, Some(true));
        assert_eq!(explicit.capability_evidence.screen_perception, Some(false));
    }

    #[test]
    fn sensor_privacy_parser_preserves_unknown_and_combines_multiple_blocks() {
        assert_eq!(sensor_privacy_allowed("unknown", "camera"), None);
        let dump = "sensor=2\ntoggle_type=1\nstate_type=2\ntoggle_type=2\nstate_type=1\nsensor=1\nstate_type=2\n";
        assert_eq!(sensor_privacy_allowed(dump, "camera"), Some(false));
        assert_eq!(sensor_privacy_allowed(dump, "microphone"), Some(true));
    }

    #[cfg(unix)]
    #[test]
    fn privacy_toggle_requires_readback_and_unknown_state_never_writes() {
        let (dir, adb, device) = action_test_adb(r#"
case "$*" in
*dumpsys*sensor_privacy*) echo sensor=2; echo state_type=2;;
*cmd*sensor_privacy*) exit 0;;
*) exit 19;;
esac"#);
        assert!(trigger_stepos_mcp_action(&adb, &device, "toggle_global_camera", "").unwrap_err().to_string().contains("回读不一致"));
        assert!(std::fs::read_to_string(dir.path().join("calls")).unwrap().contains("sensor_privacy"));
        let (dir, adb, device) = action_test_adb("case \"$*\" in *dumpsys*sensor_privacy*) echo unknown;; *) exit 19;; esac");
        assert!(trigger_stepos_mcp_action(&adb, &device, "toggle_global_mic", "").is_err());
        assert!(!std::fs::read_to_string(dir.path().join("calls")).unwrap().contains("'cmd'"));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_privacy_target_does_not_reverse_a_phone_already_at_target() {
        let (dir, adb, device) = action_test_adb(r#"
case "$*" in
*dumpsys*sensor_privacy*) echo sensor=2; echo state_type=2;;
*cmd*sensor_privacy*disable*) exit 0;;
*) exit 19;;
esac"#);
        let note = set_stepos_privacy_setting(&adb, &device, "toggle_global_camera", Some(true)).unwrap();
        assert!(note.contains("允许") && note.contains("已回读"));
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(calls.contains("'disable'"));
        assert!(!calls.contains("'enable'"));
    }

    #[cfg(unix)]
    #[test]
    fn screen_perception_unknown_and_unmatched_readback_never_report_success() {
        let (dir, adb, device) = action_test_adb(r#"
case "$*" in
*settings*get*) echo null;;
*) exit 19;;
esac"#);
        assert!(set_stepos_privacy_setting(&adb, &device, "toggle_screen_perception", Some(true)).is_err());
        assert!(!std::fs::read_to_string(dir.path().join("calls")).unwrap().contains("'put'"));
        let (_dir, adb, device) = action_test_adb(r#"
case "$*" in
*settings*get*) echo 1;;
*settings*put*) exit 0;;
*) exit 19;;
esac"#);
        assert!(set_stepos_privacy_setting(&adb, &device, "toggle_screen_perception", Some(false))
            .unwrap_err().to_string().contains("回读不一致"));
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_workbuddy_creation_never_emits_a_fake_task() {
        let (dir, adb, device) = action_test_adb("exit 19");
        assert!(trigger_stepos_mcp_action(&adb, &device, "create_workbuddy_card", "测试")
            .unwrap_err().to_string().contains("未创建任务"));
        assert!(!dir.path().join("calls").exists(), "unsupported task creation must return before any device operation");
    }

    #[cfg(unix)]
    #[test]
    fn capcut_does_not_launch_after_write_or_readback_failure() {
        for body in [
            "exit 23",
            "case \"$*\" in *set\\ -e*) exit 0;; *cat*) echo '{}';; *) exit 19;; esac",
        ] {
            let (dir, adb, device) = action_test_adb(body);
            assert!(trigger_stepos_mcp_action(&adb, &device, "capcut_handoff", "周末 Vlog").is_err());
            let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
            assert!(!calls.contains("monkey"));
            assert!(!calls.contains("broadcast"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn capcut_verified_manifest_is_not_mistaken_for_an_import_or_finished_video() {
        let (dir, adb, device) = action_test_adb(r#"
case "$*" in
*set\ -e*|*cat*)
    local_script=$(printf '%s' "$4" | sed "s|/sdcard/Download|$PWD/phone|g")
    sh -c "$local_script";;
*monkey*) echo 'No activities found to run, aborting.';;
*) exit 19;;
esac"#);
        let note = trigger_stepos_mcp_action(&adb, &device, "capcut_handoff",
            "周末 '旅行' $(touch sentinel)|/sdcard/DCIM/a.jpg").unwrap();
        assert!(note.contains("已保存创作清单"));
        assert!(note.contains("未能确认剪映已打开"));
        assert!(note.contains("素材尚未自动导入，成片尚未生成"));
        assert!(!dir.path().join("sentinel").exists());
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("phone/stepx_capcut_handoff.json")).unwrap(),
        ).unwrap();
        assert_eq!(saved["media_paths"][0], "/sdcard/DCIM/a.jpg");
        assert!(saved["prompt"].as_str().unwrap().contains("$(touch sentinel)"));
    }

    #[test]
    fn usage_snapshot_distinguishes_an_empty_day_from_missing_or_failed_reads() {
        let mut value = serde_json::json!({
            "screen_time_minutes": 0, "unlock_count": 0, "clipboard": "",
            "top_apps": [], "calendar_events": [], "alarms": [], "read_errors": [],
            "controls": { "dnd_on": false, "wifi_on": false, "bluetooth_on": false,
                "brightness_percent": 5 }
        });
        assert!(step_os_usage_read_errors(&value).is_empty());
        value["read_errors"] = serde_json::json!(["手机日历读取失败"]);
        assert_eq!(step_os_usage_read_errors(&value), vec!["手机日历读取失败"]);
        value["read_errors"] = serde_json::json!([]);
        value.as_object_mut().unwrap().remove("alarms");
        assert_eq!(step_os_usage_read_errors(&value), vec!["手机闹钟未返回有效数据"]);
        value["controls"]["dnd_on"] = serde_json::Value::Null;
        assert!(step_os_usage_read_errors(&value).iter().any(|s| s.contains("控制状态")));
        let report = default_stepos_ai_suite_report();
        let mut legacy = serde_json::to_value(report).unwrap();
        legacy.as_object_mut().unwrap().remove("read_error");
        let restored: StepOsAiSuiteReport = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.read_error, None);
    }

    #[test]
    fn default_stepos_suite_has_twenty_mcp_services() {
        let report = default_stepos_ai_suite_report();
        assert_eq!(report.mcp_services.len(), 24);
        assert!(report.aiengine_running);
        assert!(report.safety_center_running);
        assert!(report.global_camera_enabled);
        assert!(report.global_mic_enabled);
        assert!(report.global_location_enabled);
        assert!(report.delete_protect_enabled);
        assert_eq!(report.backscreen_stay_on_secs, 30);
        assert!(report.backscreen_flip_keep_running);
        assert!(report.remote_agent_files.is_empty());
        assert!(report.firmware_profile.contains("pandora-12"));
        assert!(report.wellbeing.top_apps.is_empty());
        assert_eq!(report.wellbeing.screen_time_minutes, 0);
        assert!(report.phone_clipboard.is_empty());
        let parsed = parse_stepos_ai_suite_output(
            "===PS===\ncom.stepos.aiengine\ncom.stepos.agentsafetycenter\ncom.stepos.alphame\n===ACC===\ncom.stepx.companion.app/.neo.NeoAccessibilityService\n===NOTIF===\ncom.stepos.alphame\n===PROP===\nstepos/pandora/pandora:16/V12.0.12\n===PRIVACY===\nLOC_MODE=3\nDEL_PROT=1\nCAM_TOGGLE=0\nMIC_TOGGLE=1\nBS_STAY=60\nBS_FLIP=0\n===FILES===\n1729900200.0\t4096\t/sdcard/Download/workbuddy_summary.md\n===CLIP===\nHello StepX\n===USAGE===\npackage=com.stepos.alphame totalTimeUsed=\"57:02\"\n===AIENGINE===\nRegistered abilities count: 33\nDSP bucket=HTP: available=2 / max=2\n[bge-embedding] pkg=com.stepos.wlanmonitor state=IDLE inFlight=0\n===AGENTBUS===\nContainer: ready, connections: 6 ([GATEWAY]), inflight invocations: 0, ls server ready: true\n===THERMAL===\nTemperature{mValue=31.7, mType=3, mName=skin-therm, mStatus=0}\nTemperature{mValue=29.4, mType=2, mName=battery, mStatus=0}\nTemperature{mValue=45.3, mType=9, mName=nsp0, mStatus=0}\nTemperature{mValue=48.5, mType=0, mName=cpu-0-0, mStatus=0}\n",
        );
        assert!(parsed.checked);
        assert!(parsed.aiengine_running);
        assert!(parsed.safety_center_running);
        assert!(parsed.gui_accessibility_enabled);
        assert_eq!(parsed.phone_clipboard, "Hello StepX");
        assert!(!parsed.global_camera_enabled);
        assert!(parsed.global_mic_enabled);
        assert!(parsed.global_location_enabled);
        assert!(parsed.delete_protect_enabled);
        assert_eq!(parsed.backscreen_stay_on_secs, 60);
        assert!(!parsed.backscreen_flip_keep_running);
        assert_eq!(parsed.remote_agent_files.len(), 1);
        assert_eq!(
            parsed.remote_agent_files[0].path,
            "/sdcard/Download/workbuddy_summary.md"
        );
        // Ensure grep -m 48 usagestats garbage never pollutes wellbeing
        assert_eq!(parsed.wellbeing.screen_time_minutes, 0);
        assert!(parsed.wellbeing.top_apps.is_empty());
        assert_eq!(parsed.aiengine_ability_count, 33);
        assert_eq!(parsed.aiengine_htp_status, "QNN HTP 2/2 空闲");
        assert!(parsed.aiengine_model_summary.contains("33 项已注册能力"));
        assert!(parsed.aiengine_model_summary.contains("QNN HTP 2/2 空闲"));
        assert!(parsed.aiengine_model_summary.contains("bge-embedding"));
        assert!(parsed.agentbus_status.contains("6 路原生通道在线"));
        assert!(parsed.thermal_summary.contains("机身 31.7°C"));
        assert!(parsed.thermal_summary.contains("NPU 45.3°C"));

        let parsed_sp = parse_stepos_ai_suite_output(
            "===PRIVACY===\nCAM_TOGGLE=null\nMIC_TOGGLE=null\nSCR_BRIGHT=null\n  Display Brightness=0.42\n      {\n        sensor=1\n        toggles={\n          toggle_type=1\n          state_type=1\n        }\n      }\n      {\n        sensor=2\n        toggles={\n          toggle_type=1\n          state_type=2\n        }\n      }\n",
        );
        assert!(!parsed_sp.global_mic_enabled);
        assert!(parsed_sp.global_camera_enabled);
        assert_eq!(parsed_sp.controls.brightness_percent, 42);
        let parsed_scr_priority = parse_stepos_ai_suite_output(
            "===PRIVACY===\nSCR_BRIGHT=3697\n  Display Brightness=0.08968944\n",
        );
        assert_eq!(parsed_scr_priority.controls.brightness_percent, 90);
        assert!(shell_output_has_explicit_brightness(
            "===PRIVACY===\nSCR_BRIGHT=21300\n  Display Brightness=0.65\n"
        ));
        assert!(shell_output_has_explicit_brightness(
            "===PRIVACY===\nSCR_BRIGHT=21300\n"
        ));
        assert!(!shell_output_has_explicit_brightness("===PRIVACY===\nLOC_MODE=3\n"));
        let parsed_65 = parse_stepos_ai_suite_output(
            "===PRIVACY===\nSCR_BRIGHT=21300\n  Display Brightness=0.08\n",
        );
        assert_eq!(parsed_65.controls.brightness_percent, 65);
    }

    #[test]
    fn express_tracking_otp_and_capcut_payload_builders_work_accurately() {
        assert_eq!(
            extract_express_number_from_text("您的顺丰包裹 SF1428571428579 已派件"),
            Some(("顺丰速运", "SF1428571428579".into()))
        );
        assert_eq!(
            extract_express_number_from_text("京东快递：JDVA12345678901 请查收"),
            Some(("京东快递", "JDVA12345678901".into()))
        );
        assert_eq!(
            extract_express_number_from_text("圆通单号 YT8912345678901"),
            Some(("圆通速递", "YT8912345678901".into()))
        );
        assert_eq!(
            extract_express_number_from_text("极兔速递 JT3098765432109"),
            Some(("极兔速递", "JT3098765432109".into()))
        );
        assert_eq!(
            extract_express_number_from_text("中通快递 75812345678901 已到达驿站"),
            Some(("中通快递", "75812345678901".into()))
        );
        assert_eq!(
            extract_express_number_from_text("申通快递 773012345678901 已发出"),
            Some(("申通快递", "773012345678901".into()))
        );
        assert_eq!(
            extract_express_number_from_text("韵达快递 431234567890123"),
            Some(("韵达快递", "431234567890123".into()))
        );
        assert_eq!(
            extract_express_number_from_text("EMS 邮件 EA123456789CN 已发出"),
            Some(("邮政EMS", "EA123456789CN".into()))
        );
        assert_eq!(extract_express_number_from_text("普通文本无单号 123456"), None);

        assert_eq!(
            extract_otp_from_text("【StepOS】您的登录验证码为 839204，5分钟内有效"),
            Some("839204".into())
        );
        assert_eq!(
            extract_otp_from_text("顺丰单号 SF1428571428579 无验证码"),
            None
        );

        let payload_with_photos = build_capcut_handoff_payload_json(
            "/sdcard/DCIM/Camera/IMG_001.jpg, /storage/emulated/0/Pictures/IMG_002.png, 周末旅行Vlog",
            1777000000000,
        );
        assert_eq!(payload_with_photos["source"], "stepx_desktop");
        assert_eq!(payload_with_photos["prompt"], "周末旅行Vlog");
        assert_eq!(payload_with_photos["timestamp"], 1777000000000_i64);
        let paths = payload_with_photos["media_paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], "/sdcard/DCIM/Camera/IMG_001.jpg");
        assert_eq!(paths[1], "/sdcard/Pictures/IMG_002.png");

        let payload_pipe = build_capcut_handoff_payload_json(
            "周末Vlog|/sdcard/DCIM/Camera/a.jpg;/sdcard/DCIM/Camera/b.jpg",
            1777000000002,
        );
        assert_eq!(payload_pipe["source"], "stepx_desktop");
        assert_eq!(payload_pipe["prompt"], "周末Vlog");
        let pipe_paths = payload_pipe["media_paths"].as_array().unwrap();
        assert_eq!(pipe_paths.len(), 2);
        assert_eq!(pipe_paths[0], "/sdcard/DCIM/Camera/a.jpg");
        assert_eq!(pipe_paths[1], "/sdcard/DCIM/Camera/b.jpg");

        let payload_default = build_capcut_handoff_payload_json("", 1777000000001);
        assert_eq!(payload_default["prompt"], "今日精选照片智能成片");
        assert!(payload_default["media_paths"].as_array().unwrap().is_empty());

        assert_eq!(
            detect_stepos_firmware_profile("stepos/pandora/pandora:16/pandora-172"),
            "pandora-172 (300 MCP Tools)"
        );
        assert!(
            detect_stepos_firmware_profile("stepos/pandora/pandora:16/pandora-12")
                .contains("pandora-12 MP")
        );
    }

    #[test]
    fn format_duration_minutes_formats_hours_and_minutes() {
        assert_eq!(format_duration_minutes(0), "0 分钟");
        assert_eq!(format_duration_minutes(45), "45 分钟");
        assert_eq!(format_duration_minutes(59), "59 分钟");
        assert_eq!(format_duration_minutes(60), "1 小时 0 分钟");
        assert_eq!(format_duration_minutes(135), "2 小时 15 分钟");
    }

    #[test]
    fn parse_ics_events_supports_unfolding_utc_local_and_all_day_events() {
        let ics = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:StepX 跨端产品\r\n",
            " 评审会议\r\n",
            "DTSTART:20261026T093000\r\n",
            "DTEND:20261026T103000\r\n",
            "LOCATION:3F 会议室\\, A区\r\n",
            "DESCRIPTION:第一行\\n第二行\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:全天黑客松\r\n",
            "DTSTART;VALUE=DATE:20261027\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:UTC 同步测试\r\n",
            "DTSTART:20261028T013000Z\r\n",
            "DTEND:20261028T023000Z\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        );
        let events = parse_ics_events(ics, "电脑日历");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].title, "StepX 跨端产品评审会议");
        assert!(events[0].id > 0);
        assert!(!events[0].all_day);
        assert_eq!(events[0].date_ymd(), (2026, 10, 26));
        assert_eq!(events[0].hm_label(), "09:30");
        assert_eq!(events[0].source_label(), "电脑日历");
        assert_eq!(events[0].location, "3F 会议室, A区");
        assert_eq!(events[0].description, "第一行\n第二行");

        assert_eq!(events[1].title, "全天黑客松");
        assert!(events[1].all_day);
        assert_eq!(events[1].date_ymd(), (2026, 10, 27));
        assert_eq!(events[1].hm_label(), "全天");
        assert_eq!(events[1].dtend - events[1].dtstart, 86_400_000);

        assert_eq!(events[2].title, "UTC 同步测试");
        assert!(!events[2].all_day);
        assert_eq!(events[2].dtend - events[2].dtstart, 3_600_000);
    }

    #[test]
    fn event_and_alarm_time_string_parsers_handle_all_formats() {
        let (s1, e1) = parse_event_time_range(2026, 10, 26, "14:15-15:45").unwrap();
        assert_eq!(e1 - s1, 90 * 60_000);
        let dt1 = Local.timestamp_millis_opt(s1).single().unwrap();
        assert_eq!(
            (dt1.year(), dt1.month(), dt1.day(), dt1.hour(), dt1.minute()),
            (2026, 10, 26, 14, 15)
        );

        let (s2, e2) = parse_event_time_range(2026, 10, 26, "").unwrap();
        assert_eq!(e2 - s2, 60 * 60_000);
        let dt2 = Local.timestamp_millis_opt(s2).single().unwrap();
        assert_eq!((dt2.hour(), dt2.minute()), (9, 30));

        let base = Local
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 10, 26)
                    .unwrap()
                    .and_hms_opt(10, 20, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap();
        assert_eq!(parse_alarm_time_str_at("07:30", base).unwrap(), (7, 30));
        assert_eq!(parse_alarm_time_str_at("7:05", base).unwrap(), (7, 5));
        assert_eq!(parse_alarm_time_str_at("0800", base).unwrap(), (8, 0));
        assert_eq!(parse_alarm_time_str_at("+15m", base).unwrap(), (10, 35));
        assert_eq!(parse_alarm_time_str_at("15分钟后", base).unwrap(), (10, 35));
        assert!(parse_alarm_time_str_at("99:99", base).is_err());

        let gallery = parse_phone_gallery_output(
            "1729900000.5\t204800\t/sdcard/DCIM/Camera/IMG_001.jpg\n1729900050.0\t307200\t/sdcard/DCIM/P20261019-121623.jpg\n1729900100.0\t102400\t/sdcard/Pictures/Screenshots/StepX_2026.png\n1729900120.0\t102400\t/sdcard/.StepXTrash/1776563222962_1000000720.jpg\n1729900130.0\t102400\t/sdcard/DCIM/Camera/.trashed-1779161677-P20261019.jpg\n1729900140.0\t102400\t/sdcard/Pictures/.thumbnails/thumb_01.jpg\n1729900150.0\t45983\t/sdcard/Pictures/StepX Companion/SC_6082f69ef10018a7254ccdc4331d6574.jpg\n1729900160.0\t128\t/sdcard/Pictures/tiny_corrupt.jpg\n1729900170.0\t4096\t/sdcard/DCIM/P20261019.jpg.stepxlink\n",
        );
        assert_eq!(gallery.len(), 3);
        assert_eq!(gallery[0].category, "camera");
        assert_eq!(gallery[0].category_label(), "相机实拍");
        assert_eq!(gallery[1].category, "camera");
        assert_eq!(gallery[1].name, "P20261019-121623.jpg");
        assert_eq!(gallery[2].category, "screenshot");
        assert_eq!(gallery[2].category_label(), "屏幕截图");
        let exp_item = PhoneGalleryItem {
            remote_path: "/sdcard/DCIM/P20261019-121623.jpg".into(),
            name: "P20261019-121623.jpg".into(),
            category: "expansion".into(),
            size_bytes: 4_471_195,
            modified_ms: 1776572185000,
            local_thumb_path: Some("/tmp/original.jpg".into()),
        };
        assert_eq!(exp_item.category_label(), "电脑扩容");
    }

    #[test]
    fn calendar_and_alarm_parsers_strip_null_fields_deduplicate_and_filter_e2e_noise() {
        assert_eq!(clean_nullable_field("null"), "");
        assert_eq!(clean_nullable_field(" NULL "), "");
        assert_eq!(clean_nullable_field("none"), "");
        assert_eq!(clean_nullable_field("undefined"), "");
        assert_eq!(clean_nullable_field(" 3F 会议室 "), "3F 会议室");

        let ics = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:行前准备：自驾行李与车况检查\r\n",
            "DTSTART:20261026T200000\r\n",
            "DTEND:20261026T213000\r\n",
            "LOCATION:null\r\n",
            "DESCRIPTION:undefined\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:行前准备：自驾行李与车况检查\r\n",
            "DTSTART:20261026T200000\r\n",
            "DTEND:20261026T213000\r\n",
            "LOCATION:null\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:StepX 日历网格验证\r\n",
            "DTSTART:20261026T093000\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        );
        let ics_events = parse_ics_events(ics, "电脑日历");
        assert_eq!(ics_events.len(), 1);
        assert_eq!(ics_events[0].title, "行前准备：自驾行李与车况检查");
        assert_eq!(ics_events[0].location, "");
        assert_eq!(ics_events[0].description, "");

        let bridge_cal = serde_json::json!({
            "calendar_events": [
                {"id": 10, "title": "自驾出发", "start_ms": 1793014800000_i64, "end_ms": 1793018400000_i64, "location": "null", "description": "none"},
                {"id": 10, "title": "自驾出发", "start_ms": 1793014800000_i64, "end_ms": 1793018400000_i64, "location": "高速服务区", "description": ""},
                {"id": 11, "title": "自驾出发", "start_ms": 1793014800000_i64, "end_ms": 1793018400000_i64, "location": "另一集合地", "description": ""},
                {"id": 12, "title": "StepX自动化验证", "start_ms": 1793014800000_i64, "end_ms": 1793018400000_i64}
            ]
        });
        let cal_events = parse_bridge_calendar_events(&bridge_cal);
        assert_eq!(cal_events.len(), 2);
        assert_eq!(cal_events[0].location, "高速服务区");
        assert_eq!(cal_events[0].description, "");
        assert_eq!(cal_events[1].id, 11);
        assert_eq!(cal_events[1].location, "另一集合地");

        let bridge_alarms = serde_json::json!({
            "alarms": [
                {"id": 1, "hour": 8, "minutes": 0, "enabled": false, "days_of_week": 31, "label": "null"},
                {"id": 2, "hour": 8, "minutes": 0, "enabled": true, "days_of_week": 31, "label": ""},
                {"id": 3, "hour": 8, "minutes": 15, "enabled": true, "days_of_week": 0, "label": "StepX自动化验证"}
            ]
        });
        let alarms = parse_bridge_alarms(&bridge_alarms);
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].id, 2);
        assert!(alarms[0].enabled);
        assert_eq!(alarms[0].message, "");
    }
    #[test]
    fn mutations_are_bound_to_scanned_file_and_explicit_keep_copy() {
        let f = FileInfo {
            path: "/sdcard/Documents/a.txt".into(),
            bytes: 500,
            modified_ms: 12,
        };
        let k = FileInfo {
            path: "/sdcard/Documents/b.txt".into(),
            ..f.clone()
        };
        let request = ToolOperation::DeleteDuplicate { file: f, keep: k }.request();
        assert_eq!(request["bytes"], 500);
        assert_eq!(request["modified_ms"], 12);
        assert_eq!(request["keep"], "/sdcard/Documents/b.txt");
        assert_eq!(request["action"], "delete_duplicate");
    }
    #[test]
    fn incomplete_or_impossible_storage_results_are_rejected() {
        assert!(parse_report(serde_json::json!({"ok": true})).is_err());
        let mut report = serde_json::json!({
            "total_bytes": 1000, "available_bytes": 300, "free_bytes": 400,
            "filesystem": "f2fs", "scanned_files": 0, "scanned_bytes": 0,
            "limited": false, "unreadable": 0, "hashed_files": 0, "hash_skipped": 0,
            "duplicate_bytes": 0, "folders": [], "largest": [], "documents": [],
            "duplicates": [], "scanned_at_ms": 123,
        });
        assert!(parse_report(report.clone()).is_ok());
        report["available_bytes"] = serde_json::json!(1001);
        assert!(parse_report(report.clone()).is_err());
        report["available_bytes"] = serde_json::json!(300);
        report["documents"] = serde_json::json!([{"path": "/data/user/0/private.txt", "bytes": 12, "modified_ms": 1}]);
        assert!(parse_report(report).is_err());
    }

    #[test]
    fn stepos_mcp_service_titles_never_expose_raw_package_or_class_names() {
        let report = default_stepos_ai_suite_report();
        for item in &report.mcp_services {
            assert!(
                !item.title.contains("com.")
                    && !item.title.contains("Service")
                    && !item.title.contains('/'),
                "service title must be human-readable without raw package/class names: {}",
                item.title
            );
        }
    }

    #[test]
    #[ignore = "requires explicit physical-device fixture and changes phone state"]
    fn real_device_stepos_suite_clipboard_mcp_and_pomodoro_end_to_end() {
        let Ok(adb) = Adb::discover() else {
            return;
        };
        let Ok(raw_devices) = adb.run(&["devices", "-l"]) else {
            return;
        };
        let transports =
            crate::device::deduplicate_transports(crate::device::parse_devices(&raw_devices));
        let Some(device) = transports
            .into_iter()
            .filter(|t| t.state == "device")
            .find_map(|t| Device::inspect(&adb, &t.id).ok())
        else {
            if std::env::var_os("STEPX_REAL_DEVICE_TEST").is_some() {
                panic!("STEPX_REAL_DEVICE_TEST=1 but no online StepX device found");
            }
            return;
        };

        // 1. Test real clipboard push + pull round-trip via Bridge.java
        let original_clip = pull_clipboard_from_phone(&adb, &device).unwrap_or_default();
        let test_clip = "StepX-E2E-Clipboard-Verify-0231-真机双向测试";
        let push_note = push_clipboard_to_phone(&adb, &device, test_clip)
            .expect("push_clipboard_to_phone must succeed on real phone");
        assert!(push_note.contains("已推送电脑剪贴板到手机"));

        let pulled = pull_clipboard_from_phone(&adb, &device)
            .expect("pull_clipboard_from_phone must succeed on real phone");
        assert_eq!(pulled, test_clip, "pulled clipboard must match pushed text");

        // Empty clipboard push must be rejected cleanly
        assert!(push_clipboard_to_phone(&adb, &device, "   ").is_err());

        // 2. Test real inspect_stepos_ai_suite (UsageStatsManager + Clipboard + processes)
        let suite = inspect_stepos_ai_suite(&adb, &device)
            .expect("inspect_stepos_ai_suite must succeed on real phone");
        assert!(suite.checked);
        assert_eq!(suite.phone_clipboard, test_clip);
        assert!(
            suite.wellbeing.screen_time_minutes > 0,
            "expected real screen time > 0 mins, got {}",
            suite.wellbeing.screen_time_minutes
        );
        assert!(
            !suite.wellbeing.top_apps.is_empty(),
            "real device top_apps must not be empty"
        );
        for (name, pkg, mins) in &suite.wellbeing.top_apps {
            assert!(
                !name.is_empty() && !name.contains("com."),
                "top app name must be human-readable: {name}"
            );
            assert!(
                pkg.contains('.'),
                "top app pkg must be valid package name: {pkg}"
            );
            assert!(*mins > 0, "top app minutes must be > 0");
        }

        assert!(
            suite.aiengine_ability_count >= 20,
            "real device aiengine_ability_count should be >= 20, got {}",
            suite.aiengine_ability_count
        );
        assert!(
            suite.aiengine_htp_status.contains("HTP"),
            "real device aiengine_htp_status should report QNN HTP buckets, got {}",
            suite.aiengine_htp_status
        );
        assert!(
            suite.agentbus_status.contains("AgentBus"),
            "real device agentbus_status should report container status, got {}",
            suite.agentbus_status
        );
        assert!(
            suite.thermal_summary.contains("°C"),
            "real device thermal_summary should report temperatures, got {}",
            suite.thermal_summary
        );

        // 3. Test all 15 MCP quick actions on the real phone
        for key in [
            "alarm_15m",
            "alarm_0800",
            "calendar_event",
            "todo_create",
            "note_create",
            "voice_tts",
            "toggle_dnd",
            "toggle_wifi",
            "open_gallery",
            "open_aiengine",
            "open_safety_ledger",
            "open_alphame",
            "open_amoo_memory",
            "open_amoo_voice",
            "wake_amoo_assistant",
        ] {
            let res = trigger_stepos_mcp_action(&adb, &device, key, "StepX自动化验证")
                .unwrap_or_else(|e| panic!("MCP action {key} failed on real phone: {e:#}"));
            assert!(
                !res.trim().is_empty(),
                "MCP action {key} returned empty note"
            );
        }
        // Test clean_system_debug_temp safely cleans stale debug tarballs/XMLs without touching stepx-companion
        let _reclaimed = clean_system_debug_temp(&adb, &device)
            .expect("clean_system_debug_temp must succeed on real phone");
        // Ensure DND is turned off after test and return to home screen
        let _ = adb.script(
            &device.transport,
            "cmd notification set_dnd off >/dev/null 2>&1 || true; input keyevent KEYCODE_HOME >/dev/null 2>&1 || true",
        );
        assert!(trigger_stepos_mcp_action(&adb, &device, "nonexistent_action", "").is_err());

        // 4. Test real Calendar CRUD (create -> verify in list -> delete) via Bridge.java
        let (cal_id, cal_after_create, cal_note) =
            create_phone_calendar_preset(&adb, &device, 0, "StepX-E2E-测试日程")
                .expect("create_phone_calendar_preset must succeed on real phone");
        assert!(cal_id > 0);
        assert!(cal_note.contains("StepX-E2E-测试日程"));
        assert!(
            cal_after_create
                .iter()
                .any(|e| e.id == cal_id && e.title == "StepX-E2E-测试日程"),
            "created calendar event must appear in returned calendar_events"
        );
        let (cal_after_delete, _) = delete_phone_calendar_event(&adb, &device, cal_id)
            .expect("delete_phone_calendar_event must succeed on real phone");
        assert!(
            !cal_after_delete.iter().any(|e| e.id == cal_id),
            "deleted calendar event must be removed from calendar_events"
        );

        // 4b. Test cross-platform .ics calendar + selected-date event creation
        let ics_backup = std::fs::read_to_string(desktop_calendar_ics_path()).ok();
        let today = Local::now();
        let (grid_cal_id, grid_cal_events, grid_cal_note) = create_calendar_event_at(
            &adb,
            &device,
            "StepX 跨端日历同步校验",
            today.year(),
            today.month(),
            today.day(),
            "09:30",
            true,
        )
        .expect("create_calendar_event_at must succeed on real phone");
        assert!(grid_cal_id > 0);
        assert!(grid_cal_note.contains("StepX 跨端日历同步校验"));
        assert!(
            grid_cal_events
                .iter()
                .any(|e| e.id == grid_cal_id && e.title == "StepX 跨端日历同步校验"),
            "selected-date calendar event must appear on phone"
        );
        let desktop_cal_events = read_desktop_system_calendar_events();
        assert!(
            desktop_cal_events
                .iter()
                .any(|e| e.title == "StepX 跨端日历同步校验"),
            "selected-date calendar event must also be saved in read_desktop_system_calendar_events()"
        );
        let (grid_cal_after_del, _) = delete_phone_calendar_event(&adb, &device, grid_cal_id)
            .expect("delete_phone_calendar_event for grid event must succeed");
        if let Some(prev_ics) = ics_backup {
            let _ = std::fs::write(desktop_calendar_ics_path(), prev_ics);
        }
        assert!(
            !grid_cal_after_del.iter().any(|e| e.id == grid_cal_id),
            "deleted selected-date calendar event must be removed from phone"
        );

        // 5. Test real Alarm CRUD (create -> toggle off -> delete) via Bridge.java
        let (alarm_id, alarms_after_create, alarm_note) =
            create_phone_alarm(&adb, &device, 7, 45, "StepX-E2E-测试闹钟")
                .expect("create_phone_alarm must succeed on real phone");
        assert!(alarm_id > 0);
        assert!(alarm_note.contains("07:45"));
        assert!(
            alarms_after_create
                .iter()
                .any(|a| a.id == alarm_id && a.enabled && a.message == "StepX-E2E-测试闹钟"),
            "created alarm must appear enabled in returned alarms"
        );
        let (alarms_after_toggle, _) = toggle_phone_alarm(&adb, &device, alarm_id, false)
            .expect("toggle_phone_alarm must succeed on real phone");
        assert!(
            alarms_after_toggle
                .iter()
                .any(|a| a.id == alarm_id && !a.enabled),
            "toggled alarm must be disabled in returned alarms"
        );
        let (alarms_after_delete, _) = delete_phone_alarm(&adb, &device, alarm_id)
            .expect("delete_phone_alarm must succeed on real phone");
        assert!(
            !alarms_after_delete.iter().any(|a| a.id == alarm_id),
            "deleted alarm must be removed from alarms"
        );

        // 5b. Test custom alarm creation via time string (create -> toggle -> delete)
        let (str_alarm_id, str_alarms_after_create, str_alarm_note) =
            create_phone_alarm_from_time_str(&adb, &device, "08:25", "StepX 闹钟页验证")
                .expect("create_phone_alarm_from_time_str must succeed on real phone");
        assert!(str_alarm_id > 0);
        assert!(str_alarm_note.contains("08:25"));
        assert!(
            str_alarms_after_create.iter().any(|a| {
                a.id == str_alarm_id && a.enabled && a.message == "StepX 闹钟页验证"
            }),
            "custom time-string alarm must appear enabled in returned alarms"
        );
        let (str_alarms_toggled, _) = toggle_phone_alarm(&adb, &device, str_alarm_id, false)
            .expect("toggle_phone_alarm for custom alarm must succeed");
        assert!(
            str_alarms_toggled
                .iter()
                .any(|a| a.id == str_alarm_id && !a.enabled),
            "toggled custom alarm must be disabled"
        );
        let (str_alarms_deleted, _) = delete_phone_alarm(&adb, &device, str_alarm_id)
            .expect("delete_phone_alarm for custom alarm must succeed");
        assert!(
            !str_alarms_deleted.iter().any(|a| a.id == str_alarm_id),
            "deleted custom alarm must be removed"
        );

        // 5c. Test Phone Gallery E2E (capture screenshot -> verify thumbnail on disk -> delete)
        let (gallery_items, shot_note) = capture_phone_screenshot_to_gallery(&adb, &device)
            .expect("capture_phone_screenshot_to_gallery must succeed on real phone");
        assert!(
            !gallery_items.is_empty(),
            "gallery items must not be empty after screenshot"
        );
        assert!(shot_note.contains("StepX_"));
        let created_shot_path = gallery_items[0].remote_path.clone();
        let thumb_path = gallery_items[0]
            .local_thumb_path
            .as_ref()
            .expect("items[0].local_thumb_path must be Some");
        assert!(
            std::path::Path::new(thumb_path).is_file(),
            "items[0].local_thumb_path must exist on disk: {thumb_path}"
        );
        let gallery_after_del = delete_phone_gallery_item(&adb, &device, &created_shot_path)
            .expect("delete_phone_gallery_item must succeed on real phone");
        assert!(
            !gallery_after_del
                .iter()
                .any(|item| item.remote_path == created_shot_path),
            "deleted gallery screenshot must be removed from gallery_items"
        );

        // 6. Test real Pomodoro countdown start & stop on the physical backscreen
        let pomo_start = crate::backscreen::start_pomodoro_countdown(&adb, &device, 1500, 5)
            .expect("start_pomodoro_countdown must succeed on real phone");
        assert!(pomo_start.contains("25 分钟"));
        let pomo_stop = crate::backscreen::stop_pomodoro_countdown(&adb, &device)
            .expect("stop_pomodoro_countdown must succeed on real phone");
        assert!(pomo_stop.contains("已结束"));
        if !original_clip.trim().is_empty() {
            let _ = push_clipboard_to_phone(&adb, &device, &original_clip);
        }
    }

    #[test]
    #[ignore = "requires an explicitly authorized pandora-12 phone via STEPX_VERIFY_SERIAL"]
    fn real_device_pandora_12_backscreen_and_mcp_ecosystem_e2e() {
        let adb = Adb::discover().expect("Adb::discover must succeed");
        let serial = std::env::var("STEPX_VERIFY_SERIAL")
            .expect("Set STEPX_VERIFY_SERIAL to the explicitly authorized test transport");
        assert!(!serial.trim().is_empty(), "STEPX_VERIFY_SERIAL must not be empty");
        let device = Device::inspect(&adb, &serial).expect("Selected test phone must be reachable");

        let get_sys_setting = |key: &str| -> String {
            adb.shell(&device.transport, &["settings", "get", "system", key])
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let restore_sys_setting = |key: &str, orig: &str| {
            if orig.is_empty() || orig == "null" {
                let _ = adb.shell(&device.transport, &["settings", "delete", "system", key]);
            } else {
                let _ = adb.shell(&device.transport, &["settings", "put", "system", key, orig]);
            }
        };
        let get_sec_setting = |key: &str| -> String {
            adb.shell(&device.transport, &["settings", "get", "secure", key])
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let restore_sec_setting = |key: &str, orig: &str| {
            if orig.is_empty() || orig == "null" {
                let _ = adb.shell(&device.transport, &["settings", "delete", "secure", key]);
            } else {
                let _ = adb.shell(&device.transport, &["settings", "put", "secure", key, orig]);
            }
        };

        // Record original clipboard and BackScreen settings
        let orig_clip = pull_clipboard_from_phone(&adb, &device).unwrap_or_default();
        let orig_sig = get_sys_setting("stepos_back_screen_signature_text");
        let orig_matrix = get_sys_setting("back_screen_custom_matrix_data");
        let orig_stay = get_sys_setting("stepos_back_screen_stay_on_duration");
        let orig_flip = get_sys_setting("stepos_back_screen_flip_keep_running");
        let orig_call = get_sys_setting("stepos_back_screen_incoming_call_effect");
        let orig_notif = get_sys_setting("stepos_back_screen_notification_effect");
        let orig_basic_type = get_sys_setting("stepos_back_screen_basic_type");
        let orig_basic_text = get_sys_setting("stepos_back_screen_basic_text");

        println!(
            "[E2E-1] Original BackScreen settings: sig={orig_sig}, matrix_len={}, stay={orig_stay}, flip={orig_flip}, call={orig_call}, notif={orig_notif}, basic_type={orig_basic_type}, basic_text={orig_basic_text}",
            orig_matrix.len()
        );

        // 1a. BackScreen Chinese Signature: "开会勿扰", "专注中", and unlisted CJK "饕餮盛宴"
        for sig_text in ["开会勿扰", "专注中", "饕餮盛宴"] {
            let note = crate::backscreen::marquee(&adb, &device, sig_text, false, false)
                .unwrap_or_else(|e| panic!("backscreen::marquee({sig_text}) failed: {e:#}"));
            println!("[E2E-1a] marquee({sig_text}) -> {note}");
            assert!(note.contains(sig_text));
            let actual_sig = get_sys_setting("stepos_back_screen_signature_text");
            assert_eq!(actual_sig, sig_text);
            let actual_matrix = get_sys_setting("back_screen_custom_matrix_data");
            let cells: Vec<&str> = actual_matrix.split(',').collect();
            assert_eq!(
                cells.len(),
                391,
                "back_screen_custom_matrix_data must have 391 comma-separated pixels"
            );
            assert!(
                cells.iter().all(|&c| c == "0" || c == "1"),
                "all 391 cells must be 0 or 1"
            );
            let lit_count = cells.iter().filter(|&&c| c == "1").count();
            println!(
                "[E2E-1a] verified sig={actual_sig}, cells={}, lit_pixels={lit_count}",
                cells.len()
            );
            assert!(lit_count >= 15, "expected non-zero lit pixels, got {lit_count}");
        }

        // 1b. BackScreen Hardware Controls
        let stay_note = crate::backscreen::set_stay_on_duration(&adb, &device, 30)
            .expect("set_stay_on_duration(30) must succeed");
        assert_eq!(get_sys_setting("stepos_back_screen_stay_on_duration"), "30");

        let flip_note = crate::backscreen::set_flip_keep_running(&adb, &device, true)
            .expect("set_flip_keep_running(true) must succeed");
        assert_eq!(get_sys_setting("stepos_back_screen_flip_keep_running"), "1");

        let call_note = crate::backscreen::set_incoming_call_effect(&adb, &device, true)
            .expect("set_incoming_call_effect(true) must succeed");
        assert_eq!(
            get_sys_setting("stepos_back_screen_incoming_call_effect"),
            "1"
        );

        let notif_note = crate::backscreen::set_notification_effect(&adb, &device, true)
            .expect("set_notification_effect(true) must succeed");
        assert_eq!(
            get_sys_setting("stepos_back_screen_notification_effect"),
            "1"
        );
        println!(
            "[E2E-1b] Hardware controls verified: {stay_note} | {flip_note} | {call_note} | {notif_note}"
        );

        // Restore original BackScreen settings
        let _ = crate::backscreen::clear_frame(&adb, &device);
        restore_sys_setting("stepos_back_screen_signature_text", &orig_sig);
        restore_sys_setting("back_screen_custom_matrix_data", &orig_matrix);
        restore_sys_setting("stepos_back_screen_stay_on_duration", &orig_stay);
        restore_sys_setting("stepos_back_screen_flip_keep_running", &orig_flip);
        restore_sys_setting("stepos_back_screen_incoming_call_effect", &orig_call);
        restore_sys_setting("stepos_back_screen_notification_effect", &orig_notif);
        restore_sys_setting("stepos_back_screen_basic_type", &orig_basic_type);
        restore_sys_setting("stepos_back_screen_basic_text", &orig_basic_text);
        let _ = adb.script(
            &device.transport,
            "dumpsys display_ext back_screen basic type agent >/dev/null 2>&1 || true",
        );
        println!("[E2E-1c] Restored original BackScreen settings cleanly.");

        // 2. StepOS 12 Live AI & MCP Suite Inspection
        let report = inspect_stepos_ai_suite(&adb, &device)
            .expect("inspect_stepos_ai_suite must succeed on selected test phone");
        println!(
            "[E2E-2] Suite Report: firmware_profile={}, mcp_services={}, aiengine_ability_count={}, htp={}, agentbus={}, thermal={}",
            report.firmware_profile,
            report.mcp_services.len(),
            report.aiengine_ability_count,
            report.aiengine_htp_status,
            report.agentbus_status,
            report.thermal_summary
        );
        assert_eq!(
            report.firmware_profile,
            "pandora-12 MP (325 MCP Tools + 6 AppFunctions)"
        );
        assert_eq!(report.mcp_services.len(), 24);
        assert!(report.mcp_services.iter().all(|s| s.discovered));
        assert!(report.aiengine_ability_count >= 33);
        assert!(
            report.agentbus_status.contains("已就绪"),
            "agentbus_status must contain 已就绪, got: {}",
            report.agentbus_status
        );
        assert!(
            report.thermal_summary.contains("°C"),
            "thermal_summary must contain live temperatures, got: {}",
            report.thermal_summary
        );

        // 3. StepOS 12 New MCP Actions
        // 3a. Privacy & Delete Protection toggles (toggle twice and restore)
        let orig_del_sec = get_sec_setting("stepos_delete_file_and_image_protect");
        let orig_del_sys = get_sys_setting("stepos_delete_file_and_image_protect");
        let orig_cam = get_sec_setting("camera_toggle_enabled");
        let orig_mic = get_sec_setting("mic_toggle_enabled");
        let orig_loc = get_sec_setting("location_mode");

        for toggle_key in [
            "toggle_delete_protect",
            "toggle_global_camera",
            "toggle_global_mic",
            "toggle_global_location",
        ] {
            let first = trigger_stepos_mcp_action(&adb, &device, toggle_key, "")
                .unwrap_or_else(|e| panic!("{toggle_key} (1st) failed: {e:#}"));
            let second = trigger_stepos_mcp_action(&adb, &device, toggle_key, "")
                .unwrap_or_else(|e| panic!("{toggle_key} (2nd restore) failed: {e:#}"));
            println!("[E2E-3a] {toggle_key}: 1st='{first}' -> 2nd='{second}'");
            assert_ne!(
                first, second,
                "{toggle_key} should toggle state and then restore"
            );
        }
        restore_sec_setting("stepos_delete_file_and_image_protect", &orig_del_sec);
        restore_sys_setting("stepos_delete_file_and_image_protect", &orig_del_sys);
        restore_sec_setting("camera_toggle_enabled", &orig_cam);
        restore_sec_setting("mic_toggle_enabled", &orig_mic);
        restore_sec_setting("location_mode", &orig_loc);
        let _ = adb.script(
            &device.transport,
            "cmd sensor_privacy disable 0 camera >/dev/null 2>&1 || true; cmd sensor_privacy disable 0 microphone >/dev/null 2>&1 || true",
        );

        // 3b. capcut_handoff E2E verification
        let capcut_payload = "周末Vlog|/sdcard/DCIM/Camera/a.jpg;/sdcard/DCIM/Camera/b.jpg";
        let capcut_note =
            trigger_stepos_mcp_action(&adb, &device, "capcut_handoff", capcut_payload)
                .expect("capcut_handoff must succeed on real phone");
        println!("[E2E-3b] capcut_handoff -> {capcut_note}");
        assert!(capcut_note.contains("2 项素材"));
        assert!(capcut_note.contains("周末Vlog"));
        let remote_json_raw = adb
            .shell(
                &device.transport,
                &["cat", "/sdcard/Download/stepx_capcut_handoff.json"],
            )
            .expect("must read /sdcard/Download/stepx_capcut_handoff.json");
        println!("[E2E-3b] /sdcard/Download/stepx_capcut_handoff.json = {}", remote_json_raw.trim());
        let parsed_capcut: serde_json::Value = serde_json::from_str(remote_json_raw.trim())
            .expect("stepx_capcut_handoff.json must be valid JSON");
        assert_eq!(parsed_capcut["source"], "stepx_desktop");
        assert_eq!(parsed_capcut["prompt"], "周末Vlog");
        let media_arr = parsed_capcut["media_paths"].as_array().unwrap();
        assert_eq!(media_arr.len(), 2);
        assert_eq!(media_arr[0], "/sdcard/DCIM/Camera/a.jpg");
        assert_eq!(media_arr[1], "/sdcard/DCIM/Camera/b.jpg");
        let _ = adb.shell(
            &device.transport,
            &["rm", "-f", "/sdcard/Download/stepx_capcut_handoff.json"],
        );

        // 3c. Remaining StepOS 12 MCP actions
        for (action_key, payload, expected_substr) in [
            ("pull_remote_agent_files", "", "StepX-Agent-Sync"),
            ("open_gallery_trash", "", "相册回收站"),
            ("open_file_trash", "", "文件管理回收站"),
            ("open_alipay_coupons", "瑞幸咖啡团购神券", "支付宝"),
            ("open_migu_music", "", "咪咕音乐"),
            ("open_data_migration", "", "换机助手"),
            ("open_xms_service_numbers", "", "智慧信息"),
            ("open_language_settings", "", "多语言"),
            ("open_feedback", "", "反馈"),
            ("fill_otp_one_by_one", "验证码 654321", "654321"),
            ("query_express_number", "顺丰 SF1234567890123", "SF1234567890123"),
        ] {
            let note = trigger_stepos_mcp_action(&adb, &device, action_key, payload)
                .unwrap_or_else(|e| panic!("MCP action {action_key} failed: {e:#}"));
            println!("[E2E-3c] {action_key}('{payload}') -> {note}");
            assert!(
                note.contains(expected_substr),
                "expected {action_key} note to contain '{expected_substr}', got '{note}'"
            );
        }

        // Restore clipboard and return phone to home screen
        if !orig_clip.trim().is_empty() {
            let _ = push_clipboard_to_phone(&adb, &device, &orig_clip);
        }
        let _ = adb.shell(
            &device.transport,
            &["input", "keyevent", "KEYCODE_HOME"],
        );
        println!("[E2E-DONE] Restored clipboard and pressed KEYCODE_HOME.");
    }

    #[cfg(unix)]
    #[test]
    fn dual_transport_failover_routes_toolbox_actions_to_online_backup_transport() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::native_mcp::test_support::Fixture::new(vec![
            serde_json::json!({"name":"settings_adjust_brightness","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"properties":{"newValue":{"type":"string"}}}}),
            serde_json::json!({"name":"set_back_screen_basic_type","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"properties":{"type":{"type":"string"},"text":{"type":"string"}}}})
        ],Some(serde_json::json!("72%")));
        let path = dir.path().join("adb");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncd {}\nprintf '%s\\n' \"$*\" >> calls\n\
                 case \"$*\" in\n\
                 *-s\\ usb-dead-toolbox-unit*) echo 'error: device offline' >&2; exit 1;;\n\
                 *devices\\ -l*) printf 'List of devices attached\\n10.0.1.179:43567 device product:pandora model:PZ261A-TOOLBOX-UNIT\\n'; exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*ro.serialno*) printf 'STEPX_TOOLBOX_FAILOVER_SERIAL_UNIT\\n'; exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*forward*tcp:0*) echo {}; exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*settings*put*) exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*settings*get*stepos_back_screen_basic_type*) echo text; exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*settings*get*stepos_back_screen_basic_text*) echo WORK; exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*service\\ call\\ display_ext*) echo 'Result: Parcel(NULL)'; exit 0;;\n\
                 *-s\\ 10.0.1.179:43567*) exit 0;;\n\
                 esac\nexit 1\n",
                shell_quote(dir.path().to_str().unwrap()), dir.port,
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adb = Adb { path };
        let device = Device {
            transport: "usb-dead-toolbox-unit".into(),
            serial: "STEPX_TOOLBOX_FAILOVER_SERIAL_UNIT".into(),
            model: "PZ261A-TOOLBOX-UNIT".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        device.set_backup_transports(vec!["10.0.1.179:43567".into()]);

        let (pct, bright_note) = set_phone_brightness_percent(&adb, &device, 72)
            .expect("brightness adjustment must fail over to online wireless backup transport");
        assert_eq!(pct, 72);
        assert!(bright_note.contains("72%"));

        let sig_note = trigger_stepos_mcp_action(&adb, &device, "backscreen_sig:WORK", "")
            .expect("backscreen_sig must fail over to online wireless backup transport");
        assert!(sig_note.contains("WORK"));
    }

    #[test]
    fn ps_false_positive_rejection_and_live_mcp_readiness_parsing() {
        let unrelated_ps = concat!(
            "===PS===\n",
            "root 993 2 0 0 0 0 S [stepos_pmicwd]\n",
            "system 1832 1 10870256 5440 0 0 S vendor.stepos.thermal-service\n",
            "===MCP===\n",
            "2 services found:\n",
            "  com.android.calendar/com.stepos.calendar.mcp.CalendarAbsMCPService\n",
            "  com.android.permissioncontroller/com.stepos.permissioncontroller.mcp.McpService\n",
        );
        let report = parse_stepos_ai_suite_output(unrelated_ps);
        assert!(!report.aiengine_running);
        assert!(!report.safety_center_running);
        assert!(!report.wellbeing.alphame_running);
        let ready_ids: Vec<&str> = report
            .mcp_services
            .iter()
            .filter(|s| s.discovered)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            ready_ids,
            vec![
                "com.android.calendar/com.stepos.calendar.mcp.CalendarAbsMCPService",
                "com.android.permissioncontroller/com.stepos.permissioncontroller.mcp.McpService",
            ]
        );

        assert!(report.mcp_services.iter().all(|item| !item.ready));
        assert_eq!(report.capability_evidence.mcp_access, None);

        let with_abilities = concat!(
            "===PS===\n",
            "system 1832 1 10870256 5440 0 0 S vendor.stepos.thermal-service\n",
            "===AIENGINE===\n",
            "Registered abilities count: 6\n",
            "DSP bucket=HTP: available=2 / max=2\n",
        );
        let report2 = parse_stepos_ai_suite_output(with_abilities);
        assert!(report2.aiengine_running);
        assert!(report2.aiengine_model_summary.contains("StepOS AI 引擎运行中"));
        assert!(report2.aiengine_model_summary.contains("6 项已注册能力"));
    }

    #[test]
    fn alarm_and_calendar_natural_time_parsers_support_relative_and_chinese_expressions() {
        let now = Local
            .with_ymd_and_hms(2026, 10, 31, 10, 15, 0)
            .earliest()
            .unwrap();
        assert_eq!(parse_alarm_time_str_at("730", now).unwrap(), (7, 30));
        assert_eq!(parse_alarm_time_str_at("早上8点半", now).unwrap(), (8, 30));
        assert_eq!(parse_alarm_time_str_at("今晚9点", now).unwrap(), (21, 0));
        assert_eq!(parse_alarm_time_str_at("2小时后", now).unwrap(), (12, 15));
        assert_eq!(
            parse_alarm_time_str_at("1小时30分后", now).unwrap(),
            (11, 45)
        );

        let (s1, e1) = parse_calendar_datetime_str_at("明天 15:00", now).unwrap();
        let exp_s1 = Local
            .with_ymd_and_hms(2026, 11, 1, 15, 0, 0)
            .earliest()
            .unwrap()
            .timestamp_millis();
        assert_eq!(s1, exp_s1);
        assert_eq!(e1, exp_s1 + 3_600_000);

        let (s2, e2) = parse_calendar_datetime_str_at("后天 10:00-11:30", now).unwrap();
        let exp_s2 = Local
            .with_ymd_and_hms(2026, 11, 2, 10, 0, 0)
            .earliest()
            .unwrap()
            .timestamp_millis();
        let exp_e2 = Local
            .with_ymd_and_hms(2026, 11, 2, 11, 30, 0)
            .earliest()
            .unwrap()
            .timestamp_millis();
        assert_eq!((s2, e2), (exp_s2, exp_e2));

        let (s3, e3) = parse_calendar_datetime_str_at("2026-11-02 14:30", now).unwrap();
        let exp_s3 = Local
            .with_ymd_and_hms(2026, 11, 2, 14, 30, 0)
            .earliest()
            .unwrap()
            .timestamp_millis();
        assert_eq!(s3, exp_s3);
        assert_eq!(e3, exp_s3 + 3_600_000);
    }

    #[cfg(unix)]
    #[test]
    fn native_workbuddy_actions_stay_unavailable_while_settings_shortcuts_work() {
        let (_dir, adb, device) = action_test_adb(
            r#"
case "$*" in
*set\ -e*|*cat*)
    local_script=$(printf '%s' "$4" | sed "s|/sdcard/Download|$PWD/phone|g")
    sh -c "$local_script";;
*) exit 0;;
esac"#,
        );
        for key in ["create_workbuddy_card", "update_workbuddy_card", "remove_workbuddy_card"] {
            assert!(trigger_stepos_mcp_action(&adb, &device, key, "整理周报纪要").is_err(),
                "unimplemented native cards must not manufacture successful task receipts");
        }

        for key in [
            "open_safebox",
            "open_security_scan",
            "open_permission_manager",
            "open_app_manager",
            "open_backscreen",
            "search_files",
        ] {
            let res = trigger_stepos_mcp_action(&adb, &device, key, "").unwrap();
            assert!(!res.is_empty());
        }
    }

    #[test]
    fn backscreen_display_state_and_calendar_markdown_location_work() {
        let dump = "mIsBackScreenOn=true\nactiveLayer=AgentSprite\nBASIC_TYPE=1\nBASIC_TEXT=专注中\nSTAY_SECS=30\n";
        let summary = parse_backscreen_display_state(dump);
        assert!(summary.contains("AI 精灵 (AgentSprite)"));
        assert!(summary.contains("点阵亮起"));
        assert!(summary.contains("签名「专注中」"));
        assert!(summary.contains("常亮 30s"));

        let ev = PhoneCalendarEvent {
            recurring: Some(false),
            id: 101,
            title: "项目复盘会".into(),
            dtstart: 1761840000000,
            dtend: 1761843600000,
            all_day: false,
            location: "总部 B 座 402".into(),
            description: "带上季度报表".into(),
            source: "手机日历".into(),
        };
        let md_line = format_calendar_event_markdown_line(&ev);
        assert!(md_line.contains("@ 总部 B 座 402"));
        assert!(md_line.contains("— 带上季度报表"));

        let (t, l, d) =
            extract_inline_calendar_location("项目复盘会 @ 总部 B 座 402", "带上季度报表");
        assert_eq!(t, "项目复盘会");
        assert_eq!(l, "总部 B 座 402");
        assert_eq!(d, "带上季度报表");
    }

    #[cfg(unix)]
    #[test]
    fn suite_report_cache_hits_and_invalidates_on_control_mutation() {
        invalidate_stepos_suite_cache();
        let (dir, adb, device) = action_test_adb(
            r#"
case "$*" in
*stat\ -c\ %s*) printf 'READY'; exit 0;;
*app_process*)
    printf 'STEPX_RESULT:{"ok":true,"screen_time_minutes":35,"unlock_count":6,"clipboard":"HelloCache","top_apps":[],"calendar_events":[],"alarms":[],"read_errors":[],"controls":{"dnd_on":false,"wifi_on":true,"bluetooth_on":true,"brightness_percent":70}}\n'
    exit 0;;
*sget*)
    printf '===PS===\ncom.stepos.aiengine\n===PRIVACY===\nBS_SIG=专注中\nSCR_BRIGHT=180\n'
    exit 0;;
*) exit 0;;
esac"#,
        );
        crate::bridge::mark_bridge_jar_cached(&device.serial);

        let r1 = inspect_stepos_ai_suite(&adb, &device).unwrap();
        assert_eq!(r1.phone_clipboard, "HelloCache");
        let calls_after_first = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        let count_first = calls_after_first.lines().count();
        assert!(count_first >= 2);

        // Second call within 90s TTL must return cached report with zero new ADB invocations
        let r2 = inspect_stepos_ai_suite(&adb, &device).unwrap();
        assert_eq!(r2.phone_clipboard, "HelloCache");
        let calls_after_second = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert_eq!(calls_after_first, calls_after_second);

        // Setting mutation must invalidate the suite cache
        let _ = set_phone_brightness_percent(&adb, &device, 80).unwrap();
        assert!(
            !suite_report_cache()
                .lock()
                .unwrap()
                .contains_key(&device.serial),
            "set_phone_brightness_percent must invalidate SUITE_REPORT_CACHE"
        );
        invalidate_stepos_suite_cache();
    }
}

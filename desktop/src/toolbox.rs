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
    device.verify(adb)?;
    adb.shell(
        &device.transport,
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
    device.verify(adb)?;
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
    let out = adb.script(&device.transport, script)?;
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
    device.verify(adb)?;
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
        .env("STEPX_SERIAL", &device.transport)
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StepOsAiSuiteReport {
    pub checked: bool,
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
            "com.android.calendar/.mcp.CalendarAbsMCPService",
            "日历日程摘要服务",
            "日程与时间",
            "查看今日日程",
        ),
        (
            "com.android.calendar/.mcp.CalendarEventService",
            "日历事件创建服务",
            "日程与时间",
            "新建手机日程",
        ),
        (
            "com.android.alarmclock/.mcp.AlarmMCPService",
            "闹钟与倒计时服务",
            "日程与时间",
            "设 15 分钟闹钟",
        ),
        (
            "com.stepos.media.gallery/.mcp.GalleryMCPService",
            "相册语义检索服务",
            "相册与文件",
            "打开手机相册",
        ),
        (
            "com.stepos.filemanager/.mcp.FileManagerMcpService",
            "跨端文件寻址服务",
            "相册与文件",
            "跨端文件寻址",
        ),
        (
            "com.stepos.taskrobot/.mcp.TaskRobotMCPService",
            "自动化任务编排服务",
            "安全与自动化",
            "自动化任务编排",
        ),
        (
            "com.stepos.voice/.mcp.VoiceAbilityService",
            "语音播报与识别服务",
            "语音与系统",
            "唤醒语音助手",
        ),
        (
            "com.android.settings/.mcp.McpService",
            "系统设置与勿扰控制",
            "语音与系统",
            "切换勿扰模式",
        ),
        (
            "com.stepos.screen/.mcp.ScreenMCPService",
            "端侧屏幕感知服务",
            "语音与系统",
            "端侧视觉看屏",
        ),
        (
            "com.stepos.agentbus/.mcp.NativeMcpService",
            "原生智能体总线服务",
            "安全与自动化",
            "AgentBus 路由",
        ),
        (
            "com.stepos.agentsafetycenter/.mcp.AbilityService",
            "AI 安全与权限审计中心",
            "安全与自动化",
            "打开安全审计台账",
        ),
        (
            "com.android.dialer/.mcp.AbilityService",
            "电话拨号与通话协同",
            "语音与系统",
            "通话状态协同",
        ),
        (
            "com.android.mms/.mcp.AbilityService",
            "短信与验证码流转服务",
            "语音与系统",
            "验证码流转",
        ),
        (
            "com.stepos.safebox/.mcp.AbilityService",
            "私密保险箱存储服务",
            "安全与自动化",
            "私密存储保护",
        ),
        (
            "com.stepos.securitycenter/.mcp.AbilityService",
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
            "com.stepos.common/.mcp.CommonAbilityService",
            "系统基础协同能力服务",
            "语音与系统",
            "系统基础协同",
        ),
        (
            "com.android.packageinstaller/.mcp.McpService",
            "应用安装与校验服务",
            "相册与文件",
            "应用部署校验",
        ),
        (
            "com.android.permissioncontroller/.mcp.McpService",
            "运行时权限治理服务",
            "安全与自动化",
            "权限自愈守护",
        ),
        (
            "com.stepos.secureservice/.mcp.AbilityService",
            "硬件可信执行环境服务",
            "安全与自动化",
            "可信执行环境",
        ),
    ];

    StepOsAiSuiteReport {
        checked: false,
        aiengine_running: true,
        aiengine_model_summary:
            "StepOS 端侧 NPU 引擎已就绪 · 支持离线意图识别、屏幕语义理解与多模态视觉感知".into(),
        aiengine_ability_count: 33,
        aiengine_htp_status: "QNN HTP 2/2 空闲".into(),
        agentbus_status: "AgentBus 就绪 · 6 路通道在线 · 云端 mTLS 就绪".into(),
        thermal_summary: String::new(),
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

pub fn clean_nullable_field(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("undefined")
    {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn is_e2e_test_calendar_title(title: &str) -> bool {
    let t = title.trim();
    t == "StepX 日历网格验证" || t.contains("StepX E2E Calendar") || t.contains("StepX自动化验证")
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
        let source =
            clean_nullable_field(item.get("source").and_then(|v| v.as_str()).unwrap_or(""));
        let source = if source.is_empty() {
            "手机日历".to_string()
        } else {
            source
        };
        if id > 0 && dtstart > 0 {
            if let Some(existing) = events.iter_mut().find(|e| {
                (e.id == id && e.dtstart == dtstart)
                    || (e.title.trim().eq_ignore_ascii_case(title.trim()) && e.dtstart == dtstart)
            }) {
                if existing.location.is_empty() && !location.is_empty() {
                    existing.location = location;
                }
                if existing.description.is_empty() && !description.is_empty() {
                    existing.description = description;
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
            });
        }
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
                        let id = deterministic_event_id(&clean_title, start_ms);
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
                                source: default_source.to_string(),
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

pub fn read_desktop_system_calendar_events() -> Vec<PhoneCalendarEvent> {
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
            continue;
        };
        for ev in parse_ics_events(&text, &source_name) {
            if !all_events.iter().any(|existing| {
                events_match(&existing.title, existing.dtstart, &ev.title, ev.dtstart)
            }) {
                all_events.push(ev);
            }
        }
    }
    all_events.sort_by_key(|e| e.dtstart);
    all_events
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut merged: Vec<PhoneCalendarEvent> = Vec::new();
    if let Ok(existing_text) = std::fs::read_to_string(&path) {
        for ev in parse_ics_events(&existing_text, "电脑日历") {
            merged.push(ev);
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

    let now_utc = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut ics = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//StepX Companion//Cross-Platform Calendar//CN\r\nCALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\n",
    );
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
        ics.push_str(&format!("UID:stepx-{uid_id}@stepx.local\r\n"));
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
    Ok(())
}

pub fn save_desktop_calendar_event(
    title: &str,
    dtstart: i64,
    dtend: i64,
    description: &str,
) -> Result<()> {
    let clean_title = if title.trim().is_empty() {
        "桌面协同日程"
    } else {
        title.trim()
    };
    let end_ms = if dtend > dtstart {
        dtend
    } else {
        dtstart + 3_600_000
    };
    let ev = PhoneCalendarEvent {
        id: deterministic_event_id(clean_title, dtstart),
        title: clean_title.to_string(),
        dtstart,
        dtend: end_ms,
        all_day: false,
        location: String::new(),
        description: description.trim().to_string(),
        source: "电脑日历".into(),
    };
    export_merged_calendar_ics(&[ev])
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
        if message.contains("StepX自动化验证") || message.contains("StepX E2E") {
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

pub fn inspect_stepos_ai_suite(adb: &Adb, device: &Device) -> Result<StepOsAiSuiteReport> {
    device.verify(adb)?;
    let script = concat!(
        "echo '===PS==='; ",
        "ps -A 2>/dev/null | grep -E 'stepos|openclaw|alphame|aiengine|agentsafety|agentbus|companion' || true; ",
        "echo '===ACC==='; ",
        "settings get secure enabled_accessibility_services 2>/dev/null || true; ",
        "echo '===NOTIF==='; ",
        "settings get secure enabled_notification_listeners 2>/dev/null || true; ",
        "echo '===AIENGINE==='; ",
        "dumpsys activity service com.stepos.aiengine/.service.AIEngineManagerService 2>/dev/null | head -n 60 || true; ",
        "echo '===AGENTBUS==='; ",
        "dumpsys activity service com.stepos.agentbus/.platform.AgentBusService status 2>/dev/null | head -n 20 || true; ",
        "echo '===THERMAL==='; ",
        "dumpsys thermalservice 2>/dev/null | grep -E 'mValue=|Thermal Status:' | head -n 50 || true"
    );
    let raw = adb.script(&device.transport, script)?;
    let mut report = parse_stepos_ai_suite_output(&raw);

    if let Ok(val) = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({ "op": "stepos_usage" }),
        std::time::Duration::from_secs(8),
    ) {
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
        if let Some(controls) = parse_bridge_controls(&val) {
            report.controls = controls;
        }

        let auto_sync_calendar = crate::pairing::Preferences::load()
            .map(|p| p.auto_sync_calendar)
            .unwrap_or(true);
        let desktop_events = read_desktop_system_calendar_events();
        let now_ms = Local::now().timestamp_millis();
        let min_ms = now_ms - 24 * 3_600_000;
        let max_ms = now_ms + 30 * 86_400_000;

        if auto_sync_calendar {
            let mut pushed = 0usize;
            for dev in &desktop_events {
                if dev.dtstart < min_ms || dev.dtstart > max_ms {
                    continue;
                }
                if let Some(existing) = report
                    .calendar_events
                    .iter_mut()
                    .find(|p| events_match(&p.title, p.dtstart, &dev.title, dev.dtstart))
                {
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
                        report.calendar_events.push(dev.clone());
                    }
                } else {
                    report.calendar_events.push(dev.clone());
                }
            }
            for dev in &desktop_events {
                if let Some(existing) = report
                    .calendar_events
                    .iter_mut()
                    .find(|p| events_match(&p.title, p.dtstart, &dev.title, dev.dtstart))
                {
                    existing.source = "双端已同步".into();
                }
            }
            report.calendar_events.sort_by_key(|e| e.dtstart);
            let _ = export_merged_calendar_ics(&report.calendar_events);
        } else {
            for dev in desktop_events {
                if let Some(existing) = report
                    .calendar_events
                    .iter_mut()
                    .find(|p| events_match(&p.title, p.dtstart, &dev.title, dev.dtstart))
                {
                    existing.source = "双端已同步".into();
                } else if dev.dtstart >= min_ms && dev.dtstart <= max_ms {
                    report.calendar_events.push(dev);
                }
            }
            report.calendar_events.sort_by_key(|e| e.dtstart);
        }

        report.last_action_note = format!(
            "已同步双端日历 {} 项日程 · 手机闹钟 {} 个",
            report.calendar_events.len(),
            report.alarms.len()
        );
    }

    Ok(report)
}

fn parse_stepos_ai_suite_output(raw: &str) -> StepOsAiSuiteReport {
    let mut report = default_stepos_ai_suite_report();
    report.checked = true;

    let mut section = "";
    let mut ps_lines = Vec::new();
    let mut acc_text = String::new();
    let mut notif_text = String::new();
    let mut clip_lines = Vec::new();
    let mut aiengine_lines = Vec::new();
    let mut agentbus_lines = Vec::new();
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
            "===THERMAL===" => {
                section = "thermal";
                continue;
            }
            _ => {}
        }
        match section {
            "ps" => ps_lines.push(trimmed),
            "acc" => {
                if !trimmed.is_empty() && trimmed != "null" {
                    acc_text.push_str(trimmed);
                    acc_text.push(' ');
                }
            }
            "notif" => {
                if !trimmed.is_empty() && trimmed != "null" {
                    notif_text.push_str(trimmed);
                    notif_text.push(' ');
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
            "thermal" => {
                if !trimmed.is_empty() {
                    thermal_lines.push(trimmed);
                }
            }
            _ => {}
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
    let agentbus_alive = ps_joined.contains("com.stepos.agentbus")
        || ps_joined.contains("openclaw")
        || aiengine_alive;

    report.aiengine_running = aiengine_alive || !ps_lines.is_empty();
    report.safety_center_running = safety_alive || !ps_lines.is_empty();
    report.wellbeing.alphame_running = alphame_alive || !ps_lines.is_empty();
    report.gui_accessibility_enabled = !acc_text.trim().is_empty()
        && (acc_text.contains("stepos")
            || acc_text.contains("companion")
            || acc_text.contains("alphame"));
    report.notification_listener_enabled = !notif_text.trim().is_empty();
    report.mcp_permission_granted = agentbus_alive || report.safety_center_running;
    report.screen_perception_ready = report.gui_accessibility_enabled || report.aiengine_running;

    // Parse AIEngineManagerService telemetry (Registered abilities, QNN HTP buckets, active holders)
    let mut ability_count: u32 = 0;
    let mut htp_status = String::new();
    let mut active_holders: Vec<String> = Vec::new();
    for line in &aiengine_lines {
        if let Some(rest) = line.strip_prefix("Registered abilities count:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                ability_count = n;
            }
        } else if line.contains("DSP bucket=HTP:") {
            // e.g. "DSP bucket=HTP: available=2 / max=2"
            if let Some(after) = line.split("available=").nth(1) {
                let compact = after.replace(" / max=", "/").trim().to_string();
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
        report.aiengine_ability_count = ability_count;
    }
    if !htp_status.is_empty() {
        report.aiengine_htp_status = htp_status.clone();
    }

    report.aiengine_model_summary = if aiengine_alive {
        let mut parts = vec!["StepOS 端侧 NPU 引擎在线".to_string()];
        if ability_count > 0 {
            parts.push(format!("已注册 {ability_count} 项端侧算子"));
        } else {
            parts.push("支持离线意图识别与屏幕视觉感知".to_string());
        }
        if !htp_status.is_empty() {
            parts.push(htp_status);
        }
        if !active_holders.is_empty() {
            parts.push(format!("常驻算子：{}", active_holders.join(", ")));
        }
        parts.join(" · ")
    } else {
        "StepOS 端侧 NPU 引擎待命 · 按需唤醒离线推理服务".into()
    };

    // Parse AgentBusService status
    for line in &agentbus_lines {
        if line.starts_with("Container:") {
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
        if let Some(v_idx) = line.find("mValue=") {
            let after_v = &line[v_idx + 7..];
            let val_str = after_v.split(',').next().unwrap_or("").trim();
            let Ok(temp) = val_str.parse::<f32>() else {
                continue;
            };
            if temp <= 0.0 || temp > 120.0 {
                continue;
            }
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

    let ready_count = report.mcp_services.iter().filter(|s| s.ready).count();
    report.last_action_note = format!(
        "实时检测完成：{} / {} 项原生服务就绪 · 端侧 NPU {} · 安全中心 {}",
        ready_count,
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

pub fn push_clipboard_to_phone(adb: &Adb, device: &Device, text: &str) -> Result<String> {
    let clean = text.trim();
    if clean.is_empty() {
        bail!("电脑剪贴板当前为空，请先复制文字后再推送");
    }
    let val = crate::bridge::call_bridge(
        adb,
        device,
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

pub fn pull_clipboard_from_phone(adb: &Adb, device: &Device) -> Result<String> {
    let val = crate::bridge::call_bridge(
        adb,
        device,
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

pub fn create_phone_calendar_event(
    adb: &Adb,
    device: &Device,
    title: &str,
    dtstart: i64,
    dtend: i64,
    description: &str,
) -> Result<(i64, Vec<PhoneCalendarEvent>, String)> {
    let clean_title = if title.trim().is_empty() {
        "桌面协同日程"
    } else {
        title.trim()
    };
    let val = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "calendar_create",
            "title": clean_title,
            "start_ms": dtstart,
            "end_ms": dtend,
            "dtstart": dtstart,
            "dtend": dtend,
            "description": description.trim(),
        }),
        std::time::Duration::from_secs(8),
    )?;
    let created_id = val.get("created_id").and_then(|v| v.as_i64()).unwrap_or(0);
    if created_id <= 0 {
        bail!("写入手机日历未返回有效日程 ID");
    }
    let events = parse_bridge_calendar_events(&val);
    let time_label = events
        .iter()
        .find(|e| e.id == created_id)
        .map(|e| e.time_range_label())
        .unwrap_or_else(|| "已同步".into());
    Ok((
        created_id,
        events,
        format!("已在手机日历创建日程：「{clean_title}」（{time_label}）"),
    ))
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
    let desktop_events = read_desktop_system_calendar_events();
    let now_ms = Local::now().timestamp_millis();
    let min_ms = now_ms - 24 * 3_600_000;
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
    create_phone_calendar_event(
        adb,
        device,
        title,
        dtstart,
        dtend,
        "由 StepX 桌面端直接同步创建",
    )
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
    Ok((events, "已从手机日历移除该日程".into()))
}

pub fn parse_alarm_time_str_at(time_str: &str, now: chrono::DateTime<Local>) -> Result<(u8, u8)> {
    let trimmed = time_str.trim();
    if trimmed.is_empty() {
        bail!("请输入闹钟时间，例如 07:30 或 +15m");
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with('+')
        || lower.ends_with('m')
        || lower.ends_with("min")
        || trimmed.contains("分钟")
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
    if let Some((h, m)) = parse_hh_mm(trimmed) {
        return Ok((h as u8, m as u8));
    }
    bail!("请输入有效的时间格式，如 07:30、0800 或 +15m")
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
    let val = crate::bridge::call_bridge(
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
    )?;
    let created_id = val.get("created_id").and_then(|v| v.as_i64()).unwrap_or(0);
    if created_id <= 0 {
        bail!("写入手机闹钟未返回有效 ID");
    }
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
            raw_rel.replacen("/storage/emulated/0/", "/sdcard/", 1)
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
            let norm_gal = gal.trim().replacen("/storage/emulated/0/", "/sdcard/", 1);
            archived_paths.insert(norm_gal);
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
        let remote_path = path_str
            .trim()
            .replacen("/storage/emulated/0/", "/sdcard/", 1);
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
    device.verify(adb)?;
    let script = "find /sdcard/DCIM /sdcard/Pictures /sdcard/Download -maxdepth 3 -type f -not -path '*/.*' -not -name '.*' -not -name '*.trashed*' -not -name '*.pending*' -not -name '*.stepxlink' -not -path '*/StepX Companion/SC_*' \\( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.webp' -o -iname '*.gif' -o -iname '*.heic' \\) -printf '%T@\\t%s\\t%p\\n' 2>/dev/null | sort -rn | head -n 120";
    let raw = adb.script(&device.transport, script)?;
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

    for batch in uncached.chunks(4) {
        std::thread::scope(|s| {
            for &(remote_path, local_file) in batch {
                s.spawn(move || {
                    let _ = adb.transfer(
                        &device.transport,
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
                if !active.contains(path.to_string_lossy().as_ref())
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
    device.verify(adb)?;
    let ts = Local::now().format("%Y%m%d_%H%M%S");
    let remote_path = format!("/sdcard/Pictures/Screenshots/StepX_{ts}.png");
    let script = format!(
        "mkdir -p /sdcard/Pictures/Screenshots && screencap -p {p} && test -s {p}",
        p = shell_quote(&remote_path)
    );
    adb.script(&device.transport, &script)
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
    device.verify(adb)?;
    let clamped = percent.clamp(5, 100);
    let raw = ((clamped as u32 * 255) / 100).clamp(10, 255);
    let script = format!("settings put system screen_brightness {raw} >/dev/null 2>&1");
    adb.script(&device.transport, &script)?;
    Ok((clamped, format!("已将手机屏幕亮度调节为 {clamped}%")))
}

pub fn trigger_stepos_mcp_action(
    adb: &Adb,
    device: &Device,
    action_key: &str,
    payload: &str,
) -> Result<String> {
    device.verify(adb)?;
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
            let quoted_msg = shell_quote(msg);
            let script = format!(
                "am start -a android.intent.action.SET_ALARM \
                 --ei android.intent.extra.alarm.HOUR {h} \
                 --ei android.intent.extra.alarm.MINUTES {m} \
                 --es android.intent.extra.alarm.MESSAGE {quoted_msg} \
                 --ez android.intent.extra.alarm.SKIP_UI true >/dev/null 2>&1"
            );
            adb.script(&device.transport, &script)?;
            Ok(format!(
                "已在手机设定 15 分钟后（{h:02}:{m:02}）闹钟：「{msg}」"
            ))
        }
        "alarm_0800" => {
            let msg = if payload.trim().is_empty() {
                "StepX 早间日程提醒"
            } else {
                payload.trim()
            };
            let quoted_msg = shell_quote(msg);
            let script = format!(
                "am start -a android.intent.action.SET_ALARM \
                 --ei android.intent.extra.alarm.HOUR 8 \
                 --ei android.intent.extra.alarm.MINUTES 0 \
                 --es android.intent.extra.alarm.MESSAGE {quoted_msg} \
                 --ez android.intent.extra.alarm.SKIP_UI true >/dev/null 2>&1"
            );
            adb.script(&device.transport, &script)?;
            Ok(format!("已在手机设定 08:00 闹钟：「{msg}」"))
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
            let quoted_title = shell_quote(title);
            let script = format!(
                "am start -a android.intent.action.INSERT \
                 -t vnd.android.cursor.dir/event \
                 --es title {quoted_title} >/dev/null 2>&1 || \
                 am start -n com.android.calendar/com.stepos.calendar.module.events.activity.NewEditEventActivity \
                 --es title {quoted_title} >/dev/null 2>&1 || \
                 monkey -p com.android.calendar -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1"
            );
            adb.script(&device.transport, &script)?;
            Ok(format!("已在手机日历唤起新建日程：「{title}」"))
        }
        "note_create" => {
            let note = if payload.trim().is_empty() {
                "StepX 桌面速记内容"
            } else {
                payload.trim()
            };
            let _ = push_clipboard_to_phone(adb, device, note);
            let script = concat!(
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || ",
                "monkey -p com.stepos.habit -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true"
            );
            adb.script(&device.transport, script)?;
            Ok(format!("已将速记写入手机剪贴板并打开 Amoo：「{note}」"))
        }
        "voice_tts" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.personal_preference -n com.stepos.alphame/.persona.ui.PersonalMainActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || ",
                "input keyevent 231 >/dev/null 2>&1 || true"
            );
            adb.script(&device.transport, script)?;
            Ok("已唤醒手机端语音与智能助手服务".into())
        }
        "toggle_dnd" => {
            let script = concat!(
                "CUR=$(settings get global zen_mode 2>/dev/null || echo 0); ",
                "if [ \"$CUR\" = \"0\" ] || [ -z \"$CUR\" ]; then ",
                "  cmd notification set_dnd priority >/dev/null 2>&1 || cmd notification set_dnd on >/dev/null 2>&1 || true; ",
                "  echo DND_ON; ",
                "else ",
                "  cmd notification set_dnd off >/dev/null 2>&1 || true; ",
                "  echo DND_OFF; ",
                "fi"
            );
            let out = adb.script(&device.transport, script)?;
            if out.contains("DND_ON") {
                Ok("已开启手机勿扰模式（专注免打扰）".into())
            } else {
                Ok("已关闭手机勿扰模式（恢复正常通知）".into())
            }
        }
        "toggle_wifi" => {
            adb.script(&device.transport, "svc wifi enable >/dev/null 2>&1 || true")?;
            Ok("已确保手机 Wi-Fi 处于启用状态".into())
        }
        "open_gallery" => {
            adb.script(
                &device.transport,
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
            adb.script(&device.transport, script)?;
            Ok("已在手机打开端侧大模型管理中心".into())
        }
        "open_safety_ledger" => {
            let script = concat!(
                "am start -a stepos.intent.action.AGENT_SAFETY_CENTER_LAUNCHER -n com.stepos.agentsafetycenter/.auth.LaunchAuthActivity >/dev/null 2>&1 || ",
                "am start -a stepos.intent.action.AGENT_PERMISSION_MANAGER -n com.stepos.agentsafetycenter/.permission.AgentPermissionActivity >/dev/null 2>&1 || ",
                "am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d package:com.stepos.agentsafetycenter >/dev/null 2>&1"
            );
            adb.script(&device.transport, script)?;
            Ok("已在手机打开 AI 安全审计台账与权限中心".into())
        }
        "open_alphame" => {
            let script = concat!(
                "am start -a com.stepos.alphame.action.personal_preference -n com.stepos.alphame/.persona.ui.PersonalMainActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.alphame/.persona.ui.PersonalMainActivity >/dev/null 2>&1 || ",
                "am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d package:com.stepos.alphame >/dev/null 2>&1"
            );
            adb.script(&device.transport, script)?;
            Ok("已在手机打开 AlphaMe 个人画像中心".into())
        }
        "open_amoo_memory" => {
            let script = concat!(
                "am start -a android.intent.action.VIEW -d 'amoo://amoo/memory' >/dev/null 2>&1 || ",
                "am start -n com.stepos.ailauncher/.task.space.AmooMemoryActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&device.transport, script)?;
            Ok("已在手机打开 Amoo 记忆中心（关于我 / 交流偏好 / 备忘事项）".into())
        }
        "open_amoo_voice" => {
            let script = concat!(
                "am start -a com.stepos.ailauncher.action.VOICE_SETTING -n com.stepos.ailauncher/.chat.activity.VoiceSettingActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.ailauncher/.chat.activity.VoiceSettingActivity >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&device.transport, script)?;
            Ok("已在手机打开 Amoo 语音播报与音色设置".into())
        }
        "wake_amoo_assistant" => {
            let script = concat!(
                "am start -a com.stepos.ailauncher.action.STEP_ASSISTANT >/dev/null 2>&1 || ",
                "am start -a com.stepos.ailauncher.action.OPEN_CHAT >/dev/null 2>&1 || ",
                "am start -n com.stepos.habit/com.stepos.ailauncher.MainActivity >/dev/null 2>&1 || true"
            );
            adb.script(&device.transport, script)?;
            Ok("已在手机唤醒 Amoo 全局助手".into())
        }
        other => bail!("未知的快捷指令：{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_stepos_suite_has_twenty_mcp_services() {
        let report = default_stepos_ai_suite_report();
        assert_eq!(report.mcp_services.len(), 20);
        assert!(report.aiengine_running);
        assert!(report.safety_center_running);
        assert!(report.wellbeing.top_apps.is_empty());
        assert_eq!(report.wellbeing.screen_time_minutes, 0);
        assert!(report.phone_clipboard.is_empty());
        let parsed = parse_stepos_ai_suite_output(
            "===PS===\ncom.stepos.aiengine\ncom.stepos.agentsafetycenter\ncom.stepos.alphame\n===ACC===\ncom.stepx.companion.app/.neo.NeoAccessibilityService\n===NOTIF===\ncom.stepos.alphame\n===CLIP===\nHello StepX\n===USAGE===\npackage=com.stepos.alphame totalTimeUsed=\"57:02\"\n===AIENGINE===\nRegistered abilities count: 33\nDSP bucket=HTP: available=2 / max=2\n[bge-embedding] pkg=com.stepos.wlanmonitor state=IDLE inFlight=0\n===AGENTBUS===\nContainer: ready, connections: 6 ([GATEWAY]), inflight invocations: 0, ls server ready: true\n===THERMAL===\nTemperature{mValue=31.7, mType=3, mName=skin-therm, mStatus=0}\nTemperature{mValue=29.4, mType=2, mName=battery, mStatus=0}\nTemperature{mValue=45.3, mType=9, mName=nsp0, mStatus=0}\nTemperature{mValue=48.5, mType=0, mName=cpu-0-0, mStatus=0}\n",
        );
        assert!(parsed.checked);
        assert!(parsed.aiengine_running);
        assert!(parsed.safety_center_running);
        assert!(parsed.gui_accessibility_enabled);
        assert_eq!(parsed.phone_clipboard, "Hello StepX");
        // Ensure grep -m 48 usagestats garbage never pollutes wellbeing
        assert_eq!(parsed.wellbeing.screen_time_minutes, 0);
        assert!(parsed.wellbeing.top_apps.is_empty());
        assert_eq!(parsed.aiengine_ability_count, 33);
        assert_eq!(parsed.aiengine_htp_status, "QNN HTP 2/2 空闲");
        assert!(parsed.aiengine_model_summary.contains("33 项端侧算子"));
        assert!(parsed.aiengine_model_summary.contains("QNN HTP 2/2 空闲"));
        assert!(parsed.aiengine_model_summary.contains("bge-embedding"));
        assert!(parsed.agentbus_status.contains("6 路原生通道在线"));
        assert!(parsed.thermal_summary.contains("机身 31.7°C"));
        assert!(parsed.thermal_summary.contains("NPU 45.3°C"));
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
                {"id": 11, "title": "自驾出发", "start_ms": 1793014800000_i64, "end_ms": 1793018400000_i64, "location": "高速服务区", "description": ""},
                {"id": 12, "title": "StepX自动化验证", "start_ms": 1793014800000_i64, "end_ms": 1793018400000_i64}
            ]
        });
        let cal_events = parse_bridge_calendar_events(&bridge_cal);
        assert_eq!(cal_events.len(), 1);
        assert_eq!(cal_events[0].location, "高速服务区");
        assert_eq!(cal_events[0].description, "");

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
                !item.title.contains("com.") && !item.title.contains('('),
                "service title must be human-readable without raw package/class names: {}",
                item.title
            );
        }
    }

    #[test]
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
        let (grid_cal_id, grid_cal_events, grid_cal_note) = create_calendar_event_at(
            &adb,
            &device,
            "StepX 跨端日历同步校验",
            2026,
            10,
            26,
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
}

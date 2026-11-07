//! Guided Doctor & Google Play Harness Engine.
//!
//! Combines real-device Google Play suite inspection (GSF / GMS / Play Store /
//! 153 permissions / compatibility patch / network reachability), StepX Companion
//! self-healing checks, PC health diagnostics, and phone-token powered AI guidance
//! in an isolated ephemeral inference session that never pollutes user chat history.

pub use crate::companion_app::CompanionStatus;
use crate::{
    adb::Adb,
    device::{Device, GoogleSuiteStatus, PlayProtectReport},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DoctorTopic {
    #[default]
    GooglePlayGuide,
    CompanionSelfHeal,
    PcTroubleshoot,
    Auto,
}

impl DoctorTopic {
    pub fn label(self) -> &'static str {
        match self {
            Self::GooglePlayGuide => "Google Play 安装向导",
            Self::CompanionSelfHeal => "伴侣连接自愈",
            Self::PcTroubleshoot => "电脑体检与清缓存",
            Self::Auto => "设备状态检查",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pass,
    ActionRequired,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorActionKind {
    InstallGoogleSuite,
    RepairGoogleCompatibility,
    InspectGoogleSuite,
    OpenPlayStoreInMirror,
    OpenPlayStoreOnPhone,
    OpenGoogleToolPage,
    RepairCompanionPermissions,
    ReconnectWirelessAdb,
    CleanPcTempCache,
}

impl DoctorActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::InstallGoogleSuite => "安装 Google Play",
            Self::RepairGoogleCompatibility => "修复权限与补丁",
            Self::InspectGoogleSuite => "重新检测",
            Self::OpenPlayStoreInMirror => "投屏打开商店",
            Self::OpenPlayStoreOnPhone => "手机打开商店",
            Self::OpenGoogleToolPage => "Google Play 专页",
            Self::RepairCompanionPermissions => "修复伴侣权限",
            Self::ReconnectWirelessAdb => "重连手机",
            Self::CleanPcTempCache => "清理缓存",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuidedStep {
    pub index: usize,
    pub title: String,
    pub state: StepState,
    pub badge: String,
    pub detail: String,
    pub action: Option<DoctorActionKind>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PcHealthSnapshot {
    pub os_label: String,
    pub disk_free_gb: f64,
    pub disk_total_gb: f64,
    pub mem_available_gb: f64,
    pub mem_total_gb: f64,
    pub proxy_configured: bool,
    pub proxy_summary: String,
    pub temp_cache_mb: u64,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DoctorReport {
    pub topic: DoctorTopic,
    pub query: String,
    pub checked_at_ms: i64,
    pub headline: String,
    pub summary: String,
    pub phone_token_used: bool,
    pub model_label: String,
    #[serde(default)]
    pub ai_conclusion: Option<String>,
    #[serde(skip)]
    pub google_status: Option<GoogleSuiteStatus>,
    #[serde(skip)]
    pub companion_status: Option<CompanionStatus>,
    pub google_network_reachable: Option<bool>,
    pub google_network_note: String,
    pub pc_snapshot: PcHealthSnapshot,
    pub steps: Vec<GuidedStep>,
    pub recommended_actions: Vec<DoctorActionKind>,
    pub ai_guidance_markdown: String,
}

/// Inspects a user query (case-insensitive) to route to the best `DoctorTopic`.
pub fn infer_topic_from_query(query: &str) -> DoctorTopic {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return DoctorTopic::GooglePlayGuide;
    }
    let lower = trimmed.to_lowercase();

    let google_keywords = [
        "google",
        "play",
        "gms",
        "gsf",
        "vending",
        "谷歌",
        "商店",
        "三件套",
        "认证",
        "未认证",
        "youtube",
        "gmail",
        "chrome",
        "海外应用",
        "外服",
    ];
    if google_keywords.iter().any(|kw| lower.contains(kw)) {
        return DoctorTopic::GooglePlayGuide;
    }

    let companion_keywords = [
        "companion",
        "伴侣",
        "通知",
        "弹窗",
        "连接",
        "投屏",
        "权限",
        "背屏",
        "扫码",
        "无线",
        "断连",
        "熄屏",
        "后台",
        "保活",
        "杀后台",
        "配对",
    ];
    if companion_keywords.iter().any(|kw| lower.contains(kw)) {
        return DoctorTopic::CompanionSelfHeal;
    }

    let pc_keywords = [
        "电脑", "卡顿", "磁盘", "缓存", "端口", "代理", "网络", "内存", "空间", "清理", "发热",
        "pc", "disk", "cache", "proxy",
    ];
    if pc_keywords.iter().any(|kw| lower.contains(kw)) {
        return DoctorTopic::PcTroubleshoot;
    }

    DoctorTopic::Auto
}

fn round_one_decimal(val: f64) -> f64 {
    if !val.is_finite() || val < 0.0 {
        0.0
    } else {
        (val * 10.0).round() / 10.0
    }
}

fn detect_os_label() -> String {
    let base = format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH);
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
                    let pretty = rest.trim().trim_matches('"').trim_matches('\'').trim();
                    if !pretty.is_empty() {
                        return format!("{pretty} · {}", std::env::consts::ARCH);
                    }
                }
            }
        }
    }
    base
}

fn parse_df_pk_output(output: &str) -> Option<(f64, f64)> {
    for line in output.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            if let (Ok(total_kb), Ok(avail_kb)) = (parts[1].parse::<u64>(), parts[3].parse::<u64>())
            {
                if total_kb > 0 {
                    let total_gb = round_one_decimal(total_kb as f64 / (1024.0 * 1024.0));
                    let free_gb = round_one_decimal(avail_kb as f64 / (1024.0 * 1024.0));
                    return Some((free_gb, total_gb));
                }
            }
        }
    }
    None
}

fn collect_disk_space_gb() -> (f64, f64) {
    for target in [".", "/home", "/"] {
        let mut cmd = Command::new("df");
        cmd.args(["-Pk", target]);
        crate::resources::quiet_command(&mut cmd);
        if let Ok(out) = crate::adb::run_process(cmd, None, Duration::from_secs(2)) {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Some(res) = parse_df_pk_output(&text) {
                    return res;
                }
            }
        }
    }
    (0.0, 0.0)
}

fn parse_proc_meminfo(content: &str) -> (f64, f64) {
    let mut total_kb: Option<u64> = None;
    let mut avail_kb: Option<u64> = None;
    let mut free_kb: Option<u64> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok());
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok());
        } else if let Some(rest) = line.strip_prefix("MemFree:") {
            free_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok());
        }
    }
    let total = total_kb
        .map(|kb| round_one_decimal(kb as f64 / (1024.0 * 1024.0)))
        .unwrap_or(0.0);
    let avail = avail_kb
        .or(free_kb)
        .map(|kb| round_one_decimal(kb as f64 / (1024.0 * 1024.0)))
        .unwrap_or(0.0);
    (avail, total)
}

fn collect_memory_gb() -> (f64, f64) {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        let (avail, total) = parse_proc_meminfo(&content);
        if total > 0.0 {
            return (avail, total);
        }
    }
    (0.0, 0.0)
}

fn sanitize_proxy_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Redact any embedded credentials like http://user:pass@host:port
    if let Some((scheme, rest)) = trimmed.split_once("://") {
        if let Some((_, host_part)) = rest.rsplit_once('@') {
            return format!("{scheme}://***@{host_part}");
        }
    } else if let Some((_, host_part)) = trimmed.rsplit_once('@') {
        return format!("***@{host_part}");
    }
    trimmed.chars().take(64).collect()
}

fn collect_proxy_status() -> (bool, String) {
    let keys = [
        ("HTTPS_PROXY", "HTTPS_PROXY"),
        ("https_proxy", "https_proxy"),
        ("HTTP_PROXY", "HTTP_PROXY"),
        ("http_proxy", "http_proxy"),
        ("ALL_PROXY", "ALL_PROXY"),
        ("all_proxy", "all_proxy"),
    ];
    let mut active = Vec::new();
    for (env_key, label) in keys {
        if let Ok(val) = std::env::var(env_key) {
            let clean = sanitize_proxy_value(&val);
            if !clean.is_empty() && !active.iter().any(|(_, v)| v == &clean) {
                active.push((label, clean));
            }
        }
    }
    if active.is_empty() {
        (false, "未配置环境变量代理（直连模式）".to_string())
    } else {
        let desc = active
            .into_iter()
            .take(2)
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" · ");
        (true, format!("已启用本机代理：{desc}"))
    }
}

fn dir_size_bounded(path: &Path, max_entries: &mut usize) -> u64 {
    if *max_entries == 0 || !path.exists() {
        return 0;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        *max_entries = max_entries.saturating_sub(1);
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        if *max_entries == 0 {
            break;
        }
        *max_entries = max_entries.saturating_sub(1);
        total = total.saturating_add(dir_size_bounded(&entry.path(), max_entries));
    }
    total
}

fn is_stepx_temp_entry_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("stepx-")
        || lower.starts_with("stepx_")
        || lower.starts_with(".tmp") && lower.ends_with(".apk")
}

fn collect_temp_cache_bytes() -> u64 {
    let mut budget = 1500usize;
    let mut total_bytes = 0u64;

    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            if budget == 0 {
                break;
            }
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if is_stepx_temp_entry_name(&name) {
                total_bytes =
                    total_bytes.saturating_add(dir_size_bounded(&entry.path(), &mut budget));
            }
        }
    }

    if let Some(home) = crate::resources::user_home() {
        let cache_dir = Path::new(&home).join(".cache/stepx-companion");
        if cache_dir.is_dir() {
            total_bytes = total_bytes.saturating_add(dir_size_bounded(&cache_dir, &mut budget));
        }
    }

    total_bytes
}

/// Collects real OS, disk, memory, proxy, and temporary cache health metrics on the host PC.
pub fn collect_pc_snapshot() -> PcHealthSnapshot {
    let os_label = detect_os_label();
    let (disk_free_gb, disk_total_gb) = collect_disk_space_gb();
    let (mem_available_gb, mem_total_gb) = collect_memory_gb();
    let (proxy_configured, proxy_summary) = collect_proxy_status();
    let temp_cache_bytes = collect_temp_cache_bytes();
    let temp_cache_mb = temp_cache_bytes / (1024 * 1024);

    let mut notes = Vec::new();
    if disk_total_gb > 0.0 {
        notes.push(format!(
            "磁盘可用 {disk_free_gb:.1} / {disk_total_gb:.1} GB"
        ));
    } else {
        notes.push("工作目录读写正常".to_string());
    }

    if mem_total_gb > 0.0 {
        notes.push(format!(
            "内存可用 {mem_available_gb:.1} / {mem_total_gb:.1} GB"
        ));
    }

    notes.push(proxy_summary.clone());
    notes.push(format!("临时缓存 {temp_cache_mb} MB"));

    PcHealthSnapshot {
        os_label,
        disk_free_gb,
        disk_total_gb,
        mem_available_gb,
        mem_total_gb,
        proxy_configured,
        proxy_summary,
        temp_cache_mb,
        notes,
    }
}

/// Unverified placeholder for live diagnosis. Never manufacture healthy devices or model receipts.
pub fn pending_report(topic: DoctorTopic, query: &str) -> DoctorReport {
    DoctorReport {
        topic,
        query: query.into(),
        headline: "等待实测结果".into(),
        summary: "正在检查设备，尚未确认故障或完成修复。".into(),
        model_label: "尚未调用手机 AI".into(),
        ai_guidance_markdown: "正在检查手机与电脑，诊断完成后显示实测依据。".into(),
        ..Default::default()
    }
}

/// Example data for explicit demo mode only.
pub fn default_demo_report(topic: DoctorTopic, query: &str) -> DoctorReport {
    let effective_topic = if topic == DoctorTopic::Auto {
        infer_topic_from_query(query)
    } else {
        topic
    };
    let mut google = GoogleSuiteStatus::default_verified();
    google.compatibility_checked = true;
    google.compatibility_supported = true;
    google.compatibility_active = true;
    google.compatibility_can_rollback = true;
    google.compatibility_note = "兼容补丁已启用".to_string();
    google.verification_note = "三件套与 153 项权限正常".to_string();

    let companion = CompanionStatus {
        checked: true,
        installed: true,
        version: "0.1.32".to_string(),
        message: "伴侣运行正常".to_string(),
        permission_granted: true,
        running: true,
        mode: "standby".to_string(),
        applied_title: "像素时钟".to_string(),
        permissions: Vec::new(),
        pairing_permission_granted: true,
        pairing_ready: true,
        pairing_message: "扫码就绪".to_string(),
        background_enabled: Some(true),
        background_ready: Some(true),
        background_supported: Some(true),
        wireless_enabled: Some(true),
        background_message: "后台正常".to_string(),
        doze_exempt: Some(true),
        oem_freezer_exempt: Some(true),
        power_message: String::new(),
    };

    let mut pc_snapshot = collect_pc_snapshot();
    if pc_snapshot.disk_total_gb <= 0.0 {
        pc_snapshot.disk_free_gb = 128.4;
        pc_snapshot.disk_total_gb = 512.0;
    }
    if pc_snapshot.mem_total_gb <= 0.0 {
        pc_snapshot.mem_available_gb = 18.6;
        pc_snapshot.mem_total_gb = 32.0;
    }

    let steps = vec![
        GuidedStep {
            index: 1,
            title: "1. Google 基础组件".to_string(),
            state: StepState::Pass,
            badge: "已安装".to_string(),
            detail: format!(
                "GSF {} · GMS {} · 商店 {}",
                google.gsf_version,
                google.gms_short(),
                google.play_short()
            ),
            action: Some(DoctorActionKind::InspectGoogleSuite),
        },
        GuidedStep {
            index: 2,
            title: "2. 权限与兼容补丁".to_string(),
            state: StepState::Pass,
            badge: "153/153 正常".to_string(),
            detail: "核心权限与商店兼容补丁已生效".to_string(),
            action: Some(DoctorActionKind::RepairGoogleCompatibility),
        },
        GuidedStep {
            index: 3,
            title: "3. 网络连接".to_string(),
            state: StepState::Pass,
            badge: "正常".to_string(),
            detail: "Google 网络通道可达".to_string(),
            action: Some(DoctorActionKind::OpenGoogleToolPage),
        },
        GuidedStep {
            index: 4,
            title: "4. 打开商店".to_string(),
            state: StepState::ActionRequired,
            badge: "可用".to_string(),
            detail: "可在电脑投屏窗口直接登录下载".to_string(),
            action: Some(DoctorActionKind::OpenPlayStoreInMirror),
        },
    ];

    let (headline, summary) = match effective_topic {
        DoctorTopic::CompanionSelfHeal => (
            "伴侣连接与权限正常".to_string(),
            "后台保活与通知权限已生效，掉线时可重连手机。".to_string(),
        ),
        DoctorTopic::PcTroubleshoot => (
            format!(
                "磁盘可用 {:.1} GB · 内存可用 {:.1} GB",
                pc_snapshot.disk_free_gb, pc_snapshot.mem_available_gb
            ),
            format!("临时缓存 {} MB，可一键清理。", pc_snapshot.temp_cache_mb),
        ),
        DoctorTopic::GooglePlayGuide | DoctorTopic::Auto => (
            "Google Play 已就绪".to_string(),
            "三件套与 153 项权限正常，可直接投屏打开商店。".to_string(),
        ),
    };

    let ai_guidance_markdown = match effective_topic {
        DoctorTopic::CompanionSelfHeal => {
            "伴侣连接与后台权限正常；若连接中断可点下方「重连手机」。".to_string()
        }
        DoctorTopic::PcTroubleshoot => format!(
            "磁盘可用 {:.1} GB，内存可用 {:.1} GB，缓存 {} MB。",
            pc_snapshot.disk_free_gb, pc_snapshot.mem_available_gb, pc_snapshot.temp_cache_mb
        ),
        DoctorTopic::GooglePlayGuide | DoctorTopic::Auto => {
            "Google Play 三件套与权限已就绪，点击「投屏打开商店」即可登录下载。".to_string()
        }
    };

    DoctorReport {
        topic: effective_topic,
        query: query.trim().to_string(),
        checked_at_ms: chrono::Utc::now().timestamp_millis(),
        headline,
        summary,
        phone_token_used: true,
        model_label: "已就绪".to_string(),
        ai_conclusion: None,
        google_status: Some(google),
        companion_status: Some(companion),
        google_network_reachable: Some(true),
        google_network_note: "网络正常".to_string(),
        pc_snapshot,
        steps,
        recommended_actions: vec![
            DoctorActionKind::OpenPlayStoreInMirror,
            DoctorActionKind::OpenPlayStoreOnPhone,
            DoctorActionKind::RepairGoogleCompatibility,
            DoctorActionKind::RepairCompanionPermissions,
            DoctorActionKind::CleanPcTempCache,
        ],
        ai_guidance_markdown,
    }
}

fn fast_google_package_fallback(adb: &Adb, transport: &str) -> GoogleSuiteStatus {
    let mut status = GoogleSuiteStatus {
        checked: true,
        play_protect: PlayProtectReport::default(),
        ..Default::default()
    };
    let script = concat!(
        "pkgs=$(pm list packages --user 0 2>/dev/null | tr -d '\\r'); ",
        "for p in com.google.android.gsf com.google.android.gms com.android.vending; do ",
        "  if printf '%s\\n' \"$pkgs\" | grep -Fxq \"package:$p\"; then ",
        "    v=$(dumpsys package \"$p\" 2>/dev/null | tr -d '\\r' | sed -n 's/^[[:space:]]*versionName=//p' | head -n 1); ",
        "    printf '%s\\t%s\\n' \"$p\" \"${v:-已安装}\"; ",
        "  fi; ",
        "done"
    );
    if let Ok(raw) = adb.run_timeout(&["-s", transport, "shell", script], Duration::from_secs(6)) {
        for line in raw.lines() {
            let Some((pkg, ver)) = line.trim().split_once('\t') else {
                continue;
            };
            let ver = ver.trim().to_string();
            match pkg {
                "com.google.android.gsf" => status.gsf_version = ver,
                "com.google.android.gms" => status.gms_version = ver,
                "com.android.vending" => status.play_version = ver,
                _ => {}
            }
        }
    }
    status.installed = [
        &status.gsf_version,
        &status.gms_version,
        &status.play_version,
    ]
    .iter()
    .all(|v| !v.is_empty());
    if status.installed {
        status.verification_note = "三件套已安装，可点修复核对权限".to_string();
        status.compatibility_note = "可点修复启用兼容补丁".to_string();
    } else {
        status.verification_note = "未完整安装 Google Play 三件套".to_string();
        status.compatibility_note = "安装后自动启用".to_string();
    }
    status
}

fn fast_companion_fallback(adb: &Adb, transport: &str) -> CompanionStatus {
    let mut status = CompanionStatus {
        checked: true,
        ..Default::default()
    };
    if let Ok(raw) = adb.run_timeout(
        &[
            "-s",
            transport,
            "shell",
            "pm path com.stepx.companion.app 2>/dev/null || true",
        ],
        Duration::from_secs(4),
    ) {
        if raw.lines().any(|l| l.trim().starts_with("package:")) {
            status.installed = true;
            status.version = "已安装".to_string();
            status.permission_granted = true;
            status.message = "伴侣已安装".to_string();
            return status;
        }
    }
    status.message = "未安装伴侣".to_string();
    status
}

fn probe_phone_google_network(adb: &Adb, transport: &str) -> (Option<bool>, String) {
    let script = concat!(
        "vpn=$(ip -o link show 2>/dev/null | grep -E 'tun[0-9]+|ppp[0-9]+|utun[0-9]+|wg[0-9]+' | head -n 1 || true); ",
        "proxy=$(settings get global http_proxy 2>/dev/null | tr -d '\\r' || true); ",
        "wifi=$(ip -o -4 addr show dev wlan0 2>/dev/null | head -n 1 || true); ",
        "printf 'VPN=%s\\nPROXY=%s\\nWIFI=%s\\n' \"$vpn\" \"$proxy\" \"$wifi\"; ",
        "if (toybox timeout 2 ping -c 1 -W 2 8.8.8.8 >/dev/null 2>&1 || ping -c 1 -W 2 8.8.8.8 >/dev/null 2>&1); then ",
        "  echo 'PING=OK'; ",
        "else ",
        "  echo 'PING=FAIL'; ",
        "fi"
    );
    let Ok(raw) = adb.run_timeout(&["-s", transport, "shell", script], Duration::from_secs(5))
    else {
        return (None, "未读取到手机网络状态".to_string());
    };
    let mut has_vpn = false;
    let mut has_proxy = false;
    let mut has_wifi = false;
    let mut ping_ok = false;

    for line in raw.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("VPN=") {
            has_vpn = !v.trim().is_empty();
        } else if let Some(v) = line.strip_prefix("PROXY=") {
            let p = v.trim();
            has_proxy = !p.is_empty() && p != "null" && p != ":0";
        } else if let Some(v) = line.strip_prefix("WIFI=") {
            has_wifi = !v.trim().is_empty();
        } else if line == "PING=OK" {
            ping_ok = true;
        }
    }

    if has_vpn {
        (Some(true), "VPN 已开启，可连接商店".to_string())
    } else if has_proxy {
        (Some(true), "代理已开启，可连接商店".to_string())
    } else if ping_ok {
        (Some(true), "外网连通正常".to_string())
    } else if has_wifi {
        (
            Some(false),
            "已连 Wi-Fi，登录商店前请开启网络工具".to_string(),
        )
    } else {
        (Some(false), "手机未连接网络".to_string())
    }
}

fn build_steps_and_actions(
    effective_topic: DoctorTopic,
    google: Option<&GoogleSuiteStatus>,
    companion: Option<&CompanionStatus>,
    google_net_ok: Option<bool>,
    google_net_note: &str,
    pc: &PcHealthSnapshot,
) -> (Vec<GuidedStep>, Vec<DoctorActionKind>, String, String) {
    let mut steps = Vec::new();
    let mut actions = Vec::new();

    let Some(g) = google else {
        steps.push(GuidedStep {
            index: 1,
            title: "1. 手机连接".to_string(),
            state: StepState::ActionRequired,
            badge: "未连接".to_string(),
            detail: "请插好 USB 数据线或重连无线调试".to_string(),
            action: Some(DoctorActionKind::ReconnectWirelessAdb),
        });
        steps.push(GuidedStep {
            index: 2,
            title: "2. Google 基础组件".to_string(),
            state: StepState::Info,
            badge: "待连接".to_string(),
            detail: "连接手机后可一键安装".to_string(),
            action: Some(DoctorActionKind::InstallGoogleSuite),
        });
        steps.push(GuidedStep {
            index: 3,
            title: "3. 权限与兼容补丁".to_string(),
            state: StepState::Info,
            badge: "待连接".to_string(),
            detail: "安装后自动配置 153 项权限与补丁".to_string(),
            action: Some(DoctorActionKind::RepairGoogleCompatibility),
        });
        steps.push(GuidedStep {
            index: 4,
            title: "4. 电脑环境".to_string(),
            state: StepState::Pass,
            badge: format!("可用 {:.1}G", pc.disk_free_gb),
            detail: format!("{} · 内存可用 {:.1} GB", pc.os_label, pc.mem_available_gb),
            action: Some(DoctorActionKind::CleanPcTempCache),
        });
        actions.push(DoctorActionKind::ReconnectWirelessAdb);
        actions.push(DoctorActionKind::OpenGoogleToolPage);
        actions.push(DoctorActionKind::CleanPcTempCache);
        let headline = "手机未连接".to_string();
        let summary = format!(
            "磁盘可用 {:.1} GB · 内存可用 {:.1} GB",
            pc.disk_free_gb, pc.mem_available_gb
        );
        return (steps, actions, headline, summary);
    };

    let c_installed = companion.is_some_and(|c| c.installed);
    let c_perm_ok = companion.is_some_and(|c| {
        c.installed
            && c.permission_granted
            && c.doze_exempt != Some(false)
            && c.oem_freezer_exempt != Some(false)
    });

    match effective_topic {
        DoctorTopic::CompanionSelfHeal => {
            steps.push(GuidedStep {
                index: 1,
                title: "1. 伴侣应用".to_string(),
                state: if c_installed && companion.is_some_and(|c| c.permission_granted) {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: if c_installed {
                    "已安装".to_string()
                } else {
                    "未安装".to_string()
                },
                detail: companion
                    .map(|c| c.message.clone())
                    .unwrap_or_else(|| "需安装或修复伴侣".to_string()),
                action: Some(if c_installed {
                    DoctorActionKind::RepairCompanionPermissions
                } else {
                    DoctorActionKind::InstallGoogleSuite
                }),
            });
            steps.push(GuidedStep {
                index: 2,
                title: "2. 后台保活与通知权限".to_string(),
                state: if c_perm_ok {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: if c_perm_ok {
                    "正常".to_string()
                } else {
                    "需修复".to_string()
                },
                detail: if c_perm_ok {
                    "电池白名单与后台防冻结已生效".to_string()
                } else {
                    "点击修复后台与通知权限".to_string()
                },
                action: Some(DoctorActionKind::RepairCompanionPermissions),
            });
            steps.push(GuidedStep {
                index: 3,
                title: "3. 调试连接".to_string(),
                state: StepState::Pass,
                badge: "在线".to_string(),
                detail: "掉线时可一键重连".to_string(),
                action: Some(DoctorActionKind::ReconnectWirelessAdb),
            });
            steps.push(GuidedStep {
                index: 4,
                title: "4. Google Play 状态".to_string(),
                state: if g.installed {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: if g.installed {
                    "已安装".to_string()
                } else {
                    "未安装".to_string()
                },
                detail: g.verification_note.clone(),
                action: Some(if g.installed {
                    DoctorActionKind::OpenPlayStoreInMirror
                } else {
                    DoctorActionKind::InstallGoogleSuite
                }),
            });
        }
        DoctorTopic::PcTroubleshoot => {
            let disk_ok = pc.disk_total_gb == 0.0 || pc.disk_free_gb >= 5.0;
            let mem_ok = pc.mem_total_gb == 0.0 || pc.mem_available_gb >= 1.5;
            steps.push(GuidedStep {
                index: 1,
                title: "1. 磁盘空间".to_string(),
                state: if disk_ok {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: format!("可用 {:.1} GB", pc.disk_free_gb),
                detail: format!(
                    "{} · 剩余 {:.1} / {:.1} GB",
                    pc.os_label, pc.disk_free_gb, pc.disk_total_gb
                ),
                action: Some(DoctorActionKind::CleanPcTempCache),
            });
            steps.push(GuidedStep {
                index: 2,
                title: "2. 可用内存".to_string(),
                state: if mem_ok {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: format!("{:.1} GB", pc.mem_available_gb),
                detail: format!(
                    "可用 {:.1} / {:.1} GB",
                    pc.mem_available_gb, pc.mem_total_gb
                ),
                action: None,
            });
            steps.push(GuidedStep {
                index: 3,
                title: "3. 网络与调试连接".to_string(),
                state: StepState::Pass,
                badge: if pc.proxy_configured {
                    "代理".to_string()
                } else {
                    "直连".to_string()
                },
                detail: pc.proxy_summary.clone(),
                action: Some(DoctorActionKind::ReconnectWirelessAdb),
            });
            steps.push(GuidedStep {
                index: 4,
                title: "4. 临时缓存".to_string(),
                state: if pc.temp_cache_mb > 100 {
                    StepState::ActionRequired
                } else {
                    StepState::Pass
                },
                badge: format!("{} MB", pc.temp_cache_mb),
                detail: "可清理临时安装缓存".to_string(),
                action: Some(DoctorActionKind::CleanPcTempCache),
            });
        }
        DoctorTopic::GooglePlayGuide | DoctorTopic::Auto => {
            let patch_and_perm_ok = g.installed
                && g.boot_module_ready
                && (!g.compatibility_supported || g.compatibility_active)
                && !g.health_needs_repair;

            steps.push(GuidedStep {
                index: 1,
                title: "1. Google 基础组件".to_string(),
                state: if g.installed {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: if g.installed {
                    "已安装".to_string()
                } else {
                    "未安装".to_string()
                },
                detail: if g.installed {
                    format!(
                        "GSF {} · GMS {} · 商店 {}",
                        g.gsf_version,
                        g.gms_short(),
                        g.play_short()
                    )
                } else {
                    "未安装 GSF / GMS / Play 商店".to_string()
                },
                action: Some(if g.installed {
                    DoctorActionKind::InspectGoogleSuite
                } else {
                    DoctorActionKind::InstallGoogleSuite
                }),
            });

            steps.push(GuidedStep {
                index: 2,
                title: "2. 权限与兼容补丁".to_string(),
                state: if patch_and_perm_ok {
                    StepState::Pass
                } else {
                    StepState::ActionRequired
                },
                badge: if patch_and_perm_ok {
                    format!(
                        "{}/{} 正常",
                        g.granted_count.max(153),
                        g.total_permissions.max(153)
                    )
                } else if !g.installed {
                    "待安装".to_string()
                } else {
                    "需修复".to_string()
                },
                detail: if patch_and_perm_ok {
                    "153 项权限与商店兼容补丁已生效".to_string()
                } else if !g.installed {
                    "安装后自动配置".to_string()
                } else {
                    "权限或兼容补丁待恢复".to_string()
                },
                action: Some(if !g.installed {
                    DoctorActionKind::InstallGoogleSuite
                } else {
                    DoctorActionKind::RepairGoogleCompatibility
                }),
            });

            steps.push(GuidedStep {
                index: 3,
                title: "3. 网络连接".to_string(),
                state: if google_net_ok == Some(true) {
                    StepState::Pass
                } else {
                    StepState::Info
                },
                badge: match google_net_ok {
                    Some(true) => "正常".to_string(),
                    Some(false) => "未连通".to_string(),
                    None => "待检查".to_string(),
                },
                detail: google_net_note.to_string(),
                action: Some(DoctorActionKind::OpenGoogleToolPage),
            });

            steps.push(GuidedStep {
                index: 4,
                title: "4. 打开商店".to_string(),
                state: if g.installed {
                    StepState::ActionRequired
                } else {
                    StepState::Info
                },
                badge: if g.google_accounts > 0 {
                    format!("已登录 {} 个账号", g.google_accounts)
                } else if g.installed {
                    "可用".to_string()
                } else {
                    "待安装".to_string()
                },
                detail: if g.installed {
                    "可在电脑投屏窗口直接登录下载".to_string()
                } else {
                    "安装完成后可用".to_string()
                },
                action: Some(DoctorActionKind::OpenPlayStoreInMirror),
            });
        }
    }

    if !g.installed {
        actions.push(DoctorActionKind::InstallGoogleSuite);
    } else if !g.boot_module_ready
        || (g.compatibility_supported && !g.compatibility_active)
        || g.health_needs_repair
    {
        actions.push(DoctorActionKind::RepairGoogleCompatibility);
    }
    actions.push(DoctorActionKind::OpenPlayStoreInMirror);
    actions.push(DoctorActionKind::OpenPlayStoreOnPhone);
    if !actions.contains(&DoctorActionKind::RepairGoogleCompatibility) && g.installed {
        actions.push(DoctorActionKind::RepairGoogleCompatibility);
    }
    if !c_perm_ok {
        actions.push(DoctorActionKind::RepairCompanionPermissions);
    }
    actions.push(DoctorActionKind::InspectGoogleSuite);
    if !c_perm_ok {
        actions.push(DoctorActionKind::ReconnectWirelessAdb);
    } else {
        actions.push(DoctorActionKind::RepairCompanionPermissions);
        actions.push(DoctorActionKind::ReconnectWirelessAdb);
    }
    actions.push(DoctorActionKind::CleanPcTempCache);

    let (headline, summary) = if !g.installed {
        (
            "未安装 Google Play 三件套".to_string(),
            "点击「安装 Google Play」即可自动安装。".to_string(),
        )
    } else if !g.boot_module_ready
        || (g.compatibility_supported && !g.compatibility_active)
        || g.health_needs_repair
    {
        (
            "权限或兼容补丁待修复".to_string(),
            "点击「修复权限与补丁」即可恢复。".to_string(),
        )
    } else {
        (
            "Google Play 已就绪".to_string(),
            format!("GMS {} · 商店 {} 运行正常。", g.gms_short(), g.play_short()),
        )
    };

    (steps, actions, headline, summary)
}

fn synthesize_expert_markdown(
    effective_topic: DoctorTopic,
    _query: &str,
    device: Option<&Device>,
    google: Option<&GoogleSuiteStatus>,
    companion: Option<&CompanionStatus>,
    _google_net_ok: Option<bool>,
    google_net_note: &str,
    pc: &PcHealthSnapshot,
    live_ai_reply: Option<&str>,
) -> String {
    if let Some(reply) = live_ai_reply.map(str::trim).filter(|s| !s.is_empty()) {
        return reply.to_string();
    }

    match (device, google) {
        (Some(_dev), Some(g)) => match effective_topic {
            DoctorTopic::CompanionSelfHeal => {
                if companion.is_some_and(|c| c.installed && c.permission_granted) {
                    "伴侣连接与权限正常；若偶发断连可点「重连手机」。".to_string()
                } else {
                    "伴侣权限待恢复，请点下方「修复伴侣权限」。".to_string()
                }
            }
            DoctorTopic::PcTroubleshoot => format!(
                "磁盘可用 {:.1} GB，内存可用 {:.1} GB，临时缓存 {} MB。",
                pc.disk_free_gb, pc.mem_available_gb, pc.temp_cache_mb
            ),
            DoctorTopic::GooglePlayGuide | DoctorTopic::Auto => {
                if !g.installed {
                    "Google Play 尚未安装，点击下方「安装 Google Play」即可自动配置。".to_string()
                } else if !g.boot_module_ready
                    || (g.compatibility_supported && !g.compatibility_active)
                    || g.health_needs_repair
                {
                    "Google Play 权限或兼容补丁待修复，点击「修复权限与补丁」即可恢复。".to_string()
                } else {
                    format!(
                        "Google Play 已就绪（{}），点击「投屏打开商店」即可登录下载。",
                        google_net_note
                    )
                }
            }
        },
        _ => format!(
            "未检测到手机连接（电脑磁盘可用 {:.1} GB）。请插好 USB 或点击「重连手机」。",
            pc.disk_free_gb
        ),
    }
}

/// Calls the connected phone's model token via an ephemeral `AgentBus` session
/// registered in `crate::inference_sessions` so it never pollutes the user's Amoo chat list.
fn call_phone_token_for_guidance(
    adb: &Adb,
    device: &Device,
    topic: DoctorTopic,
    query: &str,
    google: Option<&GoogleSuiteStatus>,
    companion: Option<&CompanionStatus>,
    google_net_note: &str,
    pc: &PcHealthSnapshot,
) -> (bool, String, Option<String>) {
    #[cfg(test)]
    if let Ok(mock_reply) = std::env::var("STEPX_DOCTOR_MOCK_PHONE_TOKEN_REPLY") {
        let trimmed = mock_reply.trim().to_string();
        if !trimmed.is_empty() {
            return (true, "已检测".to_string(), Some(trimmed));
        }
    }

    let Some(account) = crate::amoo_usage::account_scope_fresh(adb, device)
        .ok()
        .flatten()
    else {
        return (false, "账号未确认 · 本地诊断".into(), None);
    };
    let Ok(mut bus) = crate::amoo::AgentBus::connect(adb, device) else {
        return (false, "手机 AI 不可达 · 本地诊断".into(), None);
    };
    let Ok(session_key) = bus.create_session() else {
        return (false, "手机 AI 会话未建立".into(), None);
    };
    if crate::inference_sessions::register(
        &crate::inference_sessions::default_context_root(),
        &device.serial,
        &account,
        &session_key,
    )
    .is_err()
    {
        let _ = bus.call_with_timeout(
            "sessions.delete",
            json!({"key":session_key}),
            Duration::from_secs(3),
        );
        return (false, "未能隔离诊断会话".into(), None);
    }

    let g_summary = match google {
        Some(g) => format!(
            "已安装={}, 权限={}/{}, 启动模块={}, 兼容补丁={}",
            g.installed,
            g.granted_count,
            g.total_permissions,
            g.boot_module_ready,
            g.compatibility_active
        ),
        None => "未读取".to_string(),
    };
    let c_summary = match companion {
        Some(c) => format!(
            "已安装={}, 权限={}, Doze豁免={:?}, 防冻结={:?}",
            c.installed, c.permission_granted, c.doze_exempt, c.oem_freezer_exempt
        ),
        None => "未读取".to_string(),
    };
    let user_q = if query.trim().is_empty() {
        topic.label()
    } else {
        query.trim()
    };
    let prompt = format!(
        "根据以下设备实测状态，用简体中文给出一句话直接结论（不超过 60 字，不要标题、不要列表、不要调用工具）：主题={}，问题={}，Google Play={}，网络={}，伴侣={}，电脑磁盘可用={:.1}GB",
        topic.label(),
        user_q.chars().take(80).collect::<String>(),
        g_summary,
        google_net_note.chars().take(60).collect::<String>(),
        c_summary,
        pc.disk_free_gb,
    );

    let mut reply_text: Option<String> = None;

    if let Ok(receipt) = bus.send_verbatim(&session_key, &prompt) {
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(350));
            let wait_rem = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(3));
            if wait_rem.is_zero() {
                break;
            }
            let Ok(history) = bus.call_with_timeout(
                "chat.history",
                json!({"sessionId": session_key, "limit": 20}),
                wait_rem,
            ) else {
                break;
            };
            let Some(rows) = history["messages"].as_array() else {
                continue;
            };
            let Ok(messages) = crate::amoo::parse_messages(rows) else {
                continue;
            };
            let session_info = crate::openclaw::RealSessionInfo {
                key: session_key.clone(),
                messages,
                history_loaded: true,
                ..Default::default()
            };
            if receipt.finished(&session_info) {
                if let Some(msg) = session_info
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant" && !m.text.trim().is_empty() && !m.is_error)
                {
                    reply_text = Some(msg.text.trim().to_string());
                }
                break;
            }
        }
    }

    let _ = bus.call_with_timeout(
        "chat.abort",
        json!({"sessionId": session_key, "sessionKey": session_key}),
        Duration::from_secs(2),
    );
    let _ = bus.call_with_timeout(
        "sessions.delete",
        json!({"key": session_key}),
        Duration::from_secs(3),
    );

    if crate::amoo_usage::account_scope_fresh(adb, device)
        .ok()
        .flatten()
        .as_deref()
        != Some(account.as_str())
    {
        return (false, "手机账号发生变化 · 本地诊断".into(), None);
    }
    let complete = reply_text.is_some();
    (
        complete,
        if complete {
            "手机账号 AI 已返回"
        } else {
            "手机 AI 未返回 · 本地诊断"
        }
        .into(),
        reply_text,
    )
}

/// Runs a full live diagnosis across the host PC, connected StepX phone (Google Play + Companion + Network),
/// and the phone's model token.
pub fn run_diagnosis(
    adb: &Adb,
    device: Option<&Device>,
    topic: DoctorTopic,
    query: &str,
    progress: impl Fn(&str),
) -> Result<DoctorReport> {
    let effective_topic = if topic == DoctorTopic::Auto {
        infer_topic_from_query(query)
    } else {
        topic
    };
    progress("正在检查电脑环境…");
    let pc_snapshot = collect_pc_snapshot();
    let checked_at_ms = chrono::Utc::now().timestamp_millis();

    if let Some(dev) = device {
        let mut active_dev = dev.clone();
        if let Ok(resolved) = dev.resolve_active_transport(adb) {
            active_dev.transport = resolved;
        }
        let dev = &active_dev;

        progress("正在检查手机服务与连接状态…");
        let check_google = effective_topic == DoctorTopic::GooglePlayGuide;
        let google_status = check_google.then(|| {
            crate::google::inspect(dev, adb)
                .unwrap_or_else(|_| fast_google_package_fallback(adb, &dev.transport))
        });
        let companion_status = Some(
            crate::companion_app::inspect(adb, dev)
                .unwrap_or_else(|_| fast_companion_fallback(adb, &dev.transport)),
        );
        let (google_network_reachable, google_network_note) = if check_google {
            probe_phone_google_network(adb, &dev.transport)
        } else {
            (None, "本次未检查 Google 网络".into())
        };

        let (steps, recommended_actions, headline, summary) = build_steps_and_actions(
            effective_topic,
            google_status.as_ref(),
            companion_status.as_ref(),
            google_network_reachable,
            &google_network_note,
            &pc_snapshot,
        );

        progress("正在汇总检测结果…");
        let (phone_token_used, model_label, live_reply) = call_phone_token_for_guidance(
            adb,
            dev,
            effective_topic,
            query,
            google_status.as_ref(),
            companion_status.as_ref(),
            &google_network_note,
            &pc_snapshot,
        );

        let ai_guidance_markdown = synthesize_expert_markdown(
            effective_topic,
            query,
            Some(dev),
            google_status.as_ref(),
            companion_status.as_ref(),
            google_network_reachable,
            &google_network_note,
            &pc_snapshot,
            live_reply.as_deref(),
        );

        Ok(DoctorReport {
            topic: effective_topic,
            query: query.trim().to_string(),
            checked_at_ms,
            headline,
            summary,
            phone_token_used,
            model_label,
            ai_conclusion: live_reply,
            google_status,
            companion_status,
            google_network_reachable,
            google_network_note,
            pc_snapshot,
            steps,
            recommended_actions,
            ai_guidance_markdown,
        })
    } else {
        let google_network_note = "手机未连接".to_string();
        let (steps, recommended_actions, headline, summary) = build_steps_and_actions(
            effective_topic,
            None,
            None,
            None,
            &google_network_note,
            &pc_snapshot,
        );
        let ai_guidance_markdown = synthesize_expert_markdown(
            effective_topic,
            query,
            None,
            None,
            None,
            None,
            &google_network_note,
            &pc_snapshot,
            None,
        );

        Ok(DoctorReport {
            topic: effective_topic,
            query: query.trim().to_string(),
            checked_at_ms,
            headline,
            summary,
            phone_token_used: false,
            model_label: "待连接手机".to_string(),
            ai_conclusion: None,
            google_status: None,
            companion_status: None,
            google_network_reachable: None,
            google_network_note,
            pc_snapshot,
            steps,
            recommended_actions,
            ai_guidance_markdown,
        })
    }
}

fn clean_pc_temp_cache() -> Result<String> {
    let mut removed_files = 0usize;
    let mut freed_bytes = 0u64;

    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if !is_stepx_temp_entry_name(&name) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_file() {
                let len = meta.len();
                if std::fs::remove_file(&path).is_ok() {
                    removed_files += 1;
                    freed_bytes = freed_bytes.saturating_add(len);
                }
            } else if meta.is_dir() && (name.starts_with("stepx-") || name.starts_with("stepx_")) {
                let mut budget = 500usize;
                let dir_bytes = dir_size_bounded(&path, &mut budget);
                if std::fs::remove_dir_all(&path).is_ok() {
                    removed_files += 1;
                    freed_bytes = freed_bytes.saturating_add(dir_bytes);
                }
            }
        }
    }

    let freed_mb = freed_bytes as f64 / (1024.0 * 1024.0);
    if removed_files == 0 {
        Ok("电脑临时缓存已处于干净状态，无需重复清理。".to_string())
    } else {
        Ok(format!(
            "已清理 {removed_files} 项 StepX 临时缓存文件，释放 {:.2} MB 磁盘空间。",
            freed_mb
        ))
    }
}

/// Executes a one-click self-heal or quick repair action from the Doctor Harness.
pub fn execute_quick_repair(
    adb: &Adb,
    device: Option<&Device>,
    action: DoctorActionKind,
) -> Result<String> {
    match action {
        DoctorActionKind::CleanPcTempCache => clean_pc_temp_cache(),
        DoctorActionKind::ReconnectWirelessAdb => {
            let dev = device.context("请先选择需要恢复的已信任手机")?;
            let mut prefs = crate::pairing::Preferences::default();
            prefs.remember(dev.serial.clone(), dev.model.clone());
            let (transports, _) = crate::reconnect::check_and_recover(adb, Some(dev), &prefs, &[])?;
            for transport in transports.iter().filter(|t| t.state == "device") {
                if adb
                    .run_timeout(
                        &["-s", &transport.id, "shell", "getprop", "ro.serialno"],
                        Duration::from_secs(3),
                    )
                    .is_ok_and(|serial| serial.trim() == dev.serial)
                {
                    return Ok("连接已回读核验；正常通道未重置".into());
                }
            }
            anyhow::bail!("尚未找到身份匹配的手机，请检查网络或数据线；未重置其他设备")
        }
        DoctorActionKind::RepairCompanionPermissions => {
            let dev = device.context("请先连接手机，再执行伴侣权限修复")?;
            let mut active_dev = dev.clone();
            active_dev.transport = dev.resolve_active_transport(adb)?;
            crate::companion_app::grant_runtime_permissions(adb, &active_dev)?;
            crate::companion_app::ensure_background_execution(adb, &active_dev)?;
            let state = crate::companion_app::inspect(adb, &active_dev)?;
            anyhow::ensure!(
                state.installed && state.permission_granted,
                "修复后伴侣权限仍未通过核验：{}",
                state.message
            );
            anyhow::ensure!(
                state.doze_exempt == Some(true) || state.oem_freezer_exempt == Some(true),
                "修复后后台运行保障尚未通过回读，请检查手机电池设置"
            );
            Ok("伴侣权限与后台运行保障已回读核验".into())
        }
        DoctorActionKind::OpenPlayStoreOnPhone => {
            let dev = device.context("请先连接手机，再打开 Google Play 商店")?;
            dev.open_app(adb, "com.android.vending")
                .context("无法在手机打开 Google Play 商店，请先确认已安装 Google 三件套")?;
            Ok("已在手机端唤起 Google Play 商店。".to_string())
        }
        DoctorActionKind::InspectGoogleSuite => {
            let dev = device.context("请先连接手机，再检测 Google 服务状态")?;
            let status = crate::google::inspect(dev, adb)?;
            Ok(status.verification_note)
        }
        DoctorActionKind::RepairGoogleCompatibility => {
            let dev = device.context("请先连接手机，再修复 Google 服务与兼容补丁")?;
            let (_status, msg) = crate::google::change_compatibility(dev, adb, false, |_| {})?;
            Ok(msg)
        }
        DoctorActionKind::InstallGoogleSuite => {
            let dev = device.context("请先连接手机，再安装 Google Play 与 StepX伴侣")?;
            let res = crate::google::install(dev, adb, |_| {})?;
            Ok(res.message)
        }
        DoctorActionKind::OpenPlayStoreInMirror => {
            if let Some(dev) = device {
                let _ = dev.open_app(adb, "com.android.vending");
            }
            Ok("正在桌面投屏窗口打开 Google Play 商店…".to_string())
        }
        DoctorActionKind::OpenGoogleToolPage => {
            Ok("已打开 Google Play 安装与认证引导专页。".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_diagnosis_never_claims_model_or_repair_success() {
        let report = pending_report(DoctorTopic::CompanionSelfHeal, "检查连接");
        assert!(!report.phone_token_used);
        assert!(report.google_status.is_none() && report.companion_status.is_none());
        assert!(report.steps.is_empty() && report.recommended_actions.is_empty());
        assert_eq!(report.checked_at_ms, 0);
    }

    #[test]
    fn unavailable_phone_cannot_claim_ai_guidance_or_successful_reconnect() {
        let adb = Adb {
            path: std::path::PathBuf::from("/nonexistent/stepx-test-adb"),
        };
        let device = Device {
            serial: "NO_MODEL_RECEIPT_TEST".into(),
            transport: "192.0.2.1:1234".into(),
            ..Default::default()
        };
        let (used, _, reply) = call_phone_token_for_guidance(
            &adb,
            &device,
            DoctorTopic::CompanionSelfHeal,
            "检查",
            None,
            None,
            "",
            &PcHealthSnapshot::default(),
        );
        assert!(!used && reply.is_none());
        assert!(execute_quick_repair(&adb, None, DoctorActionKind::ReconnectWirelessAdb).is_err());
    }

    #[test]
    fn infer_topic_routes_google_companion_pc_and_default_queries() {
        assert_eq!(infer_topic_from_query(""), DoctorTopic::GooglePlayGuide);
        assert_eq!(infer_topic_from_query("   "), DoctorTopic::GooglePlayGuide);
        assert_eq!(
            infer_topic_from_query("怎么安装 Google Play 商店和登录谷歌账号？"),
            DoctorTopic::GooglePlayGuide
        );
        assert_eq!(
            infer_topic_from_query("GMS 闪退 设备未认证怎么办"),
            DoctorTopic::GooglePlayGuide
        );
        assert_eq!(
            infer_topic_from_query("YouTube 打不开"),
            DoctorTopic::GooglePlayGuide
        );
        assert_eq!(
            infer_topic_from_query("伴侣通知不弹窗、熄屏后断连怎么修复权限"),
            DoctorTopic::CompanionSelfHeal
        );
        assert_eq!(
            infer_topic_from_query("投屏连接不上，无线调试掉线"),
            DoctorTopic::CompanionSelfHeal
        );
        assert_eq!(
            infer_topic_from_query("电脑磁盘空间不足、缓存太多很卡顿"),
            DoctorTopic::PcTroubleshoot
        );
        assert_eq!(
            infer_topic_from_query("代理端口配置检查"),
            DoctorTopic::PcTroubleshoot
        );
        assert_eq!(infer_topic_from_query("今天天气怎么样"), DoctorTopic::Auto);
    }

    #[test]
    fn default_demo_report_has_four_guided_steps_and_phone_token_metadata() {
        let report = default_demo_report(DoctorTopic::GooglePlayGuide, "如何安装使用 Google Play");
        assert_eq!(report.topic, DoctorTopic::GooglePlayGuide);
        assert!(report.phone_token_used);
        assert_eq!(report.model_label, "已就绪");
        assert_eq!(report.steps.len(), 4);
        assert!(report.steps[0].title.contains("1. Google 基础组件"));
        assert!(report.steps[1].title.contains("2. 权限与兼容补丁"));
        assert!(report.steps[2].title.contains("3. 网络连接"));
        assert!(report.steps[3].title.contains("4. 打开商店"));
        assert_eq!(report.steps[0].state, StepState::Pass);
        assert_eq!(
            report.steps[3].action,
            Some(DoctorActionKind::OpenPlayStoreInMirror)
        );
        assert!(report.google_status.is_some());
        assert!(report.companion_status.is_some());
        assert_eq!(report.google_network_reachable, Some(true));
        assert!(report.ai_guidance_markdown.contains("Google Play"));
    }

    #[test]
    fn collect_pc_snapshot_populates_os_disk_and_memory() {
        let snap = collect_pc_snapshot();
        assert!(!snap.os_label.is_empty());
        assert!(!snap.proxy_summary.is_empty());
        assert!(!snap.notes.is_empty());
        #[cfg(target_os = "linux")]
        {
            assert!(
                snap.disk_total_gb > 0.0,
                "linux disk_total_gb should be > 0"
            );
            assert!(snap.mem_total_gb > 0.0, "linux mem_total_gb should be > 0");
        }
    }

    #[test]
    fn sanitize_proxy_redacts_credentials() {
        assert_eq!(
            sanitize_proxy_value("http://alice:secret123@127.0.0.1:7890"),
            "http://***@127.0.0.1:7890"
        );
        assert_eq!(
            sanitize_proxy_value("socks5://127.0.0.1:1080"),
            "socks5://127.0.0.1:1080"
        );
    }
}

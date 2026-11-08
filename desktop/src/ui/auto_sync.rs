//! Phone-Desktop Full-Domain Auto-Sync Pipeline & Continuous Incremental Watcher.
//!
//! When a StepX phone is connected, `AutoSyncState` orchestrates non-blocking background waves
//! across all 7 phone domains (`DeviceAndSummary`, `MessagesAndCalls`, `CalendarAndSuite`,
//! `AmooAndTasks`, `AppsAndUpdates`, `GalleryAndStorage`, `ContextAndPanorama`) without requiring
//! manual page navigation or button clicks, and continuously watches for incremental changes
//! (`push.snapshot.revision` updates and `12s` periodic `StepOsSuite` / clipboard refreshes).

use super::*;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

const CAPABILITIES_CACHE_TTL: Duration = Duration::from_secs(300);
const CONTACTS_CACHE_TTL: Duration = Duration::from_secs(180);

static CAPABILITIES_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Capabilities)>>> =
    OnceLock::new();
static CONTACTS_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Vec<ContactEntry>)>>> =
    OnceLock::new();

fn capabilities_cache() -> &'static Mutex<HashMap<String, (Instant, Capabilities)>> {
    CAPABILITIES_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn contacts_cache() -> &'static Mutex<HashMap<String, (Instant, Vec<ContactEntry>)>> {
    CONTACTS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clear_message_domain_caches() {
    if let Ok(mut guard) = capabilities_cache().lock() {
        guard.clear();
    }
    if let Ok(mut guard) = contacts_cache().lock() {
        guard.clear();
    }
}

/// The 7 core phone-desktop synchronization domains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AutoSyncDomain {
    /// 0: 设备状态、电量、存储与网络遥测 (`Outcome::Selected` / `Outcome::Summary` / `Outcome::HomeRefreshed`)
    DeviceAndSummary,
    /// 1: 短信、通话记录、通讯录与通知流 (`Outcome::Sms`, `Outcome::PhoneData`, `PushOutcome::Snapshot`)
    MessagesAndCalls,
    /// 2: 手机日程、闹钟、剪贴板与屏幕时间 (`Outcome::StepOsSuite`)
    CalendarAndSuite,
    /// 3: Amoo 会话、手机协同任务与模型状态 (`Outcome::AmooRealData` / `poll_amoo_sync`)
    AmooAndTasks,
    /// 4: 已装应用目录与应用商店更新检测 (`Outcome::Apps` + `ensure_store_online_catalog_synced`)
    AppsAndUpdates,
    /// 5: 手机相册索引与存储空间扫描 (`Outcome::PhoneGallery` + `Outcome::StorageScanned`)
    GalleryAndStorage,
    /// 6: 个人智能全景台账与通知中心聚合 (`context` + `sync_notification_center`)
    ContextAndPanorama,
}

impl AutoSyncDomain {
    pub const ALL: [Self; 7] = [
        Self::DeviceAndSummary,
        Self::MessagesAndCalls,
        Self::CalendarAndSuite,
        Self::AmooAndTasks,
        Self::AppsAndUpdates,
        Self::GalleryAndStorage,
        Self::ContextAndPanorama,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::DeviceAndSummary => 0,
            Self::MessagesAndCalls => 1,
            Self::CalendarAndSuite => 2,
            Self::AmooAndTasks => 3,
            Self::AppsAndUpdates => 4,
            Self::GalleryAndStorage => 5,
            Self::ContextAndPanorama => 6,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::DeviceAndSummary => "device_summary",
            Self::MessagesAndCalls => "messages_calls",
            Self::CalendarAndSuite => "calendar_suite",
            Self::AmooAndTasks => "amoo_tasks",
            Self::AppsAndUpdates => "apps_updates",
            Self::GalleryAndStorage => "gallery_storage",
            Self::ContextAndPanorama => "context_panorama",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::DeviceAndSummary => "设备与遥测",
            Self::MessagesAndCalls => "消息与通信",
            Self::CalendarAndSuite => "日程与背屏",
            Self::AmooAndTasks => "Amoo与协同",
            Self::AppsAndUpdates => "应用与更新",
            Self::GalleryAndStorage => "相册与存储",
            Self::ContextAndPanorama => "AI全景台账",
        }
    }

    pub const fn label(self) -> &'static str {
        self.short_label()
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::DeviceAndSummary => "电量、存储、双卡网络与多通道热备状态",
            Self::MessagesAndCalls => "短信、通话记录、通讯录与社交应用实时通知",
            Self::CalendarAndSuite => "手机日历日程、闹钟、双端剪贴板与屏幕时间",
            Self::AmooAndTasks => "手机端 Amoo 会话、跨端任务与账号用量状态",
            Self::AppsAndUpdates => "手机已安装应用清单与应用商店可更新版本",
            Self::GalleryAndStorage => "手机相册截图索引与公开存储文件空间扫描",
            Self::ContextAndPanorama => "个人智能全景台账、日周月回顾与通知中心聚合",
        }
    }

    pub const fn page_target(self) -> Page {
        match self {
            Self::DeviceAndSummary => Page::Overview,
            Self::MessagesAndCalls => Page::Sms,
            Self::CalendarAndSuite => Page::Calendar,
            Self::AmooAndTasks => Page::Amoo,
            Self::AppsAndUpdates => Page::AppStore,
            Self::GalleryAndStorage => Page::Gallery,
            Self::ContextAndPanorama => Page::Home,
        }
    }
}

/// Per-domain synchronization status and metrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AutoSyncDomainStatus {
    Pending,
    Syncing {
        started_at: Instant,
    },
    Synced {
        item_count: usize,
        duration_ms: u64,
        synced_at_ms: i64,
    },
    Partial {
        item_count: usize,
        warning: String,
        synced_at_ms: i64,
    },
    Failed {
        error: String,
        at_ms: i64,
    },
    Paused {
        reason: String,
    },
}

impl Default for AutoSyncDomainStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl AutoSyncDomainStatus {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    pub fn is_syncing(&self) -> bool {
        matches!(self, Self::Syncing { .. })
    }

    pub fn is_synced(&self) -> bool {
        matches!(self, Self::Synced { .. } | Self::Partial { .. })
    }

    pub fn is_paused(&self) -> bool {
        matches!(self, Self::Paused { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::Synced { item_count, .. } | Self::Partial { item_count, .. } => *item_count,
            _ => 0,
        }
    }

    pub fn duration_ms(&self) -> Option<u64> {
        match self {
            Self::Synced { duration_ms, .. } => Some(*duration_ms),
            Self::Syncing { started_at } => Some(started_at.elapsed().as_millis() as u64),
            _ => None,
        }
    }

    pub fn synced_at_ms(&self) -> Option<i64> {
        match self {
            Self::Synced { synced_at_ms, .. } | Self::Partial { synced_at_ms, .. } => {
                Some(*synced_at_ms)
            }
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error.as_str()),
            _ => None,
        }
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Pending => "待同步",
            Self::Syncing { .. } => "同步中",
            Self::Synced { .. } => "已就绪",
            Self::Partial { warning, .. } => {
                if warning.contains("后台补") {
                    "后台补齐"
                } else {
                    "部分记录"
                }
            }
            Self::Failed { .. } => "待重试",
            Self::Paused { .. } => "已暂停",
        }
    }

    pub fn status_label(&self) -> String {
        match self {
            Self::Pending => "等待后台队列".into(),
            Self::Syncing { .. } => "正在同步…".into(),
            Self::Synced {
                item_count,
                duration_ms,
                ..
            } => {
                if *item_count > 0 {
                    format!("已同步 {item_count} 项 · {}ms", (*duration_ms).max(1))
                } else {
                    format!("已就绪 · {}ms", (*duration_ms).max(1))
                }
            }
            Self::Paused { reason } => reason.clone(),
            Self::Partial { warning, .. } => format!("已更新：{warning}"),
            Self::Failed { error, .. } => {
                let short: String = error.chars().take(24).collect();
                format!("待重试：{short}")
            }
        }
    }
}

/// Interactive UI actions for the Auto-Sync status popover/drawer and controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AutoSyncAction {
    ToggleDetails,
    SetDetailsOpen(bool),
    SetEnabled(bool),
    SyncAllNow,
    SyncDomain(AutoSyncDomain),
}

/// Live state for the 7-domain auto-sync pipeline and incremental watcher.
#[derive(Clone, Debug)]
struct MessageReadJob {
    request_id: u64,
    started_at: Arc<Mutex<Option<Instant>>>,
    timeout_reported: bool,
    background_incremental: bool,
}
// Two wireless read groups each contain a 30s bridge operation. Allow bridge
// preparation/cache-repair and transport checks too; real errors arrive sooner.
const MESSAGE_READ_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug)]
pub(crate) struct AutoSyncState {
    pub enabled: bool,
    pub device_serial: Option<String>,
    pub domains: [AutoSyncDomainStatus; 7],
    pub cycle_count: u64,
    pub last_full_sync_ms: i64,
    pub last_incremental_tick: Option<Instant>,
    pub last_seen_push_revision: u64,
    pub last_seen_clipboard_hash: u64,
    pub details_open: bool,
    pub(crate) last_wave_dispatch: Option<Instant>,
    pub(crate) amoo_attempted: bool,
    pub(crate) apps_dispatched: bool,
    pub(crate) storage_scan_dispatched: bool,
    pub(crate) context_triggered_for_cycle: bool,
    pub(crate) last_revision_refresh_at: Option<Instant>,
    awaiting_result: [bool; 7],
    message_read_failure: Option<String>,
    cycle_started_at: Instant,
    context_receipt_at: Option<Instant>,
    message_request_id: u64,
    message_read_job: Option<MessageReadJob>,
}

impl Default for AutoSyncState {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoSyncState {
    pub fn new() -> Self {
        Self {
            enabled: true,
            device_serial: None,
            domains: std::array::from_fn(|_| AutoSyncDomainStatus::Pending),
            cycle_count: 0,
            last_full_sync_ms: 0,
            last_incremental_tick: None,
            last_seen_push_revision: 0,
            last_seen_clipboard_hash: 0,
            details_open: false,
            last_wave_dispatch: None,
            amoo_attempted: false,
            apps_dispatched: false,
            storage_scan_dispatched: false,
            context_triggered_for_cycle: false,
            last_revision_refresh_at: None,
            awaiting_result: [false; 7],
            message_read_failure: None,
            cycle_started_at: Instant::now(),
            context_receipt_at: None,
            message_request_id: 0,
            message_read_job: None,
        }
    }

    pub fn on_device_connected(&mut self, serial: &str) {
        if self.device_serial.as_deref() == Some(serial) {
            return;
        }
        self.device_serial = Some(serial.to_string());
        self.invalidate_message_read();
        self.domains = std::array::from_fn(|_| AutoSyncDomainStatus::Pending);
        self.last_full_sync_ms = 0;
        self.last_incremental_tick = Some(Instant::now());
        self.last_seen_push_revision = 0;
        self.last_seen_clipboard_hash = 0;
        self.last_wave_dispatch = None;
        self.amoo_attempted = false;
        self.apps_dispatched = false;
        self.storage_scan_dispatched = false;
        self.context_triggered_for_cycle = false;
        self.last_revision_refresh_at = None;
        self.awaiting_result = [false; 7];
        self.message_read_failure = None;
        self.cycle_started_at = Instant::now();
    }

    pub fn on_device_disconnected(&mut self) {
        self.device_serial = None;
        self.invalidate_message_read();
        self.domains = std::array::from_fn(|_| AutoSyncDomainStatus::Pending);
        self.last_wave_dispatch = None;
        self.amoo_attempted = false;
        self.apps_dispatched = false;
        self.storage_scan_dispatched = false;
        self.context_triggered_for_cycle = false;
        self.last_revision_refresh_at = None;
        self.awaiting_result = [false; 7];
        self.message_read_failure = None;
    }

    pub fn reset_for_manual_resync(&mut self) {
        self.invalidate_message_read();
        for status in &mut self.domains {
            *status = AutoSyncDomainStatus::Pending;
        }
        self.awaiting_result = [true; 7];
        self.cycle_started_at = Instant::now();
        self.last_wave_dispatch = None;
        self.amoo_attempted = false;
        self.apps_dispatched = false;
        self.storage_scan_dispatched = false;
        self.context_triggered_for_cycle = false;
    }

    fn invalidate_message_read(&mut self) {
        self.message_request_id = self.message_request_id.wrapping_add(1);
        self.message_read_job = None;
        clear_message_domain_caches();
    }

    fn begin_message_read(&mut self) -> Option<(u64, Arc<Mutex<Option<Instant>>>)> {
        if self.message_read_job.is_some() {
            return None;
        }
        self.message_request_id = self.message_request_id.wrapping_add(1);
        let started_at = Arc::new(Mutex::new(None));
        let background_incremental = self.domains[AutoSyncDomain::MessagesAndCalls.index()]
            .is_synced()
            && !self.awaiting_result[AutoSyncDomain::MessagesAndCalls.index()];
        self.message_read_job = Some(MessageReadJob {
            request_id: self.message_request_id,
            started_at: started_at.clone(),
            timeout_reported: false,
            background_incremental,
        });
        if !background_incremental {
            self.awaiting_result[AutoSyncDomain::MessagesAndCalls.index()] = true;
            self.domains[AutoSyncDomain::MessagesAndCalls.index()] = AutoSyncDomainStatus::Pending;
        }
        Some((self.message_request_id, started_at))
    }

    fn update_message_read_progress(&mut self) {
        let Some(job) = self.message_read_job.as_ref() else {
            return;
        };
        if job.timeout_reported {
            return;
        }
        let background_incremental = job.background_incremental;
        let Some(started_at) = job.started_at.lock().ok().and_then(|started| *started) else {
            return;
        };
        if started_at.elapsed() >= MESSAGE_READ_TIMEOUT {
            self.message_read_job.as_mut().unwrap().timeout_reported = true;
            // Cached records are still readable, but a stalled incremental
            // worker is not a successful new receipt either.
            self.message_read_failure =
                Some("消息读取超时，已保留上次内容；仍在等待读取回执".into());
            self.mark_failed(
                AutoSyncDomain::MessagesAndCalls,
                "消息读取超时，已保留上次内容；回执到达后会更新",
            );
        } else if !background_incremental {
            self.domains[AutoSyncDomain::MessagesAndCalls.index()] =
                AutoSyncDomainStatus::Syncing { started_at };
        }
    }

    fn message_refresh_due(&self, interval: Duration, now_ms: i64) -> bool {
        if self.message_read_job.is_some() {
            return false;
        }
        match self.domain_status(AutoSyncDomain::MessagesAndCalls) {
            AutoSyncDomainStatus::Synced { .. } | AutoSyncDomainStatus::Partial { .. } => self
                .last_revision_refresh_at
                .is_none_or(|at| at.elapsed() >= interval),
            AutoSyncDomainStatus::Failed { at_ms, .. } => {
                now_ms.saturating_sub(*at_ms) >= 8_000
            }
            _ => false,
        }
    }

    pub fn domain_status(&self, domain: AutoSyncDomain) -> &AutoSyncDomainStatus {
        &self.domains[domain.index()]
    }

    /// Number of domains (`0..=7`) that have reached `Synced`.
    pub fn completed_count(&self) -> usize {
        self.domains.iter().filter(|d| d.is_synced()).count()
    }

    pub fn coverage_summary(&self) -> String {
        if self.partial_count() == 1
            && matches!(
                self.domain_status(AutoSyncDomain::ContextAndPanorama),
                AutoSyncDomainStatus::Partial { .. }
            )
        {
            "部分资料待更新".into()
        } else {
            format!("{}项记录未齐", self.partial_count())
        }
    }

    pub fn partial_count(&self) -> usize {
        self.domains
            .iter()
            .filter(|d| matches!(d, AutoSyncDomainStatus::Partial { .. }))
            .count()
    }

    pub fn paused_count(&self) -> usize {
        self.domains.iter().filter(|d| d.is_paused()).count()
    }
    pub fn participating_count(&self) -> usize {
        AutoSyncDomain::ALL.len() - self.paused_count()
    }
    pub fn participating_complete(&self) -> bool {
        self.participating_count() > 0 && self.completed_count() == self.participating_count()
    }
    pub fn initial_wave_completed(&self) -> bool {
        self.cycle_count > 0 || self.participating_complete()
    }
    fn pause_suffix(&self) -> String {
        if self.paused_count() > 0 {
            format!(" · {}项暂停", self.paused_count())
        } else {
            String::new()
        }
    }
    pub fn mark_paused(&mut self, domain: AutoSyncDomain, reason: &str) {
        self.awaiting_result[domain.index()] = false;
        self.domains[domain.index()] = AutoSyncDomainStatus::Paused {
            reason: reason.into(),
        };
    }

    /// Completed source jobs, not guessed progress inside a network request.
    /// Limited coverage finishes a job but remains explicitly labelled Partial.
    pub fn overall_percent(&self) -> u8 {
        if self.device_serial.is_none() {
            return 0;
        }
        let total = self.participating_count();
        if total == 0 {
            return 0;
        }
        let completed = self.completed_count();
        if completed == total {
            return 100;
        }
        ((completed * 100 + total / 2) / total).min(99) as u8
    }

    /// Returns `true` when any domain is actively `Syncing` or waiting to be dispatched on a connected phone.
    pub fn is_syncing(&self) -> bool {
        if !self.enabled || self.device_serial.is_none() {
            return false;
        }
        self.domains.iter().any(|d| {
            matches!(
                d,
                AutoSyncDomainStatus::Syncing { .. } | AutoSyncDomainStatus::Pending
            )
        })
    }

    /// Returns the domain currently being synchronized (or the next pending domain).
    pub fn active_domain(&self) -> Option<AutoSyncDomain> {
        if self.device_serial.is_none() {
            return None;
        }
        for domain in AutoSyncDomain::ALL {
            if self.domains[domain.index()].is_syncing() {
                return Some(domain);
            }
        }
        for domain in AutoSyncDomain::ALL {
            if self.domains[domain.index()].is_pending() {
                return Some(domain);
            }
        }
        None
    }

    pub fn total_synced_items(&self) -> usize {
        self.domains
            .iter()
            .map(AutoSyncDomainStatus::item_count)
            .sum()
    }

    /// Concise status pill label for the top bar.
    /// E.g., `"全量同步 4/7 · 57%"` while syncing, `"双端实时同步 · 100%"` when complete,
    /// `"未连接手机"` when disconnected.
    pub fn top_bar_label(&self, connected: bool) -> String {
        if !connected || self.device_serial.is_none() {
            return "未连接手机".into();
        }
        if !self.enabled {
            return "自动同步已暂停".into();
        }
        let done = self.completed_count();
        let total = self.participating_count();
        let pct = self.overall_percent();
        let suffix = self.pause_suffix();
        if total == 0 {
            return "所有采集域已暂停".into();
        }
        if done >= total {
            if self.partial_count() > 0 {
                return format!("资料已更新 · {}{suffix}", self.coverage_summary());
            }
            format!("双端实时同步 · 100%{suffix}")
        } else if self.domains.iter().any(AutoSyncDomainStatus::is_failed) && !self.is_syncing() {
            format!("部分内容未同步 · {done}/{total} · 待重试{suffix}")
        } else {
            format!("全量同步 {done}/{total} · {pct}%{suffix}")
        }
    }

    /// Concise status line for the left sidebar device card.
    pub fn sidebar_status_line(&self, connected: bool) -> String {
        if !connected || self.device_serial.is_none() {
            return "连接手机后自动全量同步".into();
        }
        if !self.enabled {
            return "自动同步已暂停 · 点击恢复".into();
        }
        let done = self.completed_count();
        let total = self.participating_count();
        let pct = self.overall_percent();
        let suffix = self.pause_suffix();
        if total == 0 {
            return "所有采集域已暂停".into();
        }
        if done >= total {
            if self.partial_count() > 0 {
                return format!(
                    "{done}/{total} 项已更新 · {}{suffix}",
                    self.coverage_summary()
                );
            }
            let items = self.total_synced_items();
            format!("{done}/{total} 域已同步 · {items} 项实时守护{suffix}")
        } else if let Some(active) = self.active_domain() {
            format!(
                "正在同步{} ({done}/{total} · {pct}%){suffix}",
                active.short_label()
            )
        } else {
            format!(
                "后台同步 {done}/{total} · {pct}%{suffix}{}",
                if self.domains.iter().any(AutoSyncDomainStatus::is_failed) {
                    " · 待重试"
                } else {
                    ""
                }
            )
        }
    }

    pub fn mark_syncing(&mut self, domain: AutoSyncDomain) {
        let idx = domain.index();
        self.awaiting_result[idx] = true;
        if !matches!(self.domains[idx], AutoSyncDomainStatus::Syncing { .. }) {
            self.domains[idx] = AutoSyncDomainStatus::Syncing {
                started_at: Instant::now(),
            };
        }
    }

    pub fn mark_synced(&mut self, domain: AutoSyncDomain, item_count: usize) {
        let idx = domain.index();
        self.awaiting_result[idx] = false;
        let now_ms = Local::now().timestamp_millis();
        let (duration_ms, synced_at_ms) = match &self.domains[idx] {
            AutoSyncDomainStatus::Syncing { started_at } => {
                ((started_at.elapsed().as_millis() as u64).max(1), now_ms)
            }
            AutoSyncDomainStatus::Synced {
                item_count: prev_count,
                duration_ms,
                synced_at_ms: prev_ms,
            } => {
                if *prev_count == item_count {
                    (*duration_ms, *prev_ms)
                } else {
                    (*duration_ms, now_ms)
                }
            }
            _ => (18, now_ms),
        };
        let was_complete = self.participating_complete();
        self.domains[idx] = AutoSyncDomainStatus::Synced {
            item_count,
            duration_ms,
            synced_at_ms,
        };
        if !was_complete && self.participating_complete() {
            self.cycle_count = self.cycle_count.saturating_add(1);
            self.last_full_sync_ms = now_ms;
        } else if self.participating_complete() && self.last_full_sync_ms == 0 {
            self.cycle_count = self.cycle_count.max(1);
            self.last_full_sync_ms = now_ms;
        }
    }

    pub fn touch_synced(&mut self, domain: AutoSyncDomain, item_count: usize) {
        let idx = domain.index();
        self.awaiting_result[idx] = false;
        let now_ms = Local::now().timestamp_millis();
        let duration_ms = match &self.domains[idx] {
            AutoSyncDomainStatus::Syncing { started_at } => {
                (started_at.elapsed().as_millis() as u64).max(1)
            }
            AutoSyncDomainStatus::Synced { duration_ms, .. } => *duration_ms,
            _ => 18,
        };
        self.domains[idx] = AutoSyncDomainStatus::Synced {
            item_count,
            duration_ms,
            synced_at_ms: now_ms,
        };
    }

    pub fn mark_failed(&mut self, domain: AutoSyncDomain, error: impl Into<String>) {
        let idx = domain.index();
        self.awaiting_result[idx] = false;
        let error = error.into();
        if matches!(&self.domains[idx], AutoSyncDomainStatus::Failed { error: previous, .. } if previous == &error)
        {
            return;
        }
        let now_ms = Local::now().timestamp_millis();
        self.domains[idx] = AutoSyncDomainStatus::Failed {
            error,
            at_ms: now_ms,
        };
    }

    fn mark_partial(
        &mut self,
        domain: AutoSyncDomain,
        item_count: usize,
        warning: String,
        fresh: bool,
    ) {
        let idx = domain.index();
        self.awaiting_result[idx] = false;
        if !fresh
            && matches!(&self.domains[idx], AutoSyncDomainStatus::Partial { item_count: count, warning: previous, .. }
            if *count == item_count && previous == &warning)
        {
            return;
        }
        self.domains[idx] = AutoSyncDomainStatus::Partial {
            item_count,
            warning,
            synced_at_ms: Local::now().timestamp_millis(),
        };
    }

    pub fn to_json(&self, connected: bool) -> serde_json::Value {
        let domains_json: Vec<serde_json::Value> = AutoSyncDomain::ALL
            .iter()
            .map(|d| {
                let st = &self.domains[d.index()];
                let status_str = match st {
                    AutoSyncDomainStatus::Pending => "pending",
                    AutoSyncDomainStatus::Syncing { .. } => "syncing",
                    AutoSyncDomainStatus::Synced { .. } => "synced",
                    AutoSyncDomainStatus::Partial { .. } => "partial",
                    AutoSyncDomainStatus::Failed { .. } => "failed",
                    AutoSyncDomainStatus::Paused { .. } => "paused",
                };
                serde_json::json!({
                    "domain": d.key(),
                    "label": d.short_label(),
                    "status": status_str,
                    "status_label": st.status_label(),
                    "item_count": st.item_count(),
                    "duration_ms": st.duration_ms(),
                    "synced_at_ms": st.synced_at_ms(),
                    "error": st.error(),
                    "warning": match st { AutoSyncDomainStatus::Partial { warning, .. } => Some(warning), _ => None },
                })
            })
            .collect();

        serde_json::json!({
            "enabled": self.enabled,
            "connected": connected,
            "device_serial": self.device_serial,
            "completed_count": self.completed_count(),
            "total_domains": AutoSyncDomain::ALL.len(),
            "participating_domains":self.participating_count(),
            "paused_count":self.paused_count(),
            "partial_count":self.partial_count(),
            "overall_percent": self.overall_percent(),
            "is_syncing": self.is_syncing(),
            "active_domain": self.active_domain().map(AutoSyncDomain::key),
            "active_domain_label": self.active_domain().map(AutoSyncDomain::short_label),
            "top_bar_label": self.top_bar_label(connected),
            "sidebar_status_line": self.sidebar_status_line(connected),
            "cycle_count": self.cycle_count,
            "last_full_sync_ms": self.last_full_sync_ms,
            "total_synced_items": self.total_synced_items(),
            "last_seen_push_revision": self.last_seen_push_revision,
            "details_open": self.details_open,
            "domains": domains_json,
        })
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn grouped_message_reads<A: Send, B: Send, C: Send, D: Send>(
    wireless: bool,
    capabilities: impl FnOnce() -> A + Send,
    messages: impl FnOnce() -> B + Send,
    calls: impl FnOnce() -> C + Send,
    contacts: impl FnOnce() -> D + Send,
) -> (A, B, C, D) {
    if wireless {
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(capabilities);
            let b = scope.spawn(messages);
            (a.join().unwrap(), b.join().unwrap())
        });
        let (c, d) = std::thread::scope(|scope| {
            let c = scope.spawn(calls);
            let d = scope.spawn(contacts);
            (c.join().unwrap(), d.join().unwrap())
        });
        (a, b, c, d)
    } else {
        std::thread::scope(|scope| {
            let a = scope.spawn(capabilities);
            let b = scope.spawn(messages);
            let c = scope.spawn(calls);
            let d = scope.spawn(contacts);
            (
                a.join().unwrap(),
                b.join().unwrap(),
                c.join().unwrap(),
                d.join().unwrap(),
            )
        })
    }
}

fn cached_capabilities(adb: &Adb, device: &Device) -> Option<Capabilities> {
    if !device.serial.is_empty() {
        if let Ok(guard) = capabilities_cache().lock() {
            if let Some((at, cached)) = guard.get(&device.serial) {
                if at.elapsed() < CAPABILITIES_CACHE_TTL {
                    return Some(cached.clone());
                }
            }
        }
    }
    let fresh = bridge::capabilities(adb, device).ok()?;
    if !device.serial.is_empty() {
        if let Ok(mut guard) = capabilities_cache().lock() {
            guard.insert(device.serial.clone(), (Instant::now(), fresh.clone()));
        }
    }
    Some(fresh)
}

fn cached_contacts(adb: &Adb, device: &Device) -> std::result::Result<Vec<ContactEntry>, String> {
    if !device.serial.is_empty() {
        if let Ok(guard) = contacts_cache().lock() {
            if let Some((at, cached)) = guard.get(&device.serial) {
                if at.elapsed() < CONTACTS_CACHE_TTL {
                    return Ok(cached.clone());
                }
            }
        }
    }
    let fresh = read_result(device.contacts(adb))?;
    if !device.serial.is_empty() {
        if let Ok(mut guard) = contacts_cache().lock() {
            guard.insert(device.serial.clone(), (Instant::now(), fresh.clone()));
        }
    }
    Ok(fresh)
}

fn read_message_domain(adb: &Adb, device: &Device) -> Outcome {
    let (caps, sms, calls, contacts) = grouped_message_reads(
        is_wireless(&device.transport),
        || cached_capabilities(adb, device),
        || read_result(device.messages(adb)),
        || read_result(device.call_logs(adb)),
        || cached_contacts(adb, device),
    );
    Outcome::HomeRefreshed(String::new(), caps, sms, calls, contacts)
}

impl Companion {
    pub(super) fn spawn_message_sync(&mut self) -> bool {
        let Some(mut device) = self.selected.clone() else {
            return false;
        };
        self.auto_sync.on_device_connected(&device.serial);
        let Some((request_id, started_at)) = self.auto_sync.begin_message_read() else {
            return false;
        };
        let device_serial = device.serial.clone();
        let (tx, rx) = mpsc::channel();
        self.bg_receivers.push((Page::Sms, None, rx));
        std::thread::spawn(move || {
            if let Ok(mut started) = started_at.lock() {
                *started = Some(Instant::now());
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Adb::discover()
                    .and_then(|adb| {
                        device.transport = device.resolve_active_transport(&adb)?;
                        Ok(Box::new(read_message_domain(&adb, &device)))
                    })
                    .map_err(|error| format!("{error:#}"))
            }))
            .unwrap_or_else(|_| Err("消息读取工作线程异常结束，已保留上次内容".into()));
            let _ = tx.send(Ok(Outcome::AutoSyncMessages {
                device_serial,
                request_id,
                result,
            }));
        });
        true
    }

    pub(super) fn apply_message_sync_result(
        &mut self,
        serial: &str,
        request_id: u64,
        result: std::result::Result<Box<Outcome>, String>,
    ) {
        if self
            .selected
            .as_ref()
            .is_none_or(|device| device.serial != serial)
            || self.auto_sync.device_serial.as_deref() != Some(serial)
            || self
                .auto_sync
                .message_read_job
                .as_ref()
                .is_none_or(|job| job.request_id != request_id)
        {
            return;
        }
        let background_incremental = self
            .auto_sync
            .message_read_job
            .as_ref()
            .is_some_and(|job| job.background_incremental);
        // A local progress timeout does not discard a genuine late reply. A
        // changed device/full-resync request generation does discard it.
        self.auto_sync.message_read_job = None;
        match result {
            Ok(outcome) => {
                // The normal apply path already retains old records on Err.
                // Keep the real per-source errors instead of converting a
                // failed read into an Ok(cache) and publishing a false receipt.
                self.apply(*outcome);
                if background_incremental
                    && self
                        .auto_sync
                        .domain_status(AutoSyncDomain::MessagesAndCalls)
                        .is_synced()
                    && self.auto_sync.message_read_failure.is_none()
                {
                    self.touch_auto_sync_domain_synced(AutoSyncDomain::MessagesAndCalls);
                }
            }
            Err(error) => self.on_bg_job_failed(Page::Sms, &error),
        }
    }

    /// Spawns a non-blocking background device job onto `self.bg_receivers` without setting `self.busy`.
    pub(super) fn spawn_auto_sync_bg_job(
        &mut self,
        page: Page,
        job: impl FnOnce(&Adb, Device) -> Result<Outcome> + Send + 'static,
    ) -> bool {
        let Some(mut device) = self.selected.clone() else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        self.bg_receivers.push((page, None, rx));
        std::thread::spawn(move || {
            let outcome = Adb::discover().and_then(|adb| {
                if let Ok(active) = device.resolve_active_transport(&adb) {
                    device.transport = active;
                }
                job(&adb, device)
            });
            let _ = tx.send(outcome);
        });
        true
    }

    /// Triggers an immediate non-blocking push snapshot fetch if not already syncing.
    pub(super) fn trigger_immediate_push_sync(&mut self) {
        self.request_push_snapshot(false);
    }

    /// Computes the live item count for a given `AutoSyncDomain` from current `Companion` state.
    pub(crate) fn domain_item_count(&self, domain: AutoSyncDomain) -> usize {
        match domain {
            AutoSyncDomain::DeviceAndSummary => {
                let summary_lines = self
                    .summary
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count();
                let sim_count = self
                    .capabilities
                    .as_ref()
                    .map(|c| c.subscriptions.len())
                    .unwrap_or(0);
                (summary_lines + sim_count).max(usize::from(!self.summary.trim().is_empty()))
            }
            AutoSyncDomain::MessagesAndCalls => {
                self.sms.len()
                    + self.call_logs.len()
                    + self.contacts.len()
                    + self.push.snapshot.threads.len()
                    + self.push.snapshot.notifications.len()
            }
            AutoSyncDomain::CalendarAndSuite => {
                self.back_screen.stepos_suite.calendar_events.len()
                    + self.back_screen.stepos_suite.alarms.len()
                    + self.back_screen.stepos_suite.wellbeing.top_apps.len()
                    + usize::from(
                        !self
                            .back_screen
                            .stepos_suite
                            .phone_clipboard
                            .trim()
                            .is_empty(),
                    )
            }
            AutoSyncDomain::AmooAndTasks => {
                self.amoo_sessions.len()
                    + self.workspace.journal.tasks.len()
                    + self.amoo_skills.len()
                    + self.amoo_memories.len()
            }
            AutoSyncDomain::AppsAndUpdates => self.apps.len(),
            AutoSyncDomain::GalleryAndStorage => {
                self.back_screen.gallery_items.len()
                    + self
                        .storage_report
                        .as_ref()
                        .map(|r| r.scanned_files)
                        .unwrap_or(0)
            }
            AutoSyncDomain::ContextAndPanorama => {
                self.context.overview.items.len()
                    + self.context.overview.evidence_count
                    + self.notification_center.items.len()
            }
        }
    }

    /// Populates all 7 domains as `Synced` when running in `--demo` mode with a demo device.
    pub(super) fn sync_demo_auto_sync_domains(&mut self) {
        let Some(serial) = self.selected.as_ref().map(|d| d.serial.clone()) else {
            self.auto_sync.on_device_disconnected();
            return;
        };
        self.auto_sync.on_device_connected(&serial);
        let now_ms = Local::now().timestamp_millis();
        let demo_durations = [18u64, 42, 35, 29, 48, 38, 24];
        for (idx, domain) in AutoSyncDomain::ALL.iter().copied().enumerate() {
            let count = self.domain_item_count(domain);
            self.auto_sync.domains[idx] = AutoSyncDomainStatus::Synced {
                item_count: count,
                duration_ms: demo_durations[idx],
                synced_at_ms: now_ms,
            };
        }
        self.auto_sync.cycle_count = self.auto_sync.cycle_count.max(1);
        self.auto_sync.last_full_sync_ms = now_ms;
        self.auto_sync.last_seen_push_revision = self.push.snapshot.revision;
        self.auto_sync.last_seen_clipboard_hash =
            hash_str(self.back_screen.stepos_suite.phone_clipboard.trim());
    }

    pub(super) fn mark_auto_sync_domain_synced(&mut self, domain: AutoSyncDomain) {
        if self.selected.is_none() {
            return;
        }
        if domain == AutoSyncDomain::CalendarAndSuite {
            if let Some(error) = self.calendar_suite_sync_error() {
                self.auto_sync.mark_failed(domain, error);
                return;
            }
        }
        if let Some(serial) = self.selected.as_ref().map(|d| d.serial.clone()) {
            if self.auto_sync.device_serial.as_deref() != Some(serial.as_str()) {
                self.auto_sync.on_device_connected(&serial);
            }
        }
        let count = self.domain_item_count(domain);
        self.auto_sync.mark_synced(domain, count);
    }

    pub(super) fn touch_auto_sync_domain_synced(&mut self, domain: AutoSyncDomain) {
        if self.selected.is_none() {
            return;
        }
        // A fresh clipboard value does not repair a failed calendar write.
        if domain == AutoSyncDomain::CalendarAndSuite {
            if let Some(error) = self.calendar_suite_sync_error() {
                self.auto_sync.mark_failed(domain, error);
                return;
            }
        }
        let count = self.domain_item_count(domain);
        self.auto_sync.touch_synced(domain, count);
    }

    fn calendar_suite_sync_error(&self) -> Option<String> {
        self.back_screen
            .stepos_suite_error
            .as_deref()
            .or(self.back_screen.stepos_suite.read_error.as_deref())
            .map(str::trim)
            .filter(|error| !error.is_empty())
            .map(str::to_string)
    }

    pub(super) fn on_bg_job_failed(&mut self, completed_page: Page, err: &str) {
        match completed_page {
            Page::Overview => {
                self.auto_sync
                    .mark_failed(AutoSyncDomain::DeviceAndSummary, err);
            }
            Page::Sms | Page::Phone => {
                if !self.sms_loaded && self.sms_error.is_none() {
                    self.sms_error = Some(err.to_string());
                }
                if !self.phone_loaded && self.call_logs_error.is_none() {
                    self.call_logs_error = Some(err.to_string());
                }
                self.auto_sync
                    .mark_failed(AutoSyncDomain::MessagesAndCalls, err);
                self.auto_sync.message_read_failure = Some(err.to_string());
            }
            Page::Calendar | Page::Schedule | Page::Alarms => {
                self.back_screen.stepos_suite_loading = false;
                self.back_screen.stepos_suite.checked = true;
                self.back_screen.stepos_suite_error = Some(err.to_string());
                self.auto_sync
                    .mark_failed(AutoSyncDomain::CalendarAndSuite, err);
            }
            Page::Apps | Page::AppStore => {
                self.apps_loaded = true;
                self.apps_error = Some(err.to_string());
                self.auto_sync
                    .mark_failed(AutoSyncDomain::AppsAndUpdates, err);
            }
            Page::Gallery => {
                self.back_screen.gallery_loading = false;
                self.back_screen.gallery_checked = true;
                self.auto_sync
                    .mark_failed(AutoSyncDomain::GalleryAndStorage, err);
            }
            _ => {}
        }
    }

    // Capture fresh result errors before cache-preserving apply methods consume
    // them. Existing records remain readable, but are not evidence that the
    // latest synchronization succeeded.
    pub(super) fn auto_sync_outcome_results(
        &self,
        outcome: &Outcome,
    ) -> Vec<(AutoSyncDomain, Option<String>)> {
        use AutoSyncDomain::*;
        let mut results = vec![];
        match outcome {
            Outcome::Summary(text, _) if !text.trim().is_empty() => {
                results.push((DeviceAndSummary, None))
            }
            Outcome::HomeRefreshed(summary, _, sms, logs, contacts) => {
                if !summary.trim().is_empty() {
                    results.push((DeviceAndSummary, None));
                }
                let errors: Vec<_> = [
                    sms.as_ref().err(),
                    logs.as_ref().err(),
                    contacts.as_ref().err(),
                ]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
                results.push((
                    MessagesAndCalls,
                    (!errors.is_empty()).then(|| errors.join("；")),
                ));
            }
            Outcome::Sms(Err(error), _) => results.push((MessagesAndCalls, Some(error.clone()))),
            Outcome::PhoneData(logs, contacts) => {
                let errors: Vec<_> = [logs.as_ref().err(), contacts.as_ref().err()]
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect();
                if !errors.is_empty() {
                    results.push((MessagesAndCalls, Some(errors.join("；"))));
                }
            }
            Outcome::Apps(_) => results.push((AppsAndUpdates, None)),
            Outcome::PhoneGallery(_, _) => results.push((GalleryAndStorage, None)),
            Outcome::StepOsSuite(report) => results.push((
                CalendarAndSuite,
                report
                    .read_error
                    .as_deref()
                    .map(str::trim)
                    .filter(|error| !error.is_empty())
                    .map(str::to_string),
            )),
            Outcome::AmooRealData(_, _) => results.push((AmooAndTasks, None)),
            Outcome::AmooObservationUnavailable => results.push((AmooAndTasks, None)),
            Outcome::BulkFailed(page, _, error) => {
                if let Some(domain) = match page {
                    Page::Overview => Some(DeviceAndSummary),
                    Page::Apps | Page::AppStore => Some(AppsAndUpdates),
                    Page::Sms | Page::Phone => Some(MessagesAndCalls),
                    Page::Gallery => Some(GalleryAndStorage),
                    Page::Calendar | Page::Schedule | Page::Alarms => Some(CalendarAndSuite),
                    _ => None,
                } {
                    results.push((domain, Some(error.clone())));
                }
            }
            _ => {}
        }
        results
    }

    pub(super) fn apply_auto_sync_results(
        &mut self,
        results: Vec<(AutoSyncDomain, Option<String>)>,
    ) {
        let Some(serial) = self.selected.as_ref().map(|d| d.serial.clone()) else {
            return;
        };
        self.auto_sync.on_device_connected(&serial);
        for (domain, error) in results {
            self.auto_sync.awaiting_result[domain.index()] = false;
            if domain == AutoSyncDomain::MessagesAndCalls {
                self.auto_sync.message_read_failure = error.clone();
            }
            if let Some(error) = error {
                self.auto_sync.mark_failed(domain, error);
            } else if self.auto_sync.domain_status(domain).is_failed() {
                self.auto_sync.domains[domain.index()] = AutoSyncDomainStatus::Pending;
            }
        }
    }

    fn reconcile_ready_auto_sync_domain(&mut self, domain: AutoSyncDomain) {
        if !self.auto_sync.domain_status(domain).is_failed()
            && !self.auto_sync.awaiting_result[domain.index()]
        {
            self.mark_auto_sync_domain_synced(domain);
        }
    }

    /// Reconciles `self.auto_sync.domains` with the current loaded flags and item counts in `Companion`.
    pub(super) fn reconcile_auto_sync_domains_from_state(&mut self) {
        let Some(serial) = self.selected.as_ref().map(|d| d.serial.clone()) else {
            if self.auto_sync.device_serial.is_some() {
                self.auto_sync.on_device_disconnected();
            }
            return;
        };
        if self.auto_sync.device_serial.as_deref() != Some(serial.as_str()) {
            self.auto_sync.on_device_connected(&serial);
        }
        if self.is_demo_device() {
            self.sync_demo_auto_sync_domains();
            return;
        }

        // 0. DeviceAndSummary
        if !self.summary.trim().is_empty() {
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::DeviceAndSummary);
        }

        // 1. MessagesAndCalls
        let sms_ready = self.sms_loaded && self.sms_error.is_none();
        let phone_ready =
            self.phone_loaded && self.call_logs_error.is_none() && self.contacts_error.is_none();
        let push_ready = self.push.loaded && self.push.error.is_none();
        let message_read_active = self.auto_sync.message_read_job.is_some();
        if !message_read_active {
            if let Some(error) = self
                .sms_error
                .as_ref()
                .or(self.call_logs_error.as_ref())
                .or(self.contacts_error.as_ref())
                .or(self.push.error.as_ref())
            {
                self.auto_sync
                    .mark_failed(AutoSyncDomain::MessagesAndCalls, error.clone());
            }
        }
        if !message_read_active && sms_ready && phone_ready && push_ready {
            // Push-only failures can recover when its fresh snapshot arrives;
            // failed SMS/call/contact reads retain their separate error latch.
            if self.auto_sync.message_read_failure.is_none()
                && !self.auto_sync.awaiting_result[AutoSyncDomain::MessagesAndCalls.index()]
            {
                if self
                    .auto_sync
                    .domain_status(AutoSyncDomain::MessagesAndCalls)
                    .is_failed()
                {
                    self.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()] =
                        AutoSyncDomainStatus::Pending;
                }
            }
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::MessagesAndCalls);
        }
        self.auto_sync.update_message_read_progress();

        // 2. CalendarAndSuite
        if let Some(error) = self.calendar_suite_sync_error() {
            self.auto_sync
                .mark_failed(AutoSyncDomain::CalendarAndSuite, error);
        } else if self.back_screen.stepos_suite.checked
            || self.back_screen.stepos_suite_synced_at.is_some()
        {
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::CalendarAndSuite);
        } else if self.back_screen.stepos_suite_loading
            && self.auto_sync.domains[AutoSyncDomain::CalendarAndSuite.index()].is_pending()
        {
            self.auto_sync
                .mark_syncing(AutoSyncDomain::CalendarAndSuite);
        }

        // 3. AmooAndTasks
        if self.amoo_gateway_online
            || !self.amoo_sessions.is_empty()
            || self.amoo_snapshot.is_some()
            || (self.auto_sync.amoo_attempted && self.amoo_ui.snapshot_job.is_none())
        {
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::AmooAndTasks);
        } else if self.amoo_ui.snapshot_job.is_some()
            && self.auto_sync.domains[AutoSyncDomain::AmooAndTasks.index()].is_pending()
        {
            self.auto_sync.mark_syncing(AutoSyncDomain::AmooAndTasks);
        }

        // 4. AppsAndUpdates
        if let Some(error) = &self.apps_error {
            self.auto_sync
                .mark_failed(AutoSyncDomain::AppsAndUpdates, error.clone());
        } else if self.apps_loaded {
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::AppsAndUpdates);
        } else if self.bulk_busy
            && self.auto_sync.domains[AutoSyncDomain::AppsAndUpdates.index()].is_pending()
        {
            self.auto_sync.mark_syncing(AutoSyncDomain::AppsAndUpdates);
        }

        // 5. GalleryAndStorage
        if self.back_screen.gallery_checked {
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::GalleryAndStorage);
        } else if self.back_screen.gallery_loading
            && self.auto_sync.domains[AutoSyncDomain::GalleryAndStorage.index()].is_pending()
        {
            self.auto_sync
                .mark_syncing(AutoSyncDomain::GalleryAndStorage);
        }

        // 6. ContextAndPanorama. A user pause is not a successful collection
        // and is not a missing response that should time out.
        let context_paused = self.context_collection_paused();
        if context_paused {
            self.auto_sync.mark_paused(
                AutoSyncDomain::ContextAndPanorama,
                "采集已暂停；已有资料和成果仍可查看",
            );
            self.auto_sync.context_triggered_for_cycle = false;
        } else if self
            .auto_sync
            .domain_status(AutoSyncDomain::ContextAndPanorama)
            .is_paused()
        {
            self.auto_sync.domains[AutoSyncDomain::ContextAndPanorama.index()] =
                AutoSyncDomainStatus::Pending;
        }
        let receipt_at = self.context.collection_receipt_at();
        let fresh_receipt = receipt_at.is_some() && receipt_at != self.auto_sync.context_receipt_at;
        let context_ready = !context_paused
            && (if !self.context.persistent {
                self.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()].is_synced()
                    && !self.context.running()
            } else {
                match self
                    .context
                    .collection_result_since(self.auto_sync.cycle_started_at)
                {
                    Some(Ok(())) => true,
                    Some(Err(error)) => {
                        if self.context.collection_persisted() {
                            self.auto_sync.mark_partial(
                                AutoSyncDomain::ContextAndPanorama,
                                self.domain_item_count(AutoSyncDomain::ContextAndPanorama),
                                error.to_owned(),
                                fresh_receipt,
                            );
                        } else {
                            self.auto_sync
                                .mark_failed(AutoSyncDomain::ContextAndPanorama, error.to_owned());
                        }
                        false
                    }
                    // Idle is not a receipt. Require a persisted capture from
                    // this wave, including its per-source coverage result.
                    None => false,
                }
            });
        if context_ready {
            self.auto_sync.awaiting_result[AutoSyncDomain::ContextAndPanorama.index()] = false;
            if self
                .auto_sync
                .domain_status(AutoSyncDomain::ContextAndPanorama)
                .is_failed()
            {
                self.auto_sync.domains[AutoSyncDomain::ContextAndPanorama.index()] =
                    AutoSyncDomainStatus::Pending;
            }
            self.reconcile_ready_auto_sync_domain(AutoSyncDomain::ContextAndPanorama);
            if fresh_receipt {
                self.touch_auto_sync_domain_synced(AutoSyncDomain::ContextAndPanorama);
            }
        } else if !context_paused
            && self.auto_sync.context_triggered_for_cycle
            && self.context.has_pending_or_active_sync()
            && self.auto_sync.domains[AutoSyncDomain::ContextAndPanorama.index()].is_pending()
        {
            self.auto_sync
                .mark_syncing(AutoSyncDomain::ContextAndPanorama);
        }
        self.auto_sync.context_receipt_at = receipt_at;

        // A deadline releases the progress indicator, never invents success.
        for domain in AutoSyncDomain::ALL {
            // Message reads have an actual-worker timer and a deadline that
            // covers their underlying 30s calls, rather than queue/push time.
            if domain == AutoSyncDomain::MessagesAndCalls {
                continue;
            }
            let deadline = if domain == AutoSyncDomain::ContextAndPanorama {
                Duration::from_secs(45)
            } else {
                Duration::from_secs(20)
            };
            if let AutoSyncDomainStatus::Syncing { started_at } =
                &self.auto_sync.domains[domain.index()]
            {
                if started_at.elapsed() >= deadline {
                    self.auto_sync
                        .mark_failed(domain, "同步超时，已保留上次内容，请重试");
                }
            }
        }
    }

    /// Orchestrates non-blocking background waves and continuous incremental watching on each UI tick.
    pub(super) fn tick_auto_sync(&mut self, ctx: &mut ViewContext<Self>) {
        let prev_completed = self.auto_sync.completed_count();
        let prev_pct = self.auto_sync.overall_percent();
        self.reconcile_auto_sync_domains_from_state();

        if self.is_demo_device() || self.selected.is_none() || !self.auto_sync.enabled {
            if self.auto_sync.completed_count() != prev_completed
                || self.auto_sync.overall_percent() != prev_pct
            {
                self.sync_notification_center();
                ctx.notify();
            }
            return;
        }

        // 1. Continuous incremental watcher: detect `push.snapshot.revision` changes on the phone
        if self.push.loaded && self.push.snapshot.revision != self.auto_sync.last_seen_push_revision
        {
            let prev_rev = self.auto_sync.last_seen_push_revision;
            if self.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()].is_synced() {
                self.touch_auto_sync_domain_synced(AutoSyncDomain::MessagesAndCalls);
            }
            if prev_rev == 0 {
                self.auto_sync.last_seen_push_revision = self.push.snapshot.revision;
                self.trigger_auto_context_refresh();
                self.sync_notification_center();
                ctx.notify();
            } else if !matches!(self.page, Page::Sms | Page::Phone | Page::Notifications) {
                self.auto_sync.last_seen_push_revision = self.push.snapshot.revision;
                self.trigger_auto_context_refresh();
                self.sync_notification_center();
                ctx.notify();
            } else if self
                .auto_sync
                .last_revision_refresh_at
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(60))
                && self.auto_sync.message_read_job.is_none()
            {
                if self.spawn_message_sync() {
                    self.auto_sync.last_seen_push_revision = self.push.snapshot.revision;
                    self.auto_sync.last_revision_refresh_at = Some(Instant::now());
                    self.auto_sync.cycle_count = self.auto_sync.cycle_count.saturating_add(1);
                    self.auto_sync.last_full_sync_ms = Local::now().timestamp_millis();
                }
                self.trigger_auto_context_refresh();
                self.sync_notification_center();
                ctx.notify();
            } else {
                // Do not advance the revision while this domain is throttled
                // or already reading. Unrelated generation does not gate it.
                self.trigger_auto_context_refresh();
                self.sync_notification_center();
                ctx.notify();
            }
        }

        // 2. Continuous incremental watcher: detect phone clipboard changes
        let clip_hash = hash_str(self.back_screen.stepos_suite.phone_clipboard.trim());
        if clip_hash != self.auto_sync.last_seen_clipboard_hash {
            let had_prev = self.auto_sync.last_seen_clipboard_hash != 0;
            self.auto_sync.last_seen_clipboard_hash = clip_hash;
            self.sync_live_express_parcels();
            if had_prev {
                self.auto_sync.cycle_count = self.auto_sync.cycle_count.saturating_add(1);
                self.auto_sync.last_full_sync_ms = Local::now().timestamp_millis();
                self.touch_auto_sync_domain_synced(AutoSyncDomain::CalendarAndSuite);
                self.trigger_auto_context_refresh();
            }
            self.sync_notification_center();
            ctx.notify();
        }

        // 3. Continuous incremental watcher: transport-adaptive StepOsSuite refresh (180s USB / 240s Wireless)
        let is_wireless_active = self
            .selected
            .as_ref()
            .is_some_and(|d| is_wireless(&d.transport));
        let suite_interval = Duration::from_secs(if is_wireless_active { 240 } else { 180 });
        if self.auto_sync.domains[AutoSyncDomain::CalendarAndSuite.index()].is_synced()
            && self.bg_receivers.is_empty()
            && !self.busy
            && self
                .auto_sync
                .last_incremental_tick
                .is_none_or(|t| t.elapsed() >= suite_interval)
        {
            self.auto_sync.last_incremental_tick = Some(Instant::now());
            if !self.back_screen.stepos_suite_loading {
                self.handle_backscreen_action(&BackScreenAction::SyncCalendarAndSuite, ctx);
            }
        }

        // 3b. Unattended Messaging & Call watcher: poll every 30s on Sms/Notifications/Phone and 180s on all other pages
        let comm_page_active = matches!(self.page, Page::Sms | Page::Notifications | Page::Phone);
        let sms_poll_interval = Duration::from_secs(if comm_page_active { 30 } else { 180 });
        // One dedicated message worker is independent of page generation,
        // file transfers and other background jobs. Its own request generation
        // prevents duplicates; the transport coordinator still bounds I/O.
        if self.auto_sync.message_refresh_due(
            sms_poll_interval,
            Local::now().timestamp_millis(),
        ) {
            if self.spawn_message_sync() {
                self.auto_sync.last_revision_refresh_at = Some(Instant::now());
            }
        }

        // 4. Wave dispatcher for initial / full 7-domain synchronization
        // Throttle wave dispatch checks to every 120ms and cap concurrent `bg_receivers` (1 on wireless, 2 on USB)
        let max_bg_jobs = if is_wireless_active { 1 } else { 2 };
        if self
            .auto_sync
            .last_wave_dispatch
            .is_some_and(|t| t.elapsed() < Duration::from_millis(120))
        {
            if self.auto_sync.completed_count() != prev_completed
                || self.auto_sync.overall_percent() != prev_pct
            {
                self.sync_notification_center();
                ctx.notify();
            }
            return;
        }
        self.auto_sync.last_wave_dispatch = Some(Instant::now());

        // 4a. Automatic backoff retry: if any domain entered `Failed` due to a transient
        // wireless ADB hiccup, automatically release its one-shot latch after 8s so it recovers
        // without requiring manual user intervention.
        if !self.busy && self.bg_receivers.is_empty() {
            let now_ms = Local::now().timestamp_millis();
            for domain in AutoSyncDomain::ALL {
                if domain == AutoSyncDomain::MessagesAndCalls {
                    continue; // Dedicated worker retries without waiting for other jobs.
                }
                let should_retry = matches!(
                    self.auto_sync.domains[domain.index()],
                    AutoSyncDomainStatus::Failed { at_ms, .. } if now_ms.saturating_sub(at_ms) >= 8_000
                );
                if should_retry {
                    self.auto_sync.domains[domain.index()] = AutoSyncDomainStatus::Pending;
                    self.auto_sync.awaiting_result[domain.index()] = true;
                    match domain {
                        AutoSyncDomain::DeviceAndSummary => {}
                        AutoSyncDomain::MessagesAndCalls => {
                            self.auto_sync.message_read_failure = None;
                            self.sms_error = None;
                            self.call_logs_error = None;
                            self.contacts_error = None;
                            self.push.error = None;
                        }
                        AutoSyncDomain::CalendarAndSuite => {
                            self.back_screen.stepos_suite_error = None;
                            self.back_screen.stepos_suite.read_error = None;
                            self.back_screen.stepos_suite.checked = false;
                        }
                        AutoSyncDomain::AmooAndTasks => {
                            self.auto_sync.amoo_attempted = false;
                        }
                        AutoSyncDomain::AppsAndUpdates => {
                            self.apps_error = None;
                            self.auto_sync.apps_dispatched = false;
                        }
                        AutoSyncDomain::GalleryAndStorage => {
                            self.back_screen.gallery_checked = false;
                        }
                        AutoSyncDomain::ContextAndPanorama => {
                            self.auto_sync.context_triggered_for_cycle = false;
                            self.auto_sync.cycle_started_at = Instant::now();
                        }
                    }
                }
            }
        }

        // Domain 0: DeviceAndSummary
        if self.auto_sync.domains[AutoSyncDomain::DeviceAndSummary.index()].is_pending()
            && !self.busy
            && self.bg_receivers.len() < max_bg_jobs
        {
            self.auto_sync
                .mark_syncing(AutoSyncDomain::DeviceAndSummary);
            self.spawn_auto_sync_bg_job(Page::Overview, |adb, d| {
                Ok(Outcome::Summary(
                    d.summary(adb)?,
                    bridge::capabilities(adb, &d).ok(),
                ))
            });
        }

        // Domain 1: MessagesAndCalls
        if !self.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()].is_synced() {
            if !self.push.loaded && !self.push.syncing && self.push.error.is_none() {
                self.trigger_immediate_push_sync();
            }
            let needs_sms_or_phone = (self.auto_sync.awaiting_result
                [AutoSyncDomain::MessagesAndCalls.index()]
                && self
                    .auto_sync
                    .domain_status(AutoSyncDomain::MessagesAndCalls)
                    .is_pending())
                || (!self.sms_loaded && self.sms_error.is_none())
                || (!self.phone_loaded && self.call_logs_error.is_none());
            if needs_sms_or_phone {
                self.spawn_message_sync();
            }
        }

        // Domain 2: CalendarAndSuite (uses async_channel via BackScreenAction::SyncCalendarAndSuite)
        if self.auto_sync.domains[AutoSyncDomain::CalendarAndSuite.index()].is_pending()
            && !self.back_screen.stepos_suite.checked
            && !self.back_screen.stepos_suite_loading
        {
            self.auto_sync
                .mark_syncing(AutoSyncDomain::CalendarAndSuite);
            self.handle_backscreen_action(&BackScreenAction::SyncCalendarAndSuite, ctx);
        }

        // Domain 3: AmooAndTasks (uses dedicated amoo_sync::SyncWorker)
        if !self.auto_sync.domains[AutoSyncDomain::AmooAndTasks.index()].is_synced()
            && !self.auto_sync.amoo_attempted
            && self.amoo_ui.snapshot_job.is_none()
        {
            self.auto_sync.amoo_attempted = true;
            self.auto_sync.mark_syncing(AutoSyncDomain::AmooAndTasks);
            if !self.amoo_ui.usage_attempted {
                self.start_account_usage_sync();
            }
            self.request_amoo_session_sync(None);
        }

        let core_home_domains_ready = self.page != Page::Home
            || [
                AutoSyncDomain::DeviceAndSummary,
                AutoSyncDomain::MessagesAndCalls,
                AutoSyncDomain::CalendarAndSuite,
            ]
            .iter()
            .all(|domain| {
                matches!(
                    self.auto_sync.domain_status(*domain),
                    AutoSyncDomainStatus::Synced { .. } | AutoSyncDomainStatus::Failed { .. }
                )
            });

        // Domain 4: AppsAndUpdates
        self.ensure_store_online_catalog_synced();
        if !self.auto_sync.domains[AutoSyncDomain::AppsAndUpdates.index()].is_synced() {
            if core_home_domains_ready
                && (!self.apps_loaded
                    || (self.auto_sync.awaiting_result[AutoSyncDomain::AppsAndUpdates.index()]
                        && self
                            .auto_sync
                            .domain_status(AutoSyncDomain::AppsAndUpdates)
                            .is_pending()))
                && !self.auto_sync.apps_dispatched
                && !self.bulk_busy
                && self.bg_receivers.is_empty()
            {
                self.auto_sync.apps_dispatched = true;
                self.auto_sync.mark_syncing(AutoSyncDomain::AppsAndUpdates);
                self.spawn_auto_sync_bg_job(Page::Apps, |adb, d| {
                    Ok(Outcome::Apps(apps::catalog(adb, &d)?))
                });
            }
        } else if self.apps_loaded
            && !self.apps.is_empty()
            && !self.app_store.catalog.is_empty()
            && !self.app_store.update_checked_once
            && !self.app_store.checking_updates
        {
            self.handle_app_store_action(AppStoreAction::CheckUpdates, ctx);
        }

        // Domain 5: GalleryAndStorage
        if !self.auto_sync.domains[AutoSyncDomain::GalleryAndStorage.index()].is_synced()
            && core_home_domains_ready
        {
            if !self.back_screen.gallery_checked && !self.back_screen.gallery_loading {
                self.auto_sync
                    .mark_syncing(AutoSyncDomain::GalleryAndStorage);
                self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
            }
            if self.storage_report.is_none()
                && !self.auto_sync.storage_scan_dispatched
                && self.bg_receivers.is_empty()
            {
                self.auto_sync.storage_scan_dispatched = true;
                self.spawn_auto_sync_bg_job(Page::Tools, |adb, d| {
                    let report = toolbox::scan(adb, &d)?;
                    Ok(Outcome::StorageScanned(d.serial.clone(), report, None))
                });
            }
        }

        // Domain 6: ContextAndPanorama
        if !self.context_collection_paused()
            && !self.auto_sync.domains[AutoSyncDomain::ContextAndPanorama.index()].is_synced()
            && !self.auto_sync.context_triggered_for_cycle
            && self.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()].is_synced()
            && (!self.context.persistent || self.context.initialized)
        {
            self.auto_sync.context_triggered_for_cycle = true;
            self.auto_sync
                .mark_syncing(AutoSyncDomain::ContextAndPanorama);
            self.trigger_auto_context_refresh();
            self.sync_notification_center();
        }

        self.reconcile_auto_sync_domains_from_state();
        if self.auto_sync.completed_count() != prev_completed
            || self.auto_sync.overall_percent() != prev_pct
        {
            self.sync_notification_center();
            ctx.notify();
        }
    }

    fn open_device_sync_details(&mut self) {
        self.page = Page::Connect;
        self.auto_sync.details_open = false;
        self.notification_center.drawer_open = false;
        self.back_screen.live_capsule_popover_open = false;
    }

    pub(super) fn handle_auto_sync_action(
        &mut self,
        action: &AutoSyncAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AutoSyncAction::ToggleDetails => {
                self.handle_ui_action(&Action::Page(Page::Connect), ctx);
                self.open_device_sync_details();
                ctx.notify();
            }
            AutoSyncAction::SetDetailsOpen(open) => {
                if *open {
                    self.handle_ui_action(&Action::Page(Page::Connect), ctx);
                    self.open_device_sync_details();
                } else {
                    self.auto_sync.details_open = false;
                }
                ctx.notify();
            }
            AutoSyncAction::SetEnabled(enabled) => {
                self.auto_sync.enabled = *enabled;
                if *enabled {
                    self.tick_auto_sync(ctx);
                }
                ctx.notify();
            }
            AutoSyncAction::SyncAllNow => {
                self.auto_sync.enabled = true;
                if self.demo_mode {
                    self.sync_demo_auto_sync_domains();
                    self.status = "已完成全量 7 域双端实时同步".into();
                    ctx.notify();
                    return;
                }
                if self.selected.is_none() {
                    self.status = "请先连接手机后再执行全量同步".into();
                    self.error = true;
                    ctx.notify();
                    return;
                }
                self.auto_sync.reset_for_manual_resync();
                self.trigger_immediate_push_sync();
                if !self.back_screen.stepos_suite_loading {
                    self.handle_backscreen_action(&BackScreenAction::SyncCalendarAndSuite, ctx);
                }
                if !self.back_screen.gallery_loading {
                    self.back_screen.gallery_checked = false;
                    self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
                }
                self.request_amoo_session_sync(None);
                self.trigger_auto_context_refresh();
                self.tick_auto_sync(ctx);
                self.status = format!(
                    "已启动 {} 域后台同步…{}",
                    self.auto_sync.participating_count(),
                    self.auto_sync.pause_suffix()
                );
                ctx.notify();
            }
            AutoSyncAction::SyncDomain(domain) => {
                // Retrying a source keeps its status and result visible here.
                if !self.demo_mode && self.selected.is_some() {
                    if *domain == AutoSyncDomain::ContextAndPanorama
                        && self.context_collection_paused()
                    {
                        self.auto_sync
                            .mark_paused(*domain, "采集已暂停；已有资料和成果仍可查看");
                        self.status = "上下文采集已暂停，可在整理设置中恢复；未触发采集".into();
                        ctx.notify();
                        return;
                    }
                    if *domain == AutoSyncDomain::ContextAndPanorama {
                        self.auto_sync.cycle_started_at = Instant::now();
                    }
                    if *domain != AutoSyncDomain::MessagesAndCalls {
                        self.auto_sync.mark_syncing(*domain);
                    }
                    match domain {
                        AutoSyncDomain::DeviceAndSummary => {
                            self.spawn_auto_sync_bg_job(Page::Overview, |adb, d| {
                                Ok(Outcome::Summary(
                                    d.summary(adb)?,
                                    bridge::capabilities(adb, &d).ok(),
                                ))
                            });
                        }
                        AutoSyncDomain::MessagesAndCalls => {
                            self.auto_sync.awaiting_result
                                [AutoSyncDomain::MessagesAndCalls.index()] = true;
                            self.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()] =
                                AutoSyncDomainStatus::Pending;
                            self.trigger_immediate_push_sync();
                            if !self.spawn_message_sync() {
                                self.status =
                                    "消息与通信仍在读取，已保留现有记录；不会重复发起读取".into();
                            }
                        }
                        AutoSyncDomain::CalendarAndSuite => {
                            self.handle_backscreen_action(
                                &BackScreenAction::SyncCalendarAndSuite,
                                ctx,
                            );
                        }
                        AutoSyncDomain::AmooAndTasks => {
                            self.request_amoo_session_sync(None);
                        }
                        AutoSyncDomain::AppsAndUpdates => {
                            self.ensure_store_online_catalog_synced();
                            self.spawn_auto_sync_bg_job(Page::Apps, |adb, d| {
                                Ok(Outcome::Apps(apps::catalog(adb, &d)?))
                            });
                        }
                        AutoSyncDomain::GalleryAndStorage => {
                            self.back_screen.gallery_checked = false;
                            self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
                        }
                        AutoSyncDomain::ContextAndPanorama => {
                            self.trigger_auto_context_refresh();
                            self.sync_notification_center();
                        }
                    }
                }
                ctx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_details_share_the_device_page_and_do_not_toggle_an_overlay_or_start_reads() {
        let mut app = Companion::new_for_test();
        app.page = Page::Home;
        app.notification_center.drawer_open = true;
        for _ in 0..2 {
            app.open_device_sync_details();
            assert_eq!(app.page, Page::Connect);
            assert!(!app.auto_sync.details_open);
            assert!(!app.notification_center.drawer_open);
            assert!(!app.context.force);
        }
    }

    fn connected_app_with_cached_data() -> Companion {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device {
            serial: "sync-truth-unit".into(),
            transport: "sync-truth-unit".into(),
            ..Default::default()
        });
        app.auto_sync.on_device_connected("sync-truth-unit");
        app.summary = "已保存的设备摘要".into();
        app.sms_loaded = true;
        app.phone_loaded = true;
        app.push.loaded = true;
        app.back_screen.stepos_suite.checked = true;
        app.apps_loaded = true;
        app.amoo_gateway_online = true;
        app.back_screen.gallery_checked = true;
        app.auto_sync.context_triggered_for_cycle = true;
        app
    }

    fn empty_message_reply(serial: &str, request_id: u64) -> Outcome {
        Outcome::AutoSyncMessages {
            device_serial: serial.into(),
            request_id,
            result: Ok(Box::new(Outcome::HomeRefreshed(
                String::new(),
                None,
                Ok(vec![]),
                Ok(vec![]),
                Ok(vec![]),
            ))),
        }
    }

    #[test]
    fn background_message_failures_retain_records_without_fabricating_success() {
        let mut app = connected_app_with_cached_data();
        app.sms.push(Message {
            provider_id: Some(1),
            address: "local".into(),
            body: "cached record".into(),
            date_ms: 1,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: None,
        });
        app.reconcile_auto_sync_domains_from_state();
        let (request, _) = app.auto_sync.begin_message_read().unwrap();
        assert!(app.auto_sync.message_read_job.as_ref().unwrap().background_incremental);
        app.apply_message_sync_result("sync-truth-unit", request, Err("wireless read failed".into()));
        assert_eq!(app.sms[0].body, "cached record");
        assert!(app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_failed());
        app.reconcile_auto_sync_domains_from_state();
        assert!(app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_failed());
        assert_eq!(app.auto_sync.message_read_failure.as_deref(), Some("wireless read failed"));

        let (request, _) = app.auto_sync.begin_message_read().unwrap();
        app.apply(empty_message_reply("sync-truth-unit", request));
        assert!(app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_synced());
        assert!(app.auto_sync.message_read_failure.is_none());
    }

    #[test]
    fn background_partial_read_does_not_turn_failed_sources_into_cached_ok_values() {
        let mut app = connected_app_with_cached_data();
        app.reconcile_auto_sync_domains_from_state();
        let (request, _) = app.auto_sync.begin_message_read().unwrap();
        app.apply_message_sync_result("sync-truth-unit", request, Ok(Box::new(Outcome::HomeRefreshed(
            String::new(), None, Err("SMS unavailable".into()), Ok(vec![]), Ok(vec![]),
        ))));
        assert_eq!(app.sms_error.as_deref(), Some("SMS unavailable"));
        assert!(app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_failed());
        assert_eq!(app.auto_sync.message_read_failure.as_deref(), Some("SMS unavailable"));
    }

    #[test]
    fn background_timeout_remains_visible_until_the_actual_late_receipt() {
        let mut app = connected_app_with_cached_data();
        app.reconcile_auto_sync_domains_from_state();
        let (request, started) = app.auto_sync.begin_message_read().unwrap();
        assert!(app.auto_sync.message_read_job.as_ref().unwrap().background_incremental);
        *started.lock().unwrap() = Some(Instant::now() - MESSAGE_READ_TIMEOUT - Duration::from_secs(1));
        app.reconcile_auto_sync_domains_from_state();
        assert!(app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_failed());
        assert!(app.auto_sync.message_read_job.is_some(), "a late real reply is still accepted");
        app.apply(empty_message_reply("sync-truth-unit", request));
        assert!(app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_synced());
    }

    #[test]
    fn incremental_message_retry_uses_its_own_deadline_and_single_worker() {
        let mut state = AutoSyncState::new();
        state.mark_synced(AutoSyncDomain::MessagesAndCalls, 10);
        assert!(state.message_refresh_due(Duration::from_secs(30), 0));
        state.begin_message_read().unwrap();
        assert!(!state.message_refresh_due(Duration::ZERO, i64::MAX));
        state.message_read_job = None;
        state.mark_failed(AutoSyncDomain::MessagesAndCalls, "network interrupted");
        let AutoSyncDomainStatus::Failed { at_ms, .. } = *state.domain_status(AutoSyncDomain::MessagesAndCalls) else { panic!("failed receipt"); };
        assert!(!state.message_refresh_due(Duration::from_secs(180), at_ms + 7_999));
        assert!(state.message_refresh_due(Duration::from_secs(180), at_ms + 8_000));
    }

    #[test]
    fn message_queue_wait_and_legitimate_30_second_reads_do_not_use_the_20_second_domain_deadline()
    {
        let mut app = connected_app_with_cached_data();
        let (_, started) = app.auto_sync.begin_message_read().unwrap();
        assert!(
            app.auto_sync.begin_message_read().is_none(),
            "one domain cannot dispatch duplicate reads"
        );
        app.auto_sync.domains[AutoSyncDomain::MessagesAndCalls.index()] =
            AutoSyncDomainStatus::Syncing {
                started_at: Instant::now() - Duration::from_secs(120),
            };
        app.reconcile_auto_sync_domains_from_state();
        assert!(
            !app.auto_sync
                .domain_status(AutoSyncDomain::MessagesAndCalls)
                .is_failed(),
            "a worker not started yet cannot time out"
        );
        *started.lock().unwrap() = Some(Instant::now() - Duration::from_secs(35));
        app.reconcile_auto_sync_domains_from_state();
        assert!(
            matches!(
                app.auto_sync
                    .domain_status(AutoSyncDomain::MessagesAndCalls),
                AutoSyncDomainStatus::Syncing { .. }
            ),
            "two bridge read groups may legitimately take more than twenty seconds"
        );
    }

    #[test]
    fn a_real_late_message_reply_repairs_timeout_without_recounting_cached_data() {
        let mut app = connected_app_with_cached_data();
        let (request_id, started) = app.auto_sync.begin_message_read().unwrap();
        *started.lock().unwrap() =
            Some(Instant::now() - MESSAGE_READ_TIMEOUT - Duration::from_secs(1));
        app.reconcile_auto_sync_domains_from_state();
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_failed());
        app.reconcile_auto_sync_domains_from_state();
        assert!(
            app.auto_sync
                .domain_status(AutoSyncDomain::MessagesAndCalls)
                .is_failed(),
            "cached data cannot repair the timeout"
        );
        app.apply(empty_message_reply("sync-truth-unit", request_id));
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_synced());
        assert!(app.auto_sync.message_read_job.is_none());
        assert!(app.auto_sync.message_read_failure.is_none());
    }

    #[test]
    fn stale_message_replies_cannot_cross_device_or_resync_generation() {
        let mut app = connected_app_with_cached_data();
        app.sms.push(Message {
            provider_id: Some(1),
            address: "local".into(),
            body: "saved".into(),
            date_ms: 1,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: None,
        });
        let (old_request, _) = app.auto_sync.begin_message_read().unwrap();
        app.selected.as_mut().unwrap().serial = "another-phone".into();
        app.apply(empty_message_reply("sync-truth-unit", old_request));
        assert_eq!(
            app.sms.len(),
            1,
            "a result from another phone must not replace current records"
        );
        app.selected.as_mut().unwrap().serial = "sync-truth-unit".into();
        app.auto_sync.reset_for_manual_resync();
        let (new_request, _) = app.auto_sync.begin_message_read().unwrap();
        app.apply(empty_message_reply("sync-truth-unit", old_request));
        assert_eq!(
            app.sms.len(),
            1,
            "an older full-resync reply is not this request's evidence"
        );
        assert!(app.auto_sync.awaiting_result[AutoSyncDomain::MessagesAndCalls.index()]);
        app.apply(empty_message_reply("sync-truth-unit", new_request));
        assert!(app.sms.is_empty());
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_synced());
    }

    #[test]
    fn wireless_initial_retry_and_revision_reads_share_two_concurrent_groups() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let entered = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let first = || {
            entered.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(2);
            while entered.load(Ordering::SeqCst) < 2 {
                assert!(
                    Instant::now() < deadline,
                    "the first read pair must run concurrently"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            completed.fetch_add(1, Ordering::SeqCst);
            1
        };
        let second = || {
            assert_eq!(
                completed.load(Ordering::SeqCst),
                2,
                "calls/contacts must wait for the first pair"
            );
            2
        };
        assert_eq!(
            grouped_message_reads(true, || first(), || first(), || second(), || second()),
            (1, 1, 2, 2)
        );
    }

    #[test]
    fn failed_reads_and_timeouts_never_become_success_from_cached_data() {
        let mut app = connected_app_with_cached_data();
        app.reconcile_auto_sync_domains_from_state();
        assert_eq!(app.auto_sync.completed_count(), 7);
        app.on_bg_job_failed(Page::Apps, "offline");
        for _ in 0..3 {
            app.reconcile_auto_sync_domains_from_state();
        }
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::AppsAndUpdates)
            .is_failed());
        assert!(app.auto_sync.completed_count() < 7);
        assert!(!app.auto_sync.top_bar_label(true).contains("100%"));

        app.auto_sync.mark_syncing(AutoSyncDomain::DeviceAndSummary);
        app.auto_sync.domains[AutoSyncDomain::DeviceAndSummary.index()] =
            AutoSyncDomainStatus::Syncing {
                started_at: Instant::now() - Duration::from_secs(21),
            };
        app.reconcile_auto_sync_domains_from_state();
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::DeviceAndSummary)
            .is_failed());
        app.reconcile_auto_sync_domains_from_state();
        assert!(
            app.auto_sync
                .domain_status(AutoSyncDomain::DeviceAndSummary)
                .is_failed(),
            "Cached summary cannot make a timed-out new read succeed"
        );
        app.apply(Outcome::Summary("新读取的设备摘要".into(), None));
        assert!(
            app.auto_sync
                .domain_status(AutoSyncDomain::DeviceAndSummary)
                .is_synced(),
            "A real later response recovers the failed domain"
        );
    }

    #[test]
    fn another_phone_with_the_same_model_is_not_an_automatic_failover_target() {
        let mut app = connected_app_with_cached_data();
        app.demo_mode = true;
        app.preferences.selected_serial = None;
        app.allow_first_connect = false;
        app.selected.as_mut().unwrap().model = "PZ261A".into();
        app.selected.as_ref().unwrap().set_backup_transports(vec![]);
        let other = Transport {
            id: "different-phone-unit".into(),
            state: "device".into(),
            model: "PZ261A".into(),
            ..Default::default()
        };
        app.apply(Outcome::Transports(vec![other.clone()], vec![]));
        assert_eq!(app.selected.as_ref().unwrap().transport, "sync-truth-unit");
        app.apply(Outcome::Transports(vec![other], vec![]));
        assert!(
            app.selected.is_none(),
            "A same-model phone is not the disconnected original phone"
        );
        assert_eq!(
            app.connection_watch.detached_serial.as_deref(),
            Some("sync-truth-unit")
        );
    }

    #[test]
    fn full_resync_waits_for_fresh_responses_instead_of_recounting_the_cache() {
        let mut app = connected_app_with_cached_data();
        app.reconcile_auto_sync_domains_from_state();
        app.auto_sync.reset_for_manual_resync();
        app.reconcile_auto_sync_domains_from_state();
        assert_eq!(app.auto_sync.completed_count(), 0);
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::DeviceAndSummary)
            .is_pending());
        assert!(
            !app.summary.is_empty(),
            "Cached content remains available during a refresh"
        );
        app.apply(Outcome::Summary("新读取的设备摘要".into(), None));
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::DeviceAndSummary)
            .is_synced());
        assert!(!app
            .auto_sync
            .domain_status(AutoSyncDomain::AppsAndUpdates)
            .is_synced());
    }

    #[test]
    fn partial_message_failure_remains_visible_until_a_fresh_complete_read() {
        let mut app = connected_app_with_cached_data();
        app.sms.push(Message {
            provider_id: Some(1),
            address: "10086".into(),
            body: "本地缓存".into(),
            date_ms: 1,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: None,
        });
        app.apply(Outcome::HomeRefreshed(
            String::new(),
            None,
            Err("短信暂不可读".into()),
            Ok(vec![]),
            Ok(vec![]),
        ));
        assert_eq!(app.sms.len(), 1);
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_failed());
        // A new push-only update cannot erase an SMS read failure.
        app.push.loaded = true;
        app.push.error = None;
        app.reconcile_auto_sync_domains_from_state();
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_failed());
        app.apply(Outcome::HomeRefreshed(
            String::new(),
            None,
            Ok(vec![]),
            Ok(vec![]),
            Ok(vec![]),
        ));
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_synced());
        assert!(
            app.sms.is_empty(),
            "A successfully read empty list is different from failure"
        );
    }

    #[test]
    fn paused_domains_are_excluded_from_progress_without_fabricating_success() {
        let mut state = AutoSyncState::new();
        state.on_device_connected("pause-phone");
        state.mark_paused(AutoSyncDomain::ContextAndPanorama, "用户暂停采集");
        for domain in AutoSyncDomain::ALL
            .into_iter()
            .filter(|d| *d != AutoSyncDomain::ContextAndPanorama)
        {
            state.mark_synced(domain, 2);
        }
        assert_eq!(state.completed_count(), 6);
        assert_eq!(state.participating_count(), 6);
        assert_eq!(state.paused_count(), 1);
        assert_eq!(state.overall_percent(), 100);
        assert!(!state.is_syncing());
        assert!(state.top_bar_label(true).contains("1项暂停"));
        assert!(state.sidebar_status_line(true).contains("6/6"));
        assert!(!state
            .domain_status(AutoSyncDomain::ContextAndPanorama)
            .is_synced());
        assert!(state
            .domain_status(AutoSyncDomain::ContextAndPanorama)
            .error()
            .is_none());
        assert_eq!(state.to_json(true)["domains"][6]["status"], "paused");
        state.mark_failed(AutoSyncDomain::AppsAndUpdates, "真实读取失败");
        assert!(state.overall_percent() < 100);
        assert!(state.top_bar_label(true).contains("待重试"));
        assert_eq!(
            state.domain_status(AutoSyncDomain::AppsAndUpdates).error(),
            Some("真实读取失败")
        );
    }
    #[test]
    fn paused_context_reconciliation_neither_times_out_nor_forces_collection() {
        let mut app = connected_app_with_cached_data();
        app.context.persistent = true;
        app.context.initialized = true;
        app.handle_context_action(&super::context_state::ContextAction::Pause);
        app.auto_sync.domains[AutoSyncDomain::ContextAndPanorama.index()] =
            AutoSyncDomainStatus::Syncing {
                started_at: Instant::now() - Duration::from_secs(21),
            };
        app.reconcile_auto_sync_domains_from_state();
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::ContextAndPanorama)
            .is_paused());
        assert_eq!(app.auto_sync.completed_count(), 6);
        assert_eq!(app.auto_sync.overall_percent(), 100);
        assert!(!app.auto_sync.awaiting_result[AutoSyncDomain::ContextAndPanorama.index()]);
        app.trigger_auto_context_refresh();
        assert!(
            !app.context.force,
            "policy pause must not enqueue a collection"
        );
        app.auto_sync
            .mark_failed(AutoSyncDomain::AppsAndUpdates, "应用读取失败");
        app.reconcile_auto_sync_domains_from_state();
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::AppsAndUpdates)
            .is_failed());
    }
    #[test]
    fn auto_sync_state_progress_and_labels_transition_cleanly() {
        let mut state = AutoSyncState::new();
        assert_eq!(state.completed_count(), 0);
        assert_eq!(state.overall_percent(), 0);
        assert!(!state.is_syncing());
        assert_eq!(state.top_bar_label(false), "未连接手机");

        state.on_device_connected("STEPXTEST8DB89E4B8927");
        assert!(state.is_syncing());
        assert_eq!(state.completed_count(), 0);
        assert_eq!(state.overall_percent(), 0);
        assert_eq!(state.top_bar_label(true), "全量同步 0/7 · 0%");
        assert_eq!(
            state.active_domain(),
            Some(AutoSyncDomain::DeviceAndSummary)
        );

        state.mark_syncing(AutoSyncDomain::DeviceAndSummary);
        assert_eq!(state.overall_percent(), 0);
        assert_eq!(
            state.active_domain(),
            Some(AutoSyncDomain::DeviceAndSummary)
        );

        state.mark_synced(AutoSyncDomain::DeviceAndSummary, 6);
        state.mark_synced(AutoSyncDomain::MessagesAndCalls, 42);
        state.mark_synced(AutoSyncDomain::CalendarAndSuite, 8);
        state.mark_syncing(AutoSyncDomain::AmooAndTasks);
        assert_eq!(state.completed_count(), 3);
        assert_eq!(state.overall_percent(), 43);
        assert_eq!(state.top_bar_label(true), "全量同步 3/7 · 43%");
        assert_eq!(state.active_domain(), Some(AutoSyncDomain::AmooAndTasks));

        for domain in [
            AutoSyncDomain::AmooAndTasks,
            AutoSyncDomain::AppsAndUpdates,
            AutoSyncDomain::GalleryAndStorage,
            AutoSyncDomain::ContextAndPanorama,
        ] {
            state.mark_synced(domain, 10);
        }
        assert_eq!(state.completed_count(), 7);
        assert_eq!(state.overall_percent(), 100);
        assert!(!state.is_syncing());
        assert_eq!(state.cycle_count, 1);
        assert!(state.last_full_sync_ms > 0);
        assert_eq!(state.top_bar_label(true), "双端实时同步 · 100%");
        assert_eq!(state.total_synced_items(), 6 + 42 + 8 + 40);

        let json = state.to_json(true);
        assert_eq!(json["completed_count"], 7);
        assert_eq!(json["overall_percent"], 100);
        assert_eq!(json["is_syncing"], false);
        assert_eq!(json["domains"].as_array().unwrap().len(), 7);
    }

    #[test]
    fn reconcile_auto_sync_domains_reflects_loaded_companion_state() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device {
            serial: "STEPXTEST8DB89E4B8927".into(),
            transport: "STEPXTEST8DB89E4B8927".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        app.summary = "电量 90%\n存储 256G".into();
        app.sms_loaded = true;
        app.phone_loaded = true;
        app.push.loaded = true;
        app.back_screen.stepos_suite.checked = true;
        app.amoo_gateway_online = true;
        app.apps_loaded = true;
        app.back_screen.gallery_checked = true;
        app.auto_sync.context_triggered_for_cycle = true;

        app.reconcile_auto_sync_domains_from_state();
        assert_eq!(app.auto_sync.completed_count(), 7);
        assert_eq!(app.auto_sync.overall_percent(), 100);
        assert!(!app.auto_sync.is_syncing());
    }

    #[test]
    fn stale_while_revalidate_and_reselection_preserve_cached_records() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        let dev = Device {
            serial: "STEPXTEST8DB89E4B8927".into(),
            transport: "STEPXTEST8DB89E4B8927".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        app.selected = Some(dev.clone());
        app.summary = "电量 95%".into();
        app.sms = vec![Message {
            provider_id: Some(1),
            address: "10086".into(),
            body: "余额充足".into(),
            date_ms: 1_700_000_000_000,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: Some(1),
        }];
        app.sms_loaded = true;
        app.call_logs = vec![CallRecord {
            number: "13800009367".into(),
            name: "测试联系人".into(),
            call_type: 1,
            date_ms: 1_700_000_000_000,
            duration_sec: 60,
        }];
        app.contacts = vec![ContactEntry {
            name: "测试联系人".into(),
            number: "13800009367".into(),
        }];
        app.phone_loaded = true;

        // Transient error on refresh preserves cached SMS, call logs, and contacts
        app.apply_sms_data(Err("transient timeout".into()));
        assert_eq!(app.sms.len(), 1);
        assert!(app.sms_loaded);
        assert!(app.sms_error.is_none());

        app.apply_phone_data(
            Err("transient call log timeout".into()),
            Err("transient contact timeout".into()),
        );
        assert_eq!(app.call_logs.len(), 1);
        assert_eq!(app.contacts.len(), 1);
        assert!(app.call_logs_error.is_none());
        assert!(app.contacts_error.is_none());

        // Re-selecting the same physical phone on a wireless transport preserves summary & cached data
        let wireless_dev = Device {
            serial: "STEPXTEST8DB89E4B8927".into(),
            transport: "192.168.1.88:5555".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        app.apply(Outcome::Selected(
            wireless_dev,
            String::new(),
            None,
            None,
            Err("wireless busy".into()),
            Err("wireless busy".into()),
            Err("wireless busy".into()),
        ));
        assert_eq!(app.summary, "电量 95%");
        assert_eq!(app.sms.len(), 1);
        assert_eq!(app.call_logs.len(), 1);
        assert_eq!(app.contacts.len(), 1);
    }

    #[test]
    fn incremental_poll_stays_stable_in_flight_but_reports_a_failed_receipt(
    ) {
        let mut app = connected_app_with_cached_data();
        app.reconcile_auto_sync_domains_from_state();
        assert_eq!(app.auto_sync.completed_count(), 7);
        assert_eq!(app.auto_sync.overall_percent(), 100);

        let (request_id, started) = app.auto_sync.begin_message_read().unwrap();
        *started.lock().unwrap() = Some(Instant::now());
        app.reconcile_auto_sync_domains_from_state();
        assert!(
            app.auto_sync.domain_status(AutoSyncDomain::MessagesAndCalls).is_synced(),
            "Background incremental poll must keep Synced status instead of oscillating to 6/7 (86%)"
        );
        assert_eq!(app.auto_sync.completed_count(), 7);
        assert_eq!(app.auto_sync.overall_percent(), 100);

        // In-flight work does not flicker the sidebar, but a real failure is
        // visible and cannot renew the successful receipt from cached flags.
        app.apply(Outcome::AutoSyncMessages {
            device_serial: "sync-truth-unit".into(),
            request_id,
            result: Err("transient wireless timeout".into()),
        });
        assert!(app
            .auto_sync
            .domain_status(AutoSyncDomain::MessagesAndCalls)
            .is_failed());
        assert!(app.auto_sync.overall_percent() < 100);
        assert!(app.sms_loaded && app.phone_loaded && app.push.loaded);
    }

    #[test]
    fn idle_phone_amoo_observation_completes_domain_instead_of_marking_failed() {
        let mut app = connected_app_with_cached_data();
        app.amoo_gateway_online = false;
        app.amoo_sessions.clear();
        app.amoo_snapshot = None;
        app.auto_sync.domains[AutoSyncDomain::AmooAndTasks.index()] =
            AutoSyncDomainStatus::Syncing {
                started_at: Instant::now(),
            };
        app.apply(Outcome::AmooObservationUnavailable);
        assert!(
            app.auto_sync
                .domain_status(AutoSyncDomain::AmooAndTasks)
                .is_synced(),
            "Idle phone Amoo observer must mark AmooAndTasks as Synced rather than Failed"
        );
    }
}

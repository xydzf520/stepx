//! Unified Home Reminder Lifecycle, Auto-Expiration Policy Engine, and
//! Persistent Manual-Expire ("手动过期 · 不再提醒" / "稍后提醒" / "屏蔽来源") Governance Store.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::{OnceLock, RwLock};

/// Canonical categories for all Home page reminders, capsules, and cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderCategory {
    /// 短信/通知/剪贴板验证码 (OTP)
    OtpCode,
    /// 即时配送/外卖/生鲜到家 (美团外卖、叮咚买菜、盒马、闪购、打车等)
    TakeoutAndMobility,
    /// 快递驿站/快递柜取件码 & 待取包裹
    ExpressPickup,
    /// 在途快递物流动态 (未到站)
    ExpressInTransit,
    /// 直接 @提及 与 BOSS直聘/招聘沟通
    DirectMentionAndCareer,
    /// 普通未读 IM/短信消息
    UnreadMessage,
    /// 未接来电提醒
    MissedCall,
    /// 今日日程与会议提醒 (含会前准备清单)
    ScheduleAndMeeting,
    /// 日程 × 闹钟跨源冲突提醒
    ScheduleConflict,
    /// 天气与通勤/生活预约跨源提醒
    WeatherAndLife,
    /// 跨端工作台待确认任务 & 台账待处理事项 (ContextLedger / ActionLedger)
    LedgerAndTaskIssue,
    /// 倒计时与手机实况胶囊
    LiveTimer,
}

impl ReminderCategory {
    pub const ALL: [ReminderCategory; 11] = [
        Self::OtpCode,
        Self::TakeoutAndMobility,
        Self::ExpressPickup,
        Self::ExpressInTransit,
        Self::DirectMentionAndCareer,
        Self::UnreadMessage,
        Self::MissedCall,
        Self::ScheduleAndMeeting,
        Self::ScheduleConflict,
        Self::WeatherAndLife,
        Self::LedgerAndTaskIssue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OtpCode => "验证码",
            Self::TakeoutAndMobility => "外卖/买菜/出行",
            Self::ExpressPickup => "快递取件码",
            Self::ExpressInTransit => "在途快递",
            Self::DirectMentionAndCareer => "@提及/招聘",
            Self::UnreadMessage => "未读消息",
            Self::MissedCall => "未接来电",
            Self::ScheduleAndMeeting => "今日日程/会议",
            Self::ScheduleConflict => "日程×闹钟冲突",
            Self::WeatherAndLife => "天气与生活提醒",
            Self::LedgerAndTaskIssue => "台账与待办事项",
            Self::LiveTimer => "倒计时实况",
        }
    }

    /// Default auto-expire TTL in minutes (user-customizable per category).
    pub fn default_ttl_mins(self) -> u32 {
        match self {
            Self::OtpCode => 15,                 // 15 分钟
            Self::TakeoutAndMobility => 120,     // 2 小时 (送达后或配送更新后)
            Self::ExpressPickup => 24 * 60,      // 24 小时
            Self::ExpressInTransit => 12 * 60,   // 12 小时
            Self::DirectMentionAndCareer => 720, // 12 小时
            Self::UnreadMessage => 720,          // 12 小时
            Self::MissedCall => 720,             // 12 小时
            Self::ScheduleAndMeeting => 15,      // 会议结束后 15 分钟自动过期
            Self::ScheduleConflict => 60,        // 会议开始后立即过期 / 60m
            Self::WeatherAndLife => 360,         // 6 小时
            Self::LedgerAndTaskIssue => 48 * 60, // 48 小时
            Self::LiveTimer => 120,              // 2 小时
        }
    }

    pub fn preset_options_mins(self) -> &'static [(u32, &'static str)] {
        match self {
            Self::OtpCode => &[(5, "5分钟"), (15, "15分钟"), (30, "30分钟")],
            Self::TakeoutAndMobility => &[(30, "30分钟"), (120, "2小时"), (360, "6小时")],
            Self::ScheduleAndMeeting => &[(0, "结束即刻"), (15, "结束后15m"), (60, "结束后1h")],
            Self::ExpressPickup => &[(360, "6小时"), (1440, "24小时"), (2880, "48小时")],
            _ => &[(60, "1小时"), (360, "6小时"), (720, "12小时"), (1440, "24小时")],
        }
    }
}

/// Canonical fingerprint specification for a Home reminder item across all modules & capsules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderTargetSpec {
    /// Primary canonical key (e.g. `code:4-2-1086`, `delivery:叮咚买菜`, `im:com.tencent.mm::群`, `cal:101`, `ledger:item-1`).
    pub primary_key: String,
    /// Additional alias keys to suppress simultaneously across all Home surfaces & capsules.
    #[serde(default)]
    pub alias_keys: Vec<String>,
    pub category: ReminderCategory,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub source_label: String,
    /// Optional source key for "屏蔽此来源/联系人" (e.g. `pkg:com.sankuai.meituan`, `im:com.tencent.mm::群`, `sms:10698888`).
    #[serde(default)]
    pub mute_source_key: Option<String>,
    /// Optional copyable value (OTP code, pickup code, tracking number, phone number, or summary).
    #[serde(default)]
    pub copy_text: Option<String>,
    #[serde(default)]
    pub copy_label: Option<String>,
    /// Timestamp (ms) of the underlying event/message when suppressed.
    /// For recurring channels (`im:`, `sms:`, `delivery:`, `call:`), a newer event with `event_ts_ms > source_ts_ms`
    /// is treated as a fresh occurrence and NOT suppressed (unless the source is in `muted_sources`).
    #[serde(default)]
    pub source_ts_ms: Option<i64>,
    /// Optional content revision (for `ContextLedger` / `ActionLedger` items).
    #[serde(default)]
    pub source_revision: Option<String>,
}

impl ReminderTargetSpec {
    pub fn new(
        primary_key: impl Into<String>,
        category: ReminderCategory,
        title: impl Into<String>,
    ) -> Self {
        Self {
            primary_key: primary_key.into(),
            alias_keys: Vec::new(),
            category,
            title: title.into(),
            subtitle: String::new(),
            source_label: String::new(),
            mute_source_key: None,
            copy_text: None,
            copy_label: None,
            source_ts_ms: None,
            source_revision: None,
        }
    }

    pub fn with_aliases(mut self, aliases: impl IntoIterator<Item = String>) -> Self {
        for a in aliases {
            let trimmed = a.trim();
            if !trimmed.is_empty()
                && trimmed != self.primary_key
                && !self.alias_keys.iter().any(|k| k == trimmed)
            {
                self.alias_keys.push(trimmed.to_string());
            }
        }
        self
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn with_source(
        mut self,
        source_label: impl Into<String>,
        mute_source_key: Option<String>,
    ) -> Self {
        self.source_label = source_label.into();
        self.mute_source_key = mute_source_key.filter(|s| !s.trim().is_empty());
        self
    }

    pub fn with_copy(mut self, text: impl Into<String>, label: impl Into<String>) -> Self {
        let t = text.into();
        if !t.trim().is_empty() {
            self.copy_text = Some(t);
            self.copy_label = Some(label.into());
        }
        self
    }

    pub fn with_source_ts(mut self, ts_ms: i64) -> Self {
        if ts_ms > 0 {
            self.source_ts_ms = Some(ts_ms);
        }
        self
    }

    pub fn all_keys(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(1 + self.alias_keys.len());
        out.push(self.primary_key.clone());
        for a in &self.alias_keys {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
        out
    }
}

/// How a reminder is governed by user action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ReminderDisposition {
    /// 手动过期 · 不再提醒
    ManualExpired,
    /// 稍后提醒 (Hidden until `until_ms`, then automatically re-surfaces)
    Snoozed { until_ms: i64 },
    /// 自定义过期时间 (Visible while `now_ms < expire_at_ms`, auto-expires when `now_ms >= expire_at_ms`)
    CustomExpireAt { expire_at_ms: i64 },
}

impl ReminderDisposition {
    pub fn status_badge(&self, now_ms: i64) -> String {
        match self {
            Self::ManualExpired => "已手动过期 · 不再提醒".to_string(),
            Self::Snoozed { until_ms } => {
                if *until_ms > now_ms {
                    let mins = ((*until_ms - now_ms) / 60_000).max(1);
                    if mins >= 60 {
                        format!("稍后提醒 · 剩余 {}h{}m", mins / 60, mins % 60)
                    } else {
                        format!("稍后提醒 · 剩余 {}m", mins)
                    }
                } else {
                    "稍后提醒已到期".to_string()
                }
            }
            Self::CustomExpireAt { expire_at_ms } => {
                if *expire_at_ms <= now_ms {
                    "已到定时过期时间".to_string()
                } else {
                    let mins = ((*expire_at_ms - now_ms) / 60_000).max(1);
                    format!("将于 {}m 后自动过期", mins)
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderGovernanceEntry {
    pub spec: ReminderTargetSpec,
    pub disposition: ReminderDisposition,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutedReminderSource {
    pub source_key: String,
    pub label: String,
    pub muted_at_ms: i64,
}

/// Notice recorded when an item is automatically expired by the policy engine,
/// allowing the user to see why it disappeared or restore it if needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoExpiredNotice {
    pub primary_key: String,
    pub category: ReminderCategory,
    pub title: String,
    pub reason: String,
    pub expired_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeReminderGovernance {
    pub schema: u32,
    pub revision: u64,
    /// Keyed by `spec.primary_key`.
    pub entries: BTreeMap<String, ReminderGovernanceEntry>,
    /// Secondary index mapping `alias_key -> primary_key`.
    pub alias_index: BTreeMap<String, String>,
    /// Muted sources (`pkg:...`, `im:...`, `sms:...`).
    pub muted_sources: BTreeMap<String, MutedReminderSource>,
    /// Per-category TTL overrides in minutes (`ReminderCategory -> mins`).
    pub category_ttl_mins: BTreeMap<ReminderCategory, u32>,
    /// Keys explicitly restored/whitelisted from auto-expiration during the current day.
    pub force_keep_keys: BTreeMap<String, i64>,
    /// Recently auto-expired reminders (capped to 24 items for inspection in the Governance panel).
    #[serde(skip)]
    pub recent_auto_expired: Vec<AutoExpiredNotice>,
}

impl Default for HomeReminderGovernance {
    fn default() -> Self {
        Self {
            schema: 1,
            revision: 1,
            entries: BTreeMap::new(),
            alias_index: BTreeMap::new(),
            muted_sources: BTreeMap::new(),
            category_ttl_mins: BTreeMap::new(),
            force_keep_keys: BTreeMap::new(),
            recent_auto_expired: Vec::new(),
        }
    }
}

#[cfg(not(test))]
static ACTIVE_GOVERNANCE: OnceLock<RwLock<HomeReminderGovernance>> = OnceLock::new();

#[cfg(not(test))]
fn global_governance_lock() -> &'static RwLock<HomeReminderGovernance> {
    ACTIVE_GOVERNANCE.get_or_init(|| RwLock::new(HomeReminderGovernance::load_from_disk()))
}

#[cfg(test)]
thread_local! {
    // Unit fixtures that expire/mute a reminder must not alter concurrent home
    // report tests or read a developer's saved reminder preferences.
    static TEST_GOVERNANCE: std::cell::RefCell<HomeReminderGovernance> =
        std::cell::RefCell::new(HomeReminderGovernance::default());
}

fn mutate_active_governance(f: impl FnOnce(&mut HomeReminderGovernance)) {
    #[cfg(test)]
    TEST_GOVERNANCE.with(|g| f(&mut g.borrow_mut()));
    #[cfg(not(test))]
    if let Ok(mut guard) = global_governance_lock().write() {
        f(&mut guard);
    }
}

/// Returns a clone of the current active `HomeReminderGovernance` snapshot.
pub fn active_governance() -> HomeReminderGovernance {
    #[cfg(test)]
    return TEST_GOVERNANCE.with(|g| g.borrow().clone());
    #[cfg(not(test))]
    global_governance_lock()
        .read()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Cache keys need the revision, not a cloned copy of every rule and alias.
pub fn active_governance_revision() -> u64 {
    #[cfg(test)]
    return TEST_GOVERNANCE.with(|g| g.borrow().revision);
    #[cfg(not(test))]
    global_governance_lock()
        .read()
        .map(|g| g.revision)
        .unwrap_or(1)
}

/// Synchronizes the global active `HomeReminderGovernance` snapshot.
pub fn set_active_governance(gov: &HomeReminderGovernance) {
    mutate_active_governance(|guard| *guard = gov.clone());
}

/// Record an auto-expired notice in the global governance state (deduplicated by `primary_key`).
pub fn record_auto_expired_notice(
    primary_key: impl Into<String>,
    category: ReminderCategory,
    title: impl Into<String>,
    reason: impl Into<String>,
    now_ms: i64,
) {
    let key = primary_key.into();
    let title = title.into();
    if key.is_empty() || title.trim().is_empty() {
        return;
    }
    mutate_active_governance(|guard| {
        if guard.recent_auto_expired.iter().any(|n| n.primary_key == key) {
            return;
        }
        guard.recent_auto_expired.insert(
            0,
            AutoExpiredNotice {
                primary_key: key,
                category,
                title,
                reason: reason.into(),
                expired_at_ms: now_ms,
            },
        );
        guard.recent_auto_expired.truncate(24);
    });
}

pub fn recent_auto_expired_notices() -> Vec<AutoExpiredNotice> {
    active_governance().recent_auto_expired
}

impl HomeReminderGovernance {
    pub fn storage_path() -> PathBuf {
        #[cfg(test)]
        {
            return std::env::temp_dir().join(format!(
                "stepx-test-home_reminder_governance-{}-{:?}.json",
                std::process::id(), std::thread::current().id()
            ));
        }
        #[cfg(not(test))]
        {
            crate::resources::user_config().join("stepx-companion/home_reminder_governance.json")
        }
    }

    pub fn load_from_disk() -> Self {
        let path = Self::storage_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let mut parsed: Self = serde_json::from_str(&raw).unwrap_or_default();
        parsed.rebuild_alias_index();
        if parsed.revision == 0 {
            parsed.revision = 1;
        }
        parsed
    }

    pub fn load() -> Self {
        let gov = Self::load_from_disk();
        set_active_governance(&gov);
        gov
    }

    pub fn save(&self) {
        set_active_governance(self);
        let path = Self::storage_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }

    pub fn rebuild_alias_index(&mut self) {
        self.alias_index.clear();
        for (pk, entry) in &self.entries {
            self.alias_index.insert(pk.clone(), pk.clone());
            for alias in &entry.spec.alias_keys {
                if !alias.trim().is_empty() {
                    self.alias_index.insert(alias.clone(), pk.clone());
                }
            }
        }
    }

    pub fn lookup_entry(&self, key: &str) -> Option<&ReminderGovernanceEntry> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(entry) = self.entries.get(trimmed) {
            return Some(entry);
        }
        let pk = self.alias_index.get(trimmed)?;
        self.entries.get(pk)
    }

    /// Returns whether `key` is force-kept (restored from auto-expiration) at `now_ms`.
    pub fn is_force_kept(&self, key: &str, now_ms: i64) -> bool {
        self.force_keep_keys
            .get(key.trim())
            .is_some_and(|until| *until > now_ms)
    }

    /// Evaluates whether a single canonical key is currently suppressed at `now_ms`.
    pub fn is_key_suppressed(
        &self,
        key: &str,
        event_ts_ms: Option<i64>,
        event_rev: Option<&str>,
        now_ms: i64,
    ) -> bool {
        let Some(entry) = self.lookup_entry(key) else {
            return false;
        };
        let k = key.trim();
        let is_recurring_channel = k.starts_with("im:")
            || k.starts_with("sms:")
            || k.starts_with("delivery:")
            || k.starts_with("call:")
            || k.starts_with("task:");
        if is_recurring_channel {
            if let (Some(bound_ts), Some(ev_ts)) = (entry.spec.source_ts_ms, event_ts_ms) {
                if bound_ts > 0 && ev_ts > bound_ts {
                    return false;
                }
            }
        }
        if let (Some(bound_rev), Some(ev_rev)) =
            (entry.spec.source_revision.as_deref(), event_rev)
        {
            if !bound_rev.is_empty() && !ev_rev.is_empty() && bound_rev != ev_rev {
                return false;
            }
        }
        match entry.disposition {
            ReminderDisposition::ManualExpired => true,
            ReminderDisposition::Snoozed { until_ms } => now_ms < until_ms,
            ReminderDisposition::CustomExpireAt { expire_at_ms } => now_ms >= expire_at_ms,
        }
    }

    /// Evaluates whether any of the provided canonical keys or `mute_source_key` is suppressed at `now_ms`.
    pub fn is_any_key_suppressed(
        &self,
        keys: &[String],
        event_ts_ms: Option<i64>,
        event_rev: Option<&str>,
        mute_source_key: Option<&str>,
        now_ms: i64,
    ) -> bool {
        if let Some(src) = mute_source_key.map(str::trim).filter(|s| !s.is_empty()) {
            if self.muted_sources.contains_key(src) {
                return true;
            }
        }
        keys.iter()
            .any(|k| self.is_key_suppressed(k, event_ts_ms, event_rev, now_ms))
    }

    /// Custom expire timestamp (`expire_at_ms`) if set for any of `keys`.
    pub fn custom_expire_at_ms(&self, keys: &[String]) -> Option<i64> {
        for k in keys {
            if let Some(entry) = self.lookup_entry(k) {
                if let ReminderDisposition::CustomExpireAt { expire_at_ms } = entry.disposition {
                    return Some(expire_at_ms);
                }
            }
        }
        None
    }

    /// Effective auto-expiration TTL in milliseconds for `category`.
    pub fn effective_ttl_ms(&self, category: ReminderCategory) -> i64 {
        let mins = self
            .category_ttl_mins
            .get(&category)
            .copied()
            .unwrap_or_else(|| category.default_ttl_mins());
        (mins as i64) * 60_000
    }

    pub fn effective_ttl_mins(&self, category: ReminderCategory) -> u32 {
        self.category_ttl_mins
            .get(&category)
            .copied()
            .unwrap_or_else(|| category.default_ttl_mins())
    }

    pub fn format_ttl_label(ttl_mins: Option<u32>) -> String {
        match ttl_mins {
            None => "默认时效".to_string(),
            Some(0) => "结束即刻".to_string(),
            Some(mins) if mins >= 60 && mins % 60 == 0 => format!("{}小时", mins / 60),
            Some(mins) if mins >= 60 => format!("{}小时{}分钟", mins / 60, mins % 60),
            Some(mins) => format!("{mins}分钟"),
        }
    }

    /// Marks `spec` as permanently manually expired ("手动过期 · 不再提醒").
    pub fn manual_expire(&mut self, spec: ReminderTargetSpec, now_ms: i64) {
        self.upsert_entry(spec, ReminderDisposition::ManualExpired, now_ms);
    }

    /// Snoozes `spec` until `until_ms` ("稍后提醒").
    pub fn snooze_until(&mut self, spec: ReminderTargetSpec, until_ms: i64, now_ms: i64) {
        self.upsert_entry(spec, ReminderDisposition::Snoozed { until_ms }, now_ms);
    }

    /// Sets a custom expiration timestamp `expire_at_ms` for `spec`.
    pub fn set_custom_expire_at(&mut self, spec: ReminderTargetSpec, expire_at_ms: i64, now_ms: i64) {
        self.upsert_entry(
            spec,
            ReminderDisposition::CustomExpireAt { expire_at_ms },
            now_ms,
        );
    }

    fn upsert_entry(
        &mut self,
        spec: ReminderTargetSpec,
        disposition: ReminderDisposition,
        now_ms: i64,
    ) {
        let pk = spec.primary_key.trim().to_string();
        if pk.is_empty() {
            return;
        }
        self.force_keep_keys.remove(&pk);
        for a in &spec.alias_keys {
            self.force_keep_keys.remove(a.trim());
        }
        self.entries.insert(
            pk,
            ReminderGovernanceEntry {
                spec,
                disposition,
                created_at_ms: now_ms,
            },
        );
        self.rebuild_alias_index();
        self.bump_revision();
        self.save();
    }

    /// Restores a suppressed or auto-expired reminder by any of its keys (`primary_key` or `alias_key`).
    pub fn restore_reminder(&mut self, key: &str, now_ms: i64) -> Option<ReminderTargetSpec> {
        let trimmed = key.trim();
        let pk = self
            .alias_index
            .get(trimmed)
            .cloned()
            .unwrap_or_else(|| trimmed.to_string());
        let removed = self.entries.remove(&pk).map(|e| e.spec);
        // Also whitelist in `force_keep_keys` for 24h in case it was auto-expired
        self.force_keep_keys
            .insert(pk.clone(), now_ms.saturating_add(86_400_000));
        self.force_keep_keys
            .insert(trimmed.to_string(), now_ms.saturating_add(86_400_000));
        mutate_active_governance(|guard| {
            guard
                .recent_auto_expired
                .retain(|n| n.primary_key != pk && n.primary_key != trimmed);
        });
        self.recent_auto_expired
            .retain(|n| n.primary_key != pk && n.primary_key != trimmed);
        self.rebuild_alias_index();
        self.bump_revision();
        self.save();
        removed
    }

    pub fn mute_source(&mut self, source_key: impl Into<String>, label: impl Into<String>, now_ms: i64) {
        let sk = source_key.into().trim().to_string();
        if sk.is_empty() {
            return;
        }
        self.muted_sources.insert(
            sk.clone(),
            MutedReminderSource {
                source_key: sk,
                label: label.into(),
                muted_at_ms: now_ms,
            },
        );
        self.bump_revision();
        self.save();
    }

    pub fn unmute_source(&mut self, source_key: &str) {
        if self.muted_sources.remove(source_key.trim()).is_some() {
            self.bump_revision();
            self.save();
        }
    }

    pub fn set_category_ttl_mins(&mut self, category: ReminderCategory, ttl_mins: Option<u32>) {
        match ttl_mins {
            Some(m) => {
                self.category_ttl_mins.insert(category, m);
            }
            None => {
                self.category_ttl_mins.remove(&category);
            }
        }
        self.bump_revision();
        self.save();
    }

    pub fn clear_all_suppressed(&mut self) {
        self.entries.clear();
        self.alias_index.clear();
        self.recent_auto_expired.clear();
        mutate_active_governance(|guard| {
            guard.recent_auto_expired.clear();
        });
        self.bump_revision();
        self.save();
    }
}

// ============================================================================
// Canonical Key Builders (Deterministic across all Home surfaces & Capsules)
// ============================================================================

pub fn normalize_phone_or_addr(raw: &str) -> String {
    let trimmed = raw.trim();
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 11 && digits.starts_with("86") {
        digits[2..].to_string()
    } else if !digits.is_empty() {
        digits
    } else {
        trimmed.to_uppercase()
    }
}

pub fn canonical_keys_for_code_or_parcel(
    code: Option<&str>,
    tracking_no: Option<&str>,
    notif_key: Option<&str>,
    sms_addr: Option<&str>,
) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(c) = code.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("code:{}", c.to_uppercase()));
    }
    if let Some(t) = tracking_no.map(str::trim).filter(|s| !s.is_empty()) {
        let k = format!("code:{}", t.to_uppercase());
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    if let Some(nk) = notif_key.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("notif:{nk}"));
    }
    if let Some(addr) = sms_addr.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("sms:{}", normalize_phone_or_addr(addr)));
    }
    keys
}

pub fn canonical_keys_for_delivery(
    badge: &str,
    notif_key: Option<&str>,
    thread_id: Option<&str>,
    sms_addr: Option<&str>,
) -> Vec<String> {
    let mut keys = Vec::new();
    let b = badge.trim();
    if !b.is_empty() {
        keys.push(format!("delivery:{b}"));
    }
    if let Some(nk) = notif_key.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("notif:{nk}"));
    }
    if let Some(tid) = thread_id.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("im:{tid}"));
    }
    if let Some(addr) = sms_addr.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("sms:{}", normalize_phone_or_addr(addr)));
    }
    keys
}

pub fn canonical_keys_for_im_thread(thread_id: &str, notif_key: Option<&str>) -> Vec<String> {
    let mut keys = vec![format!("im:{}", thread_id.trim())];
    if let Some(nk) = notif_key.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(format!("notif:{nk}"));
    }
    keys
}

pub fn canonical_keys_for_calendar_event(event_id: i64) -> Vec<String> {
    vec![
        format!("cal:{event_id}"),
        format!("conflict:conflict-cal-{event_id}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_governance_lifecycle_alias_revival_and_ttl_contract() {
        let mut gov = HomeReminderGovernance::default();
        let now_ms = 1_760_000_000_000i64;

        // 1. Cross-surface alias suppression for pickup code + parcel + notification
        let aliases = canonical_keys_for_code_or_parcel(
            Some("4-2-1086"),
            Some("SF1086"),
            Some("notif-sf-1"),
            Some("10698000"),
        );
        let spec = ReminderTargetSpec::new(
            "code:4-2-1086",
            ReminderCategory::ExpressPickup,
            "取件码 · 4-2-1086",
        )
        .with_aliases(aliases.clone());
        gov.manual_expire(spec, now_ms);

        for k in &aliases {
            assert!(
                gov.is_key_suppressed(k, None, None, now_ms),
                "Alias key {k} should be suppressed simultaneously"
            );
        }

        // 2. Recurring IM channel: newer timestamp revives the reminder automatically
        let im_spec = ReminderTargetSpec::new(
            "im:com.tencent.mm::产品群",
            ReminderCategory::UnreadMessage,
            "产品群",
        )
        .with_source_ts(now_ms);
        gov.manual_expire(im_spec, now_ms);
        assert!(gov.is_key_suppressed("im:com.tencent.mm::产品群", Some(now_ms), None, now_ms));
        assert!(
            !gov.is_key_suppressed(
                "im:com.tencent.mm::产品群",
                Some(now_ms + 5_000),
                None,
                now_ms + 5_000
            ),
            "Newer IM message timestamp must revive the reminder"
        );

        // 3. Snooze and CustomExpireAt time boundaries
        let snooze_spec = ReminderTargetSpec::new(
            "delivery:叮咚买菜",
            ReminderCategory::TakeoutAndMobility,
            "叮咚买菜配送中",
        );
        gov.snooze_until(snooze_spec.clone(), now_ms + 1_800_000, now_ms);
        assert!(gov.is_key_suppressed("delivery:叮咚买菜", None, None, now_ms + 600_000));
        assert!(!gov.is_key_suppressed("delivery:叮咚买菜", None, None, now_ms + 1_800_001));

        gov.set_custom_expire_at(snooze_spec, now_ms + 3_600_000, now_ms);
        assert!(!gov.is_key_suppressed("delivery:叮咚买菜", None, None, now_ms + 1_200_000));
        assert!(gov.is_key_suppressed("delivery:叮咚买菜", None, None, now_ms + 3_600_000));

        // 4. Restore whitelists force_keep_keys and clears suppression
        let restored = gov.restore_reminder("code:SF1086", now_ms);
        assert!(restored.is_some());
        assert!(!gov.is_key_suppressed("code:4-2-1086", None, None, now_ms));
        assert!(gov.is_force_kept("code:4-2-1086", now_ms + 60_000));

        // 5. Category TTL customization & Mute source
        gov.set_category_ttl_mins(ReminderCategory::OtpCode, Some(30));
        assert_eq!(gov.effective_ttl_mins(ReminderCategory::OtpCode), 30);
        assert_eq!(HomeReminderGovernance::format_ttl_label(Some(30)), "30分钟");
        assert_eq!(HomeReminderGovernance::format_ttl_label(Some(120)), "2小时");

        gov.mute_source("pkg:com.sankuai.meituan", "美团外卖", now_ms);
        assert!(gov.is_any_key_suppressed(
            &["delivery:美团外卖".to_string()],
            None,
            None,
            Some("pkg:com.sankuai.meituan"),
            now_ms,
        ));
        gov.unmute_source("pkg:com.sankuai.meituan");
        assert!(!gov.is_any_key_suppressed(
            &["delivery:美团外卖".to_string()],
            None,
            None,
            Some("pkg:com.sankuai.meituan"),
            now_ms,
        ));
        let _ = std::fs::remove_file(HomeReminderGovernance::storage_path());
        set_active_governance(&HomeReminderGovernance::default());
    }
}

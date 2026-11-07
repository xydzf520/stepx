//! Private, incremental context preparation. Disk and model work never runs in render.
use super::icons::UiIcon;
use super::theme::*;
use super::widgets::Btn;
use super::*;
use chrono::{Datelike, Local, Timelike};
use facloud_ui::{color::ColorU, fonts::Weight};
use ledger::outcomes::Target as OutcomeTarget;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use stepx_companion::{context_ledger as ledger, context_pipeline, home_ai, phone_context, sms};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
struct Scope {
    device: String,
    account: Option<String>,
}

const CONTEXT_FLOW_NODE_SIZE: f32 = 20.;

// Screen/keyguard state intentionally does not gate background data reads.
// Changes observed by notification sync accelerate collection; quiet phones
// still refresh Amoo history without requiring an app launch or user action.
fn background_phone_refresh_due(elapsed: Option<Duration>, source_changed: bool) -> bool {
    elapsed
        .is_none_or(|elapsed| elapsed >= Duration::from_secs(if source_changed { 15 } else { 60 }))
}

fn context_flow_mark(index: usize, active_stage: usize, failed: bool) -> widgets::StepMark {
    use widgets::StepMark;
    if index < active_stage {
        StepMark::Done
    } else if index == active_stage && failed {
        StepMark::Failed
    } else if index == active_stage {
        StepMark::Active
    } else {
        StepMark::Pending
    }
}

fn context_metric_fraction(count: usize, total: usize) -> f32 {
    if total == 0 {
        0.
    } else {
        (count as f32 / total as f32).clamp(0., 1.)
    }
}

/// Read-only geometry probes are absent from production builds. They let the
/// native UI review verify fixed circles and safe card bounds after resizing.
fn context_geometry_probe(id: String, element: widgets::El) -> widgets::El {
    #[cfg(feature = "ui-capture")]
    {
        super::scenario_check::control(id, element)
    }
    #[cfg(not(feature = "ui-capture"))]
    {
        let _ = id;
        element
    }
}
impl Scope {
    fn key(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AiRecord {
    target: String,
    revision: String,
    path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PeriodBatchIntent {
    source: stepx_companion::model_source::Source,
    requested_ms: i64,
    pending: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct ScopePreferences {
    enabled: bool,
    tasks: bool,
    phone: bool,
    automatic_ai: bool,
    return_outcomes: bool,
    discussion_cursor: phone_context::Cursor,
    since_ms: i64,
    phone_since_ms: i64,
    sms_cursor: Option<sms::Cursor>,
    sms_unsupported: bool,
    ai_records: Vec<AiRecord>,
    ai_attempts: Vec<String>,
    scheduled_attempts: BTreeMap<String, i64>,
    automatic_source: Option<stepx_companion::model_source::Source>,
    automatic_period_completed: BTreeMap<String, i64>,
    automatic_next_period: bool,
    period_batch: Option<PeriodBatchIntent>,
    ai_started_ms: i64,
}
impl Default for ScopePreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            tasks: true,
            phone: true,
            automatic_ai: false,
            return_outcomes: false,
            discussion_cursor: Default::default(),
            since_ms: 0,
            phone_since_ms: 0,
            sms_cursor: None,
            sms_unsupported: false,
            ai_records: vec![],
            ai_attempts: vec![],
            scheduled_attempts: BTreeMap::new(),
            automatic_source: None,
            automatic_period_completed: BTreeMap::new(),
            automatic_next_period: true,
            period_batch: None,
            ai_started_ms: 0,
        }
    }
}

impl ScopePreferences {
    fn phone_since(&self) -> i64 {
        if self.phone_since_ms > 0 {
            self.phone_since_ms
        } else {
            self.since_ms
        }
    }
    fn sms_after(&self) -> sms::Cursor {
        self.sms_cursor.unwrap_or(sms::Cursor {
            date_ms: self.phone_since(),
            id: 0,
        })
    }
    fn configure_collection(&mut self, previous: &Self, automatic: bool, now_ms: i64) {
        if self.phone {
            if automatic {
                // Enabling automatic collection never grants a historical backfill.
                if !previous.phone
                    || !previous.enabled
                    || (previous.since_ms == 0 && previous.phone_since_ms == 0)
                {
                    self.phone_since_ms = now_ms;
                    self.sms_cursor = None;
                    self.sms_unsupported = false;
                }
            } else {
                // The explicit one-time action includes a bounded initial lookback.
                // Later runs keep the earlier boundary and cursor, including paused gaps.
                let lookback = now_ms.saturating_sub(7 * 86_400_000).max(0);
                let previous_since = if previous.phone_since_ms > 0 {
                    previous.phone_since_ms
                } else if previous.phone {
                    previous.since_ms
                } else {
                    0
                };
                let since = if previous_since > 0 {
                    previous_since.min(lookback)
                } else {
                    lookback
                };
                if previous.phone_since_ms <= 0 || since < previous_since {
                    self.sms_cursor = None;
                }
                self.phone_since_ms = since;
                self.sms_unsupported = false;
            }
        }
        self.enabled = automatic;
        if self.since_ms == 0 {
            self.since_ms = now_ms;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    last_scope: Option<Scope>,
    pending_clear: Vec<Scope>,
    scopes: BTreeMap<String, ScopePreferences>,
}

#[derive(Clone, Debug)]
pub(super) enum ContextAction {
    Setup,
    Manage,
    Attention,
    ItemDetails(String),
    SourceTasks,
    SourcePhone,
    AutoAi,
    AutoReturn,
    GenerateOutcomes,
    GenerateCurrentPeriods,
    ReturnOutcomes(OutcomeTarget),
    Apply,
    PrepareOnce,
    Pause,
    Refresh,
    Period(ledger::Period),
    Open(PathBuf),
    ExternalOpen(PathBuf),
    ClosePreview,
    PreviewPage(usize),
    Send(PathBuf),
    DeepenItem(String),
    Clear,
    ConfirmClear,
    CancelClear,
    Coverage,
    ToggleItems,
    PickLink(String),
    CancelLink,
    Link(String, String),
    RejectLink(String, String),
    Separate(String),
    Relationships(String),
    RelationshipPage(usize),
    ToggleCandidates,
    Deepen,
    StopAi,
}

#[derive(Clone, Debug)]
struct TextSourceState {
    source: Option<stepx_companion::model_source::Source>,
    manual_available: bool,
    automatic_available: bool,
    automatic_reason: String,
}
fn load_text_source_state(scope: Option<&Scope>) -> TextSourceState {
    match stepx_companion::model_source::load() {
        Ok(preferences) => {
            let capability = if let Some(scope) = scope {
                home_ai::source_capabilities_for(
                    preferences.source,
                    &scope.device,
                    scope.account.as_deref(),
                )
            } else {
                home_ai::source_capabilities(preferences.source)
            };
            TextSourceState {
                source: Some(capability.source),
                manual_available: capability.manual_available,
                automatic_available: capability.automatic_available,
                automatic_reason: capability.automatic_reason,
            }
        }
        Err(error) => TextSourceState {
            source: None,
            manual_available: false,
            automatic_available: false,
            automatic_reason: format!("模型来源尚未确认：{error}"),
        },
    }
}
fn automatic_authorized(
    config: &ScopePreferences,
    source: Option<stepx_companion::model_source::Source>,
) -> bool {
    config.automatic_ai
        && match config.automatic_source {
            Some(allowed) => Some(allowed) == source,
            // Prior versions could only authorize automatic use of an independent API.
            None => source == Some(stepx_companion::model_source::Source::IndependentApi),
        }
}
fn automatic_error_can_recheck(error: &str) -> bool {
    if [
        "同一后台请求",
        "结果未确认",
        "已发出",
        "手动重试",
        "请求记录损坏",
        "请求记录无效",
        "不属于当前",
    ]
    .iter()
    .any(|text| error.contains(text))
    {
        return false;
    }
    [
        "请连接原手机",
        "请先连接",
        "手机离线",
        "手机传输连接已变化",
        "尚未确认手机账号",
        "手机账号尚未确认",
        "纯文本通道尚未",
        "未验证专用无工具",
        "模型来源尚未确认",
    ]
    .iter()
    .any(|text| error.contains(text))
}
fn automatic_period_key(period: &ledger::PeriodBrief) -> String {
    format!("{:?}:{}", period.kind, period.begin_ms)
}
const AUTOMATIC_PERIOD_QUIET_MS: i64 = 15_000;

#[derive(Default)]
struct CollectionHealth {
    phone_checked: bool,
    phone_issue: Option<String>,
}

enum WorkerOutput {
    Preferences(Preferences, TextSourceState),
    ModelAvailable(TextSourceState),
    AccountScopeResolved(String, Option<String>),
    AiStarted(home_ai::TextJob, Scope, AiTarget),
    LoadedOverview(ledger::Overview),
    OutcomesLoaded(
        ledger::Overview,
        Vec<context_pipeline::RunResult>,
        Vec<(OutcomeTarget, String)>,
        Option<home_ai::InferenceMeta>,
    ),
    Outcome(
        context_pipeline::RunResult,
        Option<home_ai::InferenceMeta>,
        bool,
    ),
    OutcomeSkipped(String),
    AutomaticInputChanged(bool),
    DiscussionRead(Result<phone_context::Batch, String>),
    Overview(ledger::Overview, Option<sms::Cursor>, bool),
    Captured(
        ledger::Overview,
        Option<sms::Cursor>,
        bool,
        Option<phone_context::Cursor>,
        Option<Vec<sms::Message>>,
        CollectionHealth,
    ),
    RelationChanged(ledger::Overview),
    Saved,
    Cleared(Scope),
    AiSaved(PathBuf, String, String),
}
struct Worker {
    generation: u64,
    scope: Option<Scope>,
    receiver: mpsc::Receiver<std::result::Result<WorkerOutput, String>>,
    saves_preferences: bool,
    clears_scope: bool,
    starts_ai: bool,
    automatic_ai: bool,
    edits_relation: bool,
    collects_context: bool,
}
#[derive(Clone)]
enum AiTarget {
    Period(ledger::PeriodBrief),
    Item(ledger::PreparedItem),
}
impl AiTarget {
    fn key(&self) -> String {
        match self {
            Self::Period(p) => format!("period:{:?}", p.kind),
            Self::Item(i) => format!("item:{}", i.id),
        }
    }
    fn revision(&self) -> &str {
        match self {
            Self::Period(p) => &p.evidence_revision,
            Self::Item(i) => &i.input_revision,
        }
    }
    fn attempt_key(&self, automatic: bool) -> String {
        if automatic {
            if let Self::Period(p) = self {
                return format!("scheduled:{:?}:{}", p.kind, p.begin_ms);
            }
        }
        format!("manual:{}:{}", self.key(), self.revision())
    }
    fn prompt(&self) -> &str {
        match self {
            Self::Period(p) => &p.prompt,
            Self::Item(i) => &i.prompt,
        }
    }
}
fn record_ai_attempt(
    config: &mut ScopePreferences,
    target: &AiTarget,
    automatic: bool,
    now_ms: i64,
) {
    config.ai_started_ms = now_ms;
    if automatic {
        config
            .scheduled_attempts
            .retain(|_, at| *at >= now_ms.saturating_sub(90 * 86_400_000));
        config
            .scheduled_attempts
            .insert(target.attempt_key(true), now_ms);
    } else {
        config.ai_attempts.push(target.attempt_key(false));
        if config.ai_attempts.len() > 64 {
            config.ai_attempts.remove(0);
        }
    }
}
struct AiRun {
    generation: u64,
    scope: Scope,
    target: AiTarget,
    job: home_ai::TextJob,
}
struct PendingAi {
    scope: Scope,
    target: AiTarget,
    device: Option<Device>,
    use_local: bool,
}
struct AiSave {
    scope: Scope,
    target: AiTarget,
    markdown: String,
}
struct PendingRelation {
    scope: Scope,
    edit: ledger::RelationEdit,
}

const CONTEXT_PREVIEW_LIMIT: u64 = 512 * 1024;
const CONTEXT_PREVIEW_PAGE_CHARS: usize = 4000;

struct ContextPreview {
    request: u64,
    path: PathBuf,
    title: String,
    loading: bool,
    pages: Vec<String>,
    page: usize,
    scroll: ClippedScrollStateHandle,
    error: Option<String>,
}
struct PreviewWorker {
    generation: u64,
    scope: Scope,
    request: u64,
    path: PathBuf,
    receiver: mpsc::Receiver<std::result::Result<Vec<String>, String>>,
}

fn context_document_path(
    root: &std::path::Path,
    scope: &Scope,
    path: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let root = std::fs::canonicalize(root)?;
    let scope_dir = std::fs::canonicalize(
        root.join(ledger::scope_id(&scope.device, scope.account.as_deref())),
    )?;
    anyhow::ensure!(
        scope_dir.starts_with(&root),
        "资料目录已变化，请重新整理后打开"
    );
    let resolved = std::fs::canonicalize(path)?;
    anyhow::ensure!(
        resolved.starts_with(&scope_dir),
        "这份文件不属于当前设备与账号的整理资料"
    );
    let extension = resolved
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    anyhow::ensure!(
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "txt"
        ),
        "这里只预览整理后的文本资料"
    );
    anyhow::ensure!(std::fs::metadata(&resolved)?.is_file(), "所选资料不是文件");
    Ok(resolved)
}

fn context_display_citations(text: &str, citations: &mut BTreeMap<String, usize>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(begin) = rest.find("〔E:") {
        out.push_str(&rest[..begin]);
        let candidate = &rest[begin + "〔E:".len()..];
        if let Some(end) = candidate.find('〕') {
            let id = &candidate[..end];
            if id.len() == 32 && id.bytes().all(|ch| ch.is_ascii_hexdigit()) {
                let next = citations.len() + 1;
                let number = *citations.entry(id.to_owned()).or_insert(next);
                out.push_str(&format!("〔来源{number}〕"));
                rest = &candidate[end + '〕'.len_utf8()..];
                continue;
            }
        }
        out.push_str("〔E:");
        rest = candidate;
    }
    out.push_str(rest);
    out
}

pub(super) fn context_preview_plain_text(source: &str) -> String {
    let mut citations = BTreeMap::new();
    let mut output = String::with_capacity(source.len());
    let mut fence: Option<(u8, usize)> = None;
    for raw in source.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let trimmed = line.trim_start();
        let marker = trimmed.as_bytes().first().copied();
        let count = marker
            .map(|marker| trimmed.bytes().take_while(|byte| *byte == marker).count())
            .unwrap_or(0);
        if let Some((open_marker, open_count)) = fence {
            output.push_str(raw);
            if marker == Some(open_marker)
                && count >= open_count
                && trimmed[count..].trim().is_empty()
            {
                fence = None;
            }
        } else if matches!(marker, Some(b'`' | b'~')) && count >= 3 {
            fence = Some((marker.unwrap(), count));
            output.push_str(raw);
        } else {
            // Drop all metadata: destinations, actions and styles never reach a widget.
            let plain: String = markdown_parser::parse_inline_markdown(line)
                .into_iter()
                .map(|fragment| fragment.text)
                .collect();
            output.push_str(&context_display_citations(&plain, &mut citations));
            if raw.ends_with('\n') {
                output.push('\n');
            }
        }
    }
    output
}

fn read_context_preview(
    root: &std::path::Path,
    scope: &Scope,
    path: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    use std::io::Read;
    let path = context_document_path(root, scope, path)?;
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= CONTEXT_PREVIEW_LIMIT,
        "文件超过 512 KiB，暂不支持在首页预览"
    );
    let mut bytes = Vec::new();
    file.take(CONTEXT_PREVIEW_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= CONTEXT_PREVIEW_LIMIT,
        "文件读取时超过 512 KiB，请在其他应用查看"
    );
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("资料不是 UTF-8 文本，无法在首页预览"))?;
    anyhow::ensure!(
        !text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')),
        "资料包含非文本内容，无法在首页预览"
    );
    // Format only the display copy, before pagination so fenced code stays literal across pages.
    let text = context_preview_plain_text(&text);
    let mut pages = vec![];
    let mut begin = 0;
    for (index, (offset, _)) in text.char_indices().enumerate() {
        if index > 0 && index % CONTEXT_PREVIEW_PAGE_CHARS == 0 {
            pages.push(text[begin..offset].to_string());
            begin = offset;
        }
    }
    pages.push(text[begin..].to_string());
    Ok(pages)
}

/// Display-only markers: these never create a toggle or modify task state.
fn context_preview_list_item(line: &str) -> Option<(&'static str, &str, bool)> {
    for (prefix, marker) in [
        ("- [ ] ", "☐"),
        ("* [ ] ", "☐"),
        ("- [x] ", "☑"),
        ("- [X] ", "☑"),
        ("* [x] ", "☑"),
        ("* [X] ", "☑"),
    ] {
        if let Some(text) = line.strip_prefix(prefix) {
            return Some((marker, text, true));
        }
    }
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(|text| ("•", text, false))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ContextPanel {
    #[default]
    Attention,
    Review,
    Manage,
    Item(String),
}

pub(super) struct ContextUi {
    root: PathBuf,
    pub(super) persistent: bool,
    pub(super) initialized: bool,
    preferences: Preferences,
    scope: Option<Scope>,
    generation: u64,
    worker: Option<Worker>,
    generation_worker: Option<Worker>,
    discussion_worker: Option<Worker>,
    identity_worker: Option<Worker>,
    phone_issue: Option<String>,
    pending_discussion: Option<phone_context::Batch>,
    discussion_issue: Option<String>,
    discussion_backfill: bool,
    preview: Option<ContextPreview>,
    preview_worker: Option<PreviewWorker>,
    preview_request: u64,
    ai: Option<AiRun>,
    ai_save: Option<AiSave>,
    pending_ai: Option<PendingAi>,
    pending_relation: Option<PendingRelation>,
    pending_outcomes: Vec<(
        Scope,
        OutcomeTarget,
        Device,
        bool,
        bool,
        Option<String>,
        Option<(stepx_companion::model_source::Source, String)>,
    )>,
    outcomes: Vec<context_pipeline::RunResult>,
    work_cancel: Arc<AtomicBool>,
    write_failed: bool,
    model_available: bool,
    automatic_text_available: bool,
    model_source: Option<stepx_companion::model_source::Source>,
    automatic_reason: String,
    automatic_error: bool,
    automatic_retry_at: Option<Instant>,
    period_stability: BTreeMap<String, (String, i64)>,
    last_inference: Option<home_ai::InferenceMeta>,
    model_checked_at: Instant,
    session_warning: Option<String>,
    dirty: bool,
    pub(super) needs_load: bool,
    pub(super) force: bool,
    checked_at: Instant,
    phone_checked_at: Option<Instant>,
    phone_source_fingerprint: Option<u64>,
    account_resolved_for: Option<String>,
    account_resolve_checked_at: Option<Instant>,
    resolved_account: Option<(String, Option<String>)>,
    pending_captured_sms: Option<(Scope, Vec<sms::Message>)>,
    collection_result: Option<(Instant, Option<String>)>,
    // A stored snapshot with limited source coverage is usable data, not a
    // failed write. Keep this separate from the warning shown with conclusions.
    collection_persisted: bool,
    pending_fingerprint: Option<(u64, Instant)>,
    last_fingerprint: Option<u64>,
    pub(super) overview: ledger::Overview,
    period: ledger::Period,
    panel: ContextPanel,
    setup: Option<ScopePreferences>,
    confirm_clear: bool,
    coverage_open: bool,
    pub(super) show_all_items: bool,
    link_picker: Option<String>,
    relationship_details: Option<String>,
    relationship_page: usize,
    show_all_candidates: bool,
    status: String,
    error: Option<String>,
}

impl ContextUi {
    pub(super) fn productivity_scope(&self) -> Option<(String, Option<String>)> {
        self.scope
            .as_ref()
            .map(|scope| (scope.device.clone(), scope.account.clone()))
    }
    pub(super) fn productivity_problem(&self) -> Option<&str> {
        self.error
            .as_deref()
            .or_else(|| {
                self.collection_result
                    .as_ref()
                    .and_then(|(_, issue)| issue.as_deref())
            })
            .or(self.session_warning.as_deref())
    }
    pub(super) fn has_only_coverage_warning(&self) -> bool {
        self.collection_persisted()
            && self
                .collection_result
                .as_ref()
                .is_some_and(|(_, issue)| issue.is_some())
            && self.error.is_none()
            && self.session_warning.is_none()
            && !self.write_failed
    }
    fn flow_progress(&self, prepared_stage: usize) -> (usize, bool) {
        if self.write_failed {
            return (1, true);
        }
        let outcomes = self.current_outcomes();
        if outcomes
            .iter()
            .any(|result| result.delivery_error.is_some())
        {
            return (3, true);
        }
        if self.error.is_some() {
            return (2, true);
        }
        // Partial source coverage does not invalidate a verified delivery.
        if !outcomes.is_empty()
            && outcomes.iter().all(|result| {
                !result.bundle.files.is_empty()
                    && result.bundle.files.iter().all(|file| {
                        result.deliveries.iter().any(|d| {
                            d.kind == file.kind && d.sha256 == file.sha256 && d.verified_ms > 0
                        })
                    })
            })
        {
            return (4, false);
        }
        (prepared_stage, false)
    }
    pub(super) fn productivity_clearing(&self) -> bool {
        self.scope
            .as_ref()
            .is_some_and(|s| self.preferences.pending_clear.contains(s))
            || self.worker.as_ref().is_some_and(|w| w.clears_scope)
    }
    pub(super) fn productivity_generation_running(&self) -> bool {
        self.ai_active()
    }
    pub(super) fn productivity_status(&self) -> &str {
        &self.status
    }
    pub(super) fn productivity_model_ready(&self) -> bool {
        self.model_available
    }
    pub(super) fn productivity_collection_enabled(&self) -> bool {
        self.config().enabled
    }
    pub(super) fn accepted_phone_inference(
        &self,
        device: Option<&Device>,
        account: Option<&str>,
    ) -> Option<&home_ai::InferenceMeta> {
        let scope = self.scope.as_ref()?;
        let device = device?;
        let account = account?;
        if scope.device != device.serial || scope.account.as_deref() != Some(account) {
            return None;
        }
        self.last_inference.as_ref().filter(|meta| {
            meta.source == stepx_companion::model_source::Source::PhoneAccount
                && meta.account_scope.as_deref() == Some(account)
        })
    }
    fn cache_inference(&mut self, meta: Option<home_ai::InferenceMeta>) {
        self.last_inference = meta.filter(|meta| {
            meta.source != stepx_companion::model_source::Source::PhoneAccount
                || self.scope.as_ref().is_some_and(|scope| {
                    scope.account.is_some() && meta.account_scope == scope.account
                })
        });
    }

    pub(super) fn new(persistent: bool) -> Self {
        Self {
            root: stepx_companion::resources::user_state().join("stepx-companion/context"),
            persistent,
            initialized: !persistent,
            preferences: Preferences::default(),
            scope: None,
            generation: 0,
            worker: None,
            generation_worker: None,
            discussion_worker: None,
            identity_worker: None,
            phone_issue: None,
            pending_discussion: None,
            discussion_issue: None,
            discussion_backfill: false,
            preview: None,
            preview_worker: None,
            preview_request: 0,
            ai: None,
            ai_save: None,
            pending_ai: None,
            pending_relation: None,
            pending_outcomes: vec![],
            outcomes: vec![],
            work_cancel: Arc::new(AtomicBool::new(false)),
            write_failed: false,
            model_available: false,
            automatic_text_available: false,
            model_source: None,
            automatic_reason: "正在核对模型来源能力".into(),
            automatic_error: false,
            automatic_retry_at: None,
            period_stability: BTreeMap::new(),
            last_inference: None,
            model_checked_at: Instant::now(),
            session_warning: None,
            dirty: false,
            needs_load: false,
            force: false,
            checked_at: Instant::now() - Duration::from_secs(10),
            phone_checked_at: None,
            phone_source_fingerprint: None,
            account_resolved_for: None,
            account_resolve_checked_at: None,
            resolved_account: None,
            pending_captured_sms: None,
            collection_result: None,
            collection_persisted: false,
            pending_fingerprint: None,
            last_fingerprint: None,
            overview: Default::default(),
            period: ledger::Period::Day,
            panel: ContextPanel::Attention,
            setup: None,
            confirm_clear: false,
            coverage_open: false,
            show_all_items: false,
            link_picker: None,
            relationship_details: None,
            relationship_page: 0,
            show_all_candidates: false,
            status: String::new(),
            error: None,
        }
    }
    fn cache_text_source(&mut self, state: TextSourceState) {
        self.model_available = state.manual_available;
        self.automatic_text_available = state.automatic_available;
        self.model_source = state.source;
        self.automatic_reason = state.automatic_reason;
    }
    pub(super) fn automatic_source_status(&self) -> (bool, String) {
        (self.automatic_text_available, self.automatic_reason.clone())
    }
    fn config(&self) -> ScopePreferences {
        self.scope
            .as_ref()
            .and_then(|s| self.preferences.scopes.get(&s.key()))
            .cloned()
            .unwrap_or_default()
    }
    fn set_config(&mut self, config: ScopePreferences) {
        if let Some(scope) = &self.scope {
            self.preferences.scopes.insert(scope.key(), config);
            self.dirty = true;
        }
    }
    fn invalidate(&mut self) {
        self.work_cancel.store(true, Ordering::Release);
        self.work_cancel = Arc::new(AtomicBool::new(false));
        self.pending_outcomes.clear();
        self.pending_discussion = None;
        self.discussion_issue = None;
        self.discussion_backfill = false;
        self.phone_issue = None;
        self.generation = self.generation.wrapping_add(1);
        self.close_preview();
        self.ai = None;
        self.ai_save = None;
        self.pending_ai = None;
        self.pending_relation = None;
        self.pending_fingerprint = None;
        self.last_fingerprint = None;
        self.force = false;
    }
    pub(super) fn device_cleared(&mut self) {
        // Keep the last local overview for offline reading, but reject old model/results.
        self.invalidate();
        self.phone_checked_at = None;
        self.phone_source_fingerprint = None;
        self.account_resolved_for = None;
        self.account_resolve_checked_at = None;
        self.resolved_account = None;
        self.pending_captured_sms = None;
        self.collection_result = None;
        self.last_inference = None;
        self.automatic_retry_at = None;
        self.automatic_error = false;
        self.model_checked_at = Instant::now() - Duration::from_secs(61);
        if self.model_source == Some(stepx_companion::model_source::Source::PhoneAccount) {
            self.automatic_text_available = false;
        }
    }
    fn change_scope(&mut self, scope: Scope) -> bool {
        if self.scope.as_ref() == Some(&scope) {
            return false;
        }
        let same_device = self
            .scope
            .as_ref()
            .is_some_and(|old| old.device == scope.device);
        let was_forced = self.force;
        self.invalidate();
        if same_device {
            self.force = was_forced;
        }
        self.scope = Some(scope.clone());
        self.preferences.last_scope = Some(scope);
        self.dirty = true;
        self.needs_load = true;
        self.overview = Default::default();
        self.outcomes.clear();
        self.last_inference = None;
        self.period_stability.clear();
        self.automatic_error = false;
        self.automatic_retry_at = None;
        self.model_checked_at = Instant::now() - Duration::from_secs(61);
        if self.model_source == Some(stepx_companion::model_source::Source::PhoneAccount) {
            self.automatic_text_available = false;
            self.automatic_reason = "正在核对当前设备账号的手机纯文本通道".into();
        }
        self.pending_captured_sms = None;
        self.collection_result = None;
        self.setup = None;
        self.panel = ContextPanel::Attention;
        self.confirm_clear = false;
        self.coverage_open = false;
        self.show_all_items = false;
        self.link_picker = None;
        self.relationship_details = None;
        self.relationship_page = 0;
        self.show_all_candidates = false;
        self.phone_checked_at = None;
        self.phone_source_fingerprint = None;
        self.status.clear();
        self.error = None;
        self.session_warning = None;
        true
    }
    fn ai_active(&self) -> bool {
        self.generation_worker.is_some()
            || !self.pending_outcomes.is_empty()
            || self.ai.is_some()
            || self.pending_ai.is_some()
            || self.worker.as_ref().is_some_and(|w| w.starts_ai)
    }
    fn account_resolution_due(&self, serial: &str, now: Instant) -> bool {
        self.account_resolved_for.as_deref() != Some(serial)
            || self.account_resolve_checked_at.is_none_or(|checked| {
                now.saturating_duration_since(checked) >= Duration::from_secs(30)
            })
    }
    fn take_captured_sms(&mut self, scope: &Scope) -> Option<Vec<sms::Message>> {
        self.pending_captured_sms
            .take()
            .and_then(|(source, messages)| (source == *scope).then_some(messages))
    }
    fn collection_warning(&self) -> Option<String> {
        let config = self.config();
        let mut issues = Vec::new();
        if config.phone {
            if let Some(issue) = &self.phone_issue {
                issues.push(issue.as_str());
            }
        }
        if config.tasks {
            if let Some(issue) = &self.discussion_issue {
                issues.push(issue.as_str());
            }
        }
        (!issues.is_empty()).then(|| issues.join("；"))
    }
    fn relation_active(&self) -> bool {
        self.pending_relation.is_some() || self.worker.as_ref().is_some_and(|w| w.edits_relation)
    }
    pub(super) fn running(&self) -> bool {
        self.preview_worker.is_some()
            || self.worker.is_some()
            || self.generation_worker.is_some()
            || self.discussion_worker.is_some()
            || self.identity_worker.is_some()
            || self.ai.is_some()
            || self.pending_ai.is_some()
            || !self.pending_outcomes.is_empty()
            || self.pending_relation.is_some()
    }
    /// Fresh ledger collection only. Long model jobs and preview workers do not
    /// determine whether this synchronization wave actually collected data.
    pub(super) fn background_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "collecting": self.worker.as_ref().is_some_and(|w| w.collects_context),
            "generating": self.generation_worker.is_some() || self.ai.is_some(),
            "reading_discussions": self.discussion_worker.is_some(),
            "history_backfill": self.discussion_backfill,
            "resolving_account": self.identity_worker.is_some(),
            "collection_age_ms": self.collection_result.as_ref().map(|(at, _)| at.elapsed().as_millis()),
            "collection_complete": self.collection_result.as_ref().map(|(_, issue)| issue.is_none()),
            "source_issue": self.collection_warning(),
            "generation_error": self.error,
            "status": self.status,
        })
    }
    pub(super) fn collection_result_since(&self, started: Instant) -> Option<Result<(), &str>> {
        let (completed, error) = self.collection_result.as_ref()?;
        if *completed < started {
            return None;
        }
        Some(match error {
            Some(error) => Err(error.as_str()),
            None => Ok(()),
        })
    }
    pub(super) fn collection_persisted(&self) -> bool {
        self.collection_result.is_some() && self.collection_persisted
    }
    pub(super) fn collection_receipt_at(&self) -> Option<Instant> {
        self.collection_result.as_ref().map(|(at, _)| *at)
    }
    pub(super) fn has_pending_or_active_sync(&self) -> bool {
        // Generation/preview activity must not suppress the next data wave.
        self.worker.is_some()
            || self.discussion_worker.is_some()
            || self.identity_worker.is_some()
            || (self.persistent
                && (!self.initialized || self.needs_load || self.dirty || self.force))
    }
    fn current_outcomes(&self) -> Vec<&context_pipeline::RunResult> {
        if !self.initialized || self.needs_load {
            return vec![];
        }
        let Some(scope) = &self.scope else {
            return vec![];
        };
        let scope_id = ledger::scope_id(&scope.device, scope.account.as_deref());
        self.outcomes
            .iter()
            .filter(|result| {
                result.bundle.scope_id == scope_id
                    && match &result.bundle.target {
                        OutcomeTarget::Item(id) => self.overview.items.iter().any(|item| {
                            &item.id == id && item.input_revision == result.bundle.input_revision
                        }),
                        OutcomeTarget::Period(kind) => self.overview.periods.iter().any(|period| {
                            &period.kind == kind
                                && period.evidence_revision == result.bundle.input_revision
                        }),
                    }
            })
            .collect()
    }
    fn outcome_path(&self, target: &OutcomeTarget) -> Option<PathBuf> {
        self.current_outcomes()
            .into_iter()
            .find(|r| &r.bundle.target == target)
            .and_then(|r| {
                r.bundle
                    .files
                    .iter()
                    .find(|f| f.kind != "todos" && f.path.extension().is_some_and(|s| s == "md"))
            })
            .map(|f| f.path.clone())
    }
    fn readable_outcome_path(&self, target: &OutcomeTarget) -> Option<PathBuf> {
        self.outcome_path(target).or_else(|| {
            let scope = self.scope.as_ref()?;
            let scope_id = ledger::scope_id(&scope.device, scope.account.as_deref());
            self.outcomes
                .iter()
                .filter(|r| r.bundle.scope_id == scope_id && &r.bundle.target == target)
                .max_by_key(|r| r.bundle.created_ms)
                .and_then(|r| r.bundle.files.iter().find(|f| f.kind != "todos"))
                .map(|f| f.path.clone())
        })
    }
    fn outcome_for_path(&self, path: &std::path::Path) -> Option<&context_pipeline::RunResult> {
        self.current_outcomes()
            .into_iter()
            .find(|result| result.bundle.files.iter().any(|file| file.path == path))
    }
    fn is_historical_outcome(&self, path: &std::path::Path) -> bool {
        self.outcome_for_path(path).is_none()
            && self
                .outcomes
                .iter()
                .any(|r| r.bundle.files.iter().any(|f| f.path == path))
    }
    fn document_title(&self, path: &std::path::Path) -> Option<String> {
        if let Some(result) = self
            .current_outcomes()
            .into_iter()
            .find(|r| r.bundle.files.iter().any(|f| f.path == path))
        {
            return Some(result.bundle.title.clone());
        }
        if let Some(scope) = &self.scope {
            let scope_id = ledger::scope_id(&scope.device, scope.account.as_deref());
            if let Some(result) = self.outcomes.iter().find(|r| {
                r.bundle.scope_id == scope_id && r.bundle.files.iter().any(|f| f.path == path)
            }) {
                return Some(format!("{} · 上次成稿（有新资料）", result.bundle.title));
            }
        }
        self.overview
            .items
            .iter()
            .find(|item| item.path == path)
            .map(|item| item.title.clone())
            .or_else(|| {
                self.overview
                    .periods
                    .iter()
                    .find(|period| period.path == path)
                    .map(|period| period.title.clone())
            })
            .or_else(|| {
                self.home_ai_results()
                    .into_iter()
                    .find(|(_, candidate)| candidate == path)
                    .map(|(title, _)| format!("{title} · 草稿"))
            })
            .or_else(|| {
                self.config()
                    .ai_records
                    .iter()
                    .find(|record| record.path == path)
                    .map(|_| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "AI 草稿".into())
                    })
            })
    }
    pub(super) fn search_documents(&self) -> Vec<(String, String, PathBuf, i64)> {
        if !self.initialized || self.needs_load {
            return vec![];
        }
        let Some(scope) = &self.scope else {
            return vec![];
        };
        if self.preferences.pending_clear.contains(scope) {
            return vec![];
        }
        let sid = ledger::scope_id(&scope.device, scope.account.as_deref());
        let mut docs: Vec<_> = self
            .overview
            .items
            .iter()
            .filter(|i| i.scope_id == sid)
            .map(|i| {
                (
                    i.title.clone(),
                    i.summary.clone(),
                    i.path.clone(),
                    i.updated_ms,
                )
            })
            .collect();
        docs.extend(
            self.overview
                .periods
                .iter()
                .filter(|p| p.scope_id == sid)
                .map(|p| {
                    (
                        p.title.clone(),
                        p.summary.clone(),
                        p.path.clone(),
                        self.overview.updated_ms,
                    )
                }),
        );
        for r in self.current_outcomes() {
            for f in &r.bundle.files {
                docs.push((
                    format!("{} · {}", r.bundle.title, f.kind),
                    String::new(),
                    f.path.clone(),
                    r.bundle.created_ms,
                ));
            }
        }
        docs.extend(
            self.home_ai_results()
                .into_iter()
                .map(|(title, path)| (title, String::new(), path, 0)),
        );
        docs
    }
    /// Saved model drafts only; derived entirely from the loaded current scope.
    /// File existence and canonical path checks remain in the preview worker.
    fn home_ai_results(&self) -> Vec<(String, PathBuf)> {
        if !self.initialized || self.needs_load {
            return vec![];
        }
        let Some(scope) = &self.scope else {
            return vec![];
        };
        if self.preferences.pending_clear.contains(scope) {
            return vec![];
        }
        let Some(config) = self.preferences.scopes.get(&scope.key()) else {
            return vec![];
        };
        let scope_id = ledger::scope_id(&scope.device, scope.account.as_deref());
        let scope_dir = self.root.join(&scope_id);
        let mut paths = std::collections::HashSet::new();
        config
            .ai_records
            .iter()
            .rev()
            .filter_map(|record| {
                if !record.path.starts_with(&scope_dir)
                    || record
                        .path
                        .components()
                        .any(|component| component == std::path::Component::ParentDir)
                {
                    return None;
                }
                let title = self
                    .overview
                    .items
                    .iter()
                    .find(|item| {
                        item.scope_id == scope_id
                            && record.target == format!("item:{}", item.id)
                            && !item.input_revision.is_empty()
                            && record.revision == item.input_revision
                    })
                    .map(|item| {
                        if item.title.trim().is_empty() {
                            "事项草稿".into()
                        } else {
                            item.title.clone()
                        }
                    })
                    .or_else(|| {
                        self.overview
                            .periods
                            .iter()
                            .find(|period| {
                                period.scope_id == scope_id
                                    && record.target == format!("period:{:?}", period.kind)
                                    && !period.evidence_revision.is_empty()
                                    && record.revision == period.evidence_revision
                            })
                            .map(|period| {
                                if period.title.trim().is_empty() {
                                    period.kind.label().into()
                                } else {
                                    period.title.clone()
                                }
                            })
                    })?;
                paths
                    .insert(record.path.clone())
                    .then(|| (title, record.path.clone()))
            })
            .take(2)
            .collect()
    }
    fn close_preview(&mut self) {
        self.preview_request = self.preview_request.wrapping_add(1);
        self.preview = None;
    }
    fn open_preview(&mut self, path: &std::path::Path) {
        let Some(title) = self.document_title(path) else {
            self.error = Some("这份资料已不在当前来源中，请重新选择。".into());
            return;
        };
        self.preview_request = self.preview_request.wrapping_add(1);
        self.preview = Some(ContextPreview {
            request: self.preview_request,
            path: path.to_path_buf(),
            title,
            loading: true,
            pages: vec![],
            page: 0,
            scroll: Default::default(),
            error: None,
        });
        self.start_preview();
    }
    fn start_preview(&mut self) {
        if self.preview_worker.is_some() {
            return;
        }
        let Some(preview) = self.preview.as_ref().filter(|preview| preview.loading) else {
            return;
        };
        let Some(scope) = self.scope.clone() else {
            return;
        };
        let path = preview.path.clone();
        let root = self.root.clone();
        let (tx, receiver) = mpsc::channel();
        self.preview_worker = Some(PreviewWorker {
            generation: self.generation,
            scope: scope.clone(),
            request: preview.request,
            path: path.clone(),
            receiver,
        });
        // A separate, bounded reader keeps navigation and context preparation responsive.
        std::thread::spawn(move || {
            let _ = tx.send(
                read_context_preview(&root, &scope, &path).map_err(|error| error.to_string()),
            );
        });
    }
    fn poll_preview(&mut self) -> bool {
        let result = match self
            .preview_worker
            .as_ref()
            .map(|worker| worker.receiver.try_recv())
        {
            Some(Ok(result)) => result,
            Some(Err(mpsc::TryRecvError::Disconnected)) => Err("预览读取意外结束，请重试".into()),
            _ => {
                self.start_preview();
                return false;
            }
        };
        let worker = self.preview_worker.take().unwrap();
        let allowed = self.document_title(&worker.path).is_some();
        if worker.generation == self.generation && self.scope.as_ref() == Some(&worker.scope) {
            if let Some(preview) = self
                .preview
                .as_mut()
                .filter(|preview| preview.request == worker.request && preview.path == worker.path)
            {
                preview.loading = false;
                match result {
                    Ok(pages) if allowed => {
                        preview.pages = pages;
                        preview.error = None;
                    }
                    Ok(_) => preview.error = Some("资料已更新，请关闭后重新选择。".into()),
                    Err(error) => preview.error = Some(error),
                }
            }
        }
        self.start_preview();
        true
    }
    fn panel_title(&self) -> &'static str {
        if !self.initialized || self.needs_load {
            "正在读取整理记录"
        } else if self.error.is_some() {
            "整理需要处理"
        } else if self.ai_active() || self.ai_save.is_some() {
            "正在生成草稿"
        } else if self.force
            || self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.scope.is_some())
        {
            "正在整理资料"
        } else if self.overview.evidence_count > 0 {
            "讨论与成果"
        } else if self.config().enabled {
            "等待可整理的资料"
        } else if self.config().since_ms > 0 {
            "整理已暂停"
        } else {
            "把讨论变成成果"
        }
    }
    fn selected_period(&self) -> Option<&ledger::PeriodBrief> {
        self.overview.periods.iter().find(|p| p.kind == self.period)
    }
    fn visible_items(&self) -> &[ledger::PreparedItem] {
        let count = if self.show_all_items {
            self.overview.items.len()
        } else {
            self.overview.items.len().min(2)
        };
        &self.overview.items[..count]
    }
    fn attention_items_ranked(&self) -> Vec<&ledger::PreparedItem> {
        let now_ms = Local::now().timestamp_millis();
        let mut items: Vec<_> = self
            .overview
            .items
            .iter()
            .filter(|item| {
                item.presentation.as_ref().is_some_and(|presentation| {
                    presentation.priority > 0
                        && presentation
                            .valid_until_ms
                            .is_none_or(|until| until > now_ms)
                })
            })
            .collect();
        items.sort_by_key(|item| {
            let presentation = item.presentation.as_ref().unwrap();
            let schedule_at = if presentation.primary_action
                == stepx_companion::context_review::MatterAction::PrepareSchedule
            {
                presentation.valid_until_ms.unwrap_or(i64::MAX)
            } else {
                i64::MAX
            };
            (
                std::cmp::Reverse(presentation.priority),
                schedule_at,
                std::cmp::Reverse(item.updated_ms),
            )
        });
        items
    }
    fn attention_items(&self) -> Vec<&ledger::PreparedItem> {
        let mut items = self.attention_items_ranked();
        items.truncate(2);
        items
    }
    fn reconcile_panel(&mut self) {
        if matches!(&self.panel, ContextPanel::Item(id) if !self.overview.items.iter().any(|item| &item.id == id))
        {
            self.panel = ContextPanel::Manage;
        }
    }
    fn detail_items(&self) -> Vec<&ledger::PreparedItem> {
        if let ContextPanel::Item(id) = &self.panel {
            self.overview
                .items
                .iter()
                .filter(|item| &item.id == id)
                .collect()
        } else {
            self.visible_items().iter().collect()
        }
    }
    fn contains_source_object(&self, id: &str) -> bool {
        self.overview
            .items
            .iter()
            .any(|item| item.source_objects.iter().any(|source| source.id == id))
            || self
                .overview
                .link_candidates
                .iter()
                .any(|candidate| candidate.left_object == id || candidate.right_object == id)
    }
    fn start_pending_relation(&mut self) -> bool {
        if self.worker.is_some() || self.write_failed {
            return false;
        }
        let Some(pending) = self.pending_relation.take() else {
            return false;
        };
        if self.scope.as_ref() != Some(&pending.scope)
            || self.preferences.pending_clear.contains(&pending.scope)
        {
            return false;
        }
        let root = self.root.clone();
        let scope = pending.scope.clone();
        self.start_worker(Some(scope), move || {
            let now = Local::now();
            Ok(WorkerOutput::RelationChanged(ledger::apply_relation(
                &root,
                &pending.scope.device,
                pending.scope.account.as_deref(),
                pending.edit,
                now.timestamp_millis(),
                now.offset().local_minus_utc(),
            )?))
        });
        self.worker.as_mut().unwrap().edits_relation = true;
        true
    }
    fn start_worker<F>(&mut self, scope: Option<Scope>, work: F)
    where
        F: FnOnce() -> anyhow::Result<WorkerOutput> + Send + 'static,
    {
        let (tx, receiver) = mpsc::channel();
        self.worker = Some(Worker {
            generation: self.generation,
            scope,
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        std::thread::spawn(move || {
            let _ = tx.send(work().map_err(|e| e.to_string()));
        });
    }
    fn poll_worker(&mut self) -> bool {
        // Poll data independently of model latency. A slow model must never
        // occupy the single collection slot or starve its completed result.
        let ready = |worker: &Option<Worker>| match worker.as_ref().map(|w| w.receiver.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Some(Err("整理任务意外结束，可手动重试".into()))
            }
            _ => None,
        };
        let (worker, result) = if let Some(result) = ready(&self.worker) {
            (self.worker.take().unwrap(), result)
        } else if let Some(result) = ready(&self.generation_worker) {
            (self.generation_worker.take().unwrap(), result)
        } else if let Some(result) = ready(&self.discussion_worker) {
            (self.discussion_worker.take().unwrap(), result)
        } else if let Some(result) = ready(&self.identity_worker) {
            (self.identity_worker.take().unwrap(), result)
        } else {
            return false;
        };
        // A completed deletion must retire its durable tombstone even after switching scopes.
        if let Ok(WorkerOutput::Cleared(scope)) = &result {
            self.preferences.pending_clear.retain(|s| s != scope);
            self.dirty = true;
        }
        if worker.generation != self.generation
            || (worker.scope.is_some() && worker.scope != self.scope)
        {
            return true;
        }
        match result {
            Ok(WorkerOutput::Preferences(prefs, state)) => {
                self.preferences = prefs;
                for config in self.preferences.scopes.values_mut() {
                    if config.period_batch.take().is_some() {
                        self.dirty = true;
                    }
                }
                self.cache_text_source(state);
                self.initialized = true;
            }
            Ok(WorkerOutput::OutcomesLoaded(overview, outcomes, recoverable, inference)) => {
                self.cache_inference(inference);
                self.overview = overview;
                self.outcomes = outcomes;
                self.status.clear();
                self.error = None;
                let mut config = self.config();
                let mut changed = false;
                if let Some(batch) = config.period_batch.as_mut() {
                    let before = batch.pending.len();
                    batch.pending.retain(|key, revision| {
                        !self.outcomes.iter().any(|result| {
                            self.overview.periods.iter().any(|period| {
                                automatic_period_key(period) == *key
                                    && result.bundle.target == OutcomeTarget::Period(period.kind)
                                    && result.bundle.input_revision == *revision
                            })
                        })
                    });
                    changed |= before != batch.pending.len();
                    if batch.pending.is_empty() {
                        config.period_batch = None;
                        changed = true;
                    }
                }
                for (target, revision) in recoverable {
                    let key =
                        match target {
                            OutcomeTarget::Item(id)
                                if self.overview.items.iter().any(|item| {
                                    item.id == id && item.input_revision == revision
                                }) =>
                            {
                                format!("outcome:item:{id}:{revision}")
                            }
                            OutcomeTarget::Period(kind)
                                if self.overview.periods.iter().any(|period| {
                                    period.kind == kind && period.evidence_revision == revision
                                }) =>
                            {
                                format!("outcome:period:{kind:?}:{revision}")
                            }
                            _ => continue,
                        };
                    changed |= config.scheduled_attempts.remove(&key).is_some();
                }
                if changed {
                    self.set_config(config);
                }
            }
            Ok(WorkerOutput::AutomaticInputChanged(explicit_batch)) => {
                self.needs_load = true;
                self.error = None;
                if explicit_batch {
                    self.pending_outcomes
                        .retain(|pending| pending.4 || pending.6.is_none());
                    self.status =
                        "批量资料或模型来源已变化，已停止这次余下任务；请核对后再次点击整理".into();
                } else {
                    self.status =
                        "排队后资料或来源发生变化，本次未发出模型请求；等待新快照稳定后重新检查"
                            .into();
                }
            }
            Ok(WorkerOutput::DiscussionRead(result)) => {
                match result {
                    Ok(batch) => {
                        self.discussion_issue = batch.coverage_issue.clone();
                        self.discussion_backfill = batch.pending_sessions > 0;
                        self.pending_discussion = Some(batch);
                    }
                    Err(error) => {
                        self.discussion_backfill = false;
                        self.discussion_issue = Some(format!("Amoo 本轮未更新：{error}"));
                    }
                }
                self.last_fingerprint = None;
            }
            Ok(WorkerOutput::OutcomeSkipped(title)) => {
                self.status = format!("《{title}》尚无明确任务，暂未调用模型生成");
                self.error = None;
            }
            Ok(WorkerOutput::Outcome(result, inference, automatic)) => {
                // Collection can now advance while the model is running. Keep
                // the immutable draft on disk, but do not promote stale output
                // as the current plan or mark a newer summary complete.
                let current_revision = match &result.bundle.target {
                    OutcomeTarget::Item(id) => self
                        .overview
                        .items
                        .iter()
                        .find(|i| &i.id == id)
                        .map(|i| i.input_revision.as_str()),
                    OutcomeTarget::Period(kind) => self
                        .overview
                        .periods
                        .iter()
                        .find(|p| &p.kind == kind)
                        .map(|p| p.evidence_revision.as_str()),
                };
                if current_revision != Some(result.bundle.input_revision.as_str()) {
                    self.needs_load = true;
                    self.status = "成稿已保存，但期间手机资料已更新；正在核对最新版本".into();
                    return true;
                }
                let source_label = inference
                    .as_ref()
                    .filter(|meta| {
                        meta.source != stepx_companion::model_source::Source::PhoneAccount
                            || self.scope.as_ref().is_some_and(|scope| {
                                scope.account.is_some() && meta.account_scope == scope.account
                            })
                    })
                    .map(|meta| meta.source.label());
                self.cache_inference(inference);
                if automatic {
                    if let OutcomeTarget::Period(kind) = &result.bundle.target {
                        if let Some(period) = self
                            .overview
                            .periods
                            .iter()
                            .find(|period| period.kind == *kind)
                        {
                            let key = automatic_period_key(period);
                            let mut config = self.config();
                            config
                                .automatic_period_completed
                                .insert(key.clone(), Local::now().timestamp_millis());
                            if let Some(batch) = config.period_batch.as_mut() {
                                if batch.pending.get(&key) == Some(&result.bundle.input_revision) {
                                    batch.pending.remove(&key);
                                }
                                if batch.pending.is_empty() {
                                    config.period_batch = None;
                                }
                            }
                            self.set_config(config);
                        }
                    }
                }
                self.automatic_error = false;
                self.automatic_retry_at = None;
                self.model_checked_at = Instant::now() - Duration::from_secs(61);
                let target = result.bundle.target.clone();
                self.outcomes.retain(|r| r.bundle.target != target);
                let received = result
                    .bundle
                    .files
                    .iter()
                    .filter(|f| result.deliveries.iter().any(|d| d.sha256 == f.sha256))
                    .count();
                self.status = format!(
                    "已生成《{}》：{} 项待办{}",
                    result.bundle.title,
                    result.bundle.todo_count,
                    if received == result.bundle.files.len() && result.delivery_error.is_none() {
                        "，全部文件已校验并回到手机"
                    } else if received > 0 {
                        "，部分文件已回到手机，其余等待重试"
                    } else {
                        "，可打开核对"
                    }
                );
                if let Some(label) = source_label {
                    self.status.push_str(&format!(" · 成稿来源：{label}"));
                }
                self.error = result.delivery_error.clone();
                self.outcomes.insert(0, result);
                self.outcomes.truncate(32);
            }
            Ok(WorkerOutput::Captured(
                overview,
                cursor,
                unsupported,
                discussion_cursor,
                captured_sms,
                health,
            )) => {
                if health.phone_checked {
                    self.phone_issue = health.phone_issue;
                }
                let incomplete = self.collection_warning();
                self.collection_persisted = true;
                self.collection_result = Some((Instant::now(), incomplete.clone()));
                self.overview = overview;
                self.reconcile_panel();
                let mut config = self.config();
                if let Some(cursor) = cursor {
                    config.sms_cursor = Some(cursor);
                }
                if let Some(cursor) = discussion_cursor {
                    config.discussion_cursor = cursor;
                }
                config.sms_unsupported |= unsupported;
                self.set_config(config);
                self.pending_captured_sms = captured_sms
                    .and_then(|messages| worker.scope.clone().map(|scope| (scope, messages)));
                // A completed refresh must not erase an in-flight model's
                // status or its failure. Source health has its own receipt.
                if !self.ai_active() && self.error.is_none() {
                    self.status = format!(
                        "{} {} 条来源、{} 件事",
                        if incomplete.is_some() {
                            "部分更新："
                        } else {
                            "已更新"
                        },
                        self.overview.evidence_count,
                        self.overview.items.len()
                    );
                }
            }
            Ok(WorkerOutput::AccountScopeResolved(serial, account)) => {
                self.account_resolved_for = Some(serial.clone());
                self.account_resolve_checked_at = Some(Instant::now());
                self.resolved_account = Some((serial, account));
            }
            Ok(WorkerOutput::ModelAvailable(state)) => {
                self.cache_text_source(state);
            }
            Ok(WorkerOutput::AiStarted(job, scope, target)) => {
                self.ai = Some(AiRun {
                    generation: self.generation,
                    scope,
                    target,
                    job,
                });
            }
            Ok(WorkerOutput::LoadedOverview(overview)) => {
                self.overview = overview;
                // Reading an empty store is not a successful preparation run.
                self.status.clear();
                self.error = None;
            }
            Ok(WorkerOutput::Overview(overview, cursor, unsupported)) => {
                self.overview = overview;
                self.reconcile_panel();
                let mut config = self.config();
                if let Some(cursor) = cursor {
                    config.sms_cursor = Some(cursor);
                }
                config.sms_unsupported |= unsupported;
                self.set_config(config);
                self.status = if self.overview.evidence_count == 0 {
                    "已检查所选来源，尚无可整理的资料；可继续讨论或调整来源。".into()
                } else {
                    format!(
                        "已整理 {} 条依据、{} 件事；可切换日、周、月回顾查看。",
                        self.overview.evidence_count,
                        self.overview.items.len()
                    )
                };
                self.error = None;
            }
            Ok(WorkerOutput::RelationChanged(overview)) => {
                self.overview = overview;
                self.reconcile_panel();
                self.needs_load = false;
                self.link_picker = None;
                self.relationship_details = None;
                self.relationship_page = 0;
                self.status = "已更新事项关联与回顾，来源资料和原任务均保留".into();
                self.error = None;
            }
            Ok(WorkerOutput::AiSaved(path, target, revision)) => {
                let mut config = self.config();
                config.ai_records.retain(|r| r.target != target);
                config.ai_records.push(AiRecord {
                    path,
                    target,
                    revision,
                });
                if config.ai_records.len() > 96 {
                    config.ai_records.remove(0);
                }
                self.set_config(config);
                self.status = "AI 深化稿已保存，可打开审阅和编辑".into();
                self.error = self.session_warning.clone();
            }
            Ok(WorkerOutput::Cleared(_)) => {
                self.overview = Default::default();
                self.outcomes.clear();
                self.last_inference = None;
                self.period_stability.clear();
                self.model_checked_at = Instant::now() - Duration::from_secs(61);
                self.status = "已清理当前设备与账号的整理记录，手机原始数据未修改".into();
            }
            Ok(WorkerOutput::Saved) => {}
            Err(error) => {
                if worker.collects_context {
                    self.collection_persisted = false;
                    self.collection_result = Some((Instant::now(), Some(error)));
                    return true;
                }
                if worker.saves_preferences {
                    self.dirty = true;
                    self.write_failed = true;
                }
                if worker.clears_scope {
                    self.write_failed = true;
                }
                if !self.initialized {
                    self.write_failed = true;
                }
                if worker.automatic_ai {
                    self.automatic_error = automatic_error_can_recheck(&error);
                    self.automatic_retry_at = self
                        .automatic_error
                        .then(|| Instant::now() + Duration::from_secs(30));
                    self.model_checked_at = Instant::now() - Duration::from_secs(61);
                }
                self.initialized = true;
                self.error = Some(error);
                self.status.clear();
            }
        }
        true
    }
}

fn save_preferences(root: &std::path::Path, preferences: &Preferences) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)?;
    let temp = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(temp.as_file(), preferences)?;
    temp.as_file().sync_all()?;
    temp.persist(root.join("preferences.json"))?;
    Ok(())
}
fn load_preferences(root: &std::path::Path) -> anyhow::Result<Preferences> {
    let path = root.join("preferences.json");
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn row_between_expanded(
    left: widgets::El,
    right: widgets::El,
    spacing: f32,
) -> widgets::El {
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(spacing)
        .with_child(Expanded::new(1., left).finish())
        .with_child(right)
        .finish()
}

fn auto_due(period: ledger::Period, date: chrono::NaiveDate, hour: u32) -> bool {
    if hour < 18 {
        return false;
    }
    match period {
        ledger::Period::Day => true,
        ledger::Period::Week => date.weekday() == chrono::Weekday::Sun,
        ledger::Period::Month => date
            .succ_opt()
            .is_some_and(|next| next.month() != date.month()),
    }
}
fn period_auto_due(
    period: &ledger::PeriodBrief,
    now_ms: i64,
    date: chrono::NaiveDate,
    hour: u32,
) -> bool {
    period.evidence_count > 0
        && period.begin_ms <= now_ms
        && now_ms < period.end_ms
        && auto_due(period.kind, date, hour)
}

impl Companion {
    #[cfg(feature = "ui-capture")]
    pub(super) fn load_context_capture_fixture(&mut self) {
        if !self.demo_mode {
            return;
        }
        let Some(root) = std::env::var_os("STEPX_CONTEXT_DEMO_DIR").map(PathBuf::from) else {
            return;
        };
        self.context.persistent = false;
        self.context.initialized = true;
        self.context.root = root.clone();
        let device = std::env::var("STEPX_CONTEXT_DEMO_DEVICE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "context-demo".into());
        let account = std::env::var("STEPX_CONTEXT_DEMO_ACCOUNT")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let scope = Scope { device, account };
        self.context.change_scope(scope.clone());
        // Capture previews never run ledger migrations or regenerate documents.
        // Reading the saved view also keeps paused/live verification data untouched.
        let loaded = (|| -> anyhow::Result<ledger::Overview> {
            let path = root
                .join(ledger::scope_id(&scope.device, scope.account.as_deref()))
                .join("ledger.json");
            anyhow::ensure!(
                std::fs::metadata(&path)?.len() <= 64 * 1024 * 1024,
                "预览账本超过读取上限"
            );
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
            anyhow::ensure!(
                saved["device"].as_str() == Some(scope.device.as_str())
                    && saved["account"].as_str() == scope.account.as_deref(),
                "预览账本的设备或账号不匹配"
            );
            Ok(serde_json::from_value(
                saved
                    .get("overview")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("预览账本没有已保存视图"))?,
            )?)
        })();
        match loaded {
            Ok(overview) => {
                self.context.overview = overview;
                self.context.needs_load = false;
                self.context.status = "隔离验收资料 · 只读预览 · 不采集手机或请求模型".into();
            }
            Err(error) => self.context.error = Some(error.to_string()),
        }
    }
    #[cfg(feature = "ui-capture")]
    pub(super) fn seed_demo_context_overview(&mut self) {
        if !self.demo_mode
            || std::env::var_os("STEPX_CONTEXT_DEMO_DIR").is_some()
            || std::env::var_os("STEPX_EMPTY").is_some()
            || self.context.overview.evidence_count > 0
        {
            return;
        }
        let now = Local::now();
        let now_ms = now.timestamp_millis();
        let device = self
            .selected
            .as_ref()
            .map(|d| d.serial.clone())
            .unwrap_or_else(|| "STEPX_DEMO_DEVICE".into());
        let scope = Scope {
            device: device.clone(),
            account: None,
        };
        let temp_dir =
            std::env::temp_dir().join(format!("stepx-context-demo-seed-{}", std::process::id()));
        let mut task_review =
            stepx_companion::amoo_workspace::Task::new(Some(device.clone()), None);
        task_review.id = "demo-q4-review".into();
        task_review.title = "第四季度产品与桌面协同评审会方案".into();
        task_review.updated = now_ms - 25 * 60_000;
        task_review.turns.push(stepx_companion::amoo_workspace::Turn {
            at_ms: Some(now_ms - 35 * 60_000),
            user: true,
            text: "请结合张晓峰发来的会议改期短信与日程，整理明天下午 2 点 3 楼大会议室的第四季度版本规划评审提纲与待办。".into(),
        });
        let mut task_sync = stepx_companion::amoo_workspace::Task::new(Some(device.clone()), None);
        task_sync.id = "demo-cross-sync".into();
        task_sync.title = "StepX 双端日历与背屏流转架构归档".into();
        task_sync.updated = now_ms - 80 * 60_000;
        task_sync.turns.push(stepx_companion::amoo_workspace::Turn {
            at_ms: Some(now_ms - 90 * 60_000),
            user: true,
            text: "梳理手机通知、短信、日程与桌面工作区自动聚类及成果回传手机链路。".into(),
        });
        let capture = ledger::Capture {
            device,
            account: None,
            now_ms,
            utc_offset_seconds: now.offset().local_minus_utc(),
            tasks: vec![task_review, task_sync],
            push: Some(self.push.snapshot.clone()),
            sms: Some(self.sms.clone()),
            calendar: Some(self.back_screen.stepos_suite.calendar_events.clone()),
            ..Default::default()
        };
        if let Ok(overview) = ledger::process(&temp_dir, &capture) {
            self.context.persistent = false;
            self.context.initialized = true;
            self.context.change_scope(scope.clone());
            let mut cfg = ScopePreferences {
                enabled: true,
                tasks: true,
                phone: true,
                automatic_ai: true,
                return_outcomes: true,
                since_ms: now_ms - 7 * 86_400_000,
                phone_since_ms: now_ms - 7 * 86_400_000,
                ..Default::default()
            };
            cfg.scheduled_attempts.clear();
            self.context.set_config(cfg);
            self.context.dirty = false;
            self.context.overview = overview;
            self.context.needs_load = false;
            self.context.status =
                "双端智能上下文已同步 · 手机通知、短信、日程与电脑讨论已自动聚类".into();
            if let Ok(panel) = std::env::var("STEPX_CONTEXT_PANEL") {
                match panel.trim() {
                    "review" => self.context.panel = ContextPanel::Review,
                    "manage" => self.context.panel = ContextPanel::Manage,
                    _ => {}
                }
            }
        }
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    pub(super) fn context_automatic_source_status(&self) -> (bool, String) {
        let state = self.context.automatic_source_status();
        if self.context.model_source == Some(stepx_companion::model_source::Source::PhoneAccount)
            && !self.context_phone_identity_ready()
        {
            return (
                false,
                "请连接原手机并确认当前账号；后台生成等待身份核对，USB与无线均可".into(),
            );
        }
        state
    }
    pub(super) fn context_collection_paused(&self) -> bool {
        self.context.persistent && self.context.initialized && !self.context.config().enabled
    }
    pub(super) fn trigger_auto_context_refresh(&mut self) {
        if self.demo_mode || !self.context.persistent {
            return;
        }
        if !self.context.initialized {
            self.context.force = true;
            return;
        }
        self.context.change_scope(self.context_scope());
        // Domain synchronization is not consent to resume a paused collector
        // or to enable a source the user left unselected.
        if !self.context.config().enabled {
            return;
        }
        if self
            .context
            .phone_checked_at
            .map_or(true, |t| t.elapsed() >= Duration::from_secs(120))
        {
            self.context.force = true;
            self.context.phone_checked_at = None;
        }
    }

    pub(super) fn recheck_context_model_capabilities(&mut self) {
        self.context.model_available = false;
        self.context.automatic_text_available = false;
        self.context.automatic_reason = "正在核对当前设备账号的模型通道".into();
        self.context.model_checked_at = Instant::now() - Duration::from_secs(61);
    }
    pub(super) fn refresh_context_model_capabilities(&mut self) {
        self.context.invalidate();
        self.context.needs_load = true;
        let mut config = self.context.config();
        config.automatic_ai = false;
        config.period_batch = None;
        self.context.set_config(config);
        if let Some(draft) = self.context.setup.as_mut() {
            draft.automatic_ai = false;
        }
        self.context.model_available = false;
        self.context.automatic_text_available = false;
        self.context.model_source = None;
        self.context.period_stability.clear();
        self.context.automatic_reason = "正在核对当前设备账号与模型来源能力".into();
        self.context.automatic_error = false;
        self.context.automatic_retry_at = None;
        self.context.model_checked_at = Instant::now() - Duration::from_secs(61);
    }

    fn context_scope(&self) -> Scope {
        if let Some(device) = &self.selected {
            Scope {
                device: device.serial.clone(),
                account: self.amoo_ui.account_scope.clone(),
            }
        } else {
            self.context
                .preferences
                .last_scope
                .clone()
                .unwrap_or_else(|| Scope {
                    device: "desktop-local".into(),
                    account: None,
                })
        }
    }
    fn context_fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let now = Local::now();
        (now.year(), now.ordinal(), now.offset().local_minus_utc()).hash(&mut h);
        self.selected.as_ref().map(|d| &d.serial).hash(&mut h);
        self.amoo_ui.account_scope.hash(&mut h);
        let config = self.context.config();
        if config.phone {
            self.push.snapshot.revision.hash(&mut h);
            self.push.snapshot.listener_connected.hash(&mut h);
            self.push.snapshot.coverage.last_gap_ms.hash(&mut h);
            self.push.snapshot.coverage.history_truncated.hash(&mut h);
            self.push.snapshot.coverage.persistence_error.hash(&mut h);
            for thread in &self.push.snapshot.threads {
                thread.thread_id.hash(&mut h);
                for message in &thread.messages {
                    message.id.hash(&mut h);
                    message.text.hash(&mut h);
                    message.timestamp_ms.hash(&mut h);
                    message.delivery_status.hash(&mut h);
                    message.observed_at_ms.hash(&mut h);
                    message.time_known.hash(&mut h);
                }
            }
            for notification in &self.push.snapshot.notifications {
                notification.key.hash(&mut h);
                notification.title.hash(&mut h);
                notification.text.hash(&mut h);
                notification.post_time_ms.hash(&mut h);
                notification.active.hash(&mut h);
                notification.removed_time_ms.hash(&mut h);
            }
            self.push.error.hash(&mut h);
            self.back_screen.stepos_suite_error.hash(&mut h);
            for event in &self.back_screen.stepos_suite.calendar_events {
                event.id.hash(&mut h);
                event.dtstart.hash(&mut h);
                event.title.hash(&mut h);
                event.dtend.hash(&mut h);
                event.location.hash(&mut h);
                event.description.hash(&mut h);
                event.all_day.hash(&mut h);
                event.source.hash(&mut h);
            }
        }
        if config.tasks {
            for task in &self.workspace.journal.tasks {
                task.id.hash(&mut h);
                task.title.hash(&mut h);
                task.updated.hash(&mut h);
                task.turns.len().hash(&mut h);
                task.outputs.len().hash(&mut h);
                task.receipts.len().hash(&mut h);
                for turn in &task.turns {
                    turn.user.hash(&mut h);
                    turn.text.hash(&mut h);
                    turn.at_ms.hash(&mut h);
                }
                for output in &task.outputs {
                    output.path.hash(&mut h);
                    output.name.hash(&mut h);
                    output.kind.hash(&mut h);
                }
                for receipt in &task.receipts {
                    receipt.path.hash(&mut h);
                    receipt.note.hash(&mut h);
                    receipt.success.hash(&mut h);
                }
                for step in &task.steps {
                    std::mem::discriminant(&step.phase).hash(&mut h);
                    step.note.hash(&mut h);
                }
            }
        }
        h.finish()
    }
    fn poll_context_results(&mut self) -> bool {
        // Device/account selection can change before this tick runs. Retire
        // the old scope before accepting a ready worker or forwarding SMS.
        let mut changed =
            self.context.initialized && self.context.change_scope(self.context_scope());
        changed |= self.context.poll_worker();
        changed |= self.context.poll_worker();
        changed |= self.context.poll_worker();
        changed |= self.context.poll_worker();
        if let Some((serial, account)) = self.context.resolved_account.take() {
            if self.selected.as_ref().is_some_and(|d| d.serial == serial)
                && self.amoo_ui.account_scope.is_none()
                && account.is_some()
            {
                self.amoo_ui.set_account_scope(account);
                changed = true;
            }
        }
        let active_scope = self.context_scope();
        if let Some(messages) = self.context.take_captured_sms(&active_scope) {
            if !messages.is_empty() {
                for msg in messages {
                    let norm_addr = stepx_companion::sms::normalize_address(&msg.address);
                    let key1 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &msg.address,
                    );
                    let key2 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &norm_addr,
                    );
                    if let Some(del_ts) = self
                        .push
                        .prefs
                        .deleted_threads
                        .get(&key1)
                        .copied()
                        .or_else(|| self.push.prefs.deleted_threads.get(&key2).copied())
                    {
                        if msg.date_ms <= del_ts {
                            continue;
                        }
                    }
                    if let Some(clr_ts) = self
                        .push
                        .prefs
                        .cleared_threads
                        .get(&key1)
                        .copied()
                        .or_else(|| self.push.prefs.cleared_threads.get(&key2).copied())
                    {
                        if msg.date_ms <= clr_ts {
                            continue;
                        }
                    }
                    let msg_key =
                        format!("sms_msg:{norm_addr}::{}::{}", msg.date_ms, msg.body.trim());
                    if self.push.prefs.deleted_messages.contains(&msg_key) {
                        continue;
                    }
                    let exists = self.sms.iter().any(|existing| {
                        (existing.provider_id.is_some() && existing.provider_id == msg.provider_id)
                            || (existing.date_ms == msg.date_ms
                                && existing.address == msg.address
                                && existing.body == msg.body)
                    });
                    if !exists {
                        self.sms.push(msg);
                    }
                }
                self.sms.sort_by(|a, b| {
                    b.date_ms
                        .cmp(&a.date_ms)
                        .then_with(|| b.provider_id.cmp(&a.provider_id))
                });
                self.sync_read_sms_state();
                self.sync_live_express_parcels();
                changed = true;
            }
        }
        changed
    }
    pub(super) fn tick_context(&mut self, ctx: &mut ViewContext<Self>) {
        let mut changed = self.context.poll_preview();
        if !self.context.persistent {
            if changed {
                ctx.notify();
            }
            return;
        }
        changed |= self.poll_context_results();
        changed |= self.poll_context_ai();
        if !self.context.initialized {
            if self.context.worker.is_none() {
                let root = self.context.root.clone();
                self.context.start_worker(None, move || {
                    let prefs = load_preferences(&root)?;
                    let state = load_text_source_state(prefs.last_scope.as_ref());
                    Ok(WorkerOutput::Preferences(prefs, state))
                });
            }
            if changed {
                ctx.notify();
            }
            return;
        }
        changed |= self.context.change_scope(self.context_scope());
        if changed {
            ctx.notify();
        }
        if self.context.worker.is_some() || self.context.write_failed {
            return;
        }
        if self.context.automatic_error
            && self
                .context
                .automatic_retry_at
                .is_some_and(|at| Instant::now() >= at)
            && self.selected.is_some()
            && self.context.automatic_text_available
            && self.context_phone_identity_ready()
        {
            self.context.automatic_error = false;
            self.context.automatic_retry_at = None;
            self.context.error = None;
            self.context.needs_load = true;
        }
        let root = self.context.root.clone();
        if self.context.dirty {
            let prefs = self.context.preferences.clone();
            self.context.dirty = false;
            self.context.start_worker(None, move || {
                save_preferences(&root, &prefs)?;
                Ok(WorkerOutput::Saved)
            });
            self.context.worker.as_mut().unwrap().saves_preferences = true;
            return;
        }
        if let Some(scope) = self.context.preferences.pending_clear.first().cloned() {
            if self.productivity_is_busy()
                || self.context.generation_worker.is_some()
                || self.context.discussion_worker.is_some()
            {
                return;
            }
            let target = scope.clone();
            self.context.start_worker(Some(scope), move || {
                ledger::clear(&root, &target.device, target.account.as_deref())?;
                Ok(WorkerOutput::Cleared(target))
            });
            self.context.worker.as_mut().unwrap().clears_scope = true;
            return;
        }
        if self.context.start_pending_relation() {
            return;
        }
        if self.context.generation_worker.is_none()
            && !self.context.pending_outcomes.is_empty()
            && !self.productivity_is_busy()
            && !self.busy
        {
            let (
                scope,
                target,
                mut device,
                deliver,
                automatic,
                delivery_revision,
                generation_guard,
            ) = self.context.pending_outcomes.remove(0);
            let cancel = self.context.work_cancel.clone();
            let account = scope.account.clone();
            self.context.status = "正在结合手机与电脑资料生成成果；来源不一致会自动修正…".into();
            self.context.start_worker(Some(scope), move || {
                let adb = Adb::discover()?;
                if let Ok(active) = device.resolve_active_transport(&adb) {
                    device.transport = active;
                }
                let explicit_snapshot = generation_guard.is_some() && !automatic;
                let result = if let Some(revision) = delivery_revision {
                    context_pipeline::deliver_existing(
                        &root,
                        &adb,
                        &device,
                        account.as_deref(),
                        &target,
                        &revision,
                        &cancel,
                    )?
                } else {
                    let request = ledger::outcomes::prepare(
                        &root,
                        &device.serial,
                        account.as_deref(),
                        &target,
                    )?;
                    if let Some((source, revision)) = generation_guard {
                        if stepx_companion::model_source::load()?.source != source
                            || request.input_revision != revision
                        {
                            return Ok(WorkerOutput::AutomaticInputChanged(explicit_snapshot));
                        }
                    }
                    if !request.eligible_for_automation {
                        return Ok(WorkerOutput::OutcomeSkipped(request.title));
                    }
                    if !automatic && !explicit_snapshot {
                        context_pipeline::regenerate(
                            &root,
                            &device.serial,
                            account.as_deref(),
                            &target,
                            "用户在客户端明确重新成稿",
                        )?;
                    }
                    let runner = if automatic {
                        context_pipeline::run_automatic
                    } else {
                        context_pipeline::run
                    };
                    runner(
                        &root,
                        &adb,
                        &device,
                        account.as_deref(),
                        &target,
                        &cancel,
                        deliver,
                    )?
                };
                let inference = context_pipeline::load_inference_meta(
                    &root,
                    &device.serial,
                    account.as_deref(),
                    &result.bundle.target,
                    &result.bundle.input_revision,
                )
                .ok()
                .flatten();
                Ok(WorkerOutput::Outcome(result, inference, automatic))
            });
            self.context.worker.as_mut().unwrap().starts_ai = true;
            self.context.worker.as_mut().unwrap().automatic_ai = automatic;
            self.context.generation_worker = self.context.worker.take();
            return;
        }
        if let Some(save) = self.context.ai_save.take() {
            let scope = save.scope.clone();
            self.context.start_worker(Some(scope), move || {
                let path = match &save.target {
                    AiTarget::Period(period) => ledger::save_ai_brief(
                        &root,
                        &save.scope.device,
                        save.scope.account.as_deref(),
                        period,
                        &save.markdown,
                    )?,
                    AiTarget::Item(item) => ledger::save_ai_item(
                        &root,
                        &save.scope.device,
                        save.scope.account.as_deref(),
                        item,
                        &save.markdown,
                    )?,
                };
                Ok(WorkerOutput::AiSaved(
                    path,
                    save.target.key(),
                    save.target.revision().to_string(),
                ))
            });
            return;
        }
        if let Some(pending) = self.context.pending_ai.take() {
            let scope = pending.scope.clone();
            self.context.start_worker(Some(scope), move || {
                let job = if pending.use_local {
                    home_ai::start_local_text(pending.target.prompt().to_string())
                } else {
                    let phone = pending
                        .device
                        .ok_or_else(|| anyhow::anyhow!("手机已离线，请重新连接"))?;
                    home_ai::start_text(
                        Adb::discover()?,
                        phone,
                        pending.target.prompt().to_string(),
                    )
                };
                Ok(WorkerOutput::AiStarted(job, pending.scope, pending.target))
            });
            self.context.worker.as_mut().unwrap().starts_ai = true;
            return;
        }
        if self.context.model_checked_at.elapsed() >= Duration::from_secs(60) {
            self.context.model_checked_at = Instant::now();
            let scope = self.context.scope.clone();
            let target = scope.clone();
            self.context.start_worker(scope, move || {
                Ok(WorkerOutput::ModelAvailable(load_text_source_state(
                    target.as_ref(),
                )))
            });
            return;
        }
        if !self.demo_mode
            && self.amoo_ui.account_scope.is_none()
            && self.context.identity_worker.is_none()
        {
            if let Some(phone) = self.selected.clone() {
                if self
                    .context
                    .account_resolution_due(&phone.serial, Instant::now())
                {
                    let serial = phone.serial.clone();
                    self.context.account_resolved_for = Some(serial.clone());
                    self.context.account_resolve_checked_at = Some(Instant::now());
                    self.context.start_worker(None, move || {
                        let account = Adb::discover().ok().and_then(|adb| {
                            stepx_companion::amoo_usage::account_scope(&adb, &phone)
                                .ok()
                                .flatten()
                        });
                        Ok(WorkerOutput::AccountScopeResolved(serial, account))
                    });
                    self.context.identity_worker = self.context.worker.take();
                }
            }
        }
        let Some(scope) = self.context.scope.clone() else {
            return;
        };
        if self.context.needs_load {
            self.context.needs_load = false;
            let target = scope.clone();
            self.context.start_worker(Some(scope), move || {
                let overview = ledger::load(&root, &target.device, target.account.as_deref())?;
                let outcomes = context_pipeline::list_for_reading(
                    &root,
                    &target.device,
                    target.account.as_deref(),
                )?;
                let inference = outcomes
                    .iter()
                    .filter_map(|result| {
                        context_pipeline::load_inference_meta(
                            &root,
                            &target.device,
                            target.account.as_deref(),
                            &result.bundle.target,
                            &result.bundle.input_revision,
                        )
                        .ok()
                        .flatten()
                    })
                    .max_by_key(|meta| meta.observed_ms);
                Ok(WorkerOutput::OutcomesLoaded(
                    overview,
                    outcomes,
                    context_pipeline::recoverable_outcomes(
                        &root,
                        &target.device,
                        target.account.as_deref(),
                    )?,
                    inference,
                ))
            });
            return;
        }
        let config = self.context.config();
        if !config.enabled && !self.context.force {
            return;
        }
        if !self.context.force && self.context.checked_at.elapsed() < Duration::from_secs(3) {
            return;
        }
        self.context.checked_at = Instant::now();
        let fingerprint = self.context_fingerprint();
        let phone_due = (config.phone || config.tasks)
            && self.selected.is_some()
            && (self.context.force
                || (self.context.discussion_backfill
                    && self
                        .context
                        .phone_checked_at
                        .is_none_or(|at| at.elapsed() >= Duration::from_secs(10)))
                || background_phone_refresh_due(
                    self.context.phone_checked_at.map(|t| t.elapsed()),
                    self.context.phone_source_fingerprint != Some(fingerprint),
                ));
        if !self.context.force && !phone_due && self.context.last_fingerprint == Some(fingerprint) {
            self.maybe_auto_context_ai();
            return;
        }
        if !self.context.force && !phone_due {
            match self.context.pending_fingerprint {
                Some((pending, since))
                    if pending == fingerprint && since.elapsed() >= Duration::from_secs(2) => {}
                _ => {
                    self.context.pending_fingerprint = Some((fingerprint, Instant::now()));
                    return;
                }
            }
        }
        self.context.force = false;
        self.context.last_fingerprint = Some(fingerprint);
        self.context.pending_fingerprint = None;
        let now = Local::now();
        let connected = self
            .selected
            .as_ref()
            .is_some_and(|d| d.serial == scope.device)
            && self.amoo_ui.account_scope == scope.account;
        let mut coverage_notes =
            vec!["仅整理已选来源；通知预览和当前屏幕不代表 App 全部历史".into()];
        if !config.phone {
            coverage_notes.push("手机来源已停止新增；之前保存的资料仍参与回顾".into());
        }
        if !config.tasks {
            coverage_notes.push("Amoo 来源已停止新增；之前保存的资料仍参与回顾".into());
        }
        if config.sms_unsupported {
            coverage_notes.push("手机桥接不支持增量短信，请更新组件后重新开启手机来源".into());
        }
        let phone_ready = config.phone && connected && phone_due;
        let push = (phone_ready && self.push.loaded && self.push.error.is_none())
            .then(|| self.push.snapshot.clone());
        let calendar = (phone_ready
            && self.back_screen.stepos_suite.checked
            && self.back_screen.stepos_suite_synced_at.is_some()
            && self.back_screen.stepos_suite_error.is_none())
        .then(|| self.back_screen.stepos_suite.calendar_events.clone());
        if phone_ready && push.is_none() {
            coverage_notes.push("手机通知尚未成功同步".into());
        }
        if phone_ready && calendar.is_none() {
            coverage_notes.push("手机日历尚未成功同步".into());
        }
        let tasks = if config.tasks {
            self.workspace
                .journal
                .tasks
                .iter()
                .filter(|t| t.visible(Some(&scope.device), scope.account.as_deref()))
                .cloned()
                .collect()
        } else {
            vec![]
        };
        // Respect the same explicit local session exclusions as the discussion
        // UI when importing phone history, including sessions not opened here.
        let excluded_sessions: std::collections::BTreeSet<String> =
            self.workspace.deleted_sessions.iter().cloned().collect();
        let mut capture = ledger::Capture {
            device: scope.device.clone(),
            account: scope.account.clone(),
            now_ms: now.timestamp_millis(),
            utc_offset_seconds: now.offset().local_minus_utc(),
            connected,
            since_ms: config.phone_since(),
            push,
            sms: None,
            tasks,
            phone_discussions: vec![],
            excluded_phone_sessions: excluded_sessions.iter().cloned().collect(),
            calendar,
            coverage_notes,
        };
        let phone = if phone_due && connected {
            self.selected.clone()
        } else {
            None
        };
        if phone.is_some() {
            self.context.phone_checked_at = Some(Instant::now());
            self.context.phone_source_fingerprint = Some(fingerprint);
        }
        // Amoo account/history is its own lane. Its timeout must not delay
        // device-owned notifications or SMS, nor advance their cursors.
        if phone_due
            && connected
            && config.tasks
            && self.context.discussion_worker.is_none()
            && self.context.pending_discussion.is_none()
        {
            if let (Some(mut discussion_phone), Some(account)) =
                (phone.clone(), scope.account.clone())
            {
                let previous = config.discussion_cursor.clone();
                let since = if config.enabled {
                    config.since_ms
                } else {
                    capture.now_ms.saturating_sub(7 * 86_400_000)
                };
                let cancel = self.context.work_cancel.clone();
                let excluded = excluded_sessions.clone();
                self.context.start_worker(Some(scope.clone()), move || {
                    let result = (|| -> anyhow::Result<_> {
                        let adb = Adb::discover()?;
                        discussion_phone.transport =
                            discussion_phone.resolve_active_transport(&adb)?;
                        let mut batch = phone_context::collect_recent(
                            &adb,
                            &discussion_phone,
                            &account,
                            since,
                            &previous,
                            &cancel,
                        )?;
                        phone_context::exclude_sessions(&mut batch, &excluded);
                        Ok(batch)
                    })()
                    .map_err(|error| error.to_string());
                    Ok(WorkerOutput::DiscussionRead(result))
                });
                self.context.discussion_worker = self.context.worker.take();
            } else {
                self.context.discussion_issue =
                    Some("尚未确认 Amoo 账号；通知和短信继续同步".into());
            }
        }
        let discussion = self.context.pending_discussion.take();
        let mut health = CollectionHealth {
            phone_checked: config.phone && (phone_due || !connected),
            phone_issue: if config.phone && !connected {
                Some("手机离线，保留上次已同步资料".into())
            } else if config.phone && config.sms_unsupported {
                Some("手机桥接不支持增量短信".into())
            } else {
                None
            },
        };
        let cancel = self.context.work_cancel.clone();
        if !self.context.ai_active() && self.context.error.is_none() {
            self.context.status = "正在后台更新本地资料与回顾…".into();
        }
        self.context.start_worker(Some(scope), move || {
            let mut cursor = None;
            let mut unsupported = false;
            let mut discussion_cursor = None;
            if let Some(batch) = discussion {
                capture.phone_discussions = batch.messages;
                capture.coverage_notes.extend(batch.coverage_notes);
                discussion_cursor = Some(batch.cursor);
            }
            if let Some(mut phone) = phone {
                let adb = Adb::discover()?;
                if let Ok(active) = phone.resolve_active_transport(&adb) {
                    phone.transport = active;
                }
                let verify_scope = || -> anyhow::Result<()> {
                    anyhow::ensure!(!cancel.load(Ordering::Acquire), "已停止资料整理");
                    anyhow::ensure!(
                        adb.shell(&phone.transport, &["getprop", "ro.serialno"])?
                            .trim()
                            == capture.device,
                        "手机连接已变化，停止此次采集"
                    );

                    Ok(())
                };
                verify_scope()?;
                if config.phone && capture.push.is_none() {
                    match stepx_companion::push::fetch_snapshot(&adb, &phone.transport, 0) {
                        Ok(snapshot) => capture.push = Some(snapshot),
                        Err(error) => {
                            let note = format!("本次通知未更新：{error}");
                            capture.coverage_notes.push(note.clone());
                            health.phone_issue = Some(note);
                        }
                    }
                }
                anyhow::ensure!(!cancel.load(Ordering::Acquire), "已停止资料整理");
                if config.phone && !config.sms_unsupported {
                    let after = Some(config.sms_after());
                    match sms::read_page(&adb, &phone, after, 500) {
                        Ok(page) if page.supports_incremental => {
                            cursor = page.next_cursor;
                            if page.has_more {
                                health.phone_issue =
                                    Some("短信尚未补齐，将从已保存游标继续读取".into());
                                capture.coverage_notes.push(if config.enabled {
                                    "还有短信未补齐，后续同步将继续读取（每次最多 500 条）".into()
                                } else {
                                    "本次短信最多读取 500 条，仍有记录未补齐；再次整理可继续读取"
                                        .into()
                                });
                            }
                            capture.sms = Some(page.messages);
                        }
                        Ok(_) => {
                            unsupported = true;
                            health.phone_issue = Some("手机桥接不支持增量短信".into());
                            capture
                                .coverage_notes
                                .push("手机桥接版本较旧，已停止自动短信读取，需更新组件".into());
                        }
                        Err(e) => {
                            let note = format!("本次短信未读取：{e}");
                            capture.coverage_notes.push(note.clone());
                            health.phone_issue = Some(note);
                        }
                    }
                }
                verify_scope()?;
            }
            anyhow::ensure!(!cancel.load(Ordering::Acquire), "已停止资料整理");
            if health.phone_checked && health.phone_issue.is_none() {
                health.phone_issue = match capture.push.as_ref() {
                    Some(snapshot) if !snapshot.listener_connected => {
                        Some("手机通知监听未连接，无法确认新消息完整性".into())
                    }
                    Some(snapshot) if snapshot.coverage.persistence_error => {
                        Some("手机通知保存异常，部分消息可能未同步".into())
                    }
                    _ if capture.calendar.is_none() => Some("手机日历尚未成功同步".into()),
                    _ => None,
                };
            }
            let captured_sms = capture.sms.clone();
            Ok(WorkerOutput::Captured(
                ledger::process(&root, &capture)?,
                cursor,
                unsupported,
                discussion_cursor,
                captured_sms,
                health,
            ))
        });
        self.context.worker.as_mut().unwrap().collects_context = true;
        ctx.notify();
    }

    fn poll_context_ai(&mut self) -> bool {
        let mut events = vec![];
        if let Some(run) = &self.context.ai {
            loop {
                match run.job.receiver.try_recv() {
                    Ok(event) => {
                        let done = matches!(event, home_ai::TextEvent::Done(_));
                        events.push(event);
                        if done {
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        events.push(home_ai::TextEvent::Done(Err(
                            "AI 会话已结束，可手动重试".into()
                        )));
                        break;
                    }
                }
            }
        }
        let changed = !events.is_empty();
        for event in events {
            match event {
                home_ai::TextEvent::Status(s) => self.context.status = s,
                home_ai::TextEvent::Done(result) => {
                    let Some(run) = self.context.ai.take() else {
                        continue;
                    };
                    if run.generation != self.context.generation
                        || self.context.scope.as_ref() != Some(&run.scope)
                    {
                        continue;
                    }
                    match result {
                        Ok(brief) => {
                            self.context.ai_save = Some(AiSave {
                                scope: run.scope,
                                target: run.target,
                                markdown: brief.markdown,
                            });
                            if !brief.session_cleaned {
                                self.context.session_warning =
                                    Some("手机临时会话尚未确认删除，可在 Amoo 会话中清理".into());
                            }
                            self.context.status = "AI 深化完成，正在保存独立版本…".into();
                        }
                        Err(error) => {
                            self.context.error = Some(error);
                            self.context.status.clear();
                        }
                    }
                }
            }
        }
        changed
    }
    fn start_context_ai(&mut self, target: AiTarget, automatic: bool) {
        // Native unattended generation must exclusively use the source-bound
        // structured pipeline. The older Markdown helper is not that route.
        if automatic
            && self.context.model_source
                != Some(stepx_companion::model_source::Source::IndependentApi)
        {
            return;
        }
        if self.demo_mode {
            self.context.status = "演示资料不会请求真实模型".into();
            return;
        }
        if self.context.ai.is_some()
            || self.context.ai_save.is_some()
            || self.context.pending_ai.is_some()
            || self.context.write_failed
            || self.context.relation_active()
        {
            return;
        }
        if automatic
            && (!self.context.automatic_text_available || !self.context_phone_identity_ready())
        {
            return;
        }
        let Some(scope) = self.context.scope.clone() else {
            return;
        };
        if target.prompt().is_empty() {
            self.context.error = Some("还没有可深化的事实，先继续积累资料".into());
            return;
        }
        let device = self.selected.clone().filter(|d| d.serial == scope.device);
        if !self.context.model_available {
            self.context.error =
                Some("请在模型设置检查所选来源；可选择手机 Amoo，或配置独立文本 API".into());
            return;
        }
        if self.context.model_source == Some(stepx_companion::model_source::Source::PhoneAccount)
            && device.is_none()
        {
            self.context.error =
                Some("当前选择手机 Amoo，请连接原手机后点击生成；不会改用独立 API".into());
            return;
        }
        let mut config = self.context.config();
        record_ai_attempt(
            &mut config,
            &target,
            automatic,
            Local::now().timestamp_millis(),
        );
        // The worker starts only after this intent has been persisted successfully.
        self.context.set_config(config);
        self.context.pending_ai = Some(PendingAi {
            scope,
            target,
            device,
            use_local: self.context.model_source
                == Some(stepx_companion::model_source::Source::IndependentApi),
        });
        self.context.error = None;
        self.context.session_warning = None;
        self.context.status = "正在保存本次 AI 请求记录…".into();
    }
    fn maybe_auto_context_ai(&mut self) {
        self.maybe_auto_context_ai_at(Local::now());
    }
    fn maybe_auto_context_ai_at(&mut self, now: chrono::DateTime<Local>) {
        if self.busy || self.productivity_is_busy() {
            return;
        }
        let now_ms = now.timestamp_millis();
        for period in &self.context.overview.periods {
            let key = automatic_period_key(period);
            let stable = self
                .context
                .period_stability
                .entry(key)
                .or_insert_with(|| (period.evidence_revision.clone(), now_ms));
            if stable.0 != period.evidence_revision {
                *stable = (period.evidence_revision.clone(), now_ms);
            }
        }
        let config = self.context.config();
        // Returning existing, authorized files is independent of model-source
        // availability. It cannot issue a new inference or replay an existing request.
        if config.enabled
            && config.return_outcomes
            && !self.context.ai_active()
            && self.context.worker.is_none()
            && self.selected.is_some()
        {
            let retry = self
                .context
                .current_outcomes()
                .into_iter()
                .find(|r| {
                    let incomplete = r.bundle.files.iter().any(|file| {
                        !r.deliveries
                            .iter()
                            .any(|delivery| delivery.sha256 == file.sha256)
                    });
                    incomplete && now_ms.saturating_sub(r.delivery_attempt_ms) >= 5 * 60_000
                })
                .map(|r| r.bundle.target.clone());
            if let Some(target) = retry {
                self.queue_return_outcome(target);
                return;
            }
        }
        if !config.enabled
            || !automatic_authorized(&config, self.context.model_source)
            || !self.context.automatic_text_available
            || self.context.ai_active()
            || self.context.productivity_problem().is_some()
            || self.context.worker.is_some()
            || self.context.ai_save.is_some()
        {
            return;
        }
        if !self.context_phone_identity_ready() {
            return;
        }
        let item = self
            .context
            .overview
            .items
            .iter()
            .find(|item| {
                item.presentation.as_ref().is_some_and(|p| p.priority > 0)
                    && item.updated_ms <= now_ms.saturating_sub(5000)
                    && item.updated_ms >= now_ms.saturating_sub(7 * 86_400_000)
                    && !config
                        .scheduled_attempts
                        .contains_key(&format!("outcome:item:{}:{}", item.id, item.input_revision))
            })
            .map(|item| OutcomeTarget::Item(item.id.clone()));
        let mut periods: Vec<_> = self
            .context
            .overview
            .periods
            .iter()
            .filter(|period| {
                if period.evidence_count == 0 || period.begin_ms > now_ms || now_ms >= period.end_ms
                {
                    return false;
                }
                let identity = format!(
                    "outcome:period:{:?}:{}",
                    period.kind, period.evidence_revision
                );
                if config.scheduled_attempts.contains_key(&identity) {
                    return false;
                }
                let key = automatic_period_key(period);
                if !self
                    .context
                    .period_stability
                    .get(&key)
                    .is_some_and(|(revision, since)| {
                        revision == &period.evidence_revision
                            && now_ms.saturating_sub(*since) >= AUTOMATIC_PERIOD_QUIET_MS
                    })
                {
                    return false;
                }
                true
            })
            .collect();
        // Closing periods get first choice, while every current natural period
        // can receive its initial partial review on an ordinary weekday.
        periods.sort_by_key(|period| {
            (
                !period_auto_due(period, now_ms, now.date_naive(), now.hour()),
                config
                    .automatic_period_completed
                    .contains_key(&automatic_period_key(period)),
            )
        });
        let period = periods
            .first()
            .map(|period| OutcomeTarget::Period(period.kind));
        let candidate = if config.automatic_next_period {
            period.clone().or(item.clone())
        } else {
            item.clone().or(period.clone())
        };
        if let Some(target) = candidate {
            let period_turn = matches!(target, OutcomeTarget::Period(_));
            self.queue_outcome_mode_at(target, config.return_outcomes, true, now_ms);
            if !self.context.pending_outcomes.is_empty() {
                let mut config = self.context.config();
                config.automatic_next_period = !period_turn;
                self.context.set_config(config);
            }
        }
    }
    fn context_phone_identity_ready(&self) -> bool {
        if self.context.model_source != Some(stepx_companion::model_source::Source::PhoneAccount) {
            return true;
        }
        self.context.scope.as_ref().is_some_and(|scope| {
            scope.account.is_some()
                && self.amoo_ui.account_scope == scope.account
                && self
                    .selected
                    .as_ref()
                    .is_some_and(|device| device.serial == scope.device)
        })
    }
    fn queue_current_periods(&mut self) {
        if self.context.ai_active() {
            self.context.status = "已有生成正在处理，请等待完成或先停止，再整理当前日周月".into();
            return;
        }
        if !self.context.model_available {
            self.context.error = Some("请连接手机并检查所选模型来源，再点击批量整理".into());
            return;
        }
        if !self.context_phone_identity_ready() {
            self.context.error = Some("请先连接原手机并核对当前账号，再整理当前周期".into());
            return;
        }
        let Some(source) = self.context.model_source else {
            self.context.error = Some("请先确认模型来源".into());
            return;
        };
        let now = Local::now().timestamp_millis();
        let targets: Vec<_> = self
            .context
            .overview
            .periods
            .iter()
            .filter(|period| {
                period.evidence_count > 0 && period.begin_ms <= now && now < period.end_ms
            })
            .map(|period| {
                (
                    OutcomeTarget::Period(period.kind),
                    period.evidence_revision.clone(),
                )
            })
            .collect();
        if targets.is_empty() {
            self.context.error = Some("尚无当前周期的真实材料，请先整理来源".into());
            return;
        }
        // One click authorizes this fixed snapshot only. It neither enables
        // unattended collection/inference nor restores new requests on restart.
        let mut config = self.context.config();
        let deliver = config.return_outcomes;
        config.period_batch = None;
        self.context.set_config(config);
        for (target, revision) in targets {
            self.queue_outcome_mode_at(target.clone(), deliver, false, now);
            if let Some(pending) = self
                .context
                .pending_outcomes
                .iter_mut()
                .find(|pending| pending.1 == target)
            {
                pending.6 = Some((source, revision));
            }
        }
        self.context.status = format!(
            "已排入 {} 份当前周期资料，逐份整理；同一版本复用成果，自动更新设置保持不变",
            self.context.pending_outcomes.len()
        );
    }
    fn queue_return_outcome(&mut self, target: OutcomeTarget) {
        let revision = self
            .context
            .current_outcomes()
            .into_iter()
            .find(|r| r.bundle.target == target)
            .map(|r| r.bundle.input_revision.clone());
        let Some(revision) = revision else {
            self.context.error = Some("资料已变化，请先生成新版成果再回传".into());
            return;
        };
        self.queue_outcome(target.clone(), true);
        if let Some(pending) = self
            .context
            .pending_outcomes
            .iter_mut()
            .find(|(_, t, _, _, _, _, _)| t == &target)
        {
            pending.5 = Some(revision);
            self.context.status = "正在回传已生成文件，不会重新调用模型…".into();
        }
    }
    fn queue_outcome(&mut self, target: OutcomeTarget, deliver: bool) {
        self.queue_outcome_mode(target, deliver, false);
    }
    fn queue_outcome_mode(&mut self, target: OutcomeTarget, deliver: bool, automatic: bool) {
        self.queue_outcome_mode_at(target, deliver, automatic, Local::now().timestamp_millis());
    }
    fn queue_outcome_mode_at(
        &mut self,
        target: OutcomeTarget,
        deliver: bool,
        automatic: bool,
        now_ms: i64,
    ) {
        if self.demo_mode || !self.context.persistent {
            self.context.status = "演示资料不会请求真实模型".into();
            return;
        }
        if self.context.write_failed || self.context.relation_active() {
            return;
        }
        if automatic
            && (!self.context.automatic_text_available || !self.context_phone_identity_ready())
        {
            return;
        }
        let Some(scope) = self.context.scope.clone() else {
            return;
        };
        let Some(device) = self.selected.clone().filter(|d| d.serial == scope.device) else {
            self.context.error = Some("请连接原手机后生成；已有成果仍可打开".into());
            return;
        };
        if !self.context.model_available
            && (!deliver || self.context.outcome_path(&target).is_none())
        {
            self.context.error = Some("请检查模型来源：可使用已连接手机的 Amoo 账号，或配置独立文本 API；来源材料已保存在本机".into());
            return;
        }
        if self
            .context
            .pending_outcomes
            .iter()
            .any(|(_, t, _, _, _, _, _)| t == &target)
        {
            return;
        }
        let identity = match &target {
            OutcomeTarget::Item(id) => self
                .context
                .overview
                .items
                .iter()
                .find(|i| &i.id == id)
                .map(|i| format!("outcome:item:{}:{}", i.id, i.input_revision)),
            OutcomeTarget::Period(kind) => self
                .context
                .overview
                .periods
                .iter()
                .find(|p| &p.kind == kind && p.evidence_count > 0)
                .map(|p| format!("outcome:period:{:?}:{}", p.kind, p.evidence_revision)),
        };
        let Some(identity) = identity else {
            self.context.error = Some("这段资料尚不足以成稿，继续讨论后再试".into());
            return;
        };
        let mut config = self.context.config();
        config.ai_started_ms = now_ms;
        config
            .scheduled_attempts
            .insert(identity, config.ai_started_ms);
        if automatic {
            config
                .automatic_period_completed
                .retain(|_, at| *at >= now_ms.saturating_sub(90 * 86_400_000));
        }
        config
            .scheduled_attempts
            .retain(|_, at| *at >= config.ai_started_ms.saturating_sub(90 * 86_400_000));
        self.context.set_config(config);
        let generation_guard = if automatic {
            let revision = match &target {
                OutcomeTarget::Item(id) => self
                    .context
                    .overview
                    .items
                    .iter()
                    .find(|item| &item.id == id)
                    .map(|item| item.input_revision.clone()),
                OutcomeTarget::Period(kind) => self
                    .context
                    .overview
                    .periods
                    .iter()
                    .find(|period| &period.kind == kind)
                    .map(|period| period.evidence_revision.clone()),
            };
            self.context.model_source.zip(revision)
        } else {
            None
        };
        self.context.pending_outcomes.push((
            scope,
            target,
            device,
            deliver,
            automatic,
            None,
            generation_guard,
        ));
        self.context.status = "正在保存生成记录，随后生成可追溯的成果…".into();
        self.context.error = None;
    }

    fn queue_context_relation(&mut self, edit: ledger::RelationEdit) {
        if self.demo_mode {
            self.context.status = "演示资料不会修改真实事项关联".into();
            return;
        }
        if self.context.relation_active() {
            self.context.status = "正在更新事项关联，请稍候…".into();
            return;
        }
        if self.context.write_failed {
            self.context.error = Some("请先修复记录保存问题，再修改事项关联".into());
            return;
        }
        let Some(scope) = self.context.scope.clone() else {
            return;
        };
        if self.context.preferences.pending_clear.contains(&scope) {
            self.context.error = Some("正在清理当前资料，完成后才能修改关联".into());
            return;
        }
        let valid = match &edit {
            ledger::RelationEdit::Link {
                left_object,
                right_object,
            }
            | ledger::RelationEdit::Reject {
                left_object,
                right_object,
            } => {
                left_object != right_object
                    && self.context.contains_source_object(left_object)
                    && self.context.contains_source_object(right_object)
            }
            ledger::RelationEdit::Separate { object } => {
                self.context.contains_source_object(object)
            }
        };
        if !valid {
            self.context.error = Some("资料已变化，请刷新后重新选择要关联的事项".into());
            return;
        }
        // A correction invalidates a model request built from the old grouping.
        // Existing disk work finishes first; the correction uses the same worker lane.
        self.context.invalidate();
        self.context.pending_relation = Some(PendingRelation { scope, edit });
        self.context.link_picker = None;
        self.context.error = None;
        self.context.status = "正在更新事项关联与回顾…".into();
    }

    pub(super) fn handle_context_action(&mut self, action: &ContextAction) {
        if !self.context.initialized {
            return;
        }
        if self.context.scope.is_none() {
            self.context.change_scope(self.context_scope());
        }
        match action {
            ContextAction::Setup => {
                if self.context.setup.is_some() {
                    self.context.setup = None;
                    self.context.panel = ContextPanel::Attention;
                } else {
                    self.context.setup = Some(self.context.config());
                    self.context.panel = ContextPanel::Manage;
                }
                self.scroll = Default::default();
            }
            ContextAction::Manage => {
                self.context.panel = ContextPanel::Manage;
                self.context.close_preview();
                self.scroll = Default::default();
            }
            ContextAction::Attention => {
                self.context.panel = ContextPanel::Attention;
                self.context.setup = None;
                self.context.confirm_clear = false;
                self.context.close_preview();
                self.scroll = Default::default();
            }
            ContextAction::ItemDetails(id) => {
                if self
                    .context
                    .overview
                    .items
                    .iter()
                    .any(|item| &item.id == id)
                {
                    self.context.panel = ContextPanel::Item(id.clone());
                    self.context.relationship_details = None;
                    self.context.relationship_page = 0;
                    self.context.close_preview();
                    self.scroll = Default::default();
                }
            }
            ContextAction::SourceTasks => {
                if let Some(draft) = self.context.setup.as_mut() {
                    draft.tasks = !draft.tasks;
                }
            }
            ContextAction::SourcePhone => {
                if let Some(draft) = self.context.setup.as_mut() {
                    draft.phone = !draft.phone;
                }
            }
            ContextAction::AutoReturn => {
                if let Some(draft) = self.context.setup.as_mut() {
                    draft.return_outcomes = !draft.return_outcomes;
                }
            }
            ContextAction::ReturnOutcomes(target) => self.queue_return_outcome(target.clone()),
            ContextAction::GenerateOutcomes => {
                let targets: Vec<_> = self
                    .context
                    .overview
                    .items
                    .iter()
                    .filter(|i| i.presentation.as_ref().is_some_and(|p| p.priority > 0))
                    .take(3)
                    .map(|i| OutcomeTarget::Item(i.id.clone()))
                    .chain(
                        self.context
                            .overview
                            .periods
                            .iter()
                            .filter(|p| p.evidence_count > 0)
                            .map(|p| OutcomeTarget::Period(p.kind)),
                    )
                    .collect();
                if targets.is_empty() {
                    self.context.error = Some("先选取手机讨论或消息，再生成成果".into());
                }
                for target in targets {
                    self.queue_outcome(target, self.context.config().return_outcomes);
                }
            }
            ContextAction::GenerateCurrentPeriods => self.queue_current_periods(),
            ContextAction::AutoAi => {
                if self.context.automatic_text_available {
                    if let Some(draft) = self.context.setup.as_mut() {
                        draft.automatic_ai =
                            !automatic_authorized(draft, self.context.model_source);
                        draft.automatic_source = self.context.model_source;
                    }
                } else {
                    if let Some(draft) = self.context.setup.as_mut() {
                        draft.automatic_ai = false;
                    }
                    self.context.status = self.context.automatic_reason.clone();
                }
            }
            ContextAction::Apply | ContextAction::PrepareOnce => {
                if self
                    .context
                    .scope
                    .as_ref()
                    .is_some_and(|scope| self.context.preferences.pending_clear.contains(scope))
                {
                    self.context.error = Some("正在清理当前记录，完成后可重新开启".into());
                    return;
                }
                if let Some(mut draft) = self.context.setup.take() {
                    draft.automatic_ai = self.context.automatic_text_available
                        && automatic_authorized(&draft, self.context.model_source);
                    if draft.automatic_ai {
                        draft.automatic_source = self.context.model_source;
                    }
                    if !draft.tasks && !draft.phone {
                        self.context.setup = Some(draft);
                        self.context.error = Some("至少选择一种整理来源".into());
                        return;
                    }
                    let previous = self.context.config();
                    draft.configure_collection(
                        &previous,
                        matches!(action, ContextAction::Apply),
                        Local::now().timestamp_millis(),
                    );
                    self.context.invalidate();
                    self.context.set_config(draft);
                    self.context.panel = ContextPanel::Attention;
                    self.context.force = true;
                    self.context.error = None;
                    self.context.write_failed = false;
                    self.context.status = if matches!(action, ContextAction::PrepareOnce) {
                        "正在按所选来源整理一次；完成后不会自动采集或调用模型。".into()
                    } else {
                        "正在开启本地自动整理…".into()
                    };
                }
            }
            ContextAction::Pause => {
                let mut config = self.context.config();
                config.enabled = false;
                config.period_batch = None;
                self.context.invalidate();
                self.context.set_config(config);
                self.context.status = "自动整理已暂停，已有资料仍可查看".into();
            }
            ContextAction::Refresh => {
                self.context.force = true;
                self.context.error = None;
                self.context.write_failed = false;
            }
            ContextAction::Period(period) => {
                self.context.close_preview();
                self.context.period = *period;
                self.context.panel = ContextPanel::Review;
                if let Some(path) = self
                    .context
                    .readable_outcome_path(&OutcomeTarget::Period(*period))
                    .or_else(|| {
                        self.context
                            .selected_period()
                            .filter(|period| period.evidence_count > 0)
                            .map(|period| period.path.clone())
                    })
                {
                    self.context.open_preview(&path);
                }
                self.scroll = Default::default();
            }
            ContextAction::Open(path) => {
                self.context.open_preview(path);
                self.scroll = Default::default();
            }
            ContextAction::ClosePreview => self.context.close_preview(),
            ContextAction::PreviewPage(page) => {
                if let Some(preview) = self.context.preview.as_mut() {
                    preview.page = (*page).min(preview.pages.len().saturating_sub(1));
                    preview.scroll = Default::default();
                }
            }
            ContextAction::ExternalOpen(path) | ContextAction::Send(path) => {
                if self.context.document_title(path).is_none() {
                    self.context.error = Some("这份资料已不在当前来源中，请重新选择。".into());
                    return;
                }
                if matches!(action, ContextAction::Send(_)) {
                    if self.context.is_historical_outcome(path) {
                        self.context.error =
                            Some("当前来源已更新，这份旧稿仅供查阅；请生成最新成果后回传。".into());
                        return;
                    }
                    if let Some(scope) = &self.context.scope {
                        if let Err(error) = context_document_path(&self.context.root, scope, path) {
                            if let Some(preview) = self
                                .context
                                .preview
                                .as_mut()
                                .filter(|preview| &preview.path == path)
                            {
                                preview.error = Some(error.to_string());
                            } else {
                                self.context.error = Some(error.to_string());
                            }
                            return;
                        }
                    }
                    if self.demo_mode {
                        self.context.status = "演示资料不会发送到手机".into();
                    } else if let Some(target) = self
                        .context
                        .outcome_for_path(path)
                        .map(|result| result.bundle.target.clone())
                    {
                        // Outcome returns must update the durable delivery journal
                        // and reuse its original revision/content-addressed sends.
                        self.queue_return_outcome(target);
                    } else {
                        self.prepare_file_transfer(vec![path.clone()], None);
                    }
                } else if let Some(scope) = &self.context.scope {
                    match context_document_path(&self.context.root, scope, path)
                        .and_then(|path| super::work_state::open_work_path(&path))
                    {
                        Ok(()) => {
                            self.context.status = "已请求在其他应用打开；也可直接在此阅读。".into()
                        }
                        Err(error) => {
                            if let Some(preview) = self
                                .context
                                .preview
                                .as_mut()
                                .filter(|preview| &preview.path == path)
                            {
                                preview.error = Some(format!("外部应用未能打开：{error}"));
                            } else {
                                self.context.error = Some(error.to_string());
                            }
                        }
                    }
                }
            }
            ContextAction::Clear => {
                self.context.confirm_clear = true;
            }
            ContextAction::CancelClear => {
                self.context.confirm_clear = false;
            }
            ContextAction::Coverage => {
                self.context.coverage_open = !self.context.coverage_open;
            }
            ContextAction::ToggleItems => {
                self.context.show_all_items = !self.context.show_all_items;
            }
            ContextAction::PickLink(id) => {
                if self
                    .context
                    .overview
                    .items
                    .iter()
                    .any(|item| &item.id == id && !item.source_objects.is_empty())
                {
                    self.context.link_picker = Some(id.clone());
                }
            }
            ContextAction::CancelLink => {
                self.context.link_picker = None;
            }
            ContextAction::Link(left_object, right_object) => {
                self.queue_context_relation(ledger::RelationEdit::Link {
                    left_object: left_object.clone(),
                    right_object: right_object.clone(),
                })
            }
            ContextAction::RejectLink(left_object, right_object) => {
                self.queue_context_relation(ledger::RelationEdit::Reject {
                    left_object: left_object.clone(),
                    right_object: right_object.clone(),
                })
            }
            ContextAction::Separate(object) => {
                self.queue_context_relation(ledger::RelationEdit::Separate {
                    object: object.clone(),
                })
            }
            ContextAction::Relationships(id) => {
                self.context.relationship_details =
                    if self.context.relationship_details.as_ref() == Some(id) {
                        None
                    } else {
                        Some(id.clone())
                    };
                self.context.relationship_page = 0;
            }
            ContextAction::RelationshipPage(page) => {
                if let Some(item) = self.context.relationship_details.as_ref().and_then(|id| {
                    self.context
                        .overview
                        .items
                        .iter()
                        .find(|item| &item.id == id)
                }) {
                    self.context.relationship_page =
                        (*page).min(item.source_objects.len().saturating_sub(1) / 6);
                }
            }
            ContextAction::ToggleCandidates => {
                self.context.show_all_candidates = !self.context.show_all_candidates;
            }
            ContextAction::ConfirmClear => {
                self.invalidate_productivity_for_clear();
                self.context.invalidate();
                let mut config = self.context.config();
                config.enabled = false;
                config.period_batch = None;
                config.ai_records.clear();
                config.ai_attempts.clear();
                config.scheduled_attempts.clear();
                config.sms_cursor = None;
                config.since_ms = 0;
                config.phone_since_ms = 0;
                self.context.set_config(config);
                if let Some(scope) = &self.context.scope {
                    if !self.context.preferences.pending_clear.contains(scope) {
                        self.context.preferences.pending_clear.push(scope.clone());
                    }
                }
                self.context.write_failed = false;
                self.context.confirm_clear = false;
                self.context.link_picker = None;
                self.context.relationship_details = None;
                self.context.relationship_page = 0;
                self.context.overview = Default::default();
                self.context.outcomes.clear();
                self.context.last_inference = None;
                self.context.period_stability.clear();
                self.context.status = "正在清理当前来源的本地整理记录…".into();
            }
            ContextAction::Deepen => {
                if let Some(period) = self.context.selected_period().cloned() {
                    self.queue_outcome(
                        OutcomeTarget::Period(period.kind),
                        self.context.config().return_outcomes,
                    );
                }
            }
            ContextAction::DeepenItem(id) => {
                if let Some(item) = self
                    .context
                    .overview
                    .items
                    .iter()
                    .find(|i| &i.id == id)
                    .cloned()
                {
                    self.queue_outcome(
                        OutcomeTarget::Item(item.id),
                        self.context.config().return_outcomes,
                    );
                }
            }
            ContextAction::StopAi => {
                self.context.invalidate();
                let mut config = self.context.config();
                config.automatic_ai = false;
                config.period_batch = None;
                self.context.automatic_error = false;
                self.context.automatic_retry_at = None;
                self.context.set_config(config);
                self.context.status =
                    "已请求停止当前生成，并暂停自动生成；可在来源设置中重新开启".into();
            }
        }
    }

    pub(super) fn render_context_preview_text(&self, body: &str) -> widgets::El {
        let p = self.p();
        let mut rows = vec![];
        let mut paragraph = String::new();
        let mut fence: Option<(u8, usize)> = None;
        let mut has_checkmarks = false;
        let flush = |rows: &mut Vec<widgets::El>, paragraph: &mut String| {
            if !paragraph.is_empty() {
                rows.push(self.text(std::mem::take(paragraph), BODY, p.text));
            }
        };
        // Formatting stays inert. Fenced code is never interpreted as a list,
        // quote, heading, link, or task control.
        for line in body.lines() {
            let trimmed = line.trim_start();
            let marker = trimmed.as_bytes().first().copied();
            let count = marker
                .map(|marker| trimmed.bytes().take_while(|byte| *byte == marker).count())
                .unwrap_or(0);
            let inside_fence = fence.is_some();
            if let Some((open_marker, open_count)) = fence {
                if marker == Some(open_marker)
                    && count >= open_count
                    && trimmed[count..].trim().is_empty()
                {
                    fence = None;
                }
            } else if matches!(marker, Some(b'`' | b'~')) && count >= 3 {
                fence = Some((marker.unwrap(), count));
            }
            let heading = trimmed.chars().take_while(|ch| *ch == '#').count();
            if !inside_fence
                && fence.is_none()
                && (1..=6).contains(&heading)
                && trimmed.as_bytes().get(heading) == Some(&b' ')
            {
                flush(&mut rows, &mut paragraph);
                rows.push(self.text_w(
                    &trimmed[heading + 1..],
                    if heading <= 2 { BODY + 2. } else { BODY },
                    p.text,
                    Weight::Semibold,
                ));
            } else if !inside_fence
                && fence.is_none()
                && context_preview_list_item(trimmed).is_some()
            {
                flush(&mut rows, &mut paragraph);
                let (marker, text, checkmark) = context_preview_list_item(trimmed).unwrap();
                has_checkmarks |= checkmark;
                let metadata = ["性质：", "负责人：", "期限："]
                    .iter()
                    .any(|prefix| text.starts_with(prefix));
                rows.push(self.text_w(
                    format!("{marker} {text}"),
                    if metadata { CAPTION } else { BODY },
                    if metadata { p.text2 } else { p.text },
                    if checkmark {
                        Weight::Semibold
                    } else {
                        Weight::Normal
                    },
                ));
            } else if !inside_fence && fence.is_none() && trimmed.starts_with("> ") {
                flush(&mut rows, &mut paragraph);
                rows.push(self.card_box(
                    self.text(&trimmed[2..], CAPTION, p.text2),
                    p.bg,
                    p.border,
                    R_SM,
                    S2,
                    S2,
                ));
            } else if line.is_empty() {
                flush(&mut rows, &mut paragraph);
            } else {
                if !paragraph.is_empty() {
                    paragraph.push('\n');
                }
                paragraph.push_str(line);
            }
        }
        flush(&mut rows, &mut paragraph);
        if has_checkmarks {
            rows.push(self.text(
                "勾选符号仅展示原稿记录，不能据此确认任务已执行。",
                MICRO,
                p.text3,
            ));
        }
        if rows.is_empty() {
            rows.push(self.text("这份文件目前没有正文。", BODY, p.text2));
        }
        self.vstack(S2, rows)
    }

    pub(super) fn context_item_primary(
        &self,
        item: &ledger::PreparedItem,
    ) -> (&'static str, Action) {
        use stepx_companion::context_review::MatterAction;
        let config = self.context.config();
        if let Some(path) = self
            .context
            .outcome_path(&OutcomeTarget::Item(item.id.clone()))
        {
            return ("查看方案与待办", Action::Context(ContextAction::Open(path)));
        }
        if let Some(ai) = config.ai_records.iter().find(|record| {
            record.target == format!("item:{}", item.id) && record.revision == item.input_revision
        }) {
            return (
                "查看草稿",
                Action::Context(ContextAction::Open(ai.path.clone())),
            );
        }
        let mut task_ids = item.task_ids.clone();
        task_ids.extend(item.task_id.iter().cloned());
        task_ids.sort();
        task_ids.dedup();
        let presentation = item.presentation.as_ref();
        let action = presentation.map(|presentation| presentation.primary_action);
        // Association can join multiple independent conversations. A summary's
        // source excerpt is not a task selector; never silently pick the first ID.
        if task_ids.len() > 1
            && matches!(
                action,
                Some(MatterAction::ContinueDiscussion | MatterAction::ReviewOutput)
            )
        {
            return (
                if action == Some(MatterAction::ReviewOutput) {
                    "选择成果"
                } else {
                    "选择讨论"
                },
                Action::Context(ContextAction::ItemDetails(item.id.clone())),
            );
        }
        let task = task_ids.first().and_then(|id| {
            self.workspace
                .journal
                .tasks
                .iter()
                .find(|task| &task.id == id && self.workspace_visible(task))
        });
        let mut phone_sessions: Vec<String> = Vec::new();
        let mut push_phone_session = |raw_id: &str| {
            if let Some(rest) = raw_id.strip_prefix("phone-amoo:") {
                let clean = rest
                    .split_once(":topic:")
                    .map(|(base, _)| base)
                    .unwrap_or(rest)
                    .trim();
                if !clean.is_empty() && !phone_sessions.iter().any(|s| s == clean) {
                    phone_sessions.push(clean.to_string());
                }
            }
        };
        push_phone_session(&item.id);
        for so in &item.source_objects {
            push_phone_session(&so.id);
        }
        if task_ids.is_empty()
            && phone_sessions.len() > 1
            && matches!(action, Some(MatterAction::ContinueDiscussion) | None)
        {
            return (
                "选择讨论",
                Action::Context(ContextAction::ItemDetails(item.id.clone())),
            );
        }
        let phone_amoo_id = phone_sessions
            .first()
            .map(|sess| format!("phone-amoo:{sess}"));
        match action {
            Some(MatterAction::ContinueDiscussion) if task.is_some() => {
                ("接着讨论", Action::HomeResumeTask(task.unwrap().id.clone()))
            }
            Some(MatterAction::ContinueDiscussion)
                if task_ids.is_empty() && phone_amoo_id.is_some() =>
            {
                (
                    "在电脑继续对话",
                    Action::HomeResumeTask(phone_amoo_id.unwrap()),
                )
            }
            Some(MatterAction::ReviewOutput) => {
                // A unique, exact recorded filename within the one source task
                // can open the actual artifact; otherwise show the source record.
                if let (Some(task), Some(name)) = (
                    task,
                    presentation.and_then(|presentation| presentation.available_output.as_deref()),
                ) {
                    let mut outputs = task.outputs.iter().filter(|output| output.name == name);
                    if let Some(output) = outputs.next().filter(|_| outputs.next().is_none()) {
                        return (
                            "打开成果",
                            Action::HomeOpenTaskOutput {
                                task: task.id.clone(),
                                path: output.path.clone(),
                            },
                        );
                    }
                }
                (
                    "查看成果记录",
                    Action::Context(ContextAction::Open(item.path.clone())),
                )
            }
            Some(MatterAction::PrepareSchedule) if self.selected.is_some() => {
                ("打开日历", Action::Page(Page::Calendar))
            }
            Some(MatterAction::PrepareSchedule) => (
                "查看日程记录",
                Action::Context(ContextAction::Open(item.path.clone())),
            ),
            Some(MatterAction::ContinueDiscussion) => (
                "查看讨论",
                Action::Context(ContextAction::Open(item.path.clone())),
            ),
            _ if task_ids.is_empty() && phone_amoo_id.is_some() => (
                "在电脑继续对话",
                Action::HomeResumeTask(phone_amoo_id.unwrap()),
            ),
            _ => (
                "核对这件事",
                Action::Context(ContextAction::Open(item.path.clone())),
            ),
        }
    }

    pub(super) fn context_home_is_focused(&self) -> bool {
        self.context.preview.is_some() || self.context.panel != ContextPanel::Attention
    }

    fn context_paint(c: ColorU) -> stepx_companion::home_visuals::Paint {
        stepx_companion::home_visuals::Paint::rgb(c.r, c.g, c.b).with_alpha(c.a as f32 / 255.0)
    }

    pub(super) fn render_home_context_visual_banner_if_ready(&self) -> Option<widgets::El> {
        let ui = &self.context;
        if !ui.initialized || ui.needs_load {
            return None;
        }
        if (self.home_tools_expanded || self.context_home_is_focused())
            && ui.overview.evidence_count > 0
        {
            let stage = if !ui.current_outcomes().is_empty() {
                3
            } else if !ui.overview.items.is_empty() {
                2
            } else {
                1
            };
            Some(self.render_context_visual_banner("ctx-home", stage))
        } else {
            None
        }
    }

    /// Native circles have a fixed square layout. Only the connector segments
    /// grow; the marker and its label are laid out in the same stage cell.
    pub(super) fn native_flow_stepper(
        &self,
        slot_prefix: &str,
        labels: &[&str],
        active_stage: usize,
        failed: bool,
        active_color: ColorU,
    ) -> widgets::El {
        use widgets::StepMark;
        if labels.is_empty() {
            return Empty::new().finish();
        }
        let p = self.p();
        let glyph_foreground = if p.dark { p.card } else { ColorU::white() };
        let marker = |index: usize| {
            let mark = context_flow_mark(index, active_stage, failed);
            let (background, border, child) = match mark {
                StepMark::Done => (
                    p.success_text,
                    p.success_text,
                    self.ui_icon(UiIcon::Check, 12., glyph_foreground),
                ),
                StepMark::Failed => (
                    p.danger,
                    p.danger,
                    self.ui_icon(UiIcon::Close, 12., glyph_foreground),
                ),
                StepMark::Active => (
                    active_color,
                    active_color,
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background_color(glyph_foreground)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                            .finish(),
                    )
                    .with_width(6.)
                    .with_height(6.)
                    .finish(),
                ),
                StepMark::Pending => (p.card, p.border_strong, Empty::new().finish()),
            };
            let circle = ConstrainedBox::new(
                Container::new(Align::new(child).finish())
                    .with_background_color(background)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        CONTEXT_FLOW_NODE_SIZE / 2.,
                    )))
                    .finish(),
            )
            .with_width(CONTEXT_FLOW_NODE_SIZE)
            .with_height(CONTEXT_FLOW_NODE_SIZE)
            .finish();
            context_geometry_probe(format!("{slot_prefix}-stage-{index}-marker"), circle)
        };
        let label = |index: usize, text: &str| {
            let mark = context_flow_mark(index, active_stage, failed);
            let foreground = match mark {
                StepMark::Done => p.success_text,
                StepMark::Failed => p.danger,
                StepMark::Active => active_color,
                StepMark::Pending => p.text3,
            };
            context_geometry_probe(
                format!("{slot_prefix}-stage-{index}-label"),
                self.text_w(
                    text,
                    MICRO,
                    foreground,
                    if matches!(mark, StepMark::Active | StepMark::Failed) {
                        Weight::Semibold
                    } else {
                        Weight::Medium
                    },
                ),
            )
        };
        let connector = |edge: usize, visible: bool| {
            let color = if !visible {
                ColorU::transparent_black()
            } else if edge < active_stage {
                p.success_text
            } else {
                p.border_strong
            };
            Expanded::new(
                1.,
                ConstrainedBox::new(Rect::new().with_background_color(color).finish())
                    .with_height(2.)
                    .finish(),
            )
            .finish()
        };
        let wide = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_children(
                labels
                    .iter()
                    .enumerate()
                    .map(|(index, text)| {
                        let marker_row = self.hstack(
                            0.,
                            vec![
                                connector(index.saturating_sub(1), index > 0),
                                marker(index),
                                connector(index, index + 1 < labels.len()),
                            ],
                        );
                        Expanded::new(
                            1.,
                            self.vstack(
                                7.,
                                vec![
                                    marker_row,
                                    Align::new(label(index, text)).top_center().finish(),
                                ],
                            ),
                        )
                        .finish()
                    })
                    .collect::<Vec<_>>(),
            )
            .finish();
        let narrow = self.vstack(
            S2,
            labels
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    self.hstack(
                        S2,
                        vec![
                            marker(index),
                            Expanded::new(1., label(index, text)).finish(),
                        ],
                    )
                })
                .collect(),
        );
        let element = SizeConstraintSwitch::new(
            wide,
            [(
                SizeConstraintCondition::WidthLessThan(labels.len() as f32 * 95.),
                narrow,
            )],
        )
        .finish();
        context_geometry_probe(format!("{slot_prefix}-stepper"), element)
    }

    fn render_context_visual_banner(&self, slot_prefix: &str, active_stage: usize) -> widgets::El {
        use stepx_companion::home_visuals::{donut, ring_gauge};
        let p = self.p();
        let ui = &self.context;
        let (active_stage, failed) = ui.flow_progress(active_stage);
        let flow_el = self.native_flow_stepper(
            slot_prefix,
            &["1. 采集来源", "2. 事项聚合", "3. 成稿核验", "4. 手机同步"],
            active_stage,
            failed,
            p.blue,
        );

        let mut task_sources = 0usize;
        let mut msg_sources = 0usize;
        let mut schedule_sources = 0usize;
        let mut doc_sources = 0usize;
        for item in &ui.overview.items {
            if item.source_objects.is_empty() {
                task_sources += item.source_count.max(1);
            }
            for src in &item.source_objects {
                let s = src.source.as_str();
                if s.contains("短信") || s.contains("通知") || s.contains("消息") {
                    msg_sources += 1;
                } else if s.contains("日历") || s.contains("日程") {
                    schedule_sources += 1;
                } else if s.contains("成果") || s.contains("文件") || s.contains("文档") {
                    doc_sources += 1;
                } else {
                    task_sources += 1;
                }
            }
        }
        if task_sources + msg_sources + schedule_sources + doc_sources == 0
            && ui.overview.evidence_count > 0
        {
            task_sources = ui.overview.evidence_count;
        }
        let segments = [
            (task_sources as f32, Self::context_paint(p.blue)),
            (msg_sources as f32, Self::context_paint(p.teal)),
            (schedule_sources as f32, Self::context_paint(p.indigo)),
            (doc_sources as f32, Self::context_paint(p.success)),
        ];
        // Source proportions are native flexible segments. Their round caps
        // have a fixed radius instead of stretching a 540px SVG texture.
        let source_counts = [task_sources, msg_sources, schedule_sources, doc_sources];
        let source_colors = [p.blue, p.teal, p.indigo, p.success];
        let segment_children = source_counts
            .into_iter()
            .zip(source_colors)
            .filter(|(count, _)| *count > 0)
            .map(|(count, color)| {
                Expanded::new(
                    count as f32,
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background_color(color)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                            .finish(),
                    )
                    .with_height(8.)
                    .finish(),
                )
                .finish()
            })
            .collect::<Vec<_>>();
        let spectrum_el = if segment_children.is_empty() {
            ConstrainedBox::new(
                Rect::new()
                    .with_background_color(with_alpha_f(p.text3, 0.16))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            )
            .with_height(8.)
            .finish()
        } else {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(2.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(segment_children)
                .finish()
        };
        let spectrum_el = context_geometry_probe(format!("{slot_prefix}-spectrum"), spectrum_el);
        let blue_soft = with_alpha_f(p.blue, if p.dark { 0.16 } else { 0.10 });
        let legend = self.wrap(vec![
            self.pill_badge(format!("对话/任务 {task_sources}"), p.blue, blue_soft),
            self.pill_badge(format!("短信/通知 {msg_sources}"), p.teal, p.teal_soft),
            self.pill_badge(
                format!("日程安排 {schedule_sources}"),
                p.indigo,
                p.indigo_soft,
            ),
            self.pill_badge(format!("文档成果 {doc_sources}"), p.success, p.success_soft),
        ]);

        let total_items = ui.overview.items.len();
        let attention_count = ui.attention_items_ranked().len();
        let outcome_count = ui.current_outcomes().len() + ui.home_ai_results().len();
        let deferred_count = total_items.saturating_sub(attention_count);
        let surface_matte = Self::context_paint(p.surface).with_alpha(1.0);
        let ring_track = Self::context_paint(p.text3)
            .with_alpha(0.18)
            .over(surface_matte);

        let kpi_ring_cell = |slot_suffix: &str,
                             count: usize,
                             label: &'static str,
                             ratio: f32,
                             fg: ColorU|
         -> widgets::El {
            let ring_el = self.svg_canvas(
                &format!("{slot_prefix}-{slot_suffix}"),
                ring_gauge(
                    36.0,
                    4.5,
                    ratio.clamp(0., 1.0),
                    ring_track,
                    Self::context_paint(fg),
                ),
                Some(36.0),
                36.0,
                false,
            );
            let ring_el =
                context_geometry_probe(format!("{slot_prefix}-{slot_suffix}-square"), ring_el);
            self.card_box(
                self.hstack(
                    S2,
                    vec![
                        ring_el,
                        Expanded::new(
                            1.,
                            self.vstack(
                                1.,
                                vec![
                                    self.line_w(
                                        format!("{count} 项"),
                                        SUBHEAD,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                    self.line(label, MICRO, p.text2),
                                ],
                            ),
                        )
                        .finish(),
                    ],
                ),
                p.surface,
                p.border,
                R_SM,
                8.,
                6.,
            )
        };

        let donut_el = self.svg_canvas(
            &format!("{slot_prefix}-donut"),
            donut(36.0, 5.5, &segments, ring_track, 0.015),
            Some(36.0),
            36.0,
            false,
        );
        let donut_el = context_geometry_probe(format!("{slot_prefix}-donut-square"), donut_el);
        let active_cats = [task_sources, msg_sources, schedule_sources, doc_sources]
            .into_iter()
            .filter(|c| *c > 0)
            .count();
        let donut_cell = self.card_box(
            self.hstack(
                S2,
                vec![
                    donut_el,
                    Expanded::new(
                        1.,
                        self.vstack(
                            1.,
                            vec![
                                self.line_w(
                                    format!("{active_cats} 类"),
                                    SUBHEAD,
                                    p.text,
                                    Weight::Semibold,
                                ),
                                self.line("证据来源", MICRO, p.text2),
                            ],
                        ),
                    )
                    .finish(),
                ],
            ),
            p.surface,
            p.border,
            R_SM,
            8.,
            6.,
        );

        let left_flow = self.vstack(6., vec![flow_el, spectrum_el, legend]);
        let right_gauges = self.grid_cols(
            vec![
                kpi_ring_cell(
                    "ring-attention",
                    attention_count,
                    "待关注变更",
                    context_metric_fraction(attention_count, total_items),
                    p.warning,
                ),
                kpi_ring_cell(
                    "ring-outcomes",
                    outcome_count,
                    "可阅成果",
                    context_metric_fraction(outcome_count, total_items),
                    p.success,
                ),
                kpi_ring_cell(
                    "ring-deferred",
                    deferred_count,
                    "沉淀跟踪",
                    context_metric_fraction(deferred_count, total_items),
                    p.indigo,
                ),
                donut_cell,
            ],
            2,
            S1,
        );
        let body = self.responsive_columns(
            880.,
            S3,
            S2,
            CrossAxisAlignment::Center,
            vec![(1.25, left_flow), (1.0, right_gauges)],
        );

        let content = context_geometry_probe(
            format!("{slot_prefix}-visual-content"),
            self.vstack(
                8.,
                vec![
                    row_between_expanded(
                        Flex::row()
                            .with_spacing(S2)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.section_icon_badge(UiIcon::Activity, 14., 26.))
                            .with_child(
                                Expanded::new(
                                    1.,
                                    self.text_w(
                                        "端到端流水线与证据分布",
                                        CAPTION,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                )
                                .finish(),
                            )
                            .finish(),
                        self.pill_badge(
                            format!(
                                "{} 条依据 · {} 项事项",
                                ui.overview.evidence_count,
                                ui.overview.items.len()
                            ),
                            p.indigo,
                            p.indigo_soft,
                        ),
                        S2,
                    ),
                    body,
                ],
            ),
        );
        context_geometry_probe(
            format!("{slot_prefix}-visual-card"),
            self.card_box(content, p.card, p.border, R_MD, 12., 12.),
        )
    }

    pub(super) fn context_effective_height(&self) -> super::pages::home::HomeModuleHeight {
        use super::pages::home::HomeModuleId;
        self.resolve_effective_height(
            HomeModuleId::ContextLedgerHub,
            self.home_modules.module_height(HomeModuleId::ContextLedgerHub),
        )
    }

    fn context_width_budgets(
        mod_w: super::pages::home::HomeModuleWidth,
        effective_h: super::pages::home::HomeModuleHeight,
    ) -> (usize, usize, usize, usize) {
        use super::pages::home::{HomeModuleHeight, HomeModuleWidth};
        let is_compact = effective_h == HomeModuleHeight::Compact;
        let is_tall = effective_h == HomeModuleHeight::Tall;
        match mod_w {
            HomeModuleWidth::OneThird => (20, 34, 1, 1),
            HomeModuleWidth::OneHalf => (30, 50, if is_compact { 1 } else { 2 }, 1),
            HomeModuleWidth::TwoThirds => (42, 68, 2, 2),
            HomeModuleWidth::Full => (58, 96, 2, if is_tall { 3 } else { 2 }),
        }
    }

    pub(super) fn context_value_tier_spec(
        &self,
    ) -> (usize, usize, usize, usize, usize, usize, bool, bool, bool) {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let mod_w = self.home_module_content_width(HomeModuleId::ContextLedgerHub);
        let effective_h = self.context_effective_height();
        let is_compact = effective_h == HomeModuleHeight::Compact;
        let is_tall = effective_h == HomeModuleHeight::Tall;
        let is_narrow = matches!(mod_w, HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf);
        let (title_limit, summary_limit, outcome_cols, item_cols) =
            Self::context_width_budgets(mod_w, effective_h);
        let max_outcomes = match effective_h {
            HomeModuleHeight::Compact => {
                if is_narrow {
                    1
                } else {
                    2
                }
            }
            HomeModuleHeight::Tall => 4,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => 2,
        };
        let max_items = match effective_h {
            HomeModuleHeight::Compact => 1,
            HomeModuleHeight::Tall => {
                if is_narrow {
                    6
                } else {
                    8
                }
            }
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                if is_narrow {
                    2
                } else {
                    4
                }
            }
        };
        let show_pipeline_banner = match mod_w {
            HomeModuleWidth::OneThird => false,
            HomeModuleWidth::OneHalf => is_tall,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => !is_compact,
        };
        (
            title_limit,
            summary_limit,
            outcome_cols,
            item_cols,
            max_outcomes,
            max_items,
            show_pipeline_banner,
            !is_compact,
            is_tall,
        )
    }

    fn render_context_item_card(&self, item: &ledger::PreparedItem) -> widgets::El {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        use stepx_companion::context_review::MatterAction;
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::ContextLedgerHub);
        let effective_h = self.context_effective_height();
        let (title_limit, summary_limit, _, _) = Self::context_width_budgets(mod_w, effective_h);

        let presentation = item
            .presentation
            .as_ref()
            .expect("attention items have a presentation");
        let (label, action) = self.context_item_primary(item);
        let blue_soft = with_alpha_f(p.blue, if p.dark { 0.16 } else { 0.10 });
        let (cat_icon, cat_label, cat_fg, cat_bg) = if presentation.priority >= 90 {
            (UiIcon::Alert, "关键变化", p.warning, p.warning_soft)
        } else {
            match presentation.primary_action {
                MatterAction::PrepareSchedule => {
                    (UiIcon::Calendar, "近期日程", p.indigo, p.indigo_soft)
                }
                MatterAction::ReviewOutput => (UiIcon::File, "成果可阅", p.success, p.success_soft),
                MatterAction::ContinueDiscussion => (UiIcon::Chat, "继续讨论", p.blue, blue_soft),
                _ => (UiIcon::Layers, "事项跟进", p.teal, p.teal_soft),
            }
        };

        let one_line_summary = if !presentation.what_happened.trim().is_empty() {
            Self::truncate_chars(presentation.what_happened.trim(), summary_limit)
        } else {
            Self::truncate_chars(presentation.next_step.trim(), summary_limit)
        };

        let badges = vec![
            self.line_w(
                Self::truncate_chars(&presentation.headline, title_limit),
                BODY + 1.,
                p.text,
                Weight::Semibold,
            ),
            self.pill_badge_icon(cat_label, cat_icon, cat_fg, cat_bg),
        ];
        let mut text_rows = vec![
            self.wrap(badges),
            self.line(one_line_summary, CAPTION, p.text2),
        ];
        if effective_h == HomeModuleHeight::Tall
            && !presentation.next_step.trim().is_empty()
            && !presentation.what_happened.trim().is_empty()
        {
            text_rows.push(self.line(
                format!(
                    "下一步：{}",
                    Self::truncate_chars(presentation.next_step.trim(), summary_limit)
                ),
                MICRO,
                p.text3,
            ));
        }

        let mut action_btns = vec![self.button_icon(
            format!("context-primary-{}", item.id),
            label,
            UiIcon::ArrowUpRight,
            action,
            Btn::Secondary,
        )];
        if mod_w != HomeModuleWidth::OneThird {
            action_btns.push(self.button(
                format!("context-detail-{}", item.id),
                "详情",
                Action::Context(ContextAction::ItemDetails(item.id.clone())),
                Btn::Ghost,
            ));
        }
        let controls = self.hstack(S1, action_btns);

        self.card_box(
            row_between_expanded(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(S2)
                    .with_child(self.icon_badge(cat_icon, 16., cat_fg, cat_bg, 34.))
                    .with_child(Expanded::new(1., self.vstack(4., text_rows)).finish())
                    .finish(),
                controls,
                S3,
            ),
            p.card,
            with_alpha_f(cat_fg, 0.3),
            R_MD,
            12.,
            10.,
        )
    }

    pub(super) fn render_home_context_results(&self) -> Option<widgets::El> {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let outcomes = self.context.current_outcomes();
        let drafts = self.context.home_ai_results();
        if outcomes.is_empty() && drafts.is_empty() {
            return None;
        }
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::ContextLedgerHub);
        let effective_h = self.context_effective_height();
        let is_narrow = matches!(mod_w, HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf);
        let (title_limit, summary_limit, outcome_cols, _) =
            Self::context_width_budgets(mod_w, effective_h);

        let max_outcomes = match effective_h {
            HomeModuleHeight::Compact => {
                if is_narrow {
                    1
                } else {
                    2
                }
            }
            HomeModuleHeight::Tall => 4,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => 2,
        };
        let total_count = (outcomes.len() + drafts.len()).min(max_outcomes);
        let mut children = vec![row_between_expanded(
            self.hstack(
                S2,
                vec![
                    self.icon_badge(UiIcon::Sparkles, 15., p.accent, p.accent_soft, 28.),
                    self.text_w("讨论形成的成果", BODY, p.text, Weight::Semibold),
                    self.pill_badge_icon(
                        format!("{total_count} 项可查阅"),
                        UiIcon::Check,
                        p.accent,
                        p.accent_soft,
                    ),
                ],
            ),
            if mod_w == HomeModuleWidth::OneThird {
                self.pill_badge("可回传", p.teal, p.teal_soft)
            } else {
                self.pill_badge("本地核验 · 可回传手机", p.teal, p.teal_soft)
            },
            S2,
        )];

        let mut outcome_cards: Vec<widgets::El> = Vec::with_capacity(max_outcomes);
        let split_single_bundle =
            outcomes.len() == 1 && drafts.is_empty() && outcomes[0].bundle.files.len() >= 2;
        if split_single_bundle {
            let result = outcomes[0];
            let received = result
                .bundle
                .files
                .iter()
                .filter(|file| result.deliveries.iter().any(|d| d.sha256 == file.sha256))
                .count();
            for (file_index, file) in result
                .bundle
                .files
                .iter()
                .take(max_outcomes.min(2))
                .enumerate()
            {
                let (label, icon, card_badge, card_fg, card_bg) = match file.kind.as_str() {
                    "plan" => (
                        "打开方案",
                        UiIcon::File,
                        "方案成稿",
                        p.accent,
                        p.accent_soft,
                    ),
                    "todos" => (
                        "查看待办",
                        UiIcon::Check,
                        "可执行待办",
                        p.indigo,
                        p.indigo_soft,
                    ),
                    "day" => (
                        "阅读日总结",
                        UiIcon::Activity,
                        "日总结",
                        p.teal,
                        p.teal_soft,
                    ),
                    "week" => (
                        "阅读周总结",
                        UiIcon::Activity,
                        "周总结",
                        p.teal,
                        p.teal_soft,
                    ),
                    "month" => (
                        "阅读月总结",
                        UiIcon::Activity,
                        "月总结",
                        p.teal,
                        p.teal_soft,
                    ),
                    _ => (
                        "打开成果",
                        UiIcon::FolderOpen,
                        "核验成稿",
                        p.accent,
                        p.accent_soft,
                    ),
                };
                let buttons = vec![
                    self.button_icon(
                        format!("home-outcome-0-{file_index}"),
                        label,
                        icon,
                        Action::Context(ContextAction::Open(file.path.clone())),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        format!("home-outcome-send-{file_index}"),
                        if result.delivery_error.is_some() {
                            "重试回传"
                        } else {
                            "回到手机"
                        },
                        UiIcon::Smartphone,
                        Action::Context(ContextAction::ReturnOutcomes(
                            result.bundle.target.clone(),
                        )),
                        Btn::Ghost,
                    ),
                ];
                let title_str = if file.kind == "todos" {
                    format!(
                        "{} · {} 项待办",
                        Self::truncate_chars(
                            &result.bundle.title,
                            title_limit.saturating_sub(8).max(10)
                        ),
                        result.bundle.todo_count
                    )
                } else {
                    Self::truncate_chars(&result.bundle.title, title_limit)
                };
                let desc_str = if file.kind == "todos" {
                    Self::truncate_chars(
                        &format!(
                            "共提炼 {} 项可执行待办，支持核对采纳并回传手机",
                            result.bundle.todo_count
                        ),
                        summary_limit,
                    )
                } else {
                    Self::truncate_chars(&result.bundle.conclusion, summary_limit)
                };
                outcome_cards.push(self.card_box(
                    self.vstack(
                        6.,
                        vec![
                            row_between_expanded(
                                self.hstack(
                                    S1,
                                    vec![
                                        self.icon_badge(icon, 14., card_fg, card_bg, 26.),
                                        self.text_w(title_str, BODY, p.text, Weight::Semibold),
                                    ],
                                ),
                                self.pill_badge(card_badge, card_fg, card_bg),
                                S1,
                            ),
                            self.text(desc_str, CAPTION, p.text2),
                            row_between_expanded(
                                self.wrap(buttons),
                                self.pill_badge(
                                    if received > 0 {
                                        "已回传"
                                    } else {
                                        "已核验"
                                    },
                                    if received > 0 { p.success } else { p.teal },
                                    if received > 0 {
                                        p.success_soft
                                    } else {
                                        p.teal_soft
                                    },
                                ),
                                S1,
                            ),
                        ],
                    ),
                    p.card,
                    with_alpha_f(card_fg, 0.28),
                    R_MD,
                    12.,
                    10.,
                ));
            }
        } else {
            for (index, result) in outcomes.into_iter().take(max_outcomes).enumerate() {
                let received = result
                    .bundle
                    .files
                    .iter()
                    .filter(|file| result.deliveries.iter().any(|d| d.sha256 == file.sha256))
                    .count();
                let mut buttons = vec![];
                for (file_index, file) in result.bundle.files.iter().enumerate() {
                    let (label, icon) = match file.kind.as_str() {
                        "plan" => ("打开方案", UiIcon::File),
                        "todos" => ("查看待办", UiIcon::Check),
                        "day" => ("阅读日总结", UiIcon::Activity),
                        "week" => ("阅读周总结", UiIcon::Activity),
                        "month" => ("阅读月总结", UiIcon::Activity),
                        _ => ("打开成果", UiIcon::FolderOpen),
                    };
                    buttons.push(self.button_icon(
                        format!("home-outcome-{index}-{file_index}"),
                        label,
                        icon,
                        Action::Context(ContextAction::Open(file.path.clone())),
                        Btn::Secondary,
                    ));
                }
                buttons.push(self.button_icon(
                    format!("home-outcome-send-{index}"),
                    if result.delivery_error.is_some() {
                        "重试回传"
                    } else {
                        "回到手机"
                    },
                    UiIcon::Smartphone,
                    Action::Context(ContextAction::ReturnOutcomes(result.bundle.target.clone())),
                    Btn::Ghost,
                ));

                let status_badge = if received > 0 {
                    self.pill_badge_icon(
                        format!("已回传 {received} 份"),
                        UiIcon::Check,
                        p.success,
                        p.success_soft,
                    )
                } else if result.delivery_error.is_some() {
                    self.pill_badge_icon("待重试回传", UiIcon::Alert, p.warning, p.warning_soft)
                } else {
                    self.pill_badge_icon("已核验成稿", UiIcon::Shield, p.accent, p.accent_soft)
                };

                let card = self.card_box(
                    self.vstack(
                        6.,
                        vec![
                            row_between_expanded(
                                self.hstack(
                                    S2,
                                    vec![
                                        self.icon_badge(
                                            UiIcon::File,
                                            15.,
                                            p.accent,
                                            p.accent_soft,
                                            28.,
                                        ),
                                        self.text_w(
                                            Self::truncate_chars(&result.bundle.title, title_limit),
                                            BODY,
                                            p.text,
                                            Weight::Semibold,
                                        ),
                                    ],
                                ),
                                self.hstack(
                                    S1,
                                    vec![
                                        self.pill_badge(
                                            format!("{} 项待办", result.bundle.todo_count),
                                            p.indigo,
                                            p.indigo_soft,
                                        ),
                                        status_badge,
                                    ],
                                ),
                                S1,
                            ),
                            self.text(
                                Self::truncate_chars(&result.bundle.conclusion, summary_limit),
                                CAPTION,
                                p.text2,
                            ),
                            row_between_expanded(
                                self.wrap(buttons),
                                if mod_w == HomeModuleWidth::OneThird {
                                    Empty::new().finish()
                                } else {
                                    self.stepper_index(
                                        &["采集", "成稿", "核验", "回传"],
                                        if received > 0 { 3 } else { 2 },
                                    )
                                },
                                S1,
                            ),
                        ],
                    ),
                    p.card,
                    with_alpha_f(p.accent, 0.28),
                    R_MD,
                    12.,
                    10.,
                );
                outcome_cards.push(card);
            }
        }

        let remaining_slots = max_outcomes.saturating_sub(outcome_cards.len());
        for (index, (title, path)) in drafts.into_iter().take(remaining_slots.min(2)).enumerate() {
            outcome_cards.push(self.card_box(
                row_between_expanded(
                    self.hstack(
                        S2,
                        vec![
                            self.section_icon_badge(UiIcon::File, 15., 30.),
                            self.vstack(
                                3.,
                                vec![
                                    self.hstack(
                                        S2,
                                        vec![
                                            self.text_w(
                                                Self::truncate_chars(&title, title_limit),
                                                BODY,
                                                p.text,
                                                Weight::Semibold,
                                            ),
                                            self.pill_badge("AI 草稿", p.indigo, p.indigo_soft),
                                        ],
                                    ),
                                    self.text("已有草稿 · 待核对", MICRO, p.text2),
                                ],
                            ),
                        ],
                    ),
                    self.button_icon(
                        format!("home-context-draft-{index}"),
                        "打开草稿",
                        UiIcon::FolderOpen,
                        Action::Context(ContextAction::Open(path)),
                        Btn::Secondary,
                    ),
                    S2,
                ),
                p.card,
                p.border,
                R_SM,
                12.,
                10.,
            ));
        }

        let cols = outcome_cards.len().clamp(1, outcome_cols);
        children.push(self.grid_cols(outcome_cards, cols, S2));
        Some(self.vstack(S2, children))
    }

    pub(super) fn render_context_panel(&self) -> widgets::El {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let ui = &self.context;
        if let Some(preview) = &ui.preview {
            return self.render_context_preview_panel(preview);
        }
        match &ui.panel {
            ContextPanel::Manage | ContextPanel::Item(_) => {
                return self.render_context_management_panel()
            }
            ContextPanel::Review => return self.render_context_review_panel(),
            ContextPanel::Attention => {}
        }
        let p = self.p();
        let config = ui.config();
        let items = ui.attention_items_ranked();
        let mod_w = self.home_module_content_width(HomeModuleId::ContextLedgerHub);
        let effective_h = self.context_effective_height();
        let is_compact = effective_h == HomeModuleHeight::Compact;
        let is_tall = effective_h == HomeModuleHeight::Tall;
        let is_narrow = matches!(mod_w, HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf);
        let (title_limit, summary_limit, _, item_cols) =
            Self::context_width_budgets(mod_w, effective_h);

        let _ = (&config, summary_limit);
        let header = row_between_expanded(
            self.home_section_heading(UiIcon::File, "讨论与成果"),
            self.button_icon(
                "context-manage",
                "管理",
                UiIcon::Settings,
                Action::Context(ContextAction::Manage),
                Btn::Ghost,
            ),
            S2,
        );

        let mut children = vec![header];
        if !ui.initialized || ui.needs_load {
            children.push(self.card_box(
                self.hstack(
                    S2,
                    vec![
                        self.icon_badge(UiIcon::Refresh, 15., p.accent, p.accent_soft, 28.),
                        self.text("正在查看最近的讨论和变化…", CAPTION, p.text2),
                    ],
                ),
                p.card,
                p.border,
                R_SM,
                12.,
                12.,
            ));
            return self.panel(children);
        }

        let _ = is_tall;
        if is_compact {
            if let Some(context_results) = self.render_home_context_results() {
                children.push(context_results);
                if !items.is_empty() {
                    children.push(self.divider());
                }
            }
        }

        if items.is_empty() {
            let message = if ui.overview.evidence_count == 0 {
                "还没有足够内容判断下一步。选取最近的讨论或手机消息后，可以把待确认的问题和可用成果放在这里。"
            } else if ui
                .overview
                .items
                .iter()
                .all(|item| item.presentation.is_none())
            {
                "这里保留了已保存的讨论与资料，可以先回看。"
            } else {
                "目前记录里，暂未发现明确的新变化。"
            };
            let mut empty_rows = vec![
                row_between_expanded(
                    self.hstack(
                        S2,
                        vec![
                            self.icon_badge(UiIcon::Sparkles, 16., p.indigo, p.indigo_soft, 30.),
                            self.text_w(
                                Self::truncate_chars(
                                    if ui.overview.evidence_count == 0 {
                                        "等待建立整理基线"
                                    } else {
                                        "当前事项状态平稳"
                                    },
                                    title_limit,
                                ),
                                SUBHEAD,
                                p.text,
                                Weight::Semibold,
                            ),
                        ],
                    ),
                    if mod_w == HomeModuleWidth::OneThird {
                        Empty::new().finish()
                    } else {
                        self.stepper_index(
                            &["选取来源", "提炼事项", "生成方案", "回传手机"],
                            if ui.overview.evidence_count > 0 { 1 } else { 0 },
                        )
                    },
                    S2,
                ),
                self.text(
                    Self::truncate_chars(message, summary_limit * 2),
                    CAPTION,
                    p.text2,
                ),
            ];
            if ui.overview.evidence_count == 0 {
                empty_rows.push(self.wrap(vec![self.button_icon(
                    "context-start",
                    if config.enabled {
                        "更新最近内容"
                    } else {
                        "选取要整理的内容"
                    },
                    if config.enabled {
                        UiIcon::Refresh
                    } else {
                        UiIcon::Sparkles
                    },
                    Action::Context(if config.enabled {
                        ContextAction::Refresh
                    } else {
                        ContextAction::Setup
                    }),
                    Btn::Secondary,
                )]));
            }
            children.push(self.card_box(
                self.vstack(8., empty_rows),
                p.card,
                p.border,
                R_MD,
                12.,
                12.,
            ));
        }

        if !items.is_empty() {
            let max_items = match effective_h {
                HomeModuleHeight::Compact => 1,
                HomeModuleHeight::Tall => {
                    if is_narrow {
                        6
                    } else {
                        8
                    }
                }
                HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                    if is_narrow {
                        2
                    } else {
                        4
                    }
                }
            };
            let item_cards: Vec<_> = items
                .into_iter()
                .take(max_items)
                .map(|item| self.render_context_item_card(item))
                .collect();
            let cols = item_cards.len().clamp(1, item_cols);
            children.push(self.grid_cols(item_cards, cols, S2));
        }

        if !is_compact {
            if let Some(context_results) = self.render_home_context_results() {
                children.push(self.divider());
                children.push(context_results);
            }
        }

        if ui.productivity_problem().is_some() || ui.write_failed {
            children.push(self.card_box(
                row_between_expanded(
                    self.hstack(
                        S2,
                        vec![
                            self.icon_badge(UiIcon::Alert, 14., p.danger, p.danger_soft, 26.),
                            self.text("本次更新没有完成，可以查看原因后重试。", CAPTION, p.danger),
                        ],
                    ),
                    self.button(
                        "context-attention-error",
                        "查看原因",
                        Action::Context(ContextAction::Manage),
                        Btn::Ghost,
                    ),
                    S2,
                ),
                p.danger_soft,
                with_alpha_f(p.danger, 0.3),
                R_SM,
                10.,
                8.,
            ));
        } else if ui.force
            || ui.ai_active()
            || ui.ai_save.is_some()
            || ui
                .worker
                .as_ref()
                .is_some_and(|worker| worker.scope.is_some())
        {
            children.push(self.hstack(
                S2,
                vec![
                    self.icon_badge(UiIcon::Refresh, 13., p.accent, p.accent_soft, 22.),
                    self.text(ui.panel_title(), MICRO, p.text2),
                ],
            ));
        }

        self.panel(children)
    }

    fn render_context_review_panel(&self) -> widgets::El {
        let p = self.p();
        let ui = &self.context;
        let config = ui.config();

        let header = row_between_expanded(
            self.hstack(
                S2,
                vec![
                    self.section_icon_badge(UiIcon::Activity, 18., 36.),
                    self.vstack(
                        2.,
                        vec![
                            self.hstack(
                                S2,
                                vec![
                                    self.heading(ui.period.label()),
                                    self.pill_badge_icon(
                                        match ui.period {
                                            ledger::Period::Day => "日终复盘",
                                            ledger::Period::Week => "周度复盘",
                                            ledger::Period::Month => "月度沉淀",
                                        },
                                        UiIcon::Sparkles,
                                        p.indigo,
                                        p.indigo_soft,
                                    ),
                                ],
                            ),
                            self.text(
                                match ui.period {
                                    ledger::Period::Day => {
                                        "回看今天推进了什么、还有什么没定，以及下一步先做什么。"
                                    }
                                    ledger::Period::Week => {
                                        "回看本周的进展、反复卡住的问题，以及下周如何调整。"
                                    }
                                    ledger::Period::Month => {
                                        "回看已经沉淀的成果、反复出现的问题，以及接下来值得投入什么。"
                                    }
                                },
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                ],
            ),
            self.hstack(
                S1,
                vec![
                    self.button(
                        "context-review-switch-day",
                        "今天",
                        Action::Context(ContextAction::Period(ledger::Period::Day)),
                        if ui.period == ledger::Period::Day {
                            Btn::Secondary
                        } else {
                            Btn::Ghost
                        },
                    ),
                    self.button(
                        "context-review-switch-week",
                        "本周",
                        Action::Context(ContextAction::Period(ledger::Period::Week)),
                        if ui.period == ledger::Period::Week {
                            Btn::Secondary
                        } else {
                            Btn::Ghost
                        },
                    ),
                    self.button(
                        "context-review-switch-month",
                        "本月",
                        Action::Context(ContextAction::Period(ledger::Period::Month)),
                        if ui.period == ledger::Period::Month {
                            Btn::Secondary
                        } else {
                            Btn::Ghost
                        },
                    ),
                    self.button_icon(
                        "context-review-back",
                        "返回首页",
                        UiIcon::ArrowUpRight,
                        Action::Context(ContextAction::Attention),
                        Btn::Ghost,
                    ),
                ],
            ),
            S2,
        );

        let generated_outcome = ui.outcome_path(&OutcomeTarget::Period(ui.period)).is_some();
        let stage = if generated_outcome {
            3
        } else if ui.selected_period().is_some_and(|pr| pr.evidence_count > 0) {
            2
        } else if ui.overview.evidence_count > 0 {
            1
        } else {
            0
        };
        let mut children = vec![header];
        if (self.home_tools_expanded || self.context_home_is_focused())
            && ui.overview.evidence_count > 0
        {
            children.push(self.render_context_visual_banner("ctx-review", stage));
        }

        if let Some(period) = ui.selected_period() {
            let mut period_rows = vec![row_between_expanded(
                self.hstack(
                    S2,
                    vec![
                        self.icon_badge(UiIcon::File, 16., p.accent, p.accent_soft, 32.),
                        self.text_w(&period.title, BODY + 1., p.text, Weight::Semibold),
                    ],
                ),
                self.hstack(
                    S1,
                    vec![
                        self.pill_badge(
                            format!("{} 条周期依据", period.evidence_count),
                            p.accent,
                            p.accent_soft,
                        ),
                        self.pill_badge(
                            if generated_outcome {
                                "已生成总结"
                            } else {
                                "可随时生成"
                            },
                            if generated_outcome { p.success } else { p.teal },
                            if generated_outcome {
                                p.success_soft
                            } else {
                                p.teal_soft
                            },
                        ),
                    ],
                ),
                S2,
            )];

            if period.evidence_count == 0 {
                period_rows.push(self.text(
                    "目前记录不足以总结这段时间，可先查看已有内容。",
                    CAPTION,
                    p.text2,
                ));
            }
            let mut buttons = vec![self.button_icon(
                "context-open-period",
                if !generated_outcome
                    && ui
                        .readable_outcome_path(&OutcomeTarget::Period(period.kind))
                        .is_some()
                {
                    "阅读上次成稿（有新资料）"
                } else if period.evidence_count > 0 {
                    "阅读回顾"
                } else {
                    "查看已有内容"
                },
                UiIcon::FolderOpen,
                Action::Context(ContextAction::Open(
                    ui.readable_outcome_path(&OutcomeTarget::Period(period.kind))
                        .unwrap_or_else(|| period.path.clone()),
                )),
                Btn::Primary,
            )];
            if let Some(ai) = config.ai_records.iter().find(|record| {
                record.target == format!("period:{:?}", period.kind)
                    && record.revision == period.evidence_revision
            }) {
                buttons.push(self.button_icon(
                    "context-open-ai",
                    "查看完善后的草稿",
                    UiIcon::File,
                    Action::Context(ContextAction::Open(ai.path.clone())),
                    Btn::Secondary,
                ));
                buttons.push(self.button_icon(
                    "context-send-ai",
                    "发送草稿到手机",
                    UiIcon::Smartphone,
                    Action::Context(ContextAction::Send(ai.path.clone())),
                    Btn::Ghost,
                ));
            }
            let generated = ui
                .outcome_path(&OutcomeTarget::Period(period.kind))
                .is_some();
            buttons.push(self.button_icon(
                "context-send-period",
                if generated {
                    "总结回到手机"
                } else {
                    "发送已有资料"
                },
                UiIcon::Smartphone,
                Action::Context(if generated {
                    ContextAction::ReturnOutcomes(OutcomeTarget::Period(period.kind))
                } else {
                    ContextAction::Send(period.path.clone())
                }),
                Btn::Ghost,
            ));
            if period.evidence_count > 0 {
                buttons.push(self.button_icon(
                    "context-deepen",
                    if ui.ai_active() {
                        "停止生成"
                    } else if generated {
                        "重新成稿"
                    } else {
                        "生成有结论的总结"
                    },
                    UiIcon::Sparkles,
                    Action::Context(if ui.ai_active() {
                        ContextAction::StopAi
                    } else {
                        ContextAction::Deepen
                    }),
                    Btn::Ghost,
                ));
            }
            period_rows.push(self.wrap(buttons));
            if period.evidence_count > 0 {
                period_rows.push(self.text(
                    "结合当前事项、变更和未决问题生成，事实与建议分开；材料覆盖不足会明确标注。",
                    MICRO,
                    p.text3,
                ));
            }
            period_rows.push(self.button_icon(
                "context-review-coverage",
                if ui.coverage_open {
                    "收起来源"
                } else {
                    "这些内容从哪来"
                },
                UiIcon::Database,
                Action::Context(ContextAction::Coverage),
                Btn::Ghost,
            ));
            if ui.coverage_open {
                period_rows.push(self.card_box(
                    self.text(&ui.overview.coverage, CAPTION, p.text2),
                    p.surface,
                    p.border,
                    R_SM,
                    10.,
                    10.,
                ));
            }
            children.push(self.card_box(
                self.vstack(8., period_rows),
                p.card,
                p.border,
                R_MD,
                14.,
                14.,
            ));
        } else {
            children.push(self.card_box(
                self.vstack(
                    8.,
                    vec![
                        self.hstack(
                            S2,
                            vec![
                                self.icon_badge(UiIcon::Info, 16., p.indigo, p.indigo_soft, 30.),
                                self.text(
                                    "这段时间的内容还不够，暂时没有回顾。可以先选取讨论或手机消息。",
                                    BODY,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.wrap(vec![self.button_icon(
                            "context-review-setup",
                            "选取要回顾的内容",
                            UiIcon::Settings,
                            Action::Context(ContextAction::Setup),
                            Btn::Secondary,
                        )]),
                    ],
                ),
                p.card,
                p.border,
                R_MD,
                14.,
                14.,
            ));
        }
        if !ui.overview.items.is_empty() {
            children.push(row_between_expanded(
                self.hstack(
                    S2,
                    vec![
                        self.section_icon_badge(UiIcon::Layers, 14., 26.),
                        self.line_w("本周期重点事项与进展", CAPTION, p.text, Weight::Semibold),
                    ],
                ),
                self.pill_badge(
                    format!("共 {} 项事项", ui.overview.items.len()),
                    p.teal,
                    p.teal_soft,
                ),
                S2,
            ));
            for item in ui.overview.items.iter().take(4) {
                if item.presentation.is_some() {
                    children.push(self.render_context_item_card(item));
                } else {
                    children.push(self.card_box(
                        row_between_expanded(
                            self.hstack(
                                S2,
                                vec![
                                    self.section_icon_badge(UiIcon::Layers, 15., 30.),
                                    self.vstack(
                                        2.,
                                        vec![
                                            self.line_w(
                                                Self::truncate_chars(&item.title, 56),
                                                BODY,
                                                p.text,
                                                Weight::Semibold,
                                            ),
                                            self.line(
                                                format!(
                                                    "{} 条来源依据 · 持续沉淀中",
                                                    item.source_count.max(1)
                                                ),
                                                CAPTION,
                                                p.text2,
                                            ),
                                        ],
                                    ),
                                ],
                            ),
                            self.button(
                                format!("context-review-item-{}", item.id),
                                "详情",
                                Action::Context(ContextAction::ItemDetails(item.id.clone())),
                                Btn::Ghost,
                            ),
                            S2,
                        ),
                        p.card,
                        p.border,
                        R_SM,
                        12.,
                        10.,
                    ));
                }
            }
        }
        if let Some(error) = ui.productivity_problem() {
            children.push(self.text(
                error,
                CAPTION,
                if ui.error.is_some() {
                    p.danger
                } else {
                    p.warning
                },
            ));
        }
        if ui.ai_active() || ui.ai_save.is_some() {
            children.push(self.text("正在结合已有内容完善回顾…", CAPTION, p.text2));
        }
        self.panel(children)
    }

    fn render_context_preview_panel(&self, preview: &ContextPreview) -> widgets::El {
        let p = self.p();
        let mut rows = vec![row_between_expanded(
            self.hstack(
                S2,
                vec![
                    self.section_icon_badge(UiIcon::File, 17., 34.),
                    self.vstack(
                        2.,
                        vec![
                            self.wrap(vec![
                                self.text_w(
                                    format!(
                                        "资料预览 · {}",
                                        Self::truncate_chars(&preview.title, 64)
                                    ),
                                    BODY + 1.,
                                    p.text,
                                    Weight::Semibold,
                                ),
                                self.pill_badge_icon(
                                    "只读核验",
                                    UiIcon::Shield,
                                    p.teal,
                                    p.teal_soft,
                                ),
                            ]),
                            self.line("本机资料 · 只读预览", MICRO, p.text3),
                        ],
                    ),
                ],
            ),
            self.button(
                "context-preview-close",
                "关闭预览",
                Action::Context(ContextAction::ClosePreview),
                Btn::Ghost,
            ),
            S2,
        )];
        if self.context.is_historical_outcome(&preview.path) {
            rows.push(self.text("这份成稿保留了生成时的内容，尚未包含之后新增的资料；可继续阅读，或返回后按最新资料重新生成。", CAPTION, p.warning));
        }
        if preview.loading {
            rows.push(self.text("正在读取本地文件…", BODY, p.text2));
        }
        if let Some(body) = preview.pages.get(preview.page) {
            rows.push(
                ConstrainedBox::new(self.pane_scroll(
                    preview.scroll.clone(),
                    self.card_box(
                        self.render_context_preview_text(body),
                        p.bg,
                        p.border,
                        R_SM,
                        S3,
                        S3,
                    ),
                ))
                .with_height((self.window_memory.preferences.full.height - 420.).clamp(260., 900.))
                .finish(),
            );
            if preview.pages.len() > 1 {
                let mut paging = vec![self.line(
                    format!("第 {} / {} 页", preview.page + 1, preview.pages.len()),
                    CAPTION,
                    p.text2,
                )];
                if preview.page > 0 {
                    paging.push(self.button(
                        "context-preview-prev",
                        "上一页",
                        Action::Context(ContextAction::PreviewPage(preview.page - 1)),
                        Btn::Ghost,
                    ));
                }
                if preview.page + 1 < preview.pages.len() {
                    paging.push(self.button(
                        "context-preview-next",
                        "下一页",
                        Action::Context(ContextAction::PreviewPage(preview.page + 1)),
                        Btn::Ghost,
                    ));
                }
                rows.push(self.wrap(paging));
            }
        }
        if let Some(error) = &preview.error {
            rows.push(self.text(error, CAPTION, p.danger));
            rows.push(self.button(
                "context-preview-retry",
                "重新读取",
                Action::Context(ContextAction::Open(preview.path.clone())),
                Btn::Secondary,
            ));
        }
        rows.push(self.wrap(vec![
            self.button_icon(
                "context-preview-external",
                "在其他应用打开",
                UiIcon::FolderOpen,
                Action::Context(ContextAction::ExternalOpen(preview.path.clone())),
                Btn::Secondary,
            ),
            if self.context.is_historical_outcome(&preview.path) {
                Empty::new().finish()
            } else {
                self.button_icon(
                    "context-preview-send",
                    self.context
                        .outcome_for_path(&preview.path)
                        .map(|result| format!("回传这组成果（{}份）", result.bundle.files.len()))
                        .unwrap_or_else(|| "发送这份文件到手机".into()),
                    UiIcon::Smartphone,
                    Action::Context(ContextAction::Send(preview.path.clone())),
                    Btn::Ghost,
                )
            },
        ]));
        rows.push(self.text(
            if self.context.is_historical_outcome(&preview.path) {
                "旧稿可在其他应用中打开；更新成果后再回传手机。"
            } else {
                "可在其他应用中编辑，或发送到手机继续查看。"
            },
            MICRO,
            p.text3,
        ));
        self.panel(rows)
    }

    fn render_context_management_panel(&self) -> widgets::El {
        use widgets::Btn;
        let p = self.p();
        let ui = &self.context;
        let config = ui.config();
        let item_detail = matches!(&ui.panel, ContextPanel::Item(_));

        let header = row_between_expanded(
            self.hstack(
                S2,
                vec![
                    self.icon_badge(
                        if item_detail {
                            UiIcon::Layers
                        } else {
                            UiIcon::Settings
                        },
                        18.,
                        p.accent,
                        p.accent_soft,
                        36.,
                    ),
                    self.vstack(
                        2.,
                        vec![
                            self.hstack(
                                S2,
                                vec![
                                    self.heading(if item_detail {
                                        "这件事的详情"
                                    } else {
                                        "资料与设置"
                                    }),
                                    self.pill_badge(
                                        format!("{} 条依据", ui.overview.evidence_count),
                                        p.accent,
                                        p.accent_soft,
                                    ),
                                    self.pill_badge(
                                        if config.enabled {
                                            "自动整理已开启"
                                        } else {
                                            "按需手动整理"
                                        },
                                        if config.enabled { p.success } else { p.text2 },
                                        if config.enabled {
                                            p.success_soft
                                        } else {
                                            p.card2
                                        },
                                    ),
                                ],
                            ),
                            self.text(
                                if item_detail {
                                    "核对事项依据、拆分或合并来源，并直接生成可追溯的方案与待办"
                                } else {
                                    "配置跨端采集范围、自动成稿策略与事项关联规则"
                                },
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                ],
            ),
            self.button_icon(
                "context-back",
                "返回首页",
                UiIcon::ArrowUpRight,
                Action::Context(ContextAction::Attention),
                Btn::Ghost,
            ),
            S2,
        );

        let stage = if !ui.current_outcomes().is_empty() {
            3
        } else if !ui.overview.items.is_empty() {
            2
        } else if ui.overview.evidence_count > 0 {
            1
        } else {
            0
        };
        let mut children = vec![
            header,
            self.render_context_visual_banner("ctx-manage", stage),
        ];

        if !item_detail && ui.setup.is_none() {
            let mut top_actions = vec![self.button_icon(
                "context-setup",
                "选择内容与更新方式",
                UiIcon::Settings,
                Action::Context(ContextAction::Setup),
                Btn::Secondary,
            )];
            if ui.overview.evidence_count > 0 {
                top_actions.insert(
                    0,
                    self.button_icon(
                        "context-generate-outcomes",
                        if ui.ai_active() {
                            "停止生成"
                        } else {
                            "生成方案、待办与日周月总结"
                        },
                        UiIcon::Sparkles,
                        Action::Context(if ui.ai_active() {
                            ContextAction::StopAi
                        } else {
                            ContextAction::GenerateOutcomes
                        }),
                        Btn::Primary,
                    ),
                );
            }
            children.push(self.wrap(top_actions));
            if ui.overview.evidence_count > 0 {
                children.push(self.text(
                    "使用当前已选材料生成可追溯的成果；来源未变时直接打开已生成版本。",
                    CAPTION,
                    p.text2,
                ));
            }
        }
        if !ui.initialized {
            children.push(self.line("正在加载本地整理记录…", CAPTION, p.text2));
            return self.panel(children);
        }
        if let Some(draft) = &ui.setup {
            let toggle_cards: Vec<widgets::El> = [
                (
                    "context-source-tasks",
                    "手机 Amoo 讨论、电脑对话与成果",
                    "汇总跨端对话、草稿与产出文件",
                    UiIcon::Chat,
                    p.accent,
                    p.accent_soft,
                    draft.tasks,
                    ContextAction::SourceTasks,
                ),
                (
                    "context-source-phone",
                    "手机短信、通知与日历",
                    "提取近期日程、验证码外通知与工作短信",
                    UiIcon::Smartphone,
                    p.teal,
                    p.teal_soft,
                    draft.phone,
                    ContextAction::SourcePhone,
                ),
                (
                    "context-auto-ai",
                    if self.context.model_source
                        == Some(stepx_companion::model_source::Source::PhoneAccount)
                    {
                        "手机账号 · 跟随新资料自动更新"
                    } else {
                        "跟随新资料自动更新方案与回顾"
                    },
                    if self.context.automatic_text_available {
                        "新资料稳定后无人值守更新；同一版本复用"
                    } else {
                        "无人值守通道尚未核验；可点击批量整理当前资料"
                    },
                    UiIcon::Sparkles,
                    p.indigo,
                    p.indigo_soft,
                    automatic_authorized(draft, self.context.model_source)
                        && self.context.automatic_text_available,
                    ContextAction::AutoAi,
                ),
                (
                    "context-auto-return",
                    "生成后自动回到这台手机",
                    "SHA-256 完整性核验后自动同步至手机",
                    UiIcon::Check,
                    p.success,
                    p.success_soft,
                    draft.return_outcomes,
                    ContextAction::AutoReturn,
                ),
            ]
            .into_iter()
            .map(|(id, label, sub, icon, fg, bg, on, action)| {
                self.card_box(
                    row_between_expanded(
                        self.hstack(
                            S2,
                            vec![
                                self.icon_badge(icon, 15., fg, bg, 30.),
                                self.vstack(
                                    2.,
                                    vec![
                                        self.line_w(label, BODY, p.text, Weight::Semibold),
                                        self.text(sub, MICRO, p.text2),
                                    ],
                                ),
                            ],
                        ),
                        self.toggle_switch(id, on, Action::Context(action)),
                        S2,
                    ),
                    p.surface,
                    if on { with_alpha_f(fg, 0.35) } else { p.border },
                    R_SM,
                    12.,
                    10.,
                )
            })
            .collect();

            children.push(self.card_box(
                self.vstack(
                    10.,
                    vec![
                        self.text(
                            "按当前设备与账号保存，已有对话用于背景。选中手机来源后，先整理一次会包含已读取的手机快照与近 7 天短信与手机讨论（短信每次最多 500 条，未补齐会说明）；再次整理沿用上次读取位置，避免遗漏期间的新消息。",
                            CAPTION,
                            p.text2,
                        ),
                        self.grid_cols(toggle_cards, 2, S2),
                        self.text(
                            if self.context.model_source==Some(stepx_companion::model_source::Source::PhoneAccount) {
                                "本地采集不上传正文。当前手机可由你点击批量整理。无人值守更新须先核验当前账号的纯文本通道，再明确开启；通过后使用手机服务，USB和无线均可。通道未就绪或账号变化时暂停请求，不改用独立API。日周月按真实资料变化更新，同版本不重复请求。已授权文件回传独立保留。"
                            } else {
                                "本地采集不上传正文。后台生成把所选证据交给明确选择的模型来源；模型用量与可用额度由手机或所选服务决定。首次补齐当前日周月，后续资料变化稳定15秒后更新，与事项生成轮流处理；同一来源版本复用成果。服务未返回的额度不显示，也不按本地请求次数推算。"
                            },
                            CAPTION,
                            p.text2,
                        ),
                        self.text(
                            "开启自动整理时，手机消息从现在开始积累。关闭来源仅停止新增，已有资料仍参与回顾；清理记录可移除。仅覆盖实际收到的通知与已读到的数据。",
                            CAPTION,
                            p.text2,
                        ),
                        self.wrap(vec![
                            self.button_icon(
                                "context-once",
                                "先整理一次",
                                UiIcon::Sparkles,
                                Action::Context(ContextAction::PrepareOnce),
                                Btn::Primary,
                            ),
                            self.button_icon(
                                "context-apply",
                                "开启自动整理",
                                UiIcon::Check,
                                Action::Context(ContextAction::Apply),
                                Btn::Secondary,
                            ),
                            self.button(
                                "context-cancel",
                                "取消",
                                Action::Context(ContextAction::Setup),
                                Btn::Ghost,
                            ),
                        ]),
                    ],
                ),
                p.card,
                p.border,
                R_MD,
                14.,
                14.,
            ));
        }
        if ui.overview.evidence_count == 0 {
            children.push(self.card_box(
                self.vstack(
                    6.,
                    vec![
                        self.text(
                            if config.enabled {
                                "尚无可整理的资料。所选来源出现新讨论、手机消息或成果后，会在这里按事项汇总。"
                            } else {
                                "把同一件事的讨论、手机消息和文件放在一起，找出已决定什么、还缺什么，以及下一步可以做什么。"
                            },
                            BODY,
                            p.text2,
                        ),
                        if ui.setup.is_none() && !config.enabled {
                            self.text(
                                "先选择来源，可只整理一次；开启自动整理后才会持续更新。",
                                CAPTION,
                                p.text2,
                            )
                        } else {
                            Empty::new().finish()
                        },
                    ],
                ),
                p.card,
                p.border,
                R_MD,
                12.,
                12.,
            ));
        }
        if !ui.overview.items.is_empty() || !ui.overview.periods.is_empty() {
            children.push(self.line(
                format!(
                    "本地整理 · {} 条依据 · {}",
                    ui.overview.evidence_count,
                    if config.enabled {
                        "自动更新已开启"
                    } else {
                        "自动更新未开启"
                    }
                ),
                CAPTION,
                p.text2,
            ));
            let detail_items = ui.detail_items();
            let page_key = format!("context-details:{}",ui.scope.as_ref().map(Scope::key).unwrap_or_default());
            let range = self.list_page_range(&page_key,detail_items.len(),20);
            children.push(self.list_page_controls(&page_key,detail_items.len(),20));
            for item in detail_items.into_iter().skip(range.start).take(range.len()) {
                let mut item_children = Vec::new();
                if item_detail {
                    if let Some(presentation) = &item.presentation {
                        let mut pres_rows = vec![
                            self.text_w(
                                &presentation.headline,
                                BODY + 1.,
                                p.text,
                                Weight::Semibold,
                            ),
                            self.text(&presentation.what_happened, BODY, p.text),
                        ];
                        if !presentation.next_step.is_empty() {
                            pres_rows.push(self.text(
                                format!("可以接着做：{}", presentation.next_step),
                                CAPTION,
                                p.text2,
                            ));
                        }
                        if !presentation.basis_note.is_empty() {
                            pres_rows.push(self.text(&presentation.basis_note, MICRO, p.text3));
                        }
                        item_children.push(self.card_box(
                            self.vstack(5., pres_rows),
                            p.surface,
                            p.border,
                            R_SM,
                            10.,
                            10.,
                        ));
                    }
                }
                let mut buttons = vec![self.button_icon(
                    format!("context-item-{}", item.id),
                    "查看资料",
                    UiIcon::FolderOpen,
                    Action::Context(ContextAction::Open(item.path.clone())),
                    Btn::Secondary,
                )];
                let outcome = ui
                    .current_outcomes()
                    .into_iter()
                    .find(|r| r.bundle.target == OutcomeTarget::Item(item.id.clone()));
                if let Some(result) = outcome {
                    for (index, file) in result.bundle.files.iter().enumerate() {
                        buttons.push(self.button_icon(
                            format!("context-item-outcome-{}-{index}", item.id),
                            if file.kind == "todos" {
                                "查看待办"
                            } else {
                                "打开方案"
                            },
                            if file.kind == "todos" {
                                UiIcon::Check
                            } else {
                                UiIcon::File
                            },
                            Action::Context(ContextAction::Open(file.path.clone())),
                            Btn::Primary,
                        ));
                    }
                    buttons.push(self.button_icon(
                        format!("context-item-return-{}", item.id),
                        "成果回到手机",
                        UiIcon::Smartphone,
                        Action::Context(ContextAction::ReturnOutcomes(
                            result.bundle.target.clone(),
                        )),
                        Btn::Ghost,
                    ));
                } else {
                    buttons.push(self.button_icon(
                        format!("context-item-ai-{}", item.id),
                        if ui.ai_active() {
                            "停止生成"
                        } else {
                            "生成方案与待办"
                        },
                        UiIcon::Sparkles,
                        Action::Context(if ui.ai_active() {
                            ContextAction::StopAi
                        } else {
                            ContextAction::DeepenItem(item.id.clone())
                        }),
                        Btn::Ghost,
                    ));
                }
                if let Some(ai) = config.ai_records.iter().find(|r| {
                    r.target == format!("item:{}", item.id) && r.revision == item.input_revision
                }) {
                    buttons.push(self.button_icon(
                        format!("context-item-open-ai-{}", item.id),
                        "查看 AI 稿",
                        UiIcon::File,
                        Action::Context(ContextAction::Open(ai.path.clone())),
                        Btn::Primary,
                    ));
                    buttons.push(self.button_icon(
                        format!("context-item-send-ai-{}", item.id),
                        "发送手机",
                        UiIcon::Smartphone,
                        Action::Context(ContextAction::Send(ai.path.clone())),
                        Btn::Ghost,
                    ));
                }
                let mut task_ids = item.task_ids.clone();
                if task_ids.is_empty() {
                    task_ids.extend(item.task_id.iter().cloned());
                }
                task_ids.sort();
                task_ids.dedup();
                if task_ids.len() == 1 {
                    buttons.push(self.button(
                        format!("context-task-{}", item.id),
                        "继续原任务",
                        Action::HomeResumeTask(task_ids[0].clone()),
                        Btn::Ghost,
                    ));
                } else if task_ids.len() > 1 {
                    buttons.push(self.button(
                        format!("context-task-choice-{}", item.id),
                        format!("选择原任务（{}）", task_ids.len()),
                        Action::Context(ContextAction::Relationships(item.id.clone())),
                        Btn::Ghost,
                    ));
                }
                if !item.source_objects.is_empty() {
                    buttons.push(self.button(
                        format!("context-relations-{}", item.id),
                        if ui.relationship_details.as_ref() == Some(&item.id) {
                            "收起来源与关联".into()
                        } else {
                            format!("来源与关联（{}）", item.source_objects.len())
                        },
                        Action::Context(ContextAction::Relationships(item.id.clone())),
                        Btn::Ghost,
                    ));
                    if ui.overview.items.len() > 1 {
                        buttons.push(self.button(
                            format!("context-link-picker-{}", item.id),
                            "关联其他事项",
                            Action::Context(ContextAction::PickLink(item.id.clone())),
                            Btn::Ghost,
                        ));
                    }
                }
                let source_summary = if item.source_objects.is_empty() {
                    format!("{} 条资料 · 来源关联尚未整理", item.source_count)
                } else {
                    let mut sources: Vec<_> = item
                        .source_objects
                        .iter()
                        .map(|source| source.source.as_str())
                        .collect();
                    sources.sort();
                    sources.dedup();
                    format!(
                        "{} 个来源 · {} 条资料 · {}",
                        item.source_objects.len(),
                        item.source_count,
                        sources.join("、")
                    )
                };
                item_children.push(row_between_expanded(
                    self.hstack(
                        S2,
                        vec![
                            self.icon_badge(UiIcon::Layers, 16., p.accent, p.accent_soft, 32.),
                            self.vstack(
                                2.,
                                vec![
                                    self.text_w(
                                        Self::truncate_chars(&item.title, 72),
                                        BODY + 1.,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                    self.text(source_summary, CAPTION, p.text2),
                                ],
                            ),
                        ],
                    ),
                    self.pill_badge(
                        format!("{} 条依据", item.source_count.max(1)),
                        p.indigo,
                        p.indigo_soft,
                    ),
                    S2,
                ));
                if let Some(reason) = item.relationship_notes.first() {
                    item_children.push(self.text(reason, MICRO, p.text2));
                }
                item_children.push(self.wrap(buttons));
                if item_detail && task_ids.len() > 1 {
                    item_children.push(self.text(
                        "这件事关联了多段讨论，选择你要继续的一段：",
                        CAPTION,
                        p.text2,
                    ));
                    for task in task_ids.iter().filter_map(|id| {
                        self.workspace
                            .journal
                            .tasks
                            .iter()
                            .find(|task| &task.id == id && self.workspace_visible(task))
                    }) {
                        item_children.push(row_between_expanded(
                            self.text(Self::truncate_chars(&task.title, 72), CAPTION, p.text),
                            self.button(
                                format!("context-choose-task-{}-{}", item.id, task.id),
                                "继续这段讨论",
                                Action::HomeResumeTask(task.id.clone()),
                                Btn::Secondary,
                            ),
                            S2,
                        ));
                        for index in task.current_output_indices() {
                            let output = &task.outputs[index];
                            item_children.push(row_between_expanded(
                                self.text(Self::truncate_chars(&output.name, 64), CAPTION, p.text2),
                                self.button(
                                    format!(
                                        "context-choose-output-{}-{}-{index}",
                                        item.id, task.id
                                    ),
                                    "打开成果",
                                    Action::HomeOpenTaskOutput {
                                        task: task.id.clone(),
                                        path: output.path.clone(),
                                    },
                                    Btn::Ghost,
                                ),
                                S2,
                            ));
                        }
                    }
                }
                if ui.relationship_details.as_ref() == Some(&item.id) {
                    if item.title.chars().count() > 72 {
                        item_children.push(self.text(
                            format!("完整事项标题：{}", item.title),
                            CAPTION,
                            p.text,
                        ));
                    }
                    for note in &item.relationship_notes {
                        item_children.push(self.text(note, MICRO, p.text2));
                    }
                    let page = ui
                        .relationship_page
                        .min(item.source_objects.len().saturating_sub(1) / 6);
                    for (index, source) in item
                        .source_objects
                        .iter()
                        .enumerate()
                        .skip(page * 6)
                        .take(6)
                    {
                        item_children.push(self.text(
                            format!("{} · {}", source.source, source.title),
                            CAPTION,
                            p.text,
                        ));
                        let mut source_actions = vec![];
                        if let Some(task_id) = &source.task_id {
                            source_actions.push(self.button(
                                format!("context-source-task-{}-{index}", item.id),
                                "继续此任务",
                                Action::HomeResumeTask(task_id.clone()),
                                Btn::Secondary,
                            ));
                        }
                        if item.source_objects.len() > 1 {
                            source_actions.push(self.button(
                                format!("context-separate-{}-{index}", item.id),
                                "这份资料单独整理",
                                Action::Context(ContextAction::Separate(source.id.clone())),
                                Btn::Ghost,
                            ));
                        }
                        if !source_actions.is_empty() {
                            item_children.push(self.wrap(source_actions));
                        }
                    }
                    if item.source_objects.len() > 6 {
                        let mut paging = vec![self.line(
                            format!(
                                "第 {} / {} 页来源",
                                page + 1,
                                item.source_objects.len().div_ceil(6)
                            ),
                            MICRO,
                            p.text2,
                        )];
                        if page > 0 {
                            paging.push(self.button(
                                format!("context-source-prev-{}", item.id),
                                "上一页",
                                Action::Context(ContextAction::RelationshipPage(page - 1)),
                                Btn::Ghost,
                            ));
                        }
                        if (page + 1) * 6 < item.source_objects.len() {
                            paging.push(self.button(
                                format!("context-source-next-{}", item.id),
                                "下一页",
                                Action::Context(ContextAction::RelationshipPage(page + 1)),
                                Btn::Ghost,
                            ));
                        }
                        item_children.push(self.wrap(paging));
                    }
                    item_children.push(self.text(
                        "修改关联只影响这里的整理与回顾，原消息、文件和任务均保留。",
                        MICRO,
                        p.text3,
                    ));
                }
                children.push(self.card_box(
                    self.vstack(6., item_children),
                    p.card,
                    p.border,
                    R_MD,
                    12.,
                    12.,
                ));
            }
            if !item_detail && ui.overview.items.len() > 2 {
                children.push(self.wrap(vec![self.button(
                    "context-more-items",
                    if ui.show_all_items {
                        "收起".into()
                    } else {
                        format!("查看其余 {} 项", ui.overview.items.len() - 2)
                    },
                    Action::Context(ContextAction::ToggleItems),
                    Btn::Ghost,
                )]));
            }
            if let Some(left) = ui
                .link_picker
                .as_ref()
                .and_then(|id| ui.overview.items.iter().find(|item| &item.id == id))
            {
                if let Some(left_source) = left.source_objects.first() {
                    let mut choices = vec![];
                    let matching: Vec<_> = ui.overview.items.iter().filter(|item|item.id!=left.id).collect();
                    let page_key = format!("context-link-choices:{}",left.id);
                    let range = self.list_page_range(&page_key,matching.len(),20);
                    choices.push(self.list_page_controls(&page_key,matching.len(),20));
                    for right in matching.into_iter().skip(range.start).take(range.len()) {
                        if let Some(right_source) = right.source_objects.first() {
                            choices.push(self.button(
                                format!("context-link-choice-{}", right.id),
                                format!("合并：{}", Self::truncate_chars(&right.title, 22)),
                                Action::Context(ContextAction::Link(
                                    left_source.id.clone(),
                                    right_source.id.clone(),
                                )),
                                Btn::Secondary,
                            ));
                        }
                    }
                    choices.push(self.button(
                        "context-link-cancel",
                        "取消关联",
                        Action::Context(ContextAction::CancelLink),
                        Btn::Ghost,
                    ));
                    children.push(self.card_box(
                        self.vstack(
                            6.,
                            vec![
                                self.text(
                                    format!("将“{}”与哪一项一起整理？", left.title),
                                    BODY,
                                    p.text,
                                ),
                                self.text(
                                    "确认属于同一件事后选择合并；两边来源和原任务都会保留，之后仍可拆开。",
                                    MICRO,
                                    p.text2,
                                ),
                                self.wrap(choices),
                            ],
                        ),
                        p.surface,
                        with_alpha_f(p.accent, 0.35),
                        R_SM,
                        12.,
                        12.,
                    ));
                }
            }
            if !item_detail && !ui.overview.link_candidates.is_empty() {
                children.push(self.wrap(vec![self.button(
                    "context-candidates-more",
                    if ui.show_all_candidates {
                        "收起待确认关联".into()
                    } else {
                        format!("查看 {} 组待确认关联", ui.overview.link_candidates.len())
                    },
                    Action::Context(ContextAction::ToggleCandidates),
                    Btn::Ghost,
                )]));
                if ui.show_all_candidates {
                    let range = self.list_page_range("context-link-candidates",ui.overview.link_candidates.len(),20);
                    children.push(self.list_page_controls("context-link-candidates",ui.overview.link_candidates.len(),20));
                    for (index, candidate) in ui.overview.link_candidates.iter().enumerate().skip(range.start).take(range.len()) {
                        children.push(self.card_box(
                            self.vstack(
                                6.,
                                vec![
                                    self.hstack(
                                        S2,
                                        vec![
                                            self.icon_badge(
                                                UiIcon::Link,
                                                14.,
                                                p.indigo,
                                                p.indigo_soft,
                                                26.,
                                            ),
                                            self.text_w(
                                                format!(
                                                    "{} ↔ {}",
                                                    candidate.left_title, candidate.right_title
                                                ),
                                                BODY,
                                                p.text,
                                                Weight::Semibold,
                                            ),
                                        ],
                                    ),
                                    self.text(&candidate.reason, MICRO, p.text2),
                                    self.wrap(vec![
                                        self.button(
                                            format!("context-candidate-accept-{index}"),
                                            "关联为同一事项",
                                            Action::Context(ContextAction::Link(
                                                candidate.left_object.clone(),
                                                candidate.right_object.clone(),
                                            )),
                                            Btn::Secondary,
                                        ),
                                        self.button(
                                            format!("context-candidate-reject-{index}"),
                                            "不关联",
                                            Action::Context(ContextAction::RejectLink(
                                                candidate.left_object.clone(),
                                                candidate.right_object.clone(),
                                            )),
                                            Btn::Ghost,
                                        ),
                                    ]),
                                ],
                            ),
                            p.surface,
                            p.border,
                            R_SM,
                            10.,
                            10.,
                        ));
                    }
                }
            }
            if ui.relation_active() {
                children.push(self.text(
                    "正在更新事项关联，完成后会刷新资料和回顾…",
                    MICRO,
                    p.text2,
                ));
            }
            children.push(self.button(
                "context-coverage",
                if ui.coverage_open {
                    "收起来源与覆盖范围"
                } else {
                    "查看来源与覆盖范围"
                },
                Action::Context(ContextAction::Coverage),
                Btn::Ghost,
            ));
            if ui.coverage_open {
                children.push(self.text(&ui.overview.coverage, MICRO, p.text3));
            }
        }
        if config.enabled
            && config.automatic_ai
            && (!ui.automatic_text_available || !automatic_authorized(&config, ui.model_source))
        {
            children.push(self.text(if !ui.automatic_text_available {ui.automatic_reason.clone()}
                else {"AI来源已变化，请在来源设置中重新确认这个来源的后台生成；本地采集与文件回传继续运行".into()},CAPTION,p.text2));
        }
        if !self.demo_mode
            && ui
                .overview
                .periods
                .iter()
                .any(|period| period.evidence_count > 0)
        {
            children.push(self.button_icon(
                "context-generate-current-periods",
                "立即整理当前日周月",
                UiIcon::Activity,
                Action::Context(ContextAction::GenerateCurrentPeriods),
                Btn::Secondary,
            ));
        }
        let repeats_overview = ui.overview.evidence_count > 0
            && (ui.status.starts_with("已整理 ") || ui.status.starts_with("已更新事项关联与回顾"));
        if !ui.status.is_empty() && !repeats_overview {
            children.push(self.line(&ui.status, MICRO, p.text2));
        }
        if let Some(error) = ui.productivity_problem() {
            children.push(self.text(
                error,
                CAPTION,
                if ui.error.is_some() {
                    p.danger
                } else {
                    p.warning
                },
            ));
            if !ui.write_failed {
                children.push(self.button(
                    "context-retry",
                    "重试整理",
                    Action::Context(ContextAction::Refresh),
                    Btn::Secondary,
                ));
            }
        }
        if let Some(warning) = &ui.session_warning {
            if ui.error.as_ref() != Some(warning) {
                children.push(self.text(warning, CAPTION, p.warning));
            }
        }
        if ui.write_failed {
            children.push(self.text(
                "设置未能保存，已停止新采集和模型请求。修复存储后重试。",
                CAPTION,
                p.danger,
            ));
            children.push(self.button(
                "context-storage-retry",
                "重试保存",
                Action::Context(ContextAction::Refresh),
                Btn::Secondary,
            ));
        }
        if ui.confirm_clear {
            children.push(self.text(
                "清理当前设备与账号的本地资料、回顾及 AI 稿，并暂停整理。已编辑的整理文档也会删除；手机原件不受影响。",
                CAPTION,
                p.danger,
            ));
            children.push(self.wrap(vec![
                self.button(
                    "context-clear-confirm",
                    "确认清理",
                    Action::Context(ContextAction::ConfirmClear),
                    Btn::Danger,
                ),
                self.button(
                    "context-clear-cancel",
                    "保留",
                    Action::Context(ContextAction::CancelClear),
                    Btn::Ghost,
                ),
            ]));
        } else if !item_detail && (config.enabled || ui.overview.evidence_count > 0) {
            let mut buttons = vec![self.button(
                "context-clear",
                "清理记录",
                Action::Context(ContextAction::Clear),
                Btn::Ghost,
            )];
            if config.enabled {
                buttons.insert(
                    0,
                    self.button(
                        "context-pause",
                        "暂停整理",
                        Action::Context(ContextAction::Pause),
                        Btn::Ghost,
                    ),
                );
                buttons.insert(
                    0,
                    self.button(
                        "context-refresh",
                        "更新本地回顾",
                        Action::Context(ContextAction::Refresh),
                        Btn::Ghost,
                    ),
                );
            }
            children.push(self.wrap(buttons));
        }
        self.panel(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_partial_capture_finishes_sync_without_hiding_write_failure_or_reusing_old_receipts(
    ) {
        use super::super::auto_sync::{AutoSyncDomain as Domain, AutoSyncDomainStatus as Status};
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device {
            serial: "partial-sync-phone".into(),
            transport: "partial-sync-phone".into(),
            ..Default::default()
        });
        app.auto_sync.on_device_connected("partial-sync-phone");
        app.context.persistent = true;
        app.context.initialized = true;
        app.context.change_scope(Scope {
            device: "partial-sync-phone".into(),
            account: Some("one".into()),
        });
        app.context.set_config(ScopePreferences {
            enabled: true,
            tasks: true,
            ..Default::default()
        });
        for domain in Domain::ALL {
            app.auto_sync.mark_synced(domain, 1);
        }
        app.auto_sync.mark_syncing(Domain::ContextAndPanorama);
        app.context.discussion_issue = Some("Amoo 讨论仅覆盖部分记录，结论需结合来源核对".into());
        let finish = |app: &mut Companion, result| {
            let (tx, receiver) = mpsc::channel();
            app.context.worker = Some(Worker {
                generation: app.context.generation,
                scope: app.context.scope.clone(),
                receiver,
                saves_preferences: false,
                clears_scope: false,
                starts_ai: false,
                automatic_ai: false,
                edits_relation: false,
                collects_context: true,
            });
            tx.send(result).unwrap();
            assert!(app.context.poll_worker());
            app.reconcile_auto_sync_domains_from_state();
        };
        let captured = || {
            Ok(WorkerOutput::Captured(
                ledger::Overview::default(),
                None,
                false,
                None,
                None,
                CollectionHealth::default(),
            ))
        };
        finish(&mut app, captured());
        for _ in 0..30 {
            app.reconcile_auto_sync_domains_from_state();
        }
        assert!(matches!(
            app.auto_sync.domain_status(Domain::ContextAndPanorama),
            Status::Partial { .. }
        ));
        assert_eq!(app.auto_sync.completed_count(), 7);
        assert!(
            !app.auto_sync.is_syncing(),
            "bounded history is not an endless network request"
        );
        assert_eq!(
            app.auto_sync.top_bar_label(true),
            "资料已更新 · 部分资料待更新"
        );
        assert!(!app.auto_sync.top_bar_label(true).contains("100%"));

        finish(&mut app, Err("ledger write failed".into()));
        assert!(app
            .auto_sync
            .domain_status(Domain::ContextAndPanorama)
            .is_failed());
        assert!(!app.context.collection_persisted());
        app.context.discussion_issue = None;
        finish(&mut app, captured());
        assert!(matches!(
            app.auto_sync.domain_status(Domain::ContextAndPanorama),
            Status::Synced { .. }
        ));

        app.auto_sync.reset_for_manual_resync();
        app.reconcile_auto_sync_domains_from_state();
        assert!(
            !app.auto_sync
                .domain_status(Domain::ContextAndPanorama)
                .is_synced(),
            "manual refresh requires a fresh persisted receipt"
        );
        finish(&mut app, captured());
        assert!(app
            .auto_sync
            .domain_status(Domain::ContextAndPanorama)
            .is_synced());
        app.context.change_scope(Scope {
            device: "other-phone".into(),
            account: None,
        });
        assert!(
            !app.context.collection_persisted(),
            "receipt cannot cross phones or accounts"
        );
    }

    #[test]
    fn blocked_generation_does_not_block_collection_and_old_scope_is_rejected() {
        let mut app = Companion::new_for_test();
        app.context.change_scope(Scope {
            device: "phone".into(),
            account: Some("one".into()),
        });
        let (finish_tx, finish_rx) = mpsc::channel();
        let scope = app.context.scope.clone();
        app.context.start_worker(scope.clone(), move || {
            finish_rx.recv().unwrap();
            Ok(WorkerOutput::OutcomeSkipped("old draft".into()))
        });
        app.context.worker.as_mut().unwrap().starts_ai = true;
        app.context.generation_worker = app.context.worker.take();
        assert!(app.context.ai_active());
        app.context.start_worker(scope, || {
            Ok(WorkerOutput::Captured(
                ledger::Overview::default(),
                None,
                false,
                None,
                Some(vec![]),
                CollectionHealth::default(),
            ))
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.context.collection_result.is_none() && Instant::now() < deadline {
            app.context.poll_worker();
            std::thread::yield_now();
        }
        assert!(
            app.context.collection_result.is_some(),
            "collection completed while model still blocked"
        );
        assert!(app.context.generation_worker.is_some());
        assert!(app.context.worker.is_none());
        assert!(
            !app.context.has_pending_or_active_sync(),
            "a blocked model must not suppress a new sync wave"
        );
        app.context.change_scope(Scope {
            device: "other".into(),
            account: Some("two".into()),
        });
        finish_tx.send(()).unwrap();
        while app.context.generation_worker.is_some() && Instant::now() < deadline {
            app.context.poll_worker();
            std::thread::yield_now();
        }
        assert!(app.context.generation_worker.is_none());
        assert!(!app.context.status.contains("old draft"));
    }

    #[test]
    fn blocked_discussion_and_identity_do_not_block_capture_or_leak_after_scope_switch() {
        let mut ui = ContextUi::new(false);
        ui.change_scope(Scope {
            device: "phone".into(),
            account: Some("one".into()),
        });
        ui.set_config(ScopePreferences {
            phone: true,
            tasks: true,
            ..Default::default()
        });
        let scope = ui.scope.clone();
        let (discussion_tx, discussion_rx) = mpsc::channel();
        ui.start_worker(scope.clone(), move || {
            discussion_rx.recv().unwrap();
            Ok(WorkerOutput::DiscussionRead(Ok(
                phone_context::Batch::default(),
            )))
        });
        ui.discussion_worker = ui.worker.take();
        let (identity_tx, identity_rx) = mpsc::channel();
        ui.start_worker(scope.clone(), move || {
            identity_rx.recv().unwrap();
            Ok(WorkerOutput::AccountScopeResolved(
                "phone".into(),
                Some("one".into()),
            ))
        });
        ui.identity_worker = ui.worker.take();
        ui.start_worker(scope, || {
            Ok(WorkerOutput::Captured(
                ledger::Overview::default(),
                None,
                false,
                None,
                Some(vec![]),
                CollectionHealth {
                    phone_checked: true,
                    phone_issue: None,
                },
            ))
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while ui.collection_result.is_none() && Instant::now() < deadline {
            ui.poll_worker();
            std::thread::yield_now();
        }
        assert!(
            ui.collection_result.is_some(),
            "SMS capture must finish while both account and discussion wait"
        );
        assert!(
            ui.collection_result.as_ref().unwrap().1.is_none(),
            "independent background work is not a source failure"
        );
        assert!(!ui.status.starts_with("部分更新"));
        assert!(ui.pending_captured_sms.is_some());
        assert!(ui.discussion_worker.is_some() && ui.identity_worker.is_some());
        ui.change_scope(Scope {
            device: "other".into(),
            account: Some("two".into()),
        });
        discussion_tx.send(()).unwrap();
        identity_tx.send(()).unwrap();
        while (ui.discussion_worker.is_some() || ui.identity_worker.is_some())
            && Instant::now() < deadline
        {
            ui.poll_worker();
            std::thread::yield_now();
        }
        assert!(ui.discussion_worker.is_none() && ui.identity_worker.is_none());
        assert!(ui.pending_discussion.is_none() && ui.resolved_account.is_none());
    }

    #[test]
    fn local_merge_preserves_source_failures_until_that_source_recovers() {
        let mut ui = ContextUi::new(false);
        ui.change_scope(Scope {
            device: "phone".into(),
            account: Some("one".into()),
        });
        ui.set_config(ScopePreferences {
            phone: true,
            tasks: true,
            ..Default::default()
        });
        let complete = |ui: &mut ContextUi, health| {
            let (tx, receiver) = mpsc::channel();
            ui.worker = Some(Worker {
                generation: ui.generation,
                scope: ui.scope.clone(),
                receiver,
                saves_preferences: false,
                clears_scope: false,
                starts_ai: false,
                automatic_ai: false,
                edits_relation: false,
                collects_context: true,
            });
            tx.send(Ok(WorkerOutput::Captured(
                ledger::Overview::default(),
                None,
                false,
                None,
                None,
                health,
            )))
            .unwrap();
            assert!(ui.poll_worker());
        };
        ui.discussion_issue = Some("Amoo 读取超时".into());
        complete(
            &mut ui,
            CollectionHealth {
                phone_checked: true,
                phone_issue: Some("短信读取失败".into()),
            },
        );
        complete(&mut ui, CollectionHealth::default());
        let issue = ui.collection_result.as_ref().unwrap().1.as_ref().unwrap();
        assert!(issue.contains("短信读取失败") && issue.contains("Amoo 读取超时"));
        complete(
            &mut ui,
            CollectionHealth {
                phone_checked: true,
                phone_issue: None,
            },
        );
        assert_eq!(
            ui.collection_result.as_ref().unwrap().1.as_deref(),
            Some("Amoo 读取超时")
        );
        ui.discussion_issue = None;
        complete(&mut ui, CollectionHealth::default());
        assert!(ui.collection_result.as_ref().unwrap().1.is_none());
        ui.error = Some("模型云连接中断".into());
        ui.status = "生成失败，资料继续更新".into();
        complete(
            &mut ui,
            CollectionHealth {
                phone_checked: true,
                phone_issue: None,
            },
        );
        assert_eq!(ui.error.as_deref(), Some("模型云连接中断"));
        assert_eq!(ui.status, "生成失败，资料继续更新");
        assert!(ui.collection_result.as_ref().unwrap().1.is_none());
    }

    #[test]
    fn passive_phone_refresh_polls_quiet_sources_and_coalesces_changes() {
        assert!(background_phone_refresh_due(None, false));
        assert!(!background_phone_refresh_due(
            Some(Duration::from_secs(14)),
            true
        ));
        assert!(background_phone_refresh_due(
            Some(Duration::from_secs(15)),
            true
        ));
        assert!(!background_phone_refresh_due(
            Some(Duration::from_secs(59)),
            false
        ));
        assert!(background_phone_refresh_due(
            Some(Duration::from_secs(60)),
            false
        ));
    }

    #[test]
    fn native_flow_keeps_failed_and_future_steps_distinct_from_completed_steps() {
        use widgets::StepMark;
        assert_eq!(
            (0..4)
                .map(|i| context_flow_mark(i, 1, true))
                .collect::<Vec<_>>(),
            vec![
                StepMark::Done,
                StepMark::Failed,
                StepMark::Pending,
                StepMark::Pending
            ]
        );
        assert_eq!(
            (0..4)
                .map(|i| context_flow_mark(i, 2, false))
                .collect::<Vec<_>>(),
            vec![
                StepMark::Done,
                StepMark::Done,
                StepMark::Active,
                StepMark::Pending
            ]
        );
        assert!((0..4).all(|i| context_flow_mark(i, 4, false) == StepMark::Done));
    }

    #[test]
    fn context_banner_zero_counts_render_empty_ring_tracks_without_fake_progress() {
        let mut app = Companion::new_for_test();
        app.context.overview = Default::default();
        app.context.outcomes.clear();
        for dark in [false, true] {
            app.palette.set(Palette::for_theme(dark));
            let _ = app.svg_slots.take_pending();
            let _ = app.render_context_visual_banner("zero-geometry", 0);
            let pending = app.svg_slots.take_pending();
            assert_eq!(
                pending.len(),
                4,
                "flow nodes and source bars are native elements, not stretched SVGs"
            );
            for suffix in ["ring-attention", "ring-outcomes", "ring-deferred"] {
                let (_, _, svg) = pending
                    .iter()
                    .find(|(slot, _, _)| slot == &format!("zero-geometry-{suffix}"))
                    .unwrap();
                assert!(svg.contains("<circle"));
                assert!(
                    !svg.contains("<path"),
                    "zero values must not acquire a colored arc: {suffix}"
                );
            }
        }
        assert_eq!(context_metric_fraction(0, 10), 0.);
        assert_eq!(context_metric_fraction(0, 0), 0.);
        assert_eq!(context_metric_fraction(2, 5), 0.4);
        assert_eq!(context_metric_fraction(9, 5), 1.);
    }

    #[test]
    fn preview_citations_are_short_and_consistent_without_changing_literal_code() {
        let a = "123456789012345678901234567890ab";
        let b = "abcdefabcdefabcdefabcdefabcdefab";
        let code = format!("```text\n〔E:{a}〕\n```\n");
        let source =
            format!("结论〔E:{a}〕〔E:{b}〕\n{code}来源附录：〔E:{b}〕〔E:{a}〕\n无效〔E:abc123〕");
        let display = context_preview_plain_text(&source);
        assert!(display.starts_with("结论〔来源1〕〔来源2〕"));
        assert!(display.contains("来源附录：〔来源2〕〔来源1〕"));
        assert!(display.contains(&code));
        assert!(display.ends_with("无效〔E:abc123〕"));
        assert!(source.starts_with(&format!("结论〔E:{a}〕")));
    }

    #[test]
    fn context_defaults_do_not_collect_or_upload() {
        let config = ScopePreferences::default();
        assert!(config.enabled && config.tasks && config.phone && !config.automatic_ai);
    }
    #[test]
    fn domain_sync_does_not_resume_paused_context_or_enable_an_unselected_source() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.context.initialized = true;
        app.context.change_scope(app.context_scope());
        app.context.needs_load = false;
        app.context.set_config(ScopePreferences {
            enabled: true,
            tasks: true,
            phone: false,
            ..Default::default()
        });
        app.trigger_auto_context_refresh();
        assert!(app.context.force);
        assert!(!app.context.config().phone);
        app.handle_context_action(&ContextAction::Pause);
        app.trigger_auto_context_refresh();
        assert!(!app.context.force && !app.context.config().enabled && !app.context.config().phone);
    }
    fn preview_fixture() -> (tempfile::TempDir, Scope, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope {
            device: "preview-phone".into(),
            account: Some("preview-account".into()),
        };
        let dir = root
            .path()
            .join(ledger::scope_id(&scope.device, scope.account.as_deref()));
        std::fs::create_dir_all(&dir).unwrap();
        (root, scope, dir.join("review.md"))
    }
    #[test]
    fn context_preview_inline_formatting_hides_destinations_and_keeps_citations() {
        let source = "**预算已更新**：`480 元`。参见 [事项资料](/home/test/context/012345/prepared/abcdef.md) 〔E:abc123〕\n";
        assert_eq!(
            context_preview_plain_text(source),
            "预算已更新：480 元。参见 事项资料 〔E:abc123〕\n"
        );
        let untrusted = r#"[打开](javascript:alert(1)) <script>do_not_execute()</script> <img src="https://example.invalid/track">"#;
        let display = context_preview_plain_text(untrusted);
        assert!(!display.contains("javascript:"));
        assert!(display.contains("打开"));
        assert!(display.contains("<script>do_not_execute()</script>"));
        assert!(display.contains("<img"));
        // The result is fed only to native Text; no HTML parser, link action or resource loader.
        let app = Companion::new_for_test();
        let _ = app.render_context_preview_text(&display);
    }
    #[test]
    fn context_preview_inline_formatting_preserves_fenced_code_verbatim() {
        let code =
            "````md\n**保留标记** [原链接](/private/path) `code`\n```\n# 原始代码内容\n````\n";
        let source = format!("**正文**\n{code}**之后** [资料](/private/other)\n");
        assert_eq!(
            context_preview_plain_text(&source),
            format!("正文\n{code}之后 资料\n")
        );
        let unfinished = "~~~txt\n**保留** [链接](/private/path)";
        assert_eq!(context_preview_plain_text(unfinished), unfinished);
    }
    #[test]
    fn context_preview_lists_are_read_only_and_keep_stored_checks_distinct() {
        assert_eq!(
            context_preview_list_item("- [ ] 整理审查结论"),
            Some(("☐", "整理审查结论", true))
        );
        assert_eq!(
            context_preview_list_item("- [x] 原稿记录"),
            Some(("☑", "原稿记录", true))
        );
        assert_eq!(
            context_preview_list_item("- 负责人：我；期限：未约定"),
            Some(("•", "负责人：我；期限：未约定", false))
        );
        assert!(context_preview_list_item("正文中的 [ ] 不是待办").is_none());
        let source =
            "- [ ] 审阅成果\n  - 负责人：未约定\n> 原文引用\n```md\n- [x] 保持代码原样\n```\n";
        let display = context_preview_plain_text(source);
        assert!(display.contains("```md\n- [x] 保持代码原样\n```"));
        let app = Companion::new_for_test();
        let _ = app.render_context_preview_text(&display);
        assert!(app.context.pending_outcomes.is_empty() && app.interaction.review.is_none());
    }
    #[test]
    fn context_preview_reads_bounded_unicode_pages_without_executing_content() {
        let (root, scope, path) = preview_fixture();
        let body = format!(
            "# 测试回顾\n[链接](file:///etc/passwd)\n<script>not executed</script>\n{}",
            "依据。".repeat(3000)
        );
        std::fs::write(&path, &body).unwrap();
        let pages = read_context_preview(root.path(), &scope, &path).unwrap();
        assert!(pages.len() > 1);
        assert_eq!(pages.concat(), context_preview_plain_text(&body));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            body,
            "显示格式化不得修改原文件"
        );
        assert!(pages
            .iter()
            .all(|page| page.chars().count() <= CONTEXT_PREVIEW_PAGE_CHARS));
        let app = Companion::new_for_test();
        let _ = app.render_context_preview_text(&pages[0]);
        std::fs::write(&path, [0u8, 0xff]).unwrap();
        assert!(read_context_preview(root.path(), &scope, &path).is_err());
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(CONTEXT_PREVIEW_LIMIT + 1).unwrap();
        assert!(read_context_preview(root.path(), &scope, &path).is_err());
    }
    #[test]
    fn context_preview_rejects_unknown_paths_and_other_scope_files() {
        let (root, scope, path) = preview_fixture();
        std::fs::write(&path, "current scope").unwrap();
        let other = Scope {
            device: "other-phone".into(),
            account: None,
        };
        assert!(read_context_preview(root.path(), &other, &path).is_err());
        let mut app = Companion::new_for_test();
        app.context.root = root.path().into();
        app.context.change_scope(scope);
        app.handle_context_action(&ContextAction::Open(path));
        assert!(app.context.preview.is_none());
        assert!(app.context.preview_worker.is_none());
        assert!(app.context.error.as_deref().unwrap().contains("当前来源"));
    }
    #[cfg(unix)]
    #[test]
    fn context_preview_rejects_symlinks_outside_the_scope() {
        let (root, scope, path) = preview_fixture();
        let outside = root.path().join("outside.md");
        std::fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(read_context_preview(root.path(), &scope, &path).is_err());
    }
    #[test]
    fn context_preview_close_and_scope_change_discard_late_reads() {
        for change_scope in [false, true] {
            let mut app = Companion::new_for_test();
            let scope = Scope {
                device: "old-phone".into(),
                account: None,
            };
            app.context.change_scope(scope.clone());
            let path = PathBuf::from("/not-read-by-this-test.md");
            app.context.overview.periods.push(ledger::PeriodBrief {
                path: path.clone(),
                ..Default::default()
            });
            app.context.preview = Some(ContextPreview {
                request: 7,
                path: path.clone(),
                title: "old".into(),
                loading: true,
                pages: vec![],
                page: 0,
                scroll: Default::default(),
                error: None,
            });
            let (tx, receiver) = mpsc::channel();
            app.context.preview_worker = Some(PreviewWorker {
                generation: app.context.generation,
                scope,
                request: 7,
                path,
                receiver,
            });
            if change_scope {
                app.context.change_scope(Scope {
                    device: "new-phone".into(),
                    account: None,
                });
            } else {
                app.handle_context_action(&ContextAction::ClosePreview);
            }
            tx.send(Ok(vec!["old body".into()])).unwrap();
            assert!(app.context.poll_preview());
            assert!(app.context.preview.is_none());
        }
    }
    #[test]
    fn context_preview_rechecks_whitelist_before_showing_result() {
        let mut app = Companion::new_for_test();
        let scope = Scope {
            device: "phone".into(),
            account: None,
        };
        app.context.change_scope(scope.clone());
        let path = PathBuf::from("/removed-review.md");
        app.context.preview = Some(ContextPreview {
            request: 9,
            path: path.clone(),
            title: "removed".into(),
            loading: true,
            pages: vec![],
            page: 0,
            scroll: Default::default(),
            error: None,
        });
        let (tx, receiver) = mpsc::channel();
        app.context.preview_worker = Some(PreviewWorker {
            generation: app.context.generation,
            scope,
            request: 9,
            path,
            receiver,
        });
        tx.send(Ok(vec!["removed body".into()])).unwrap();
        app.context.poll_preview();
        let preview = app.context.preview.as_ref().unwrap();
        assert!(!preview.loading);
        assert!(preview.pages.is_empty());
        assert!(preview.error.as_deref().unwrap().contains("资料已更新"));
    }
    #[test]
    fn context_empty_load_does_not_claim_a_review_was_created() {
        let mut ui = ContextUi::new(false);
        ui.change_scope(Scope {
            device: "empty".into(),
            account: None,
        });
        ui.set_config(ScopePreferences {
            enabled: false,
            ..Default::default()
        });
        ui.needs_load = false;
        ui.status = "old status".into();
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: ui.scope.clone(),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::LoadedOverview(Default::default())))
            .unwrap();
        assert!(ui.poll_worker());
        assert!(ui.status.is_empty());
        assert_eq!(ui.panel_title(), "把讨论变成成果");
        assert!(!ui.config().enabled);
    }
    #[test]
    fn context_empty_capture_reports_missing_material_and_error_has_priority() {
        let mut ui = ContextUi::new(false);
        ui.change_scope(Scope {
            device: "empty".into(),
            account: None,
        });
        ui.needs_load = false;
        let mut config = ui.config();
        config.enabled = true;
        ui.set_config(config);
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: ui.scope.clone(),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        assert_eq!(ui.panel_title(), "正在整理资料");
        tx.send(Ok(WorkerOutput::Overview(Default::default(), None, false)))
            .unwrap();
        ui.poll_worker();
        assert!(ui.status.contains("尚无可整理的资料"));
        assert_eq!(ui.panel_title(), "等待可整理的资料");
        ui.error = Some("disk failure".into());
        assert_eq!(ui.panel_title(), "整理需要处理");
    }
    #[test]
    fn context_prepare_once_does_not_enable_automatic_collection_or_ai() {
        let mut app = Companion::new_for_test();
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        app.handle_context_action(&ContextAction::Setup);
        app.handle_context_action(&ContextAction::AutoAi);
        app.handle_context_action(&ContextAction::PrepareOnce);
        assert!(!app.context.config().enabled);
        assert!(app.context.force);
        assert!(app.context.pending_ai.is_none());
        assert!(app.context.config().since_ms > 0);
        app.demo_mode = false;
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        assert!(app.context.config().automatic_ai);
        app.context.overview.periods.push(ledger::PeriodBrief {
            evidence_count: 1,
            prompt: "synthetic evidence".into(),
            begin_ms: 0,
            end_ms: i64::MAX,
            ..Default::default()
        });
        app.maybe_auto_context_ai_at(Local::now().with_hour(20).unwrap());
        assert!(app.context.pending_ai.is_none());
        app.handle_context_action(&ContextAction::Pause);
        assert!(!app.context.force);
    }
    #[test]
    fn context_one_time_snapshot_includes_preclick_sms_and_repeated_runs_keep_the_gap() {
        let root = tempfile::tempdir().unwrap();
        let now = 1_793_324_000_000_i64;
        let previous = ScopePreferences::default();
        let mut first = previous.clone();
        first.phone = true;
        first.configure_collection(&previous, false, now);
        assert_eq!(first.since_ms, now);
        assert_eq!(first.phone_since(), now - 7 * 86_400_000);
        let message = |id, at, body: &str| sms::Message {
            provider_id: Some(id),
            address: "test-source".into(),
            body: body.into(),
            date_ms: at,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        };
        let before_click = message(1, now - 2 * 86_400_000, "[项目:窗口验证] 场地报价需要确认");
        let mut capture = ledger::Capture {
            device: "once-phone".into(),
            now_ms: now,
            since_ms: first.phone_since(),
            sms: Some(vec![before_click.clone()]),
            ..Default::default()
        };
        let first_result = ledger::process(root.path(), &capture).unwrap();
        assert_eq!(
            first_result.evidence_count, 1,
            "建立记录时间不能过滤已授权的点击前资料"
        );
        first.sms_cursor = Some(sms::Cursor {
            date_ms: before_click.date_ms,
            id: 1,
        });
        let later = now + 3 * 86_400_000;
        let mut repeated = first.clone();
        repeated.configure_collection(&first, false, later);
        assert_eq!(repeated.phone_since(), first.phone_since());
        assert_eq!(repeated.sms_cursor, first.sms_cursor);
        assert_eq!(repeated.since_ms, now);
        let in_gap = message(
            2,
            now + 86_400_000,
            "[项目:窗口验证] 场地报价改为四百元，请更新方案",
        );
        let after = repeated.sms_after();
        assert!(
            (in_gap.date_ms, in_gap.provider_id.unwrap()) > (after.date_ms, after.id),
            "两次点击之间的消息仍在增量读取范围内"
        );
        capture.now_ms = later;
        capture.since_ms = repeated.phone_since();
        capture.sms = Some(vec![in_gap]);
        assert_eq!(
            ledger::process(root.path(), &capture)
                .unwrap()
                .evidence_count,
            2
        );
    }
    #[test]
    fn context_one_time_snapshot_resets_cursor_only_for_first_or_earlier_boundary() {
        let now = 1_793_324_000_000_i64;
        let cursor = sms::Cursor {
            date_ms: now - 1000,
            id: 9,
        };
        let mut recent = ScopePreferences::default();
        recent.phone = true;
        recent.since_ms = now - 86_400_000;
        recent.phone_since_ms = recent.since_ms;
        recent.sms_cursor = Some(cursor);
        let mut expanded = recent.clone();
        expanded.configure_collection(&recent, false, now);
        assert_eq!(expanded.phone_since_ms, now - 7 * 86_400_000);
        assert!(expanded.sms_cursor.is_none());
        assert_eq!(expanded.sms_after().date_ms, expanded.phone_since_ms);
        let mut earlier = recent.clone();
        earlier.phone_since_ms = now - 10 * 86_400_000;
        let mut repeated = earlier.clone();
        repeated.configure_collection(&earlier, false, now);
        assert_eq!(repeated.phone_since_ms, earlier.phone_since_ms);
        assert_eq!(repeated.sms_cursor, Some(cursor));
        assert_eq!(repeated.sms_after(), cursor);
        let mut automatic = repeated.clone();
        automatic.configure_collection(&repeated, true, now);
        assert!(automatic.enabled);
        assert_eq!(automatic.phone_since_ms, now, "自动开启仍只从现在积累");
        assert!(automatic.sms_cursor.is_none());
        let mut legacy = recent.clone();
        legacy.phone_since_ms = 0;
        assert_eq!(legacy.phone_since(), legacy.since_ms);
        let mut initial = legacy.clone();
        initial.configure_collection(&legacy, false, now);
        assert!(initial.sms_cursor.is_none());
        assert_eq!(initial.phone_since(), now - 7 * 86_400_000);
    }
    #[test]
    fn context_home_results_require_saved_current_drafts_not_prepared_sources() {
        let mut app = Companion::new_for_test();
        let scope = Scope {
            device: "phone-a".into(),
            account: Some("one".into()),
        };
        app.context.change_scope(scope.clone());
        app.context.needs_load = false;
        let scope_id = ledger::scope_id(&scope.device, scope.account.as_deref());
        app.context.overview.items.push(ledger::PreparedItem {
            id: "activity".into(),
            title: "读书会活动方案".into(),
            input_revision: "current-item".into(),
            scope_id: scope_id.clone(),
            ..Default::default()
        });
        app.context.overview.periods.push(ledger::PeriodBrief {
            kind: ledger::Period::Week,
            title: "周复盘 · 2026-10-26".into(),
            evidence_revision: "current-week".into(),
            scope_id: scope_id.clone(),
            ..Default::default()
        });
        assert!(
            app.render_home_context_results().is_none(),
            "Prepared source documents are not generated drafts"
        );
        let draft_path = app
            .context
            .root
            .join(&scope_id)
            .join("ai-reviews/week-internal-hash.md");
        let mut config = app.context.config();
        config.ai_records = vec![
            AiRecord {
                target: "item:activity".into(),
                revision: "old-item".into(),
                path: app.context.root.join(&scope_id).join("ai-items/old.md"),
            },
            AiRecord {
                target: "period:Week".into(),
                revision: "current-week".into(),
                path: draft_path.clone(),
            },
        ];
        app.context.set_config(config);
        assert_eq!(
            app.context.home_ai_results(),
            vec![("周复盘 · 2026-10-26".into(), draft_path.clone())]
        );
        assert!(
            app.render_home_context_results().is_some(),
            "Rendering uses the saved record without reading the file"
        );
        assert_eq!(
            app.context.document_title(&draft_path).as_deref(),
            Some("周复盘 · 2026-10-26 · 草稿")
        );
        app.context.overview.periods[0].evidence_revision = "new-week".into();
        assert!(app.context.home_ai_results().is_empty());
        assert!(app.render_home_context_results().is_none());
    }

    #[test]
    fn context_home_results_are_scope_bound_even_with_stale_in_memory_overview() {
        let mut ui = ContextUi::new(false);
        let one = Scope {
            device: "phone-a".into(),
            account: Some("one".into()),
        };
        let two = Scope {
            device: "phone-a".into(),
            account: Some("two".into()),
        };
        ui.change_scope(one.clone());
        ui.needs_load = false;
        let scope_id = ledger::scope_id(&one.device, one.account.as_deref());
        ui.overview.items.push(ledger::PreparedItem {
            id: "same-id".into(),
            title: "账号一的方案".into(),
            input_revision: "revision".into(),
            scope_id: scope_id.clone(),
            ..Default::default()
        });
        let mut config = ui.config();
        config.ai_records.push(AiRecord {
            target: "item:same-id".into(),
            revision: "revision".into(),
            path: ui.root.join(&scope_id).join("ai-items/draft.md"),
        });
        ui.set_config(config.clone());
        assert_eq!(ui.home_ai_results().len(), 1);
        let previous_overview = ui.overview.clone();
        ui.change_scope(two.clone());
        assert!(ui.home_ai_results().is_empty());
        ui.overview = previous_overview;
        ui.needs_load = false;
        ui.set_config(config);
        assert!(
            ui.home_ai_results().is_empty(),
            "A copied record pointing into another scope must not appear"
        );
        let mut copied = ui.config();
        copied.ai_records[0].path = ui
            .root
            .join(ledger::scope_id(&two.device, two.account.as_deref()))
            .join("ai-items/copied.md");
        ui.set_config(copied);
        assert!(
            ui.home_ai_results().is_empty(),
            "The overview itself must also belong to the current scope"
        );
    }

    #[test]
    fn context_home_results_show_only_the_latest_two_distinct_saved_drafts() {
        let mut ui = ContextUi::new(false);
        let scope = Scope {
            device: "phone".into(),
            account: None,
        };
        ui.change_scope(scope.clone());
        ui.needs_load = false;
        let scope_id = ledger::scope_id(&scope.device, None);
        let mut config = ui.config();
        for index in 0..3 {
            ui.overview.items.push(ledger::PreparedItem {
                id: format!("matter-{index}"),
                title: format!("方案{index}"),
                input_revision: "current".into(),
                scope_id: scope_id.clone(),
                ..Default::default()
            });
            config.ai_records.push(AiRecord {
                target: format!("item:matter-{index}"),
                revision: "current".into(),
                path: ui.root.join(&scope_id).join(format!("ai-items/{index}.md")),
            });
        }
        config
            .ai_records
            .push(config.ai_records.last().unwrap().clone());
        ui.set_config(config);
        assert_eq!(
            ui.home_ai_results()
                .into_iter()
                .map(|(title, _)| title)
                .collect::<Vec<_>>(),
            vec!["方案2", "方案1"]
        );
        ui.preferences.pending_clear.push(scope);
        assert!(ui.home_ai_results().is_empty());
    }

    #[test]
    fn context_schedule_primary_opens_calendar_online_and_saved_record_offline() {
        use stepx_companion::context_review::{MatterAction, MatterPresentation};
        let mut app = Companion::new_for_test();
        let item = ledger::PreparedItem {
            id: "trip".into(),
            path: "/tmp/schedule-record.md".into(),
            presentation: Some(MatterPresentation {
                primary_action: MatterAction::PrepareSchedule,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (label, offline) = app.context_item_primary(&item);
        assert_eq!(label, "查看日程记录");
        assert!(
            matches!(offline, Action::Context(ContextAction::Open(ref path)) if path == &item.path)
        );
        app.selected = Some(stepx_companion::device::Device {
            serial: "phone".into(),
            transport: "phone".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        let (label, online) = app.context_item_primary(&item);
        assert_eq!(label, "打开日历");
        assert!(matches!(online, Action::Page(Page::Calendar)));
    }

    #[test]
    fn context_primary_requires_a_choice_for_multiple_original_tasks() {
        use stepx_companion::{
            amoo_workspace::Task,
            context_review::{MatterAction, MatterPresentation},
        };
        let mut app = Companion::new_for_test();
        let mut first = Task::new(None, None);
        first.id = "a-first".into();
        let mut latest = Task::new(None, None);
        latest.id = "z-latest".into();
        app.workspace.journal.tasks = vec![first.clone(), latest.clone()];
        let mut item = ledger::PreparedItem {
            id: "shared-matter".into(),
            task_id: Some(first.id.clone()),
            task_ids: vec![latest.id.clone(), first.id.clone()],
            ..Default::default()
        };
        for primary_action in [MatterAction::ContinueDiscussion, MatterAction::ReviewOutput] {
            item.presentation = Some(MatterPresentation {
                primary_action,
                ..Default::default()
            });
            let (_, action) = app.context_item_primary(&item);
            assert!(
                matches!(action, Action::Context(ContextAction::ItemDetails(ref id)) if id == &item.id),
                "An association must not silently select the lexicographically first task"
            );
        }
        item.task_ids = vec![latest.id.clone()];
        item.task_id = Some(latest.id.clone());
        item.presentation.as_mut().unwrap().primary_action = MatterAction::ContinueDiscussion;
        assert!(
            matches!(app.context_item_primary(&item).1, Action::HomeResumeTask(ref id) if id == &latest.id)
        );
    }

    #[test]
    fn context_primary_opens_only_an_unambiguous_recorded_output() {
        use stepx_companion::{
            amoo_workspace::{Output, Task},
            context_review::{MatterAction, MatterPresentation},
        };
        let mut app = Companion::new_for_test();
        let mut task = Task::new(None, None);
        task.outputs = vec![Output {
            name: "活动方案.md".into(),
            path: "/tmp/plan-v1.md".into(),
            kind: "document".into(),
            lab_id: None,
        }];
        app.workspace.journal.tasks = vec![task.clone()];
        let item = ledger::PreparedItem {
            id: "matter".into(),
            task_id: Some(task.id.clone()),
            path: "/tmp/source.md".into(),
            presentation: Some(MatterPresentation {
                primary_action: MatterAction::ReviewOutput,
                available_output: Some("活动方案.md".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(
            matches!(app.context_item_primary(&item).1, Action::HomeOpenTaskOutput { ref task, ref path }
            if task == &item.task_id.clone().unwrap() && path == &std::path::PathBuf::from("/tmp/plan-v1.md"))
        );
        app.workspace.journal.tasks[0].outputs.push(Output {
            path: "/tmp/plan-v2.md".into(),
            ..task.outputs[0].clone()
        });
        assert!(
            matches!(app.context_item_primary(&item).1, Action::Context(ContextAction::Open(ref path)) if path == &item.path),
            "A shared filename does not identify which version the evidence means"
        );
        app.workspace.journal.tasks.clear();
        assert!(
            matches!(app.context_item_primary(&item).1, Action::Context(ContextAction::Open(ref path)) if path == &item.path)
        );
    }

    #[test]
    fn context_attention_orders_upcoming_schedules_by_event_time_after_work_changes() {
        use stepx_companion::context_review::{MatterAction, MatterPresentation};
        let mut ui = ContextUi::new(false);
        let now = Local::now().timestamp_millis();
        for (id, offset, updated_ms) in [
            ("tomorrow-departure", 86_400_000, 50),
            ("tonight-check", 3_600_000, 10),
        ] {
            ui.overview.items.push(ledger::PreparedItem {
                id: id.into(),
                updated_ms,
                presentation: Some(MatterPresentation {
                    priority: 30,
                    primary_action: MatterAction::PrepareSchedule,
                    valid_until_ms: Some(now + offset),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        assert_eq!(
            ui.attention_items()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["tonight-check", "tomorrow-departure"]
        );
        ui.overview.items.push(ledger::PreparedItem {
            id: "budget-change".into(),
            updated_ms: 1,
            presentation: Some(MatterPresentation {
                priority: 95,
                primary_action: MatterAction::ReviewSources,
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(
            ui.attention_items()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["budget-change", "tonight-check"]
        );
    }

    #[test]
    fn context_attention_recommends_only_supported_priorities_and_keeps_detail_reachable() {
        use stepx_companion::context_review::MatterPresentation;
        let mut ui = ContextUi::new(false);
        for (id, priority) in [
            ("background", 0),
            ("schedule", 30),
            ("change", 95),
            ("waiting", 90),
        ] {
            ui.overview.items.push(ledger::PreparedItem {
                id: id.into(),
                presentation: Some(MatterPresentation {
                    priority,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        ui.overview.items.push(ledger::PreparedItem {
            id: "legacy-no-facts".into(),
            ..Default::default()
        });
        ui.overview.items.push(ledger::PreparedItem {
            id: "expired-priority".into(),
            presentation: Some(MatterPresentation {
                priority: 255,
                valid_until_ms: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(
            ui.attention_items()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["change", "waiting"]
        );
        ui.panel = ContextPanel::Item("legacy-no-facts".into());
        assert_eq!(ui.detail_items()[0].id, "legacy-no-facts");
        assert_eq!(ui.overview.items.len(), 6, "折叠仅影响首页展示，不删除资料");
    }
    #[test]
    fn context_focused_navigation_returns_to_attention_without_starting_work() {
        let mut app = Companion::new_for_test();
        app.context.overview.items.push(ledger::PreparedItem {
            id: "matter".into(),
            ..Default::default()
        });
        assert!(!app.context_home_is_focused());
        app.handle_context_action(&ContextAction::Manage);
        assert!(app.context_home_is_focused());
        app.handle_context_action(&ContextAction::Attention);
        assert!(!app.context_home_is_focused());
        // The first action establishes a scope and clears a previous unspecific view.
        app.context.overview.items.push(ledger::PreparedItem {
            id: "matter".into(),
            ..Default::default()
        });
        app.handle_context_action(&ContextAction::ItemDetails("matter".into()));
        assert_eq!(app.context.panel, ContextPanel::Item("matter".into()));
        assert!(app.context_home_is_focused());
        app.handle_context_action(&ContextAction::Attention);
        app.handle_context_action(&ContextAction::Period(ledger::Period::Week));
        assert_eq!(app.context.panel, ContextPanel::Review);
        assert!(app.context_home_is_focused());
        app.handle_context_action(&ContextAction::Attention);
        assert!(!app.context_home_is_focused());
        assert!(
            app.context.pending_ai.is_none()
                && !app.context.force
                && !app.context.config().automatic_ai
        );
    }
    #[test]
    fn context_period_navigation_never_collects_or_starts_a_model() {
        let mut app = Companion::new_for_test();
        for period in [
            ledger::Period::Day,
            ledger::Period::Week,
            ledger::Period::Month,
        ] {
            app.handle_context_action(&ContextAction::Period(period));
            assert_eq!(app.context.period, period);
            assert!(!app.context.config().automatic_ai);
            assert!(!app.context.force);
            assert!(app.context.pending_ai.is_none());
            let _ = app.render_context_panel();
        }
    }
    #[test]
    fn context_scope_change_discards_previous_view_and_generation() {
        let mut ui = ContextUi::new(false);
        let scope = Scope {
            device: "a".into(),
            account: Some("one".into()),
        };
        ui.change_scope(scope.clone());
        ui.overview.evidence_count = 12;
        let generation = ui.generation;
        assert!(!ui.change_scope(scope));
        assert!(ui.change_scope(Scope {
            device: "a".into(),
            account: Some("two".into())
        }));
        assert_eq!(ui.overview.evidence_count, 0);
        assert!(ui.generation > generation);
    }
    #[test]
    fn context_stale_worker_cannot_restore_cleared_data() {
        let mut ui = ContextUi::new(false);
        let scope = Scope {
            device: "a".into(),
            account: None,
        };
        ui.change_scope(scope.clone());
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: Some(scope),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        ui.invalidate();
        let mut old = ledger::Overview::default();
        old.evidence_count = 99;
        tx.send(Ok(WorkerOutput::Overview(old, None, false)))
            .unwrap();
        assert!(ui.poll_worker());
        assert_eq!(ui.overview.evidence_count, 0);
    }
    #[test]
    fn context_preferences_roundtrip_preserves_scope_and_disabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut prefs = Preferences::default();
        let scope = Scope {
            device: "phone".into(),
            account: Some("account".into()),
        };
        prefs.last_scope = Some(scope.clone());
        let mut config = ScopePreferences::default();
        config.enabled = false;
        config.phone = true;
        config.since_ms = 42;
        prefs.scopes.insert(scope.key(), config);
        save_preferences(temp.path(), &prefs).unwrap();
        let restored = load_preferences(temp.path()).unwrap();
        assert_eq!(restored.last_scope, Some(scope.clone()));
        let config = restored.scopes.get(&scope.key()).unwrap();
        assert!(config.phone);
        assert!(!config.enabled);
        assert_eq!(config.since_ms, 42);
    }
    #[test]
    fn context_controls_keep_local_output_while_paused() {
        let mut app = Companion::new_for_test();
        app.handle_context_action(&ContextAction::Setup);
        app.handle_context_action(&ContextAction::Apply);
        assert!(app.context.config().enabled);
        app.context.overview.evidence_count = 4;
        app.handle_context_action(&ContextAction::Pause);
        assert!(!app.context.config().enabled);
        assert_eq!(app.context.overview.evidence_count, 4);
        let _ = app.render_context_panel();
    }
    fn ready_test_automatic(app: &mut Companion, source: stepx_companion::model_source::Source) {
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(source);
        app.context.automatic_reason = "synthetic pure-text capability".into();
    }
    fn rolling_test_app(now: chrono::DateTime<Local>) -> Companion {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.selected = Some(Device {
            serial: "rolling-test-phone".into(),
            ..Default::default()
        });
        app.context.change_scope(Scope {
            device: "rolling-test-phone".into(),
            account: None,
        });
        ready_test_automatic(
            &mut app,
            stepx_companion::model_source::Source::IndependentApi,
        );
        app.context.set_config(ScopePreferences {
            enabled: true,
            automatic_ai: true,
            automatic_source: Some(stepx_companion::model_source::Source::IndependentApi),
            ..Default::default()
        });
        app.context.overview.periods = [
            ledger::Period::Day,
            ledger::Period::Week,
            ledger::Period::Month,
        ]
        .into_iter()
        .map(|kind| ledger::PeriodBrief {
            kind,
            begin_ms: now.timestamp_millis() - 3_600_000,
            end_ms: now.timestamp_millis() + 48 * 3_600_000,
            evidence_count: 1,
            evidence_revision: "v1".into(),
            prompt: "synthetic scheduler evidence".into(),
            ..Default::default()
        })
        .collect();
        app
    }
    fn finish_test_queue(app: &mut Companion) {
        app.context.pending_outcomes.clear();
        let mut config = app.context.config();
        config.ai_started_ms = 0;
        app.context.set_config(config);
    }
    #[test]
    fn rolling_periods_fill_weekday_and_update_new_stable_revision_without_local_quota() {
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 11, 2, 12, 0, 0)
            .single()
            .unwrap();
        let mut app = rolling_test_app(now);
        app.maybe_auto_context_ai_at(now);
        assert!(app.context.pending_outcomes.is_empty());
        let mut selected = vec![];
        for seconds in [16, 77, 138] {
            app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(seconds));
            assert_eq!(app.context.pending_outcomes.len(), 1);
            selected.push(app.context.pending_outcomes[0].1.clone());
            finish_test_queue(&mut app);
        }
        for kind in [
            ledger::Period::Day,
            ledger::Period::Week,
            ledger::Period::Month,
        ] {
            assert!(selected.contains(&OutcomeTarget::Period(kind)));
        }
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(200));
        assert!(app.context.pending_outcomes.is_empty());
        app.context.overview.periods[0].evidence_revision = "v2".into();
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(201));
        assert!(
            app.context.pending_outcomes.is_empty(),
            "changed data still requires the short stability wait"
        );
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(217));
        assert_eq!(
            app.context.pending_outcomes[0].1,
            OutcomeTarget::Period(ledger::Period::Day)
        );
        assert!(
            app.context.pending_outcomes[0].6.is_some(),
            "queued input/source identity must be fixed"
        );
        app.context.pending_outcomes.clear();
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(218));
        assert!(
            app.context.pending_outcomes.is_empty(),
            "the same accepted or queued revision must not spend again"
        );
    }
    #[test]
    fn new_stable_period_revisions_have_no_local_daily_or_fixed_interval_limit() {
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 11, 2, 12, 0, 0)
            .single()
            .unwrap();
        let mut app = rolling_test_app(now);
        app.context
            .overview
            .periods
            .retain(|period| period.kind == ledger::Period::Day);
        for revision in 0..15 {
            app.context.overview.periods[0].evidence_revision = format!("revision-{revision}");
            let changed = now + chrono::Duration::seconds(revision * 20);
            app.maybe_auto_context_ai_at(changed);
            assert!(app.context.pending_outcomes.is_empty());
            app.maybe_auto_context_ai_at(changed + chrono::Duration::seconds(16));
            assert_eq!(
                app.context.pending_outcomes.len(),
                1,
                "each real stable revision may request an update"
            );
            app.context.pending_outcomes.clear();
            // Keep ai_started_ms: the previous request must not establish a
            // desktop-defined minute, hourly or daily inference allowance.
        }
        assert_eq!(app.context.config().scheduled_attempts.len(), 15);
    }
    #[test]
    fn rolling_periods_and_items_alternate_without_starving_either() {
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 11, 2, 12, 0, 0)
            .single()
            .unwrap();
        let mut app = rolling_test_app(now);
        app.context.overview.items = ["a", "b"]
            .into_iter()
            .map(|id| ledger::PreparedItem {
                id: id.into(),
                input_revision: "v1".into(),
                updated_ms: now.timestamp_millis() - 10_000,
                presentation: Some(stepx_companion::context_review::MatterPresentation {
                    priority: 80,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        app.context.error = Some("hold synthetic queue while observing stability".into());
        app.maybe_auto_context_ai_at(now);
        app.context.error = None;
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(16));
        assert!(matches!(
            app.context.pending_outcomes[0].1,
            OutcomeTarget::Period(_)
        ));
        finish_test_queue(&mut app);
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(77));
        assert!(matches!(
            app.context.pending_outcomes[0].1,
            OutcomeTarget::Item(_)
        ));
        finish_test_queue(&mut app);
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(138));
        assert!(matches!(
            app.context.pending_outcomes[0].1,
            OutcomeTarget::Period(_)
        ));
    }
    #[test]
    fn explicit_phone_period_batch_uses_manual_snapshot_without_unattended_authorization() {
        let mut app = rolling_test_app(Local::now());
        let periods = app.context.overview.periods.clone();
        app.context.change_scope(Scope {
            device: "rolling-test-phone".into(),
            account: Some("one".into()),
        });
        app.context.overview.periods = periods;
        app.amoo_ui.set_account_scope(Some("one".into()));
        app.context.model_source = Some(stepx_companion::model_source::Source::PhoneAccount);
        app.context.model_available = true;
        app.context.automatic_text_available = false;
        app.context.set_config(ScopePreferences::default());
        app.queue_current_periods();
        assert_eq!(app.context.pending_outcomes.len(), 3);
        assert!(app.context.pending_outcomes.iter().all(|pending| !pending.3
            && !pending.4
            && pending.5.is_none()
            && pending.6.as_ref().is_some_and(
                |(source, _)| *source == stepx_companion::model_source::Source::PhoneAccount
            )));
        assert!(!app.context.config().automatic_ai);
        assert!(app.context.config().period_batch.is_none());
        let (tx, receiver) = mpsc::channel();
        app.context.worker = Some(Worker {
            generation: app.context.generation,
            scope: app.context.scope.clone(),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: true,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::AutomaticInputChanged(true)))
            .unwrap();
        app.context.poll_worker();
        assert!(
            app.context.pending_outcomes.is_empty(),
            "changed snapshot stops the remaining explicit batch"
        );
        assert!(!app.context.config().automatic_ai);
        let mut config = app.context.config();
        config.return_outcomes = true;
        app.context.set_config(config);
        app.queue_current_periods();
        assert_eq!(app.context.pending_outcomes.len(), 3);
        assert!(app.context.pending_outcomes.iter().all(|pending|pending.3&&!pending.4),"explicit batch must honor authorized phone return without enabling unattended inference");
        app.handle_context_action(&ContextAction::StopAi);
        assert!(app.context.pending_outcomes.is_empty());
        assert!(!app.context.config().automatic_ai);
        assert!(
            !app.context.config().scheduled_attempts.is_empty(),
            "stop must retain prior request identities"
        );
    }
    #[test]
    fn phone_automatic_requires_explicit_source_authorization_and_matching_account() {
        let now = Local::now();
        let mut app = rolling_test_app(now);
        app.context.change_scope(Scope {
            device: "rolling-test-phone".into(),
            account: Some("one".into()),
        });
        app.amoo_ui.set_account_scope(Some("one".into()));
        ready_test_automatic(
            &mut app,
            stepx_companion::model_source::Source::PhoneAccount,
        );
        app.context.overview.items.push(ledger::PreparedItem {
            id: "a".into(),
            input_revision: "v1".into(),
            updated_ms: now.timestamp_millis() - 10_000,
            presentation: Some(stepx_companion::context_review::MatterPresentation {
                priority: 80,
                ..Default::default()
            }),
            ..Default::default()
        });
        app.context.set_config(ScopePreferences {
            enabled: true,
            automatic_ai: true,
            ..Default::default()
        });
        app.start_context_ai(AiTarget::Item(app.context.overview.items[0].clone()), true);
        assert!(
            app.context.pending_ai.is_none(),
            "Phone automatic capability must not invoke the legacy AgentBus path"
        );
        app.maybe_auto_context_ai_at(now);
        assert!(
            app.context.pending_outcomes.is_empty(),
            "old API consent cannot authorize phone calls"
        );
        let mut config = app.context.config();
        config.automatic_source = Some(stepx_companion::model_source::Source::PhoneAccount);
        app.context.set_config(config);
        app.amoo_ui.set_account_scope(Some("two".into()));
        app.maybe_auto_context_ai_at(now);
        assert!(app.context.pending_outcomes.is_empty());
        app.amoo_ui.set_account_scope(Some("one".into()));
        app.maybe_auto_context_ai_at(now);
        assert_eq!(app.context.pending_outcomes.len(), 1);
        let cancel = app.context.work_cancel.clone();
        app.refresh_context_model_capabilities();
        assert!(cancel.load(Ordering::Acquire));
        assert!(app.context.pending_outcomes.is_empty());
        assert!(!app.context.config().automatic_ai);
    }
    #[test]
    fn automatic_failure_recovery_never_reissues_ambiguous_or_failed_paid_requests() {
        assert!(automatic_error_can_recheck("请连接原手机后生成"));
        assert!(!automatic_error_can_recheck(
            "同一后台请求已记录，未重复发送；请手动重试"
        ));
        assert!(!automatic_error_can_recheck(
            "上次模型请求已发出但结果未确认或失败；后台不会自动重发"
        ));
        assert!(!automatic_error_can_recheck(
            "后台请求记录损坏，已停止自动请求"
        ));
    }
    #[test]
    fn accepted_phone_inference_is_account_scoped_and_cleared_with_context() {
        let mut app = Companion::new_for_test();
        let device = Device {
            serial: "receipt-phone".into(),
            ..Default::default()
        };
        app.context.change_scope(Scope {
            device: device.serial.clone(),
            account: Some("one".into()),
        });
        let meta = home_ai::InferenceMeta {
            source: stepx_companion::model_source::Source::PhoneAccount,
            model: "actual-reported-model".into(),
            provider: "reported-provider".into(),
            usage: Some(stepx_companion::amoo_usage::UsageMetrics {
                input_tokens: Some(12),
                output_tokens: None,
                ..Default::default()
            }),
            account_scope: Some("one".into()),
            observed_ms: 100,
            session_cleaned: false,
            temporary_session_key: Some("private-recovery-handle".into()),
            channel: None,
            isolation: None,
            session_cleanup: None,
        };
        app.context.cache_inference(Some(meta.clone()));
        let shown = app
            .context
            .accepted_phone_inference(Some(&device), Some("one"))
            .unwrap();
        assert_eq!(shown.model, "actual-reported-model");
        assert_eq!(shown.usage.as_ref().unwrap().output_tokens, None);
        assert!(app
            .context
            .accepted_phone_inference(Some(&device), Some("two"))
            .is_none());
        assert!(app
            .context
            .accepted_phone_inference(Some(&device), None)
            .is_none());
        let other = Device {
            serial: "another-phone".into(),
            ..Default::default()
        };
        assert!(app
            .context
            .accepted_phone_inference(Some(&other), Some("one"))
            .is_none());
        let mut wrong = meta.clone();
        wrong.account_scope = Some("two".into());
        app.context.cache_inference(Some(wrong));
        assert!(app.context.last_inference.is_none());
        let mut api = meta.clone();
        api.source = stepx_companion::model_source::Source::IndependentApi;
        app.context.cache_inference(Some(api));
        assert!(app
            .context
            .accepted_phone_inference(Some(&device), Some("one"))
            .is_none());
        app.context.cache_inference(Some(meta.clone()));
        app.handle_context_action(&ContextAction::ConfirmClear);
        assert!(app.context.last_inference.is_none());
        app.context.cache_inference(Some(meta));
        app.context.change_scope(Scope {
            device: device.serial,
            account: Some("two".into()),
        });
        assert!(app.context.last_inference.is_none());
    }
    #[test]
    fn phone_model_supports_manual_generation_without_background_inference() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        let scope = Scope {
            device: "phone-source-test".into(),
            account: Some("synthetic-account".into()),
        };
        app.selected = Some(Device {
            serial: scope.device.clone(),
            transport: "192.0.2.2:5555".into(),
            ..Default::default()
        });
        app.context.change_scope(scope);
        app.context.model_available = true;
        app.context.automatic_text_available = false;
        app.context.model_source = Some(stepx_companion::model_source::Source::PhoneAccount);
        app.context.set_config(ScopePreferences {
            enabled: true,
            automatic_ai: true,
            ..Default::default()
        });
        let item = ledger::PreparedItem {
            id: "matter".into(),
            input_revision: "v1".into(),
            prompt: "synthetic evidence".into(),
            updated_ms: Local::now().timestamp_millis() - 10_000,
            presentation: Some(stepx_companion::context_review::MatterPresentation {
                priority: 80,
                ..Default::default()
            }),
            ..Default::default()
        };
        app.context.overview.items.push(item.clone());
        app.maybe_auto_context_ai_at(Local::now());
        assert!(app.context.pending_outcomes.is_empty() && app.context.pending_ai.is_none());
        assert!(
            app.context.config().scheduled_attempts.is_empty(),
            "phone selection cannot spend from a timer"
        );
        app.queue_outcome_mode(OutcomeTarget::Item(item.id.clone()), false, true);
        assert!(
            app.context.pending_outcomes.is_empty(),
            "automatic queue must independently reject phone mode"
        );
        app.start_context_ai(AiTarget::Item(item.clone()), false);
        let pending = app
            .context
            .pending_ai
            .as_ref()
            .expect("manual phone request should be accepted");
        assert!(!pending.use_local);
        assert!(pending.device.is_some());
        app.context.pending_ai = None;
        app.queue_outcome(OutcomeTarget::Item(item.id), false);
        assert_eq!(app.context.pending_outcomes.len(), 1);
        assert!(!app.context.pending_outcomes[0].4);
    }
    #[test]
    fn phone_source_auto_toggle_stays_off_without_disabling_local_collection() {
        let mut app = Companion::new_for_test();
        app.context.model_available = true;
        app.context.automatic_text_available = false;
        app.context.model_source = Some(stepx_companion::model_source::Source::PhoneAccount);
        app.context.automatic_reason = "手机可点击生成，后台纯文本能力未核验".into();
        app.handle_context_action(&ContextAction::Setup);
        app.handle_context_action(&ContextAction::AutoAi);
        assert!(!app.context.setup.as_ref().unwrap().automatic_ai);
        assert_eq!(
            app.context.status, app.context.automatic_reason,
            "unverified source must explain its actual capability restriction"
        );
        assert!(!app.context.status.trim().is_empty());
        app.handle_context_action(&ContextAction::Apply);
        assert!(app.context.config().enabled);
        assert!(!app.context.config().automatic_ai);
        assert!(
            app.context.config().automatic_source.is_none(),
            "a rejected toggle must not authorize the phone source"
        );
        assert!(app.context.pending_ai.is_none() && app.context.pending_outcomes.is_empty());
    }
    #[test]
    fn context_model_request_waits_for_durable_intent() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.change_scope(Scope {
            device: "test".into(),
            account: None,
        });
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        let target = AiTarget::Period(ledger::PeriodBrief {
            prompt: "synthetic evidence".into(),
            evidence_revision: "facts-1".into(),
            begin_ms: 100,
            evidence_count: 1,
            ..Default::default()
        });
        let key = target.attempt_key(true);
        ready_test_automatic(
            &mut app,
            stepx_companion::model_source::Source::IndependentApi,
        );
        app.start_context_ai(target, true);
        assert!(app.context.ai.is_none());
        assert!(app.context.worker.is_none());
        assert!(app.context.pending_ai.is_some());
        assert!(app.context.dirty);
        assert!(app.context.config().scheduled_attempts.contains_key(&key));
    }
    #[test]
    fn context_stop_rejects_a_model_that_finishes_starting_later() {
        let mut app = Companion::new_for_test();
        let scope = Scope {
            device: "test".into(),
            account: None,
        };
        app.context.change_scope(scope.clone());
        let (tx, receiver) = mpsc::channel();
        app.context.worker = Some(Worker {
            generation: app.context.generation,
            scope: Some(scope.clone()),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: true,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_model_tx, model_rx) = mpsc::channel();
        app.handle_context_action(&ContextAction::StopAi);
        tx.send(Ok(WorkerOutput::AiStarted(
            home_ai::TextJob {
                receiver: model_rx,
                cancel: cancel.clone(),
            },
            scope,
            AiTarget::Period(Default::default()),
        )))
        .unwrap();
        assert!(app.context.poll_worker());
        assert!(app.context.ai.is_none());
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn context_failed_preference_write_blocks_dependent_jobs() {
        let mut ui = ContextUi::new(false);
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: None,
            receiver,
            saves_preferences: true,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Err("disk full".into())).unwrap();
        ui.poll_worker();
        assert!(ui.write_failed && ui.dirty);
        assert_eq!(ui.error.as_deref(), Some("disk full"));
    }
    #[test]
    fn context_clear_survives_restart_and_other_scope_completion() {
        let temp = tempfile::tempdir().unwrap();
        let old = Scope {
            device: "old-phone".into(),
            account: None,
        };
        let new = Scope {
            device: "new-phone".into(),
            account: None,
        };
        let mut prefs = Preferences::default();
        prefs.pending_clear.push(old.clone());
        save_preferences(temp.path(), &prefs).unwrap();
        let mut ui = ContextUi::new(false);
        ui.preferences = load_preferences(temp.path()).unwrap();
        assert_eq!(ui.preferences.pending_clear, vec![old.clone()]);
        ui.change_scope(new);
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: Some(old.clone()),
            receiver,
            saves_preferences: false,
            clears_scope: true,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::Cleared(old))).unwrap();
        ui.poll_worker();
        assert!(ui.preferences.pending_clear.is_empty());
        assert!(ui.dirty);
    }
    #[test]
    fn context_schedule_obeys_local_day_week_and_month_boundaries() {
        use chrono::NaiveDate;
        let thu = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        assert!(!auto_due(ledger::Period::Day, thu, 17));
        assert!(auto_due(ledger::Period::Day, thu, 18));
        assert!(!auto_due(ledger::Period::Week, thu, 18));
        assert!(!auto_due(ledger::Period::Month, thu, 18));
        let sun = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert!(auto_due(ledger::Period::Week, sun, 18));
        assert!(!auto_due(ledger::Period::Month, sun, 18));
        let last = NaiveDate::from_ymd_opt(2028, 2, 29).unwrap();
        assert!(auto_due(ledger::Period::Month, last, 18));
        assert!(!auto_due(
            ledger::Period::Month,
            last.pred_opt().unwrap(),
            18
        ));
        let a = AiTarget::Period(ledger::PeriodBrief {
            begin_ms: 42,
            evidence_revision: "a".into(),
            ..Default::default()
        });
        let b = AiTarget::Period(ledger::PeriodBrief {
            begin_ms: 42,
            evidence_revision: "b".into(),
            ..Default::default()
        });
        assert_eq!(a.attempt_key(true), b.attempt_key(true));
        assert_ne!(a.attempt_key(false), b.attempt_key(false));
    }
    #[test]
    fn context_fingerprint_observes_calendar_details_and_edited_turns() {
        let mut app = Companion::new_for_test();
        app.context.change_scope(app.context_scope());
        app.context.set_config(ScopePreferences {
            phone: true,
            tasks: true,
            ..Default::default()
        });
        app.back_screen
            .stepos_suite
            .calendar_events
            .push(toolbox::PhoneCalendarEvent {
                id: 1,
                title: "评审".into(),
                dtstart: 100,
                dtend: 200,
                ..Default::default()
            });
        let a = app.context_fingerprint();
        app.back_screen.stepos_suite.calendar_events[0].location = "新会议室".into();
        let b = app.context_fingerprint();
        assert_ne!(a, b);
        app.back_screen.stepos_suite.calendar_events[0].dtend = 300;
        let c = app.context_fingerprint();
        assert_ne!(b, c);
        let mut task = stepx_companion::amoo_workspace::Task::new(None, None);
        task.turns.push(stepx_companion::amoo_workspace::Turn {
            user: true,
            text: "先讨论".into(),
            at_ms: Some(20),
        });
        app.workspace.journal.tasks.push(task);
        let d = app.context_fingerprint();
        app.workspace.journal.tasks.last_mut().unwrap().turns[0].text = "改为周五".into();
        assert_ne!(d, app.context_fingerprint());
    }
    #[test]
    fn context_manual_activity_does_not_evict_scheduled_attempts() {
        let mut config = ScopePreferences::default();
        let target = AiTarget::Period(ledger::PeriodBrief {
            begin_ms: 42,
            evidence_revision: "facts".into(),
            ..Default::default()
        });
        record_ai_attempt(&mut config, &target, true, 1000);
        for n in 0..100 {
            record_ai_attempt(&mut config, &target, false, 1001 + n);
        }
        assert_eq!(config.ai_attempts.len(), 64);
        assert!(config
            .scheduled_attempts
            .contains_key(&target.attempt_key(true)));
    }
    #[test]
    fn context_cold_start_does_not_schedule_yesterdays_saved_period() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let old = ledger::PeriodBrief {
            begin_ms: 100,
            end_ms: 200,
            evidence_count: 5,
            ..Default::default()
        };
        assert!(!period_auto_due(&old, 300, today, 18));
        assert!(!period_auto_due(&old, 200, today, 18));
        assert!(period_auto_due(&old, 199, today, 18));
    }
    #[test]
    fn context_capture_failure_blocks_automatic_model_spending() {
        use chrono::TimeZone;
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.selected = Some(Device {
            serial: "test".into(),
            ..Default::default()
        });
        app.context.change_scope(Scope {
            device: "test".into(),
            account: None,
        });
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        app.context.set_config(ScopePreferences {
            enabled: true,
            automatic_ai: true,
            ..Default::default()
        });
        let now = Local
            .with_ymd_and_hms(2026, 10, 29, 18, 0, 0)
            .single()
            .unwrap();
        app.context.overview.periods.push(ledger::PeriodBrief {
            begin_ms: now.timestamp_millis() - 3_600_000,
            end_ms: now.timestamp_millis() + 3_600_000,
            evidence_count: 1,
            prompt: "synthetic evidence".into(),
            evidence_revision: "facts".into(),
            ..Default::default()
        });
        app.context.error = Some("账本保存失败".into());
        ready_test_automatic(
            &mut app,
            stepx_companion::model_source::Source::IndependentApi,
        );
        app.maybe_auto_context_ai_at(now);
        assert!(app.context.pending_outcomes.is_empty());
        assert!(app.context.config().scheduled_attempts.is_empty());
        app.context.error = None;
        app.maybe_auto_context_ai_at(now + chrono::Duration::seconds(16));
        assert_eq!(app.context.pending_outcomes.len(), 1);
        assert!(
            app.context.pending_outcomes[0].4,
            "Automatic generation must enforce the source eligibility gate"
        );
        assert_eq!(app.context.config().scheduled_attempts.len(), 1);
    }
    #[test]
    fn context_items_expand_and_reset_for_another_scope() {
        let mut app = Companion::new_for_test();
        let scope = Scope {
            device: "one".into(),
            account: None,
        };
        app.context.change_scope(scope.clone());
        app.context.overview.items = (0..5)
            .map(|i| ledger::PreparedItem {
                id: i.to_string(),
                title: format!("事项 {i}"),
                ..Default::default()
            })
            .collect();
        assert_eq!(app.context.visible_items().len(), 2);
        app.handle_context_action(&ContextAction::ToggleItems);
        assert_eq!(app.context.visible_items().len(), 5);
        assert_eq!(app.context.visible_items()[3].id, "3");
        let _ = app.render_context_panel();
        assert!(!app.context.change_scope(scope));
        assert!(app.context.show_all_items);
        app.handle_context_action(&ContextAction::ToggleItems);
        assert_eq!(app.context.visible_items().len(), 2);
        app.handle_context_action(&ContextAction::ToggleItems);
        app.context.change_scope(Scope {
            device: "two".into(),
            account: None,
        });
        assert!(!app.context.show_all_items);
    }

    fn wait_for_context_worker(ui: &mut ContextUi, phase: &str) {
        // This is a persistence/scope-isolation test, not a latency benchmark.
        // The real worker queues behind LEDGER_WRITES and performs durable file
        // and SQLite commits. Parallel native Windows CI can spend more than
        // five seconds in that queue alone; retain a bounded deadlock guard
        // without treating shared-runner scheduling as a product regression.
        let started = Instant::now();
        let deadline = started + Duration::from_secs(30);
        while ui.worker.is_some() && Instant::now() < deadline {
            ui.poll_worker();
            if ui.worker.is_some() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(
            ui.worker.is_none(),
            "context worker did not complete during {phase} after {:?}; scope={:?}, generation={}, error={:?}",
            started.elapsed(), ui.scope, ui.generation, ui.error,
        );
    }

    #[test]
    fn context_manual_relation_persists_and_late_split_cannot_replace_another_scope() {
        use stepx_companion::amoo_workspace::{Task, Turn};
        let temp = tempfile::tempdir().unwrap();
        let now = Local::now();
        let scope = Scope {
            device: "relation-phone".into(),
            account: None,
        };
        let mut task = Task::new(Some(scope.device.clone()), None);
        task.id = "relation-discussion".into();
        task.title = "展台方案".into();
        task.turns.push(Turn {
            at_ms: Some(now.timestamp_millis() - 1_000),
            user: true,
            text: "请准备明天评审的展台方案。".into(),
        });
        let mut capture = ledger::Capture {
            device: scope.device.clone(),
            now_ms: now.timestamp_millis(),
            utc_offset_seconds: now.offset().local_minus_utc(),
            tasks: vec![task],
            sms: Some(vec![sms::Message {
                provider_id: Some(22),
                address: "测试会务".into(),
                body: "场地安排改期，请确认可参加的时间。".into(),
                date_ms: now.timestamp_millis() - 500,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            }]),
            ..Default::default()
        };
        let initial = ledger::process(temp.path(), &capture).unwrap();
        assert_eq!(initial.items.len(), 2);
        let objects: Vec<_> = initial
            .items
            .iter()
            .flat_map(|item| item.source_objects.iter().map(|source| source.id.clone()))
            .collect();
        assert_eq!(objects.len(), 2);
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.root = temp.path().into();
        app.context.change_scope(scope.clone());
        app.context.overview = initial;
        app.handle_context_action(&ContextAction::Link(objects[0].clone(), objects[1].clone()));
        assert!(app.context.relation_active());
        assert!(app.context.start_pending_relation());
        wait_for_context_worker(&mut app.context, "persist link");
        assert!(app.context.error.is_none(), "{:?}", app.context.error);
        assert_eq!(app.context.overview.items.len(), 1);
        assert_eq!(app.context.overview.items[0].source_objects.len(), 2);
        assert_eq!(
            ledger::load(temp.path(), &scope.device, None)
                .unwrap()
                .items
                .len(),
            1
        );
        capture.now_ms += 1_000;
        assert_eq!(
            ledger::process(temp.path(), &capture).unwrap().items.len(),
            1
        );

        app.handle_context_action(&ContextAction::Separate(objects[1].clone()));
        assert!(app.context.start_pending_relation());
        let other = Scope {
            device: "another-phone".into(),
            account: None,
        };
        app.context.change_scope(other.clone());
        wait_for_context_worker(&mut app.context, "persist split after scope change");
        assert!(app.context.overview.items.is_empty());
        assert_eq!(app.context.scope, Some(other.clone()));
        assert!(ledger::load(temp.path(), &other.device, None)
            .unwrap()
            .items
            .is_empty());
        assert_eq!(
            ledger::load(temp.path(), &scope.device, None)
                .unwrap()
                .items
                .len(),
            2
        );
        capture.now_ms += 1_000;
        assert_eq!(
            ledger::process(temp.path(), &capture).unwrap().items.len(),
            2
        );
    }

    #[test]
    fn context_relation_invalidates_pending_ai_and_scope_switch_discards_edit_intent() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        let scope = Scope {
            device: "relation-phone".into(),
            account: Some("one".into()),
        };
        app.context.change_scope(scope);
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        app.context.overview.items = ["a", "b"]
            .into_iter()
            .map(|id| ledger::PreparedItem {
                id: id.into(),
                source_objects: vec![ledger::SourceObject {
                    id: id.into(),
                    title: id.into(),
                    source: "测试来源".into(),
                    task_id: None,
                }],
                ..Default::default()
            })
            .collect();
        let target = AiTarget::Item(ledger::PreparedItem {
            prompt: "synthetic text".into(),
            ..Default::default()
        });
        app.start_context_ai(target.clone(), false);
        assert!(app.context.pending_ai.is_some());
        let generation = app.context.generation;
        app.handle_context_action(&ContextAction::Link("a".into(), "b".into()));
        assert!(app.context.pending_ai.is_none());
        assert!(app.context.generation > generation);
        app.start_context_ai(target, false);
        assert!(app.context.pending_ai.is_none());
        app.context.link_picker = Some("a".into());
        app.context.relationship_details = Some("a".into());
        let _ = app.render_context_panel();
        app.context.change_scope(Scope {
            device: "relation-phone".into(),
            account: Some("two".into()),
        });
        assert!(app.context.pending_relation.is_none());
        assert!(app.context.link_picker.is_none());
        assert!(app.context.relationship_details.is_none());
    }
    #[test]
    fn outcome_request_waits_for_persistence_and_scope_change_cancels_queue() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        app.selected = Some(Device {
            serial: "outcome-phone".into(),
            ..Default::default()
        });
        app.context.change_scope(Scope {
            device: "outcome-phone".into(),
            account: Some("a".into()),
        });
        app.context.overview.items.push(ledger::PreparedItem {
            id: "matter".into(),
            input_revision: "current".into(),
            ..Default::default()
        });
        app.queue_outcome(OutcomeTarget::Item("matter".into()), true);
        assert!(app.context.dirty && app.context.worker.is_none());
        assert_eq!(app.context.pending_outcomes.len(), 1);
        let cancellation = app.context.work_cancel.clone();
        app.context.change_scope(Scope {
            device: "outcome-phone".into(),
            account: Some("b".into()),
        });
        assert!(app.context.pending_outcomes.is_empty());
        assert!(cancellation.load(Ordering::Acquire));
        assert!(!app.context.work_cancel.load(Ordering::Acquire));
    }
    fn outcome_send_fixture() -> (tempfile::TempDir, Companion, Vec<PathBuf>) {
        let (root, scope, plan) = preview_fixture();
        let todos = plan.with_file_name("todos.md");
        for path in [&plan, &todos] {
            std::fs::write(path, "# 已生成的原版成果\n").unwrap();
        }
        let target = OutcomeTarget::Item("matter".into());
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.context.root = root.path().into();
        app.selected = Some(Device {
            serial: scope.device.clone(),
            ..Default::default()
        });
        app.context.change_scope(scope.clone());
        app.context.needs_load = false;
        app.context.overview.items.push(ledger::PreparedItem {
            id: "matter".into(),
            input_revision: "original-revision".into(),
            ..Default::default()
        });
        app.context.outcomes.push(context_pipeline::RunResult {
            bundle: ledger::outcomes::OutcomeBundle {
                render_version: ledger::outcomes::RENDER_VERSION,
                scope_id: ledger::scope_id(&scope.device, scope.account.as_deref()),
                target,
                input_revision: "original-revision".into(),
                title: "成果".into(),
                conclusion: "有来源的判断".into(),
                files: vec![
                    ledger::outcomes::OutcomeFile {
                        kind: "plan".into(),
                        path: plan.clone(),
                        sha256: "plan-hash".into(),
                    },
                    ledger::outcomes::OutcomeFile {
                        kind: "todos".into(),
                        path: todos.clone(),
                        sha256: "todos-hash".into(),
                    },
                ],
                manifest_path: plan.with_file_name("bundle.json"),
                todo_count: 1,
                created_ms: 1,
            },
            deliveries: vec![],
            reused: true,
            delivery_error: None,
            delivery_attempt_ms: 0,
        });
        (root, app, vec![plan, todos])
    }
    #[test]
    fn incoming_sources_keep_previous_draft_readable_with_an_explicit_stale_label() {
        let (_root, mut app, files) = outcome_send_fixture();
        app.context.initialized = true;
        app.context.overview.items[0].input_revision = "new-incoming-message".into();
        let target = OutcomeTarget::Item("matter".into());
        assert!(app.context.outcome_path(&target).is_none());
        assert_eq!(
            app.context.readable_outcome_path(&target),
            Some(files[0].clone())
        );
        assert!(app
            .context
            .document_title(&files[0])
            .unwrap()
            .contains("上次成稿（有新资料）"));
        assert!(
            app.context.current_outcomes().is_empty(),
            "readable history must never become current"
        );
        app.context.scope.as_mut().unwrap().account = Some("another-account".into());
        assert!(app.context.readable_outcome_path(&target).is_none());
        assert!(app.context.document_title(&files[0]).is_none());
    }
    #[test]
    fn partial_source_coverage_does_not_turn_verified_delivery_into_failure() {
        let (_root, mut app, _) = outcome_send_fixture();
        app.context.collection_result =
            Some((Instant::now(), Some("Amoo 讨论仅覆盖部分记录".into())));
        assert_eq!(
            app.context.flow_progress(3),
            (3, false),
            "source coverage is not a delivery failure"
        );
        let result = &mut app.context.outcomes[0];
        result.deliveries = result
            .bundle
            .files
            .iter()
            .map(|file| context_pipeline::Delivery {
                kind: file.kind.clone(),
                transfer_id: "verified-transfer".into(),
                sha256: file.sha256.clone(),
                path: "/phone/test".into(),
                verified_ms: 1,
            })
            .collect();
        assert_eq!(app.context.flow_progress(3), (4, false));
        app.context.outcomes[0].deliveries[0].sha256 = "stale-hash".into();
        assert_eq!(
            app.context.flow_progress(3),
            (3, false),
            "old receipts cannot mark new files delivered"
        );
        app.context.outcomes[0].delivery_error = Some("回传校验失败".into());
        assert_eq!(app.context.flow_progress(3), (3, true));
        app.context.outcomes[0].delivery_error = None;
        app.context.error = Some("模型返回失败".into());
        assert_eq!(app.context.flow_progress(3), (2, true));
    }
    #[test]
    fn preview_send_returns_the_original_bundle_through_the_delivery_pipeline_only() {
        let (_root, mut app, files) = outcome_send_fixture();
        for path in files {
            app.handle_context_action(&ContextAction::Send(path));
            assert_eq!(
                app.context.pending_outcomes.len(),
                1,
                "Either document routes to the same complete bundle"
            );
            let (_, target, _, deliver, automatic, revision, _) = &app.context.pending_outcomes[0];
            assert_eq!(target, &OutcomeTarget::Item("matter".into()));
            assert!(*deliver && !*automatic);
            assert_eq!(
                revision.as_deref(),
                Some("original-revision"),
                "Return cannot request a new model draft"
            );
            assert!(
                app.interaction.review.is_none(),
                "No generic TransferFiles action is queued"
            );
            assert!(
                app.context.ai.is_none()
                    && app.context.pending_ai.is_none()
                    && app.context.worker.is_none()
            );
        }
    }
    #[test]
    fn preview_send_rejects_stale_bundle_identity_and_preserves_single_source_file_send() {
        for stale_scope in [true, false] {
            let (_root, mut app, files) = outcome_send_fixture();
            if stale_scope {
                app.context.outcomes[0].bundle.scope_id = "another-account".into();
            } else {
                app.context.outcomes[0].bundle.input_revision = "outdated-revision".into();
            }
            assert!(app.context.outcome_for_path(&files[0]).is_none());
            app.handle_context_action(&ContextAction::Send(files[0].clone()));
            assert!(app.context.pending_outcomes.is_empty());
            assert!(app.interaction.review.is_none());
            assert!(app
                .context
                .error
                .as_deref()
                .is_some_and(|error| error.contains("当前来源")));
        }
        let (_root, mut app, files) = outcome_send_fixture();
        let source = files[0].with_file_name("original-source.md");
        std::fs::write(&source, "# 原始讨论资料\n").unwrap();
        app.context.overview.items[0].path = source.clone();
        app.handle_context_action(&ContextAction::Send(source.clone()));
        assert!(app.context.pending_outcomes.is_empty());
        assert!(
            matches!(&app.interaction.review.as_ref().unwrap().action, Action::TransferFiles(paths, None) if paths == &vec![source])
        );
    }
    #[test]
    fn automatic_outcomes_debounce_changes_and_do_not_repeat_same_revision() {
        use chrono::TimeZone;
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        app.selected = Some(Device {
            serial: "outcome-phone".into(),
            ..Default::default()
        });
        app.context.change_scope(Scope {
            device: "outcome-phone".into(),
            account: Some("a".into()),
        });
        app.context.set_config(ScopePreferences {
            enabled: true,
            automatic_ai: true,
            ..Default::default()
        });
        let now = Local
            .with_ymd_and_hms(2026, 10, 30, 13, 0, 0)
            .single()
            .unwrap();
        app.context.overview.items.push(ledger::PreparedItem {
            id: "matter".into(),
            input_revision: "v1".into(),
            updated_ms: now.timestamp_millis(),
            presentation: Some(stepx_companion::context_review::MatterPresentation {
                priority: 80,
                ..Default::default()
            }),
            ..Default::default()
        });
        ready_test_automatic(
            &mut app,
            stepx_companion::model_source::Source::IndependentApi,
        );
        app.maybe_auto_context_ai_at(now);
        assert!(app.context.pending_outcomes.is_empty());
        app.context.overview.items[0].updated_ms -= 10_000;
        app.maybe_auto_context_ai_at(now);
        assert_eq!(app.context.pending_outcomes.len(), 1);
        app.context.pending_outcomes.clear();
        let mut config = app.context.config();
        config.ai_started_ms = 0;
        app.context.set_config(config);
        app.maybe_auto_context_ai_at(now);
        assert!(
            app.context.pending_outcomes.is_empty(),
            "An unchanged revision cannot spend twice"
        );
        app.context.overview.items[0].input_revision = "v2".into();
        app.maybe_auto_context_ai_at(now);
        assert_eq!(
            app.context.pending_outcomes.len(),
            1,
            "New evidence should get a new draft"
        );
    }

    #[test]
    fn scope_load_releases_only_proven_interrupted_current_outcome_intents() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.context.persistent = true;
        app.context.model_available = true;
        app.context.automatic_text_available = true;
        app.context.model_source = Some(stepx_companion::model_source::Source::IndependentApi);
        let scope = Scope {
            device: "recovery-phone".into(),
            account: Some("one".into()),
        };
        app.selected = Some(Device {
            serial: scope.device.clone(),
            ..Default::default()
        });
        app.context.change_scope(scope.clone());
        let now = Local::now();
        let mut overview = ledger::Overview::default();
        overview.items = ["interrupted", "completed"]
            .into_iter()
            .map(|id| ledger::PreparedItem {
                id: id.into(),
                input_revision: "v1".into(),
                updated_ms: now.timestamp_millis() - 10_000,
                presentation: Some(stepx_companion::context_review::MatterPresentation {
                    priority: 80,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        let mut config = ScopePreferences {
            enabled: true,
            automatic_ai: true,
            ..Default::default()
        };
        for key in [
            "outcome:item:interrupted:v1",
            "outcome:item:completed:v1",
            "outcome:item:interrupted:old",
            "legacy:day",
        ] {
            config.scheduled_attempts.insert(key.into(), 1);
        }
        app.context.set_config(config);
        app.context.dirty = false;
        let other = Scope {
            device: scope.device.clone(),
            account: Some("two".into()),
        };
        app.context.preferences.scopes.insert(
            other.key(),
            ScopePreferences {
                scheduled_attempts: BTreeMap::from([("outcome:item:interrupted:v1".into(), 2)]),
                ..Default::default()
            },
        );
        let (tx, receiver) = mpsc::channel();
        app.context.worker = Some(Worker {
            generation: app.context.generation,
            scope: Some(scope),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::OutcomesLoaded(
            overview,
            vec![],
            vec![
                (OutcomeTarget::Item("interrupted".into()), "v1".into()),
                (OutcomeTarget::Item("interrupted".into()), "old".into()),
            ],
            None,
        )))
        .unwrap();
        app.context.poll_worker();
        let config = app.context.config();
        assert!(!config
            .scheduled_attempts
            .contains_key("outcome:item:interrupted:v1"));
        for key in [
            "outcome:item:completed:v1",
            "outcome:item:interrupted:old",
            "legacy:day",
        ] {
            assert!(config.scheduled_attempts.contains_key(key));
        }
        assert!(app.context.preferences.scopes[&other.key()]
            .scheduled_attempts
            .contains_key("outcome:item:interrupted:v1"));
        assert!(app.context.dirty && app.context.worker.is_none());
        ready_test_automatic(
            &mut app,
            stepx_companion::model_source::Source::IndependentApi,
        );
        app.maybe_auto_context_ai_at(now);
        assert_eq!(app.context.pending_outcomes.len(), 1);
        assert!(
            app.context.pending_outcomes[0].4,
            "Recovery must use run_automatic without retry_failed"
        );
    }

    #[test]
    fn ordinary_capture_and_late_scope_load_do_not_release_scheduling_intents() {
        let mut ui = ContextUi::new(false);
        let scope = Scope {
            device: "one".into(),
            account: None,
        };
        ui.change_scope(scope.clone());
        let mut config = ScopePreferences::default();
        config
            .scheduled_attempts
            .insert("outcome:item:matter:v1".into(), 1);
        ui.set_config(config);
        let mut overview = ledger::Overview::default();
        overview.items.push(ledger::PreparedItem {
            id: "matter".into(),
            input_revision: "v1".into(),
            ..Default::default()
        });
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: Some(scope.clone()),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::Captured(
            overview.clone(),
            None,
            false,
            None,
            None,
            CollectionHealth::default(),
        )))
        .unwrap();
        ui.poll_worker();
        assert!(ui
            .config()
            .scheduled_attempts
            .contains_key("outcome:item:matter:v1"));
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation.wrapping_sub(1),
            scope: Some(scope),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::OutcomesLoaded(
            overview,
            vec![],
            vec![(OutcomeTarget::Item("matter".into()), "v1".into())],
            None,
        )))
        .unwrap();
        ui.poll_worker();
        assert!(ui
            .config()
            .scheduled_attempts
            .contains_key("outcome:item:matter:v1"));
    }

    #[test]
    fn worker_resolves_account_scope_and_forwards_captured_sms() {
        let mut ui = ContextUi::new(false);
        let scope = Scope {
            device: "phone-1".into(),
            account: Some("user-1".into()),
        };
        ui.change_scope(scope.clone());
        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: None,
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        tx.send(Ok(WorkerOutput::AccountScopeResolved(
            "phone-1".into(),
            Some("user-1".into()),
        )))
        .unwrap();
        assert!(ui.poll_worker());
        assert_eq!(
            ui.resolved_account,
            Some(("phone-1".into(), Some("user-1".into())))
        );

        let (tx, receiver) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            scope: Some(scope),
            receiver,
            saves_preferences: false,
            clears_scope: false,
            starts_ai: false,
            automatic_ai: false,
            edits_relation: false,
            collects_context: false,
        });
        let msg = sms::Message {
            provider_id: Some(42),
            address: "10690000".into(),
            body: "【顺丰速运】您的快件SF1234567890123已到达驿站，凭取件码8-2-1024取件".into(),
            date_ms: 1_760_000_000_000,
            outgoing: false,
            read: true,
            subscription: Some(1),
            kind: 1,
        };
        tx.send(Ok(WorkerOutput::Captured(
            ledger::Overview::default(),
            None,
            false,
            None,
            Some(vec![msg]),
            CollectionHealth::default(),
        )))
        .unwrap();
        assert!(ui.poll_worker());
        assert_eq!(
            ui.pending_captured_sms
                .as_ref()
                .map(|(_, messages)| messages.len()),
            Some(1)
        );
    }

    #[test]
    fn switched_phone_or_account_rejects_a_ready_old_capture_before_sms_merge() {
        for new_scope in [
            Scope {
                device: "phone-two".into(),
                account: Some("account-one".into()),
            },
            Scope {
                device: "phone-one".into(),
                account: Some("account-two".into()),
            },
        ] {
            let mut app = Companion::new_for_test();
            let old_scope = Scope {
                device: "phone-one".into(),
                account: Some("account-one".into()),
            };
            app.selected = Some(Device {
                serial: old_scope.device.clone(),
                ..Default::default()
            });
            app.amoo_ui.set_account_scope(old_scope.account.clone());
            app.context.change_scope(old_scope.clone());
            let old_message = sms::Message {
                provider_id: Some(42),
                address: "10690000".into(),
                body: "old phone private message".into(),
                date_ms: 1000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            };
            let (tx, receiver) = mpsc::channel();
            app.context.worker = Some(Worker {
                generation: app.context.generation,
                scope: Some(old_scope.clone()),
                receiver,
                saves_preferences: false,
                clears_scope: false,
                starts_ai: false,
                automatic_ai: false,
                edits_relation: false,
                collects_context: false,
            });
            tx.send(Ok(WorkerOutput::Captured(
                ledger::Overview::default(),
                None,
                false,
                None,
                Some(vec![old_message.clone()]),
                CollectionHealth::default(),
            )))
            .unwrap();
            // Also cover a result polled just before the user changed selection.
            app.context.pending_captured_sms = Some((old_scope, vec![old_message]));
            app.selected = Some(Device {
                serial: new_scope.device.clone(),
                ..Default::default()
            });
            app.amoo_ui.set_account_scope(new_scope.account.clone());
            app.poll_context_results();
            assert_eq!(app.context.scope, Some(new_scope));
            assert!(
                app.sms.is_empty(),
                "The new phone/account must not receive a ready old scope's SMS"
            );
            assert!(app.context.pending_captured_sms.is_none());
        }
    }

    #[test]
    fn failed_account_resolution_retries_with_backoff_and_keeps_current_sms() {
        let mut app = Companion::new_for_test();
        let scope = Scope {
            device: "phone-one".into(),
            account: Some("account-one".into()),
        };
        app.selected = Some(Device {
            serial: scope.device.clone(),
            ..Default::default()
        });
        app.amoo_ui.set_account_scope(scope.account.clone());
        app.context.change_scope(scope.clone());
        let message = sms::Message {
            provider_id: Some(42),
            address: "10690000".into(),
            body: "current phone message".into(),
            date_ms: 1000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        };
        app.context.pending_captured_sms = Some((scope, vec![message]));
        app.poll_context_results();
        assert_eq!(
            app.sms.len(),
            1,
            "A matching scope still forwards captured SMS"
        );
        let now = Instant::now();
        app.context.account_resolved_for = Some("phone-one".into());
        app.context.account_resolve_checked_at = Some(now);
        assert!(!app
            .context
            .account_resolution_due("phone-one", now + Duration::from_secs(29)));
        assert!(
            app.context
                .account_resolution_due("phone-one", now + Duration::from_secs(30)),
            "A transient account read must not disable collection forever"
        );
        assert!(app.context.account_resolution_due("phone-two", now));
    }

    #[test]
    fn collection_status_distinguishes_fresh_capture_from_ai_and_old_cached_results() {
        let mut ui = ContextUi::new(false);
        let scope = Scope {
            device: "phone".into(),
            account: Some("one".into()),
        };
        ui.change_scope(scope.clone());
        let started = Instant::now();
        let finish = |ui: &mut ContextUi, collects_context: bool, result| {
            let (tx, receiver) = mpsc::channel();
            ui.worker = Some(Worker {
                generation: ui.generation,
                scope: Some(scope.clone()),
                receiver,
                saves_preferences: false,
                clears_scope: false,
                starts_ai: !collects_context,
                automatic_ai: false,
                edits_relation: false,
                collects_context,
            });
            tx.send(result).unwrap();
            assert!(ui.poll_worker());
        };
        finish(&mut ui, false, Err("model request failed".into()));
        assert!(
            ui.collection_result_since(started).is_none(),
            "A model failure is not a collection failure"
        );
        finish(&mut ui, true, Err("ledger unavailable".into()));
        assert_eq!(
            ui.collection_result_since(started),
            Some(Err("ledger unavailable"))
        );
        finish(
            &mut ui,
            true,
            Ok(WorkerOutput::Captured(
                ledger::Overview::default(),
                None,
                false,
                None,
                None,
                CollectionHealth::default(),
            )),
        );
        assert_eq!(ui.collection_result_since(started), Some(Ok(())));
        finish(&mut ui, false, Err("another model failure".into()));
        assert_eq!(ui.collection_result_since(started), Some(Ok(())));
        assert!(
            ui.collection_result_since(Instant::now() + Duration::from_millis(1))
                .is_none(),
            "A future synchronization wave cannot reuse an old collection result"
        );
        ui.change_scope(Scope {
            device: "another-phone".into(),
            account: None,
        });
        assert!(ui.collection_result_since(started).is_none());
    }

    #[test]
    fn stopping_generation_persists_automatic_pause_across_restart_and_scope_changes() {
        let root = tempfile::tempdir().unwrap();
        let mut app = Companion::new_for_test();
        let scope = Scope {
            device: "stopped-phone".into(),
            account: Some("one".into()),
        };
        app.context.change_scope(scope.clone());
        app.context.set_config(ScopePreferences {
            enabled: true,
            automatic_ai: true,
            ..Default::default()
        });
        app.context.dirty = false;
        let cancel = app.context.work_cancel.clone();
        app.handle_context_action(&ContextAction::StopAi);
        assert!(cancel.load(Ordering::Acquire));
        assert!(app.context.dirty);
        assert!(!app.context.config().automatic_ai);
        assert!(app.context.config().enabled);
        save_preferences(root.path(), &app.context.preferences).unwrap();
        let mut restored = ContextUi::new(false);
        restored.preferences = load_preferences(root.path()).unwrap();
        restored.change_scope(scope.clone());
        assert!(!restored.config().automatic_ai);
        restored.change_scope(Scope {
            device: "another-phone".into(),
            account: None,
        });
        restored.change_scope(scope);
        assert!(!restored.config().automatic_ai);
        assert!(app.context.status.contains("暂停自动生成"));
    }

    #[test]
    fn phone_amoo_ledger_item_routes_to_resume_on_desktop() {
        let app = Companion::new_for_test();
        let item = ledger::PreparedItem {
            id: "phone-amoo:sess-abc-123".into(),
            title: "讨论下周发布计划".into(),
            ..Default::default()
        };
        let (label, action) = app.context_item_primary(&item);
        assert_eq!(label, "在电脑继续对话");
        let Action::HomeResumeTask(target) = action else {
            panic!("expected Action::HomeResumeTask");
        };
        assert_eq!(target, "phone-amoo:sess-abc-123");

        // Real PreparedItem produced by context_ledger::process has a SHA-256 digest id
        // and may carry :topic:<digest> on SourceObject.id.
        let hashed_item = ledger::PreparedItem {
            id: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".into(),
            title: "跨端架构评审".into(),
            source_objects: vec![ledger::SourceObject {
                id: "phone-amoo:sess-topic-99:topic:deadbeef".into(),
                title: "跨端架构评审".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (label2, action2) = app.context_item_primary(&hashed_item);
        assert_eq!(label2, "在电脑继续对话");
        let Action::HomeResumeTask(target2) = action2 else {
            panic!("expected Action::HomeResumeTask for hashed item");
        };
        assert_eq!(target2, "phone-amoo:sess-topic-99");

        // Multiple distinct phone-amoo sessions should prompt disambiguation ("选择讨论")
        let multi_session_item = ledger::PreparedItem {
            id: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            title: "合并讨论".into(),
            source_objects: vec![
                ledger::SourceObject {
                    id: "phone-amoo:sess-1:topic:aaa".into(),
                    ..Default::default()
                },
                ledger::SourceObject {
                    id: "phone-amoo:sess-2:topic:bbb".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (label3, action3) = app.context_item_primary(&multi_session_item);
        assert_eq!(label3, "选择讨论");
        assert!(matches!(
            action3,
            Action::Context(ContextAction::ItemDetails(_))
        ));
    }

    #[test]
    fn context_ledger_hub_adapts_budgets_and_renders_across_4x4_width_height_matrix() {
        use super::super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let (_root, mut app, _files) = outcome_send_fixture();
        app.context.initialized = true;
        app.context.needs_load = false;
        app.page = Page::Overview;

        // 1. Auto keeps a stable viewport budget regardless of arriving data.
        app.home_modules
            .set_module_height(HomeModuleId::ContextLedgerHub, HomeModuleHeight::Auto);
        let saved_outcomes = std::mem::take(&mut app.context.outcomes);
        let saved_items = std::mem::take(&mut app.context.overview.items);
        assert_eq!(app.context_effective_height(), HomeModuleHeight::Standard);

        app.context.overview.items = saved_items;
        app.context.outcomes = saved_outcomes;
        assert_eq!(app.context.current_outcomes().len(), 1);
        assert_eq!(app.context_effective_height(), HomeModuleHeight::Standard);

        for i in 0..8 {
            app.context.overview.items.push(ledger::PreparedItem {
                id: format!("matter-{i}"),
                title: format!("关键架构事项 #{i}：跨端同步协议与讨论成果回传验收"),
                source_count: 3,
                presentation: Some(stepx_companion::context_review::MatterPresentation {
                    priority: 85 + i as u8,
                    headline: format!("关键架构事项 #{i}：跨端同步协议与讨论成果回传验收"),
                    what_happened: "手机端与电脑端已完成多轮讨论并沉淀初步方案与待办清单".into(),
                    next_step: "核对各项待办负责人与截止时间后回传手机伴侣".into(),
                    primary_action: stepx_companion::context_review::MatterAction::ReviewOutput,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        assert_eq!(app.context.attention_items_ranked().len(), 8);
        assert_eq!(app.context_effective_height(), HomeModuleHeight::Standard);

        // 2. Verify distinct value tier specs and budgets across (OneThird, Compact), (OneHalf, Standard), (TwoThirds, Standard), (Full, Tall)
        let mut tier_specs = Vec::new();
        for (w, h, expected_spec) in [
            (
                HomeModuleWidth::OneThird,
                HomeModuleHeight::Compact,
                (20, 34, 1, 1, 1, 1, false, false, false),
            ),
            (
                HomeModuleWidth::OneHalf,
                HomeModuleHeight::Standard,
                (30, 50, 2, 1, 2, 2, false, true, false),
            ),
            (
                HomeModuleWidth::TwoThirds,
                HomeModuleHeight::Standard,
                (42, 68, 2, 2, 2, 4, true, true, false),
            ),
            (
                HomeModuleWidth::Full,
                HomeModuleHeight::Tall,
                (58, 96, 2, 3, 4, 8, true, true, true),
            ),
        ] {
            app.home_modules
                .set_module_width(HomeModuleId::ContextLedgerHub, w);
            app.home_modules
                .set_module_height(HomeModuleId::ContextLedgerHub, h);
            let spec = app.context_value_tier_spec();
            assert_eq!(spec, expected_spec);
            assert!(!tier_specs.contains(&spec));
            tier_specs.push(spec);
        }

        // 3. Render across all 4 widths x 4 heights without panicking
        for w in HomeModuleWidth::ALL {
            for h in HomeModuleHeight::ALL {
                app.home_modules
                    .set_module_width(HomeModuleId::ContextLedgerHub, w);
                app.home_modules
                    .set_module_height(HomeModuleId::ContextLedgerHub, h);
                let _ = std::hint::black_box(app.render_context_panel());
            }
        }
    }
}

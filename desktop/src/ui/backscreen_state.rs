//! Backscreen editor: edits are local until the user starts playback.
use stepx_companion::file_dialog::DialogKind;
use super::*;
use facloud_ui::{clipboard::ClipboardContent, TypedActionView};
use stepx_companion::{backscreen as bs, backscreen_amoo as ba, companion_app as ca};

/// UI requests only advance an atomic generation. Phone commands serialize on workers;
/// once a stop/restart supersedes a queued command, that command never reaches ADB.
// Shared across state resets too: reconnecting the same phone must not let a new stop
// race an already-running start from the previous UI state. Only workers take this lock.
static POMODORO_WORKER_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Default)]
struct PomodoroCommandLane {
    generation: std::sync::atomic::AtomicU64,
}

impl PomodoroCommandLane {
    fn advance(&self) -> u64 {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            .wrapping_add(1)
    }

    fn current(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    fn run_if_current<T>(&self, generation: u64, operation: impl FnOnce() -> T) -> Option<T> {
        let _worker = POMODORO_WORKER_GATE.lock().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(std::sync::atomic::Ordering::Acquire) != generation {
            return None;
        }
        Some(operation())
    }
}

fn days_in_calendar_month(year: i32, month: u32) -> u32 {
    let (next_y, next_m) = if month >= 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let Some(first_of_this) = chrono::NaiveDate::from_ymd_opt(year, month, 1) else {
        return 30;
    };
    let Some(first_of_next) = chrono::NaiveDate::from_ymd_opt(next_y, next_m, 1) else {
        return 30;
    };
    (first_of_next - first_of_this).num_days().clamp(28, 31) as u32
}

#[cfg(test)]
fn rewrite_local_calendar_ics(_events: &[stepx_companion::toolbox::PhoneCalendarEvent]) {}

#[cfg(not(test))]
fn rewrite_local_calendar_ics(events: &[stepx_companion::toolbox::PhoneCalendarEvent]) {
    let path =
        stepx_companion::resources::user_config().join("stepx-companion/desktop-calendar.ics");
    let _ = std::fs::remove_file(&path);
    let _ = stepx_companion::toolbox::export_merged_calendar_ics(events);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum LiveCapsuleFilter {
    #[default]
    All,
    LiveOnly,
    ImportantOnly,
}

#[derive(Clone, Debug)]
pub(super) enum BackScreenAction {
    Tab(u8),
    SelectPreset(bs::Preset),
    SelectNative(bs::NativeFeature),
    SelectApp(String, String),
    RefreshGallery,
    RecentImage(usize),
    DisplayMode(String),
    SaveBrightness,
    SaveMarquee,
    SetMarqueePreset(String),
    ToggleMarquee,
    CameraOff,
    PrepareClear,
    CancelClear,
    ClearFrame,
    Brightness(u8),
    Fps(u8),
    Duration(u32),
    Invert,
    TogglePreview,
    Play,
    Stop,
    PickImage,
    Dither,
    Threshold,
    Fit,
    AmooMode(bool),
    Generate,
    CancelGenerate,
    Prompt(String),
    Advanced,
    CompanionInspect,
    CompanionInstall,
    GameArm,
    GameStart,
    CompanionStop,
    ToggleAutoAiBreath,
    ToggleAutoNotifyFlash,
    ToggleSmartDisplay,
    SelectSmartPreviewKind(bs::SmartMessageIconKind),
    SelectSmartPreviewCount(u32),
    SelectSmartPreviewMode(u8),
    SyncSmartDisplayNow,
    StartPomodoro(u32),
    StopPomodoro,
    InspectStepOsSuite,
    SyncCalendarAndSuite,
    PushDesktopClipboard,
    PullPhoneClipboard,
    TriggerMcpQuickAction(String),
    FillOtpOnPhone(String),
    CalendarCreatePreset(u8),
    CalendarDelete(i64),
    CalendarPrevMonth,
    CalendarNextMonth,
    CalendarToday,
    CalendarSelectDate(i32, u32, u32),
    CalendarOpenOccurrence(i64, i64),
    CalendarCreateOnSelectedDate(Option<&'static str>),
    ToggleAutoSyncCalendar,
    ToggleAutoSyncClipboard,
    AlarmCreatePreset(u8),
    AlarmSelectQuickTime(&'static str),
    AlarmCreateCustom,
    AlarmToggle(i64, bool),
    AlarmDelete(i64),
    GalleryRefresh,
    GalleryCategory(u8),
    GallerySelect(usize),
    GalleryZoomIn,
    GalleryZoomOut,
    GalleryZoomSet(u16),
    GalleryToggleLightbox,
    GalleryPrevPhoto,
    GalleryNextPhoto,
    GalleryRotateRight(String),
    GallerySetGridCols(u8),
    GalleryCopyPath(String),
    GallerySaveToDesktop(String),
    GalleryOpenOnDesktop(String),
    GallerySendToAmoo(String),
    GalleryImportFromDesktop,
    GalleryCapturePhoneScreenshot,
    GalleryDelete(String),
    PresetFilter(u8),
    McpQuickOpenCalendar,
    McpQuickOpenAlarm,
    McpToggleDnd,
    SetBrightnessPercent(u8),
    ApplyPreset(bs::Preset),
    CalendarToggleDone(i64),
    CalendarEditEvent(i64),
    CalendarCancelEdit,
    CalendarSendReminderToPhone(i64),
    GalleryExportFiltered,
    GalleryOpenExportFolder,
    LocalSynthesize,
    TogglePixel(usize, usize),
    InvertFrame,
    ShiftFrame(i32, i32),
    ClearDiyFrame,
    SetThresholdValue(Option<u8>),
    RotateImage90,
    FlipImageH,
    CalendarStartCreateOnDate(i32, u32, u32),
    CalendarQuickAddOnDate {
        year: i32,
        month: u32,
        day: u32,
        title: String,
        time: String,
    },
    CalendarClearDay(i32, u32, u32),
    CalendarDuplicateToTomorrow(i64),
    CalendarPostponeOneDay(i64),
    CalendarPrefillFromText {
        title: String,
        time_hint: String,
    },
    AlarmEdit(i64),
    AlarmCancelEdit,
    AlarmShiftMinutes(i64, i32),
    AlarmToggleAll(bool),
    McpCategoryFilter(u8),
    PushSpecificClipboard(String),
    ToggleLiveCapsulePopover,
    CloseLiveCapsulePopover,
    SetLiveCapsuleFilter(LiveCapsuleFilter),
    CompleteCapsuleItem {
        id: String,
        pickup_code: Option<String>,
        sms_address: Option<String>,
        im_thread_id: Option<String>,
        notif_key: Option<String>,
    },
    OpenFulfillment { id: String },
    OpenCapsuleTarget {
        sms_address: Option<String>,
        im_thread_id: Option<String>,
        open_calendar: bool,
        open_alarms: bool,
    },
    PushPhoneLiveCapsule {
        tag: String,
        pill_text: String,
        title: String,
        subtitle: String,
        countdown_secs: Option<u64>,
        sync_backscreen: bool,
    },
    DismissPhoneLiveCapsule(String),
    ClearAllPhoneLiveCapsules,
}

enum CompanionEvent {
    Progress(String),
    Inspected(ca::CompanionStatus),
    Applied {
        title: String,
        revision: u64,
        message: String,
        mode: String,
    },
    TemporaryStarted {
        playback: bs::Playback,
        title: String,
        revision: u64,
    },
    Game {
        message: String,
        mode: String,
    },
    Stopped(String),
    Failed(String),
}

pub(super) struct BackScreenUiState {
    pub tab: u8,
    pub source: bs::Source,
    pub config: bs::PlaybackConfig,
    pub image_options: bs::ImageOptions,
    pub image_path: Option<PathBuf>,
    pub native_feature: Option<bs::NativeFeature>,
    pub applying_native: bool,
    pub converting_image: bool,
    pub clear_confirmation: bool,
    pub title: String,
    pub description: String,
    pub origin: String,
    pub frame: Vec<u8>,
    pub preview_running: bool,
    preview_visible: bool,
    resume_preview: bool,
    preview_initialized: bool,
    gallery_loaded: bool,
    gallery_rx: Option<mpsc::Receiver<Vec<PathBuf>>>,
    pub playing: bool,
    pub applied_title: String,
    pub applied_status: String,
    pub applied_frame: Vec<u8>,
    pub applied_elapsed_ms: u64,
    pub applied_to_phone: bool,
    pub companion_status: String,
    pub companion_installed: Option<bool>,
    pub companion_busy: bool,
    pub companion_mode: String,
    pub companion_permission: bool,
    pub(super) revision: u64,
    applied_revision: Option<u64>,
    pending_play: bool,
    companion_checked_device: Option<String>,
    companion_rx: Option<mpsc::Receiver<CompanionEvent>>,
    pub elapsed_ms: u64,
    pub frames_sent: u64,
    pub status: String,
    pub error: Option<String>,
    pub advanced: bool,
    pub amoo_image: bool,
    pub generating: bool,
    pub generator_status: String,
    pub recent_images: Vec<PathBuf>,
    pub scroll: ClippedScrollStateHandle,
    started: Instant,
    playback: Option<bs::Playback>,
    generation: Option<ba::Job>,
    active_device: Option<String>,
    native_rx: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    image_rx: Option<mpsc::Receiver<std::result::Result<(PathBuf, String, Vec<u8>), String>>>,
    pub auto_ai_breath: bool,
    pub auto_notify_flash: bool,
    pub smart_display_enabled: bool,
    pub smart_display_active_on_phone: bool,
    pub last_synced_autonomous_cfg: Option<(bool, bool, u8, bool)>,
    pub smart_display_last_pushed: Option<(bs::SmartMessageIconKind, u32)>,
    pub smart_display_last_pushed_slides: Option<Vec<bs::SmartDisplaySlide>>,
    pub smart_display_live_slides: Vec<bs::SmartDisplaySlide>,
    pub smart_display_cleared_ids: std::collections::HashSet<String>,
    pub smart_display_seen_unread_ids: std::collections::HashSet<String>,
    pub smart_display_live_kind: bs::SmartMessageIconKind,
    pub smart_display_live_count: u32,
    pub smart_display_live_summary: String,
    pub smart_display_preview_kind: bs::SmartMessageIconKind,
    pub smart_display_preview_count: u32,
    pub smart_display_preview_mode: u8,
    pub smart_display_preview_otp: String,
    pub pomodoro_active: bool,
    pub pomodoro_started_at: Option<Instant>,
    pub pomodoro_duration_secs: u32,
    pub pomodoro_last_remaining_secs: Option<u32>,
    pub stepos_suite: stepx_companion::toolbox::StepOsAiSuiteReport,
    pub stepos_suite_loading: bool,
    pub stepos_suite_synced_at: Option<Instant>,
    pub stepos_suite_error: Option<String>,
    // Per-operation identity survives resets through its captured Instant, so an old
    // device's callback cannot overwrite a later request on the same device.
    stepos_suite_request: Option<Instant>,
    mcp_request: Option<Instant>,
    pomodoro_request: Option<Instant>,
    pomodoro_device: Option<String>,
    pomodoro_lane: std::sync::Arc<PomodoroCommandLane>,
    pub calendar_year: i32,
    pub calendar_month: u32,
    pub selected_calendar_day: u32,
    pub calendar_month_motion_started: Option<Instant>,
    pub calendar_day_motion_started: Option<Instant>,
    pub gallery_items: Vec<stepx_companion::toolbox::PhoneGalleryItem>,
    pub gallery_loading: bool,
    pub gallery_checked: bool,
    pub gallery_category: u8,
    pub gallery_selected: usize,
    pub gallery_zoom_pct: u16,
    pub gallery_lightbox: bool,
    pub gallery_grid_cols: u8,
    pub gallery_rotations: std::collections::HashMap<String, u16>,
    pub completed_calendar_events: std::collections::HashSet<i64>,
    pub(super) calendar_state_scope: Option<String>,
    pub editing_calendar_event_id: Option<i64>,
    pub editing_alarm_id: Option<i64>,
    pub preset_filter: u8,
    pub last_synced_desktop_clip: String,
    pub last_synced_phone_clip: String,
    pub clipboard_sync_in_flight: bool,
    pub last_clipboard_check: Instant,
    pub clipboard_history: Vec<String>,
    pub mcp_category_filter: u8,
    pub live_capsule_popover_open: bool,
    pub live_capsule_filter: LiveCapsuleFilter,
    pub live_capsule_scroll: ClippedScrollStateHandle,
    pub dismissed_capsule_ids: std::collections::HashSet<String>,
    pub active_phone_capsule: Option<(String, String, String, Instant)>,
}

impl BackScreenUiState {
    fn cancel_pomodoro_commands(&mut self) {
        self.pomodoro_lane.advance();
        self.pomodoro_request = None;
        self.pomodoro_device = None;
    }

    fn accept_pomodoro_reply(
        &mut self,
        request: Instant,
        expected_serial: &str,
        selected_serial: Option<&str>,
    ) -> bool {
        if self.pomodoro_request != Some(request) {
            return false;
        }
        self.pomodoro_request = None;
        self.pomodoro_device = None;
        selected_serial == Some(expected_serial)
    }

    pub fn record_clipboard_history(&mut self, text: &str) {
        let clean = text.trim();
        if clean.is_empty() {
            return;
        }
        self.clipboard_history.retain(|item| item != clean);
        self.clipboard_history.insert(0, clean.to_string());
        if self.clipboard_history.len() > 8 {
            self.clipboard_history.truncate(8);
        }
    }
}

impl Default for BackScreenUiState {
    fn default() -> Self {
        use chrono::Datelike;
        let today = chrono::Local::now().date_naive();
        let source = bs::Source::Preset(bs::Preset::Astronaut);
        let config = bs::PlaybackConfig::default();
        let frame = Vec::new(); // No animation or disk scan until the module is opened.
        #[allow(unused_mut)] // Capture builds can select a local image during startup.
        let mut state = Self {
            tab: std::env::args()
                .position(|arg| arg == "--backscreen-tab")
                .and_then(|index| std::env::args().nth(index + 1))
                .map(|tab| match tab.as_str() {
                    "amoo" => 1,
                    "images" => 2,
                    "android" => 3,
                    "games" => 4,
                    "smart" => 5,
                    _ => 0,
                })
                .unwrap_or(0),
            source,
            config,
            image_options: bs::ImageOptions::default(),
            image_path: None,
            native_feature: None,
            applying_native: false,
            converting_image: false,
            clear_confirmation: false,
            title: bs::Preset::Astronaut.label().into(),
            description: bs::Preset::Astronaut.description().into(),
            origin: "内置动画".into(),
            frame,
            preview_running: false,
            preview_visible: false,
            resume_preview: true,
            preview_initialized: false,
            gallery_loaded: false,
            gallery_rx: None,
            playing: false,
            applied_title: String::new(),
            applied_status: "尚未应用图案".into(),
            applied_frame: vec![0; bs::SLOTS],
            applied_elapsed_ms: 0,
            applied_to_phone: false,
            companion_status: "连接手机后检测 StepX伴侣".into(),
            companion_installed: None,
            companion_busy: false,
            companion_mode: String::new(),
            companion_permission: false,
            revision: 0,
            applied_revision: None,
            pending_play: false,
            companion_checked_device: None,
            companion_rx: None,
            elapsed_ms: 0,
            frames_sent: 0,
            status: "点击图案预览，再应用到手机背屏".into(),
            error: None,
            advanced: false,
            amoo_image: false,
            generating: false,
            generator_status: String::new(),
            recent_images: Vec::new(),
            scroll: Default::default(),
            started: Instant::now(),
            playback: None,
            generation: None,
            active_device: None,
            native_rx: None,
            image_rx: None,
            auto_ai_breath: true,
            auto_notify_flash: true,
            smart_display_enabled: true,
            smart_display_active_on_phone: false,
            last_synced_autonomous_cfg: None,
            smart_display_last_pushed: None,
            smart_display_last_pushed_slides: None,
            smart_display_live_slides: Vec::new(),
            smart_display_cleared_ids: std::collections::HashSet::new(),
            smart_display_seen_unread_ids: std::collections::HashSet::new(),
            smart_display_live_kind: bs::SmartMessageIconKind::GeneralMessage,
            smart_display_live_count: 0,
            smart_display_live_summary: "正在监听手机真实通知 · 手机端自主驱动背屏（断开电脑仍生效），无通知时自动播放旋转太空人动画".into(),
            smart_display_preview_kind: bs::SmartMessageIconKind::WeChat,
            smart_display_preview_count: 1,
            smart_display_preview_mode: 0,
            smart_display_preview_otp: String::new(),
            pomodoro_active: false,
            pomodoro_started_at: None,
            pomodoro_duration_secs: 25 * 60,
            pomodoro_last_remaining_secs: None,
            stepos_suite: stepx_companion::toolbox::default_stepos_ai_suite_report(),
            stepos_suite_loading: false,
            stepos_suite_synced_at: None,
            stepos_suite_error: None,
            stepos_suite_request: None,
            mcp_request: None,
            pomodoro_request: None,
            pomodoro_device: None,
            pomodoro_lane: Default::default(),
            calendar_year: today.year(),
            calendar_month: today.month(),
            selected_calendar_day: today.day(),
            calendar_month_motion_started: None,
            calendar_day_motion_started: None,
            gallery_items: Vec::new(),
            gallery_loading: false,
            gallery_checked: false,
            gallery_category: 0,
            gallery_selected: 0,
            gallery_zoom_pct: 100,
            gallery_lightbox: false,
            gallery_grid_cols: 3,
            gallery_rotations: std::collections::HashMap::new(),
            completed_calendar_events: std::collections::HashSet::new(),
            calendar_state_scope: None,
            editing_calendar_event_id: None,
            editing_alarm_id: None,
            preset_filter: 0,
            last_synced_desktop_clip: String::new(),
            last_synced_phone_clip: String::new(),
            clipboard_sync_in_flight: false,
            last_clipboard_check: Instant::now(),
            clipboard_history: Vec::new(),
            mcp_category_filter: 0,
            live_capsule_popover_open: false,
            live_capsule_filter: LiveCapsuleFilter::All,
            live_capsule_scroll: Default::default(),
            dismissed_capsule_ids: std::collections::HashSet::new(),
            active_phone_capsule: None,
        };
        if !cfg!(test) {
            state.load_completed_calendar_events();
            state.load_gallery_rotations();
        }
        #[cfg(feature = "ui-capture")]
        if let Some(path) = std::env::args()
            .position(|arg| arg == "--backscreen-image")
            .and_then(|index| std::env::args().nth(index + 1))
        {
            state.tab = 2;
            let _ = state.apply_image(PathBuf::from(path), "Amoo 生成图片");
        }
        state
    }
}

impl Drop for BackScreenUiState {
    fn drop(&mut self) {
        // Device switches replace this state; invalidate any workers still in the lane.
        self.pomodoro_lane.advance();
        if let Some(playback) = &self.playback {
            playback.cancel();
        }
        if let Some(generation) = &self.generation {
            generation.cancel();
        }
    }
}

pub(super) const MAX_GALLERY_ROTATIONS: usize = 256;

impl BackScreenUiState {
    /// Bound `gallery_rotations` at `MAX_GALLERY_ROTATIONS` (256) entries so rotating
    /// many photos in long sessions never grows memory unboundedly.
    pub(super) fn insert_gallery_rotation(&mut self, remote_path: String, deg: u16) {
        if !self.gallery_rotations.contains_key(&remote_path) {
            let active_remote = self
                .gallery_items
                .get(self.gallery_selected)
                .map(|i| i.remote_path.as_str());
            while self.gallery_rotations.len() >= MAX_GALLERY_ROTATIONS {
                let victim = self
                    .gallery_rotations
                    .keys()
                    .find(|k| Some(k.as_str()) != active_remote)
                    .or_else(|| self.gallery_rotations.keys().next())
                    .cloned();
                let Some(victim) = victim else {
                    break;
                };
                self.gallery_rotations.remove(&victim);
            }
        }
        self.gallery_rotations.insert(remote_path, deg);
        self.save_gallery_rotations();
    }

    pub(super) fn calendar_state_dir(&self) -> Option<PathBuf> {
        self.calendar_state_scope.as_ref().map(|scope| super::ui_user_config_dir()
            .join("stepx-companion").join("calendar-state")
            .join(stepx_companion::context_ledger::scope_id(scope, None)))
    }

    pub fn completed_calendar_events_path(&self) -> Option<PathBuf> {
        self.calendar_state_dir().map(|dir| dir.join("completed-calendar-events.json"))
    }

    pub fn save_completed_calendar_events(&self) {
        let Some(path) = self.completed_calendar_events_path() else { return; };
        let mut sorted: Vec<i64> = self.completed_calendar_events.iter().copied().collect();
        sorted.sort_unstable();
        if let Ok(json) = serde_json::to_string_pretty(&sorted) {
            let _ = persist_calendar_state(&path, json.as_bytes());
        }
    }

    pub fn load_completed_calendar_events(&mut self) {
        self.completed_calendar_events.clear();
        let Some(path) = self.completed_calendar_events_path() else { return; };
        if path.is_symlink() || std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) { return; }
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&raw) {
            self.completed_calendar_events = ids.into_iter().collect();
        }
    }

    pub fn gallery_rotations_path() -> PathBuf {
        super::ui_user_config_dir()
            .join("stepx-companion")
            .join("gallery-rotations.json")
    }

    pub fn save_gallery_rotations(&self) {
        let path = Self::gallery_rotations_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.gallery_rotations) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_gallery_rotations(&mut self) {
        let path = Self::gallery_rotations_path();
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, u16>>(&raw) {
            self.gallery_rotations = map;
        }
    }

    pub fn pomodoro_remaining_secs(&self) -> u32 {
        if !self.pomodoro_active {
            return self.pomodoro_duration_secs;
        }
        let elapsed = self
            .pomodoro_started_at
            .map(|t| t.elapsed().as_secs() as u32)
            .unwrap_or(0);
        self.pomodoro_duration_secs.saturating_sub(elapsed)
    }

    pub(super) fn set_preview_visible(&mut self, visible: bool) {
        if self.preview_visible == visible {
            return;
        }
        if visible {
            if !self.preview_initialized {
                self.refresh_frame();
                self.preview_initialized = true;
            }
            self.started = Instant::now() - Duration::from_millis(self.elapsed_ms);
            self.preview_running = self.resume_preview;
        } else {
            self.resume_preview = self.preview_running;
            self.preview_running = false;
        }
        self.preview_visible = visible;
    }

    pub(super) fn needs_poll(&self) -> bool {
        self.preview_visible
            || self.pomodoro_active
            || self.pomodoro_request.is_some()
            || self.playback.is_some()
            || self.generation.is_some()
            || self.companion_rx.is_some()
            || self.native_rx.is_some()
            || self.image_rx.is_some()
            || self.gallery_rx.is_some()
    }

    fn load_gallery(&mut self) {
        if self.gallery_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.gallery_rx = Some(rx);
        self.gallery_loaded = true;
        std::thread::spawn(move || {
            let _ = tx.send(recent_images());
        });
    }

    pub fn has_unapplied_changes(&self) -> bool {
        self.applied_revision != Some(self.revision)
            || !self.applied_to_phone
            || self.title != self.applied_title
    }
    pub fn temporary_app(&self) -> bool {
        matches!(self.source, bs::Source::App { .. })
    }
    pub fn applying(&self) -> bool {
        self.companion_busy || self.pending_play || self.applying_native
    }
    pub fn selection(&self) -> Option<bs::Preset> {
        match self.source {
            bs::Source::Preset(preset) | bs::Source::App { preset, .. } => Some(preset),
            _ => None,
        }
    }
    fn reset_preview(&mut self) {
        self.started = Instant::now();
        self.elapsed_ms = 0;
        self.resume_preview = true;
        self.preview_running = self.preview_visible;
        self.preview_initialized = true;
        self.refresh_frame();
    }
    pub(super) fn refresh_frame(&mut self) {
        self.frame = bs::frame(
            &self.source,
            &self.config,
            Duration::from_millis(self.elapsed_ms),
            &bs::LiveData::default(),
        );
    }
    fn reconvert(&mut self) -> Result<()> {
        if let Some(path) = self.image_path.clone() {
            let origin = self.origin.clone();
            let description = self.description.clone();
            self.apply_image(path, &origin)?;
            self.description = description;
        }
        Ok(())
    }
    pub fn apply_image(&mut self, path: PathBuf, origin: &str) -> Result<()> {
        anyhow::ensure!(path.is_file(), "图片文件已移动或删除，请重新选择");
        if self.image_path.as_ref() != Some(&path) {
            self.image_options.fit = if origin.starts_with("Amoo") {
                bs::ImageFit::Cover
            } else {
                bs::ImageFit::Contain
            };
        }
        let options = self.image_options.clone();
        let origin = origin.to_string();
        let (sender, receiver) = mpsc::channel();
        self.image_rx = Some(receiver);
        self.converting_image = true;
        self.description = "图片正转为 17 × 23 灰度点阵，可调节裁切、反色和抖动后播放。".into();
        self.error = None;
        self.status = "正在转换图片，完成后更新点阵预览…".into();
        std::thread::spawn(move || {
            let result = bs::image_frame(&path, &options)
                .map(|frame| (path, origin, frame))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        Ok(())
    }
}

impl Companion {
    pub(super) fn stepos_read_error_is_fatal(err: Option<&str>) -> bool {
        let Some(raw) = err.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        raw.split('；')
            .map(str::trim)
            .filter(|seg| !seg.is_empty())
            .any(|seg| !seg.starts_with("部分电脑") && !seg.starts_with("双端日历未能保存"))
    }

    /// An incomplete bridge read must not replace a useful snapshot with default zeros.
    /// `checked` prevents an automatic retry loop; the Home retry button is explicit.
    pub(super) fn reject_incomplete_stepos_snapshot(
        &mut self,
        report: &stepx_companion::toolbox::StepOsAiSuiteReport,
    ) -> bool {
        if !Self::stepos_read_error_is_fatal(report.read_error.as_deref()) {
            return false;
        }
        let Some(error) = &report.read_error else {
            return false;
        };
        self.back_screen.stepos_suite.checked = true;
        self.back_screen.stepos_suite_error = Some(error.clone());
        self.error = true;
        self.status = format!("部分手机数据未读取，已保留上次快照：{error}");
        self.tools_feedback = Some((self.tool_page, self.status.clone(), true));
        true
    }

    pub(super) fn tick_auto_clipboard_sync(&mut self, ctx: &mut ViewContext<Self>) {
        if self.demo_mode
            || !self.prefs.auto_sync_clipboard
            || self.busy
            || self.back_screen.clipboard_sync_in_flight
            || self.back_screen.last_clipboard_check.elapsed() < Duration::from_secs(8)
        {
            return;
        }
        let Some(device) = self.selected.clone() else {
            return;
        };
        self.back_screen.last_clipboard_check = Instant::now();
        let desk_clip = ctx.clipboard().read().plain_text.trim().to_string();
        if !desk_clip.is_empty()
            && desk_clip.len() <= 4096
            && desk_clip != self.back_screen.last_synced_desktop_clip
            && desk_clip != self.back_screen.stepos_suite.phone_clipboard.trim()
        {
            self.back_screen.last_synced_desktop_clip = desk_clip.clone();
            self.back_screen.clipboard_sync_in_flight = true;
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let res = Adb::discover().and_then(|adb| {
                    stepx_companion::toolbox::push_clipboard_to_phone(&adb, &device, &desk_clip)
                        .map(|_| desk_clip)
                });
                let _ = tx.send_blocking(res);
            });
            ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                me.back_screen.clipboard_sync_in_flight = false;
                if let Ok(Ok(pushed)) = result {
                    me.back_screen.last_synced_phone_clip = pushed.clone();
                    me.back_screen.stepos_suite.phone_clipboard = pushed;
                    me.sync_live_express_parcels();
                    ctx.notify();
                }
            });
        } else if !self.back_screen.stepos_suite_loading {
            self.back_screen.clipboard_sync_in_flight = true;
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let res = Adb::discover().and_then(|adb| {
                    stepx_companion::toolbox::pull_clipboard_from_phone(&adb, &device)
                });
                let _ = tx.send_blocking(res);
            });
            ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                me.back_screen.clipboard_sync_in_flight = false;
                if let Ok(Ok(phone_raw)) = result {
                    let phone_clip = phone_raw.trim().to_string();
                    if !phone_clip.is_empty()
                        && phone_clip != me.back_screen.last_synced_phone_clip
                        && phone_clip != me.back_screen.last_synced_desktop_clip
                    {
                        me.back_screen.last_synced_phone_clip = phone_clip.clone();
                        me.back_screen.last_synced_desktop_clip = phone_clip.clone();
                        me.back_screen.stepos_suite.phone_clipboard = phone_clip.clone();
                        me.sync_live_express_parcels();
                        ctx.clipboard()
                            .write(ClipboardContent::plain_text(phone_clip));
                        ctx.notify();
                    }
                }
            });
        }
    }

    pub(super) fn sync_gallery_image_previews(&mut self) {
        let sel = self.back_screen.gallery_selected;
        for (idx, item) in self.back_screen.gallery_items.iter().enumerate() {
            if idx == sel {
                continue;
            }
            if let Some(p) = item.local_thumb_path.as_deref() {
                if !self.work.image_previews.contains_key(p) && std::path::Path::new(p).is_file() {
                    self.work
                        .insert_image_preview(p.to_string(), PathBuf::from(p));
                }
            }
        }
        if let Some(sel_item) = self.back_screen.gallery_items.get(sel) {
            if let Some(p) = sel_item.local_thumb_path.as_deref() {
                if !self.work.image_previews.contains_key(p) && std::path::Path::new(p).is_file() {
                    self.work
                        .insert_image_preview(p.to_string(), PathBuf::from(p));
                }
            }
        }
    }

    pub(super) fn filtered_gallery_indices(&self) -> Vec<usize> {
        let active_cat = self.back_screen.gallery_category;
        let query = self.fields[F_GALLERY_SEARCH].text.trim().to_lowercase();
        self.back_screen
            .gallery_items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                let cat_ok = match active_cat {
                    1 => item.category == "camera",
                    2 => item.category == "screenshot" || item.category == "screenshots",
                    3 => item.category == "pictures" || item.category == "weixin",
                    4 => {
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        (item.modified_ms > 0 && now_ms - item.modified_ms <= 7 * 86_400_000)
                            || idx < 8
                    }
                    5 => item.category == "expansion",
                    _ => true,
                };
                let search_ok = query.is_empty()
                    || item.name.to_lowercase().contains(&query)
                    || item.remote_path.to_lowercase().contains(&query)
                    || item.category_label().to_lowercase().contains(&query)
                    || item.time_label().to_lowercase().contains(&query)
                    || item.full_datetime_label().to_lowercase().contains(&query);
                if cat_ok && search_ok {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn set_calendar_focus(&mut self, year: i32, month: u32, day: u32) {
        let day = day.clamp(1, days_in_calendar_month(year, month));
        let calendar = &mut self.back_screen;
        let now = Instant::now();
        if (calendar.calendar_year, calendar.calendar_month) != (year, month) {
            calendar.calendar_month_motion_started = Some(now);
        }
        if (calendar.calendar_year, calendar.calendar_month, calendar.selected_calendar_day)
            != (year, month, day)
        {
            calendar.calendar_day_motion_started = Some(now);
        }
        calendar.calendar_year = year;
        calendar.calendar_month = month;
        calendar.selected_calendar_day = day;
    }

    pub(super) fn handle_backscreen_action(
        &mut self,
        action: &BackScreenAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(
            action,
            BackScreenAction::Tab(_)
                | BackScreenAction::CalendarPrevMonth
                | BackScreenAction::CalendarNextMonth
                | BackScreenAction::CalendarToday
                | BackScreenAction::CalendarSelectDate(_, _, _)
                | BackScreenAction::CalendarOpenOccurrence(_, _)
                | BackScreenAction::CalendarStartCreateOnDate(_, _, _)
                | BackScreenAction::GalleryCategory(_)
                | BackScreenAction::GallerySelect(_)
                | BackScreenAction::GalleryPrevPhoto
                | BackScreenAction::GalleryNextPhoto
                | BackScreenAction::GalleryToggleLightbox
                | BackScreenAction::GallerySetGridCols(_)
                | BackScreenAction::PresetFilter(_)
        ) {
            self.motion_started = Instant::now();
        }
        if let Err(error) = self.do_backscreen_action(action, ctx) {
            self.back_screen.error = Some(format!("{error:#}"));
            self.back_screen.status = format!("{error:#}");
            self.status = format!("{error:#}");
            self.error = true;
        }
    }
    fn is_desktop_native_mcp_quick_action(action_key: &str) -> bool {
        matches!(
            action_key,
            "open_filemanager"
                | "search_files"
                | "open_file_trash"
                | "open_safebox"
                | "open_app_manager"
                | "open_dialer"
                | "open_xms_service_numbers"
                | "open_mms"
                | "open_gallery"
                | "open_gallery_trash"
                | "open_backscreen"
                | "calendar_event"
                | "note_create"
                | "todo_create"
                | "open_remote_agent_hub"
                | "open_aiengine"
                | "open_aiengine_settings"
                | "open_operate_phone"
                | "open_alphame"
                | "open_alphame_address"
                | "open_amoo_memory"
                | "open_alphame_wifi"
                | "open_safety_ledger"
                | "open_agent_perm_manager"
                | "open_permission_manager"
                | "open_screen_perception"
        )
    }
    fn reject_offline_phone_action(&mut self, action: &BackScreenAction) -> bool {
        let requires_phone = match action {
            BackScreenAction::TriggerMcpQuickAction(k) => {
                !Self::is_desktop_native_mcp_quick_action(k)
            }
            _ => matches!(
                action,
                BackScreenAction::CalendarCreatePreset(_)
                    | BackScreenAction::CalendarDelete(_)
                    | BackScreenAction::CalendarCreateOnSelectedDate(_)
                    | BackScreenAction::CalendarQuickAddOnDate { .. }
                    | BackScreenAction::CalendarClearDay(_, _, _)
                    | BackScreenAction::CalendarDuplicateToTomorrow(_)
                    | BackScreenAction::CalendarPostponeOneDay(_)
                    | BackScreenAction::AlarmCreatePreset(_)
                    | BackScreenAction::AlarmCreateCustom
                    | BackScreenAction::AlarmToggle(_, _)
                    | BackScreenAction::AlarmDelete(_)
                    | BackScreenAction::AlarmShiftMinutes(_, _)
                    | BackScreenAction::AlarmToggleAll(_)
                    | BackScreenAction::SetBrightnessPercent(_)
                    | BackScreenAction::FillOtpOnPhone(_)
                    | BackScreenAction::McpToggleDnd
            ),
        };
        if !self.demo_mode && self.selected.is_none() && requires_phone {
            self.error = true;
            self.status = "手机未连接，操作尚未执行；原有快照与输入已保留".into();
            return true;
        }
        false
    }

    fn do_backscreen_action(
        &mut self,
        action: &BackScreenAction,
        ctx: &mut ViewContext<Self>,
    ) -> Result<()> {
        let calendar_id = match action {
            BackScreenAction::CalendarDelete(id)
            | BackScreenAction::CalendarToggleDone(id)
            | BackScreenAction::CalendarEditEvent(id)
            | BackScreenAction::CalendarDuplicateToTomorrow(id)
            | BackScreenAction::CalendarPostponeOneDay(id)
            | BackScreenAction::CalendarSendReminderToPhone(id) => Some(*id),
            BackScreenAction::CalendarCreateOnSelectedDate(_) => self.back_screen.editing_calendar_event_id,
            BackScreenAction::CalendarClearDay(y, m, d) => self.back_screen.stepos_suite.calendar_events.iter()
                .find(|event| event.date_ymd() == (*y, *m, *d)
                    && self.calendar_event_action_limit(event.id).is_some()).map(|event| event.id),
            _ => None,
        };
        if let Some(reason) = calendar_id.and_then(|id| self.calendar_event_action_limit(id)) {
            self.record_notice("日程未修改", reason);
            anyhow::bail!(reason);
        }
        // Reject before optimistic mutations, including nested calendar/alarm actions.
        if self.reject_offline_phone_action(action) {
            return Ok(());
        }
        let is_smart_or_stepos = matches!(
            action,
            BackScreenAction::SetMarqueePreset(_)
                | BackScreenAction::ToggleAutoAiBreath
                | BackScreenAction::ToggleAutoNotifyFlash
                | BackScreenAction::ToggleSmartDisplay
                | BackScreenAction::SelectSmartPreviewKind(_)
                | BackScreenAction::SelectSmartPreviewCount(_)
                | BackScreenAction::SelectSmartPreviewMode(_)
                | BackScreenAction::SyncSmartDisplayNow
                | BackScreenAction::StartPomodoro(_)
                | BackScreenAction::StopPomodoro
                | BackScreenAction::InspectStepOsSuite
                | BackScreenAction::SyncCalendarAndSuite
                | BackScreenAction::PushDesktopClipboard
                | BackScreenAction::PushSpecificClipboard(_)
                | BackScreenAction::PullPhoneClipboard
                | BackScreenAction::ToggleLiveCapsulePopover
                | BackScreenAction::CloseLiveCapsulePopover
                | BackScreenAction::SetLiveCapsuleFilter(_)
                | BackScreenAction::CompleteCapsuleItem { .. }
                | BackScreenAction::OpenFulfillment { .. }
                | BackScreenAction::OpenCapsuleTarget { .. }
                | BackScreenAction::PushPhoneLiveCapsule { .. }
                | BackScreenAction::DismissPhoneLiveCapsule(_)
                | BackScreenAction::ClearAllPhoneLiveCapsules
                | BackScreenAction::TriggerMcpQuickAction(_)
                | BackScreenAction::FillOtpOnPhone(_)
                | BackScreenAction::CalendarCreatePreset(_)
                | BackScreenAction::CalendarDelete(_)
                | BackScreenAction::CalendarPrevMonth
                | BackScreenAction::CalendarNextMonth
                | BackScreenAction::CalendarToday
                | BackScreenAction::CalendarSelectDate(_, _, _)
                | BackScreenAction::CalendarOpenOccurrence(_, _)
                | BackScreenAction::CalendarCreateOnSelectedDate(_)
                | BackScreenAction::CalendarStartCreateOnDate(_, _, _)
                | BackScreenAction::CalendarQuickAddOnDate { .. }
                | BackScreenAction::CalendarClearDay(_, _, _)
                | BackScreenAction::CalendarDuplicateToTomorrow(_)
                | BackScreenAction::CalendarPostponeOneDay(_)
                | BackScreenAction::CalendarPrefillFromText { .. }
                | BackScreenAction::CalendarToggleDone(_)
                | BackScreenAction::CalendarEditEvent(_)
                | BackScreenAction::CalendarSendReminderToPhone(_)
                | BackScreenAction::ToggleAutoSyncCalendar
                | BackScreenAction::ToggleAutoSyncClipboard
                | BackScreenAction::AlarmCreatePreset(_)
                | BackScreenAction::AlarmSelectQuickTime(_)
                | BackScreenAction::AlarmCreateCustom
                | BackScreenAction::AlarmToggle(_, _)
                | BackScreenAction::AlarmDelete(_)
                | BackScreenAction::AlarmEdit(_)
                | BackScreenAction::AlarmCancelEdit
                | BackScreenAction::AlarmShiftMinutes(_, _)
                | BackScreenAction::AlarmToggleAll(_)
                | BackScreenAction::GalleryRefresh
                | BackScreenAction::GalleryCategory(_)
                | BackScreenAction::GallerySelect(_)
                | BackScreenAction::GalleryZoomIn
                | BackScreenAction::GalleryZoomOut
                | BackScreenAction::GalleryZoomSet(_)
                | BackScreenAction::GalleryToggleLightbox
                | BackScreenAction::GalleryPrevPhoto
                | BackScreenAction::GalleryNextPhoto
                | BackScreenAction::GalleryRotateRight(_)
                | BackScreenAction::GallerySetGridCols(_)
                | BackScreenAction::GalleryCopyPath(_)
                | BackScreenAction::GallerySaveToDesktop(_)
                | BackScreenAction::GalleryOpenOnDesktop(_)
                | BackScreenAction::GallerySendToAmoo(_)
                | BackScreenAction::GalleryImportFromDesktop
                | BackScreenAction::GalleryCapturePhoneScreenshot
                | BackScreenAction::GalleryDelete(_)
                | BackScreenAction::PresetFilter(_)
                | BackScreenAction::McpQuickOpenCalendar
                | BackScreenAction::McpQuickOpenAlarm
                | BackScreenAction::McpToggleDnd
                | BackScreenAction::SetBrightnessPercent(_)
                | BackScreenAction::CalendarCancelEdit
                | BackScreenAction::GalleryExportFiltered
                | BackScreenAction::GalleryOpenExportFolder
                | BackScreenAction::LocalSynthesize
                | BackScreenAction::TogglePixel(_, _)
                | BackScreenAction::InvertFrame
                | BackScreenAction::ShiftFrame(_, _)
                | BackScreenAction::ClearDiyFrame
        );
        if !is_smart_or_stepos {
            if self.back_screen.converting_image
                && !matches!(
                    action,
                    BackScreenAction::Tab(_) | BackScreenAction::Advanced
                )
            {
                anyhow::bail!("正在转换图片，请稍候再修改");
            }
            if self.back_screen.applying_native {
                anyhow::bail!("正在应用手机联动，请等待完成");
            }
            if self.back_screen.companion_busy
                && !matches!(
                    action,
                    BackScreenAction::Tab(_) | BackScreenAction::Advanced
                )
            {
                anyhow::bail!("正在与 StepX伴侣同步，请稍候");
            }
            if self.back_screen.pending_play
                && !matches!(
                    action,
                    BackScreenAction::Stop | BackScreenAction::Tab(_) | BackScreenAction::Advanced
                )
            {
                anyhow::bail!("正在切换背屏图案，请等待应用完成");
            }
            if self.back_screen.generating
                && !matches!(
                    action,
                    BackScreenAction::CancelGenerate
                        | BackScreenAction::Tab(_)
                        | BackScreenAction::Advanced
                )
            {
                anyhow::bail!("Amoo 正在创作，请等待结果或取消后修改设计");
            }
        }
        self.back_screen.error = None;
        if matches!(
            action,
            BackScreenAction::DisplayMode(_)
                | BackScreenAction::SaveBrightness
                | BackScreenAction::SaveMarquee
                | BackScreenAction::CameraOff
                | BackScreenAction::ClearFrame
        ) {
            anyhow::ensure!(
                !self.back_screen.playing
                    && !(self.back_screen.applied_to_phone
                        && self.back_screen.companion_mode != "native"),
                "请先停止当前图案，再保存手机默认设置"
            );
        }
        if matches!(
            action,
            BackScreenAction::SelectPreset(_)
                | BackScreenAction::SelectNative(_)
                | BackScreenAction::SelectApp(_, _)
                | BackScreenAction::Brightness(_)
                | BackScreenAction::Fps(_)
                | BackScreenAction::Duration(_)
                | BackScreenAction::Invert
        ) {
            self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
        }
        match action {
            BackScreenAction::Tab(tab) => {
                if self.page != Page::BackScreen {
                    self.page = Page::BackScreen;
                }
                if self.back_screen.tab != *tab {
                    self.back_screen.tab = *tab;
                    if *tab == 5 {
                        if self.back_screen.smart_display_live_count > 0 {
                            self.back_screen.smart_display_preview_kind =
                                self.back_screen.smart_display_live_kind;
                            self.back_screen.smart_display_preview_count =
                                self.back_screen.smart_display_live_count;
                        }
                        self.apply_smart_preview_to_canvas();
                    }
                    self.trigger_subtab_switch();
                }
                self.back_screen.scroll = Default::default();
            }
            BackScreenAction::SelectPreset(preset) => {
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Preset(*preset);
                self.back_screen.image_path = None;
                self.back_screen.title = preset.label().into();
                self.back_screen.description = preset.description().into();
                self.back_screen.origin = "内置动画".into();
                self.back_screen.status = "已选择，点击「应用到背屏」后更换手机显示".into();
                self.back_screen.reset_preview();
            }
            BackScreenAction::SelectNative(feature) => {
                self.back_screen.native_feature = Some(*feature);
                self.back_screen.image_path = None;
                self.back_screen.title = feature.label().into();
                self.back_screen.origin = "Android 原生联动".into();
                self.back_screen.description = native_description(*feature).into();
                self.back_screen.source = bs::Source::Preset(match feature {
                    bs::NativeFeature::Clock => bs::Preset::Clock,
                    bs::NativeFeature::Camera => bs::Preset::Countdown,
                    bs::NativeFeature::Notifications => bs::Preset::Heart,
                });
                self.back_screen.status = "已选择，应用后由 Android 系统管理显示".into();
                self.back_screen.reset_preview();
            }
            BackScreenAction::SelectApp(package, label) => {
                self.back_screen.native_feature = None;
                let preset = self.back_screen.selection().unwrap_or(bs::Preset::Meteor);
                self.back_screen.source = bs::Source::App {
                    package: package.clone(),
                    preset,
                };
                self.back_screen.image_path = None;
                self.back_screen.title = format!("{} · {}", label, preset.label());
                self.back_screen.origin = "Android 应用联动".into();
                self.back_screen.description = format!(
                    "在手机打开 {} 时显示{}；离开应用后恢复原显示。保持桌面连接。",
                    label,
                    preset.label()
                );
                self.back_screen.status = "应用联动待播放，桌面不会自动打开手机应用".into();
                self.back_screen.reset_preview();
            }
            BackScreenAction::RefreshGallery => {
                self.back_screen.load_gallery();
            }
            BackScreenAction::RecentImage(index) => {
                let path = self
                    .back_screen
                    .recent_images
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("这张作品已不在图库中，请刷新图库"))?;
                self.back_screen.apply_image(path, "Amoo 作品图库")?;
                self.back_screen.tab = 2;
            }
            BackScreenAction::DisplayMode(mode) => {
                let mode = mode.clone();
                self.start_backscreen_setting(move |adb, device| {
                    bs::set_display_mode(adb, device, &mode)
                })?;
            }
            BackScreenAction::SaveBrightness => {
                let brightness = self.back_screen.config.brightness;
                self.start_backscreen_setting(move |adb, device| {
                    bs::set_brightness(adb, device, brightness)
                })?;
            }
            BackScreenAction::SaveMarquee => {
                let text = self.fields[F_BACK_MARQUEE].text.clone();
                let scroll = self.matrix_marquee_on;
                let invert = self.back_screen.config.invert;
                self.start_backscreen_setting(move |adb, device| {
                    bs::marquee(adb, device, &text, scroll, invert)
                })?;
            }
            BackScreenAction::SetMarqueePreset(preset) => {
                self.fields[F_BACK_MARQUEE].set(preset);
                self.back_screen.stepos_suite.backscreen_signature = preset.trim().to_string();
                self.matrix_marquee_on = true;
                let frame = bs::synthesize_chinese_signature_frame(
                    preset.trim(),
                    self.back_screen.config.invert,
                );
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Image(frame.clone());
                self.back_screen.frame = frame;
                self.back_screen.title = format!("背屏状态 · {}", preset.trim());
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                self.back_screen.status =
                    format!("已填入跑马灯预设「{preset}」，点击保存即可同步至手机背屏");
                self.status = self.back_screen.status.clone();
                self.error = false;
            }
            BackScreenAction::ToggleMarquee => self.matrix_marquee_on = !self.matrix_marquee_on,
            BackScreenAction::CameraOff => self
                .start_backscreen_setting(|adb, device| bs::set_camera_link(adb, device, false))?,
            BackScreenAction::PrepareClear => self.back_screen.clear_confirmation = true,
            BackScreenAction::CancelClear => self.back_screen.clear_confirmation = false,
            BackScreenAction::ClearFrame => {
                anyhow::ensure!(
                    self.back_screen.clear_confirmation,
                    "请先确认清除旧背屏图案"
                );
                self.back_screen.clear_confirmation = false;
                self.start_backscreen_setting(|adb, device| {
                    let transport = device.resolve_active_transport(adb)?;
                    let _ = bs::release_custom_frame_owner(adb, device);
                    bs::clear_frame(adb, device)?;
                    let state = adb.shell(&transport, &["dumpsys", "display_ext"])?;
                    anyhow::ensure!(
                        state.contains("appUsesCustomFrame=false"),
                        "背屏仍被其他应用占用，请先关闭该应用的背屏功能"
                    );
                    Ok("已清除旧自定义图案，恢复手机系统显示；现在可以播放新设计".into())
                })?;
            }
            BackScreenAction::Brightness(level) => {
                self.back_screen.config.brightness = (*level).clamp(1, 8);
                self.back_screen.refresh_frame();
                self.sync_smart_backscreen_display();
            }
            BackScreenAction::Fps(fps) => {
                self.back_screen.config.fps = *fps;
                self.back_screen.refresh_frame();
            }
            BackScreenAction::Duration(seconds) => {
                self.back_screen.config.duration_secs = *seconds;
                self.back_screen.reset_preview();
            }
            BackScreenAction::Invert => {
                self.back_screen.config.invert = !self.back_screen.config.invert;
                self.back_screen.refresh_frame();
                self.sync_smart_backscreen_display();
            }
            BackScreenAction::TogglePreview => {
                self.back_screen.preview_running = !self.back_screen.preview_running;
                if self.back_screen.preview_running {
                    self.back_screen.started =
                        Instant::now() - Duration::from_millis(self.back_screen.elapsed_ms);
                }
            }
            BackScreenAction::Play => {
                self.begin_backscreen_apply()?;
            }
            BackScreenAction::Stop => {
                self.back_screen.pending_play = false;
                if let Some(playback) = &self.back_screen.playback {
                    playback.cancel();
                }
                self.back_screen.status = "正在停止，并恢复手机原来的背屏设置…".into();
            }
            BackScreenAction::PickImage => {
                self.choose_path(
                    stepx_companion::file_dialog::FileDialog::new()
                        .set_title("选择图片，预览背屏点阵")
                        .add_filter("图片", &["png", "jpg", "jpeg", "webp", "gif"]),
                    DialogKind::OpenFile,
                    move |app, path, _ctx| {
                        app.back_screen.apply_image(path, "本地图片")?;
                        app.back_screen.tab = 2;
                        Ok(())
                    },
                );
            }
            BackScreenAction::Dither => {
                self.back_screen.image_options.dither = !self.back_screen.image_options.dither;
                self.back_screen.reconvert()?;
            }
            BackScreenAction::Threshold => {
                self.back_screen.image_options.threshold =
                    if self.back_screen.image_options.threshold.is_some() {
                        None
                    } else {
                        Some(128)
                    };
                self.back_screen.reconvert()?;
            }
            BackScreenAction::Fit => {
                self.back_screen.image_options.fit = match self.back_screen.image_options.fit {
                    bs::ImageFit::Contain => bs::ImageFit::Cover,
                    bs::ImageFit::Cover => bs::ImageFit::Contain,
                };
                self.back_screen.reconvert()?;
            }
            BackScreenAction::AmooMode(image) => self.back_screen.amoo_image = *image,
            BackScreenAction::Generate => {
                anyhow::ensure!(!self.back_screen.generating, "Amoo 正在生成，请等待或取消");
                let prompt = self.fields[F_BACK_PROMPT].text.trim().to_string();
                anyhow::ensure!(!prompt.is_empty(), "先描述想要的画面、功能、亮度或播放时长");
                // Always synthesize an instant local 17x23 pixel preview first so the user sees immediate results!
                let (local_title, local_frame) = bs::synthesize_prompt_frame(&prompt);
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Image(local_frame);
                self.back_screen.title = local_title.clone();
                self.back_screen.origin = "本地点阵极速合成".into();
                self.back_screen.description =
                    format!("已根据「{prompt}」即时合成 17×23 像素图案，可直接应用或手绘微调。");
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                self.back_screen.reset_preview();
                if let Some(device) = self.selected.clone() {
                    if let Ok(adb) = Adb::discover() {
                        self.back_screen.active_device = Some(device.serial.clone());
                        self.back_screen.generation = Some(ba::start(
                            adb,
                            device,
                            prompt,
                            if self.back_screen.amoo_image {
                                ba::Mode::Image
                            } else {
                                ba::Mode::Auto
                            },
                        ));
                        self.back_screen.generating = true;
                        self.back_screen.generator_status =
                            "已生成本地即时预览 · 正在同步联系手机 Amoo 优化…".into();
                        self.back_screen.tab = 1;
                        return Ok(());
                    }
                }
                self.back_screen.status = format!(
                    "已即时合成背屏点阵「{local_title}」（离线可用，连接手机后可一键应用）"
                );
                self.status = self.back_screen.status.clone();
                self.back_screen.tab = 1;
            }
            BackScreenAction::CancelGenerate => {
                if let Some(job) = &self.back_screen.generation {
                    job.cancel();
                }
                self.back_screen.generator_status = "正在取消…".into();
            }
            BackScreenAction::Prompt(prompt) => {
                self.fields[F_BACK_PROMPT].text = prompt.clone();
                self.back_screen.tab = 1;
                self.focus_field(F_BACK_PROMPT, ctx);
            }
            BackScreenAction::Advanced => {
                self.back_screen.advanced = !self.back_screen.advanced;
                self.back_screen.scroll = Default::default();
            }
            BackScreenAction::CompanionInspect
            | BackScreenAction::CompanionInstall
            | BackScreenAction::GameArm
            | BackScreenAction::GameStart
            | BackScreenAction::CompanionStop => self.start_companion_action(action.clone())?,
            BackScreenAction::ToggleAutoAiBreath => {
                self.back_screen.auto_ai_breath = !self.back_screen.auto_ai_breath;
                let note = if self.back_screen.auto_ai_breath {
                    "已开启：Amoo 思考时自动开启背屏呼吸光环联动"
                } else {
                    "已关闭：Amoo 思考时背屏呼吸光环自动联动"
                };
                self.back_screen.status = note.into();
                self.status = note.into();
            }
            BackScreenAction::ToggleAutoNotifyFlash => {
                self.back_screen.auto_notify_flash = !self.back_screen.auto_notify_flash;
                self.sync_smart_backscreen_display();
                let note = if self.back_screen.auto_notify_flash {
                    "已开启：收到验证码或重要消息时背屏闪烁提醒"
                } else {
                    "已关闭：收到验证码或重要消息时背屏闪烁提醒"
                };
                self.back_screen.status = note.into();
                self.status = note.into();
            }
            BackScreenAction::ToggleSmartDisplay => {
                self.back_screen.smart_display_enabled = !self.back_screen.smart_display_enabled;
                if self.back_screen.smart_display_enabled {
                    self.back_screen.smart_display_last_pushed = None;
                    self.back_screen.smart_display_last_pushed_slides = None;
                    self.back_screen.smart_display_preview_mode = 0;
                    self.sync_smart_backscreen_display();
                    let note = "已开启背屏智能显示：由手机端伴侣自主监听通知并驱动背屏轮播（断开电脑仍生效），无通知播放太空人动画";
                    self.back_screen.status = note.into();
                    self.status = note.into();
                } else {
                    self.sync_smart_backscreen_display();
                    self.back_screen.smart_display_live_summary = "智能显示已关闭".into();
                    let note = "已关闭背屏智能显示";
                    self.back_screen.status = note.into();
                    self.status = note.into();
                }
            }
            BackScreenAction::SelectSmartPreviewKind(kind) => {
                self.back_screen.smart_display_preview_kind = *kind;
                if self.back_screen.smart_display_preview_mode == 0
                    || self.back_screen.smart_display_preview_mode == 3
                    || self.back_screen.smart_display_preview_mode == 4
                {
                    self.back_screen.smart_display_preview_mode = 1;
                }
                self.apply_smart_preview_to_canvas();
                self.push_current_smart_preview_to_phone();
            }
            BackScreenAction::SelectSmartPreviewCount(count) => {
                self.back_screen.smart_display_preview_count = (*count).max(1);
                if self.back_screen.smart_display_preview_mode == 0
                    || self.back_screen.smart_display_preview_mode == 3
                    || self.back_screen.smart_display_preview_mode == 4
                {
                    self.back_screen.smart_display_preview_mode = 1;
                }
                self.apply_smart_preview_to_canvas();
                self.push_current_smart_preview_to_phone();
            }
            BackScreenAction::SelectSmartPreviewMode(mode) => {
                self.back_screen.smart_display_preview_mode = *mode;
                self.apply_smart_preview_to_canvas();
                self.push_current_smart_preview_to_phone();
            }
            BackScreenAction::SyncSmartDisplayNow => {
                self.back_screen.smart_display_enabled = true;
                self.back_screen.smart_display_last_pushed = None;
                self.back_screen.smart_display_last_pushed_slides = None;
                self.back_screen.smart_display_cleared_ids.clear();
                self.back_screen.smart_display_preview_mode = 0;
                self.sync_smart_backscreen_display();
                self.apply_smart_preview_to_canvas();
                self.push_current_smart_preview_to_phone();
            }
            BackScreenAction::StartPomodoro(seconds) => {
                // A restart supersedes any earlier start/stop callback, even offline.
                self.back_screen.pomodoro_request = None;
                self.back_screen.pomodoro_device = None;
                let dispatch_generation = self.back_screen.pomodoro_lane.advance();
                let raw_secs = if *seconds <= 60 {
                    (*seconds).saturating_mul(60)
                } else {
                    *seconds
                };
                let dur = raw_secs.clamp(60, 3600);
                let minutes = (dur / 60).max(1);
                self.back_screen.active_phone_capsule = Some((
                    "stepx_pomodoro".into(),
                    format!("🍅 {minutes}:00"),
                    format!("番茄专注 {minutes} 分钟"),
                    Instant::now(),
                ));
                self.back_screen.pomodoro_active = true;
                self.back_screen.pomodoro_started_at = Some(Instant::now());
                self.back_screen.pomodoro_duration_secs = dur;
                self.back_screen.pomodoro_last_remaining_secs = Some(dur);
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Preset(bs::Preset::Countdown);
                self.back_screen.config.duration_secs = dur;
                self.back_screen.image_path = None;
                let title = format!("专注番茄钟 · {} 分钟", dur / 60);
                self.back_screen.title = title.clone();
                self.back_screen.description =
                    "桌面与手机物理背屏同步专注倒计时，计时结束自动恢复".into();
                self.back_screen.origin = "背屏专注番茄钟".into();
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                self.back_screen.reset_preview();
                if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                    let brightness = self.back_screen.config.brightness;
                    let revision = self.back_screen.revision;
                    let request = Instant::now();
                    let serial = device.serial.clone();
                    self.back_screen.pomodoro_request = Some(request);
                    self.back_screen.pomodoro_device = Some(serial.clone());
                    let lane = self.back_screen.pomodoro_lane.clone();
                    self.back_screen.status =
                        format!("正在向手机物理背屏同步 {} 分钟专注倒计时…", dur / 60);
                    self.status = self.back_screen.status.clone();
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let Some(res) = lane.run_if_current(dispatch_generation, || {
                            Adb::discover().and_then(|adb| {
                                let capsule_req =
                                    stepx_companion::push::PhoneLiveCapsuleRequest {
                                        tag: "stepx_pomodoro".into(),
                                        pill_text: format!("🍅 {minutes}:00"),
                                        title: format!("番茄专注 {minutes} 分钟"),
                                        subtitle: "桌面与手机顶部胶囊、背屏点阵同步倒计时".into(),
                                        badge_text: "专注计时".into(),
                                        live_type: 11,
                                        countdown_secs: Some(u64::from(dur)),
                                        ongoing: true,
                                    };
                                let _ = stepx_companion::push::post_phone_live_capsule(
                                    &adb,
                                    &device.serial,
                                    &capsule_req,
                                );
                                let _ =
                                    stepx_companion::backscreen_amoo::sync_sprite_state_to_backscreen(
                                        &adb,
                                        &device,
                                        stepx_companion::backscreen_amoo::AgentSpriteState::TimerCountdown,
                                        0,
                                    );
                                bs::start_pomodoro_countdown(&adb, &device, dur, brightness)
                            })
                        }) else {
                            return;
                        };
                        let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                    });
                    ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                        if !me.back_screen.accept_pomodoro_reply(
                            request,
                            &serial,
                            me.selected.as_ref().map(|device| device.serial.as_str()),
                        ) {
                            return;
                        }
                        match result {
                            Ok(Ok(msg)) => {
                                me.back_screen.applied_title = title;
                                me.back_screen.applied_revision = Some(revision);
                                me.back_screen.applied_to_phone = true;
                                me.back_screen.applied_status = msg.clone();
                                me.back_screen.companion_mode = "animation".into();
                                me.back_screen.status = msg.clone();
                                me.status = msg;
                            }
                            Ok(Err(err)) => {
                                me.back_screen.status =
                                    format!("桌面番茄钟已开启（手机背屏同步提示：{err}）");
                                me.status = me.back_screen.status.clone();
                            }
                            Err(_) => {
                                me.back_screen.status =
                                    "桌面番茄钟已开启；手机背屏回执中断，同步结果未确认".into();
                                me.status = me.back_screen.status.clone();
                            }
                        }
                        ctx.notify();
                    });
                } else {
                    self.back_screen.status = format!(
                        "已开启 {} 分钟桌面专注番茄钟（尚未同步手机背屏）",
                        dur / 60
                    );
                    self.status = self.back_screen.status.clone();
                }
            }
            BackScreenAction::StopPomodoro => {
                // Invalidate the start immediately; stopping does not wait for its receipt.
                self.back_screen.pomodoro_request = None;
                self.back_screen.pomodoro_device = None;
                let dispatch_generation = self.back_screen.pomodoro_lane.advance();
                self.back_screen.pomodoro_active = false;
                self.back_screen.pomodoro_started_at = None;
                self.back_screen.pomodoro_last_remaining_secs = None;
                if self
                    .back_screen
                    .active_phone_capsule
                    .as_ref()
                    .is_some_and(|(tag, _, _, _)| tag == "stepx_pomodoro")
                {
                    self.back_screen.active_phone_capsule = None;
                }
                self.back_screen.status = "已结束桌面专注番茄钟；手机背屏停止状态尚未确认".into();
                self.status = self.back_screen.status.clone();
                if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                    let request = Instant::now();
                    let serial = device.serial.clone();
                    self.back_screen.pomodoro_request = Some(request);
                    self.back_screen.pomodoro_device = Some(serial.clone());
                    let lane = self.back_screen.pomodoro_lane.clone();
                    self.back_screen.status = "桌面计时已停止，正在停止手机背屏倒计时…".into();
                    self.status = self.back_screen.status.clone();
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let Some(res) = lane.run_if_current(dispatch_generation, || {
                            Adb::discover().and_then(|adb| {
                                let _ = stepx_companion::push::dismiss_phone_live_capsule(
                                    &adb,
                                    &device.serial,
                                    "stepx_pomodoro",
                                );
                                bs::stop_pomodoro_countdown(&adb, &device)
                            })
                        }) else {
                            return;
                        };
                        let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                    });
                    ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                        if !me.back_screen.accept_pomodoro_reply(
                            request,
                            &serial,
                            me.selected.as_ref().map(|device| device.serial.as_str()),
                        ) {
                            return;
                        }
                        match result {
                            Ok(Ok(msg)) => {
                                me.back_screen.applied_to_phone = false;
                                me.back_screen.applied_revision = None;
                                me.back_screen.applied_title.clear();
                                me.back_screen.applied_status = msg.clone();
                                me.back_screen.status = msg.clone();
                                me.status = msg;
                            }
                            Ok(Err(err)) => {
                                me.error = true;
                                me.back_screen.status = format!(
                                    "桌面计时已停止；手机背屏停止未确认：{err}"
                                );
                                me.status = me.back_screen.status.clone();
                            }
                            Err(_) => {
                                me.error = true;
                                me.back_screen.status =
                                    "桌面计时已停止；手机背屏停止回执中断，请重试".into();
                                me.status = me.back_screen.status.clone();
                            }
                        }
                        ctx.notify();
                    });
                }
            }
            BackScreenAction::InspectStepOsSuite | BackScreenAction::SyncCalendarAndSuite => {
                if self.back_screen.stepos_suite_loading {
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再读取使用统计与剪贴板".into();
                    return Ok(());
                };
                self.back_screen.stepos_suite_loading = true;
                self.back_screen.stepos_suite_error = None;
                let request = Instant::now();
                let serial = device.serial.clone();
                self.back_screen.stepos_suite_request = Some(request);
                self.back_screen.stepos_suite.last_action_note =
                    "正在读取手机屏幕使用统计、剪贴板与系统服务状态…".into();
                let (tx, rx) = async_channel::bounded(1);
                let sync_calendar = matches!(action, BackScreenAction::SyncCalendarAndSuite);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        let mut report = stepx_companion::toolbox::inspect_stepos_ai_suite(&adb, &device)?;
                        // This is the explicit calendar/suite synchronization
                        // job, not a passive capability/status probe.
                        if sync_calendar { stepx_companion::toolbox::sync_stepos_ai_suite_calendar(&adb, &device, &mut report)?; }
                        Ok(report)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.back_screen.stepos_suite_request != Some(request) {
                        return;
                    }
                    me.back_screen.stepos_suite_request = None;
                    me.back_screen.stepos_suite_loading = false;
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    let was_background_synced = me
                        .auto_sync
                        .domain_status(AutoSyncDomain::CalendarAndSuite)
                        .is_synced()
                        && me.back_screen.stepos_suite_synced_at.is_some();
                    match result {
                        Ok(Ok(report)) => {
                            if me.reject_incomplete_stepos_snapshot(&report) {
                                if was_background_synced {
                                    me.back_screen.stepos_suite_error = None;
                                    me.touch_auto_sync_domain_synced(AutoSyncDomain::CalendarAndSuite);
                                } else {
                                    me.apply_auto_sync_results(vec![(
                                        AutoSyncDomain::CalendarAndSuite,
                                        report.read_error.clone(),
                                    )]);
                                    me.reconcile_auto_sync_domains_from_state();
                                }
                                ctx.notify();
                                return;
                            }
                            let sync_error = report.read_error.as_deref()
                                .map(str::trim)
                                .filter(|error| !error.is_empty())
                                .map(str::to_string);
                            me.back_screen.stepos_suite_synced_at = Some(Instant::now());
                            me.back_screen.stepos_suite_error = sync_error.clone();
                            let phone_clip = report.phone_clipboard.trim().to_string();
                            if !phone_clip.is_empty() {
                                me.back_screen.record_clipboard_history(&phone_clip);
                            }
                            if me.prefs.auto_sync_clipboard {
                                if !phone_clip.is_empty()
                                    && phone_clip != me.back_screen.last_synced_phone_clip
                                    && phone_clip != me.back_screen.last_synced_desktop_clip
                                {
                                    me.back_screen.last_synced_phone_clip = phone_clip.clone();
                                    me.back_screen.last_synced_desktop_clip = phone_clip.clone();
                                    ctx.clipboard()
                                        .write(ClipboardContent::plain_text(phone_clip));
                                }
                            }
                            let sig_text = report.backscreen_signature.trim().to_string();
                            if !sig_text.is_empty() {
                                me.back_screen.frame = bs::synthesize_chinese_signature_frame(
                                    &sig_text,
                                    me.back_screen.config.invert,
                                );
                            }
                            let note = report.last_action_note.clone();
                            me.back_screen.stepos_suite = report;
                            me.reconcile_calendar_event_states_from_descriptions();
                            me.sync_live_express_parcels();
                            if !me.busy {
                                me.status = note.clone();
                                me.error = sync_error.is_some();
                            }
                            me.tools_feedback = Some((me.tool_page, note, sync_error.is_some()));
                        }
                        Ok(Err(err)) => {
                            if was_background_synced {
                                me.back_screen.stepos_suite_error = None;
                                me.touch_auto_sync_domain_synced(AutoSyncDomain::CalendarAndSuite);
                                ctx.notify();
                                return;
                            }
                            me.back_screen.stepos_suite_error = Some(err.clone());
                            me.back_screen.stepos_suite.checked = true;
                            me.back_screen.stepos_suite.last_action_note =
                                format!("读取状态未完成：{err}");
                            if !me.busy {
                                me.status = format!("读取手机统计未完成：{err}");
                                me.error = true;
                            }
                            me.tools_feedback = Some((me.tool_page,
                                format!("读取手机统计未完成：{err}"), true));
                        }
                        Err(_) => {
                            if was_background_synced {
                                me.back_screen.stepos_suite_error = None;
                                me.touch_auto_sync_domain_synced(AutoSyncDomain::CalendarAndSuite);
                                ctx.notify();
                                return;
                            }
                            me.back_screen.stepos_suite.checked = true;
                            me.back_screen.stepos_suite_error = Some("读取任务已中断，请重试".into());
                            me.tools_feedback = Some((me.tool_page, "读取任务已中断，请重试".into(), true));
                        }
                    }
                    me.apply_auto_sync_results(vec![(
                        AutoSyncDomain::CalendarAndSuite,
                        me.back_screen.stepos_suite_error.clone(),
                    )]);
                    me.reconcile_auto_sync_domains_from_state();
                    ctx.notify();
                });
            }
            BackScreenAction::PushDesktopClipboard => {
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再推送剪贴板".into();
                    self.error = true;
                    return Ok(());
                };
                let typed = self.fields[F_OV_TEXT].text.trim().to_string();
                let raw_clip = ctx.clipboard().read().plain_text.trim().to_string();
                let clip_text = if !typed.is_empty() {
                    typed
                } else if !raw_clip.is_empty() {
                    raw_clip
                } else {
                    self.status = "电脑剪贴板为空，请先复制文字或在输入框填写内容".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = "正在推送电脑剪贴板到手机…".into();
                self.status = "正在推送电脑剪贴板到手机…".into();
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::push_clipboard_to_phone(&adb, &device, &clip_text)
                            .map(|note| {
                                let preview: String = clip_text.chars().take(48).collect();
                                let capsule_req =
                                    stepx_companion::push::PhoneLiveCapsuleRequest {
                                        tag: "stepx_clipboard".into(),
                                        pill_text: "📋 剪贴板已达".into(),
                                        title: "电脑已推送剪贴板到手机".into(),
                                        subtitle: preview,
                                        badge_text: "双端协同".into(),
                                        live_type: 1,
                                        countdown_secs: None,
                                        ongoing: false,
                                    };
                                let _ = stepx_companion::push::post_phone_live_capsule(
                                    &adb,
                                    &device.serial,
                                    &capsule_req,
                                );
                                (clip_text, note)
                            })
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((pushed_text, note))) => {
                            me.back_screen.record_clipboard_history(&pushed_text);
                            me.back_screen.last_synced_desktop_clip = pushed_text.clone();
                            me.back_screen.last_synced_phone_clip = pushed_text.clone();
                            me.back_screen.stepos_suite.phone_clipboard = pushed_text;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.back_screen.active_phone_capsule = Some((
                                "stepx_clipboard".into(),
                                "📋 剪贴板已达".into(),
                                "电脑已推送剪贴板到手机".into(),
                                Instant::now(),
                            ));
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("剪贴板推送失败：{err}");
                            me.error = true;
                            me.status = format!("剪贴板推送失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::PushSpecificClipboard(text) => {
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再推送剪贴板".into();
                    self.error = true;
                    return Ok(());
                };
                let clip_text = text.trim().to_string();
                if clip_text.is_empty() {
                    return Ok(());
                }
                self.back_screen.stepos_suite.last_action_note = "正在推送剪贴板记录到手机…".into();
                self.status = "正在推送剪贴板记录到手机…".into();
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::push_clipboard_to_phone(&adb, &device, &clip_text)
                            .map(|note| {
                                let preview: String = clip_text.chars().take(48).collect();
                                let capsule_req =
                                    stepx_companion::push::PhoneLiveCapsuleRequest {
                                        tag: "stepx_clipboard".into(),
                                        pill_text: "📋 剪贴板已达".into(),
                                        title: "电脑已推送剪贴板到手机".into(),
                                        subtitle: preview,
                                        badge_text: "双端协同".into(),
                                        live_type: 1,
                                        countdown_secs: None,
                                        ongoing: false,
                                    };
                                let _ = stepx_companion::push::post_phone_live_capsule(
                                    &adb,
                                    &device.serial,
                                    &capsule_req,
                                );
                                (clip_text, note)
                            })
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((pushed_text, note))) => {
                            me.back_screen.record_clipboard_history(&pushed_text);
                            me.back_screen.last_synced_desktop_clip = pushed_text.clone();
                            me.back_screen.last_synced_phone_clip = pushed_text.clone();
                            me.back_screen.stepos_suite.phone_clipboard = pushed_text;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.back_screen.active_phone_capsule = Some((
                                "stepx_clipboard".into(),
                                "📋 剪贴板已达".into(),
                                "电脑已推送剪贴板到手机".into(),
                                Instant::now(),
                            ));
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("剪贴板推送失败：{err}");
                            me.error = true;
                            me.status = format!("剪贴板推送失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::PullPhoneClipboard => {
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再拉取手机剪贴板".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note =
                    "正在从手机拉取最新剪贴板内容…".into();
                self.status = "正在从手机拉取剪贴板…".into();
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::pull_clipboard_from_phone(&adb, &device)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok(text)) => {
                            ctx.clipboard()
                                .write(ClipboardContent::plain_text(text.clone()));
                            me.back_screen.record_clipboard_history(&text);
                            me.back_screen.last_synced_phone_clip = text.clone();
                            me.back_screen.last_synced_desktop_clip = text.clone();
                            let preview: String = text.chars().take(28).collect();
                            let note = format!(
                                "已拉取手机剪贴板到电脑：「{}{}」",
                                preview,
                                if text.chars().count() > 28 { "…" } else { "" }
                            );
                            me.back_screen.stepos_suite.phone_clipboard = text;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.sync_live_express_parcels();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("拉取手机剪贴板失败：{err}");
                            me.error = true;
                            me.status = format!("拉取手机剪贴板失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::McpCategoryFilter(cat) => {
                self.back_screen.mcp_category_filter = *cat;
            }
            BackScreenAction::TriggerMcpQuickAction(_) | BackScreenAction::FillOtpOnPhone(_) => {
                let (action_key, displayed_code) = match action {
                    BackScreenAction::TriggerMcpQuickAction(key) => (key.as_str(), None),
                    BackScreenAction::FillOtpOnPhone(code) => ("fill_otp_one_by_one", Some(code.as_str())),
                    _ => unreachable!(),
                };
                match action_key {
                    "open_filemanager" | "search_files" | "open_file_trash" | "open_safebox" => {
                        self.handle_action(&Action::Page(Page::Files), ctx);
                        self.status = "已在桌面端打开文件管理器".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_app_manager" => {
                        self.handle_action(&Action::Page(Page::Apps), ctx);
                        self.status = "已在桌面端打开应用与权限管理".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_dialer" | "open_xms_service_numbers" => {
                        self.handle_action(&Action::Page(Page::Phone), ctx);
                        self.status = "已在桌面端打开电话".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_mms" => {
                        self.handle_action(&Action::Page(Page::Sms), ctx);
                        self.status = "已在桌面端打开消息".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_gallery" | "open_gallery_trash" => {
                        self.handle_action(&Action::Page(Page::Gallery), ctx);
                        if action_key == "open_gallery_trash" {
                            self.back_screen.gallery_category = 0;
                        }
                        self.status = "已在桌面端打开图库与相册管理".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_backscreen" => {
                        self.handle_action(&Action::Page(Page::BackScreen), ctx);
                        self.status = "已在桌面端打开背屏点阵工坊".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "calendar_event" => {
                        self.handle_action(&Action::Page(Page::Calendar), ctx);
                        self.status = "已在桌面端打开日历日程".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "note_create" | "todo_create" => {
                        self.handle_action(&Action::Page(Page::Schedule), ctx);
                        self.status = "已在桌面端打开日程与待办工作台".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_remote_agent_hub"
                    | "open_aiengine"
                    | "open_aiengine_settings"
                    | "open_operate_phone" => {
                        self.handle_action(&Action::Page(Page::Amoo), ctx);
                        self.status = "已在桌面端打开 Amoo 智能助手工作台".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_alphame" | "open_alphame_address" | "open_amoo_memory" => {
                        self.handle_action(&Action::AmooOpenMemory, ctx);
                        return Ok(());
                    }
                    "open_alphame_wifi" => {
                        self.handle_action(&Action::Page(Page::Connect), ctx);
                        self.status = "已在桌面端打开无线调试与设备连接".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    "open_safety_ledger"
                    | "open_agent_perm_manager"
                    | "open_permission_manager"
                    | "open_screen_perception" => {
                        self.handle_action(&Action::OpenTool(Some(ToolPage::AiSafety)), ctx);
                        self.status = "已在桌面端打开 AI 安全审计与权限台账".into();
                        self.back_screen.stepos_suite.last_action_note = self.status.clone();
                        self.error = false;
                        return Ok(());
                    }
                    _ => {}
                }
                let key = action_key.to_owned();
                let payload = if let Some(code) = displayed_code {
                    anyhow::ensure!(
                        (4..=8).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_digit()),
                        "当前验证码格式无效，请刷新后重试"
                    );
                    code.to_owned()
                } else if key == "open_clip_url_phone"
                    || key == "fill_otp_one_by_one"
                    || key == "query_express_number"
                {
                    let typed = self.fields[F_OV_TEXT].text.trim();
                    if !typed.is_empty() {
                        typed.to_string()
                    } else {
                        self.back_screen
                            .stepos_suite
                            .phone_clipboard
                            .trim()
                            .to_string()
                    }
                } else if key == "capcut_handoff" {
                    let typed = self.fields[F_PHONE_SCHEDULE_TITLE].text.trim();
                    if !typed.is_empty() {
                        typed.to_string()
                    } else if let Some(sel) = self
                        .back_screen
                        .gallery_items
                        .get(self.back_screen.gallery_selected)
                    {
                        sel.remote_path.clone()
                    } else {
                        self.back_screen
                            .gallery_items
                            .iter()
                            .take(4)
                            .map(|i| i.remote_path.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    }
                } else {
                    let s = self.fields[F_OV_TEXT].text.trim().to_string();
                    if s.is_empty() {
                        self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string()
                    } else {
                        s
                    }
                };
                if let Some(sig) = key.strip_prefix("backscreen_sig:") {
                    let stripped = sig.trim_start_matches(|c: char| {
                        c.is_whitespace()
                            || (c as u32) >= 0x1F000
                            || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                    });
                    let sig_clean = if stripped.trim().is_empty() {
                        sig.trim().to_string()
                    } else {
                        stripped.trim().to_string()
                    };
                    self.fields[F_BACK_MARQUEE].set(&sig_clean);
                    self.back_screen.stepos_suite.backscreen_signature = sig_clean.clone();
                    self.matrix_marquee_on = true;
                    let frame = bs::synthesize_chinese_signature_frame(
                        &sig_clean,
                        self.back_screen.config.invert,
                    );
                    self.back_screen.native_feature = None;
                    self.back_screen.source = bs::Source::Image(frame.clone());
                    self.back_screen.frame = frame;
                    self.back_screen.title = format!("背屏状态 · {sig_clean}");
                    self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                }
                if !self.demo_mode && self.selected.is_none() {
                    self.status = "请先连接手机后再执行快捷指令".into();
                    self.error = true;
                    return Ok(());
                }
                let prev_dnd = self.back_screen.stepos_suite.controls.dnd_on;
                let prev_wifi = self.back_screen.stepos_suite.controls.wifi_on;
                let prev_bt = self.back_screen.stepos_suite.controls.bluetooth_on;
                let evidence = &self.back_screen.stepos_suite.capability_evidence;
                let privacy_state = match key.as_str() {
                    "toggle_delete_protect" => Some(evidence.delete_protect_enabled),
                    "toggle_global_camera" => Some(evidence.camera_allowed),
                    "toggle_global_mic" => Some(evidence.microphone_allowed),
                    "toggle_global_location" => Some(evidence.location_enabled),
                    "toggle_screen_perception" => Some(evidence.screen_perception),
                    _ => None,
                };
                let is_privacy = privacy_state.is_some();
                let privacy_target = privacy_state.flatten().map(|enabled| !enabled);
                if is_privacy && !self.demo_mode {
                    if self.back_screen.mcp_request.is_some() {
                        self.status = "正在等待手机操作回执，请稍候再修改隐私设置".into();
                        return Ok(());
                    }
                    if privacy_target.is_none() {
                        self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
                        self.status = "先读取隐私开关状态，未修改手机；读取后请再次确认操作".into();
                        return Ok(());
                    }
                }
                let prev_flip = self.back_screen.stepos_suite.backscreen_flip_keep_running;
                let prev_stay = self.back_screen.stepos_suite.backscreen_stay_on_secs;

                let optimistic_note = if let Some(sig) = key.strip_prefix("backscreen_sig:") {
                    let stripped = sig.trim_start_matches(|c: char| {
                        c.is_whitespace()
                            || (c as u32) >= 0x1F000
                            || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                    });
                    let s = if stripped.trim().is_empty() {
                        sig.trim()
                    } else {
                        stripped.trim()
                    };
                    format!("已将背屏汉字签名设为：「{s}」")
                } else if key == "toggle_dnd" {
                    let next = !prev_dnd;
                    self.back_screen.stepos_suite.controls.dnd_on = next;
                    if next {
                        "已开启手机勿扰模式（专注免打扰）".into()
                    } else {
                        "已关闭手机勿扰模式（恢复正常通知）".into()
                    }
                } else if key == "set_dnd_on" {
                    self.back_screen.stepos_suite.controls.dnd_on = true;
                    "已开启手机勿扰模式（专注免打扰）".into()
                } else if key == "set_dnd_off" {
                    self.back_screen.stepos_suite.controls.dnd_on = false;
                    "已关闭手机勿扰模式（恢复正常通知）".into()
                } else if key == "toggle_wifi" {
                    let next = !prev_wifi;
                    self.back_screen.stepos_suite.controls.wifi_on = next;
                    if next {
                        "已开启手机 Wi-Fi 网络".into()
                    } else {
                        "已关闭手机 Wi-Fi 网络".into()
                    }
                } else if key == "toggle_bluetooth" {
                    let next = !prev_bt;
                    self.back_screen.stepos_suite.controls.bluetooth_on = next;
                    if next {
                        "已开启手机蓝牙连接".into()
                    } else {
                        "已关闭手机蓝牙连接".into()
                    }
                } else if is_privacy {
                    "正在保存手机系统设置并回读确认…".into()
                } else if key == "backscreen_toggle_flip_keep" {
                    let next = !prev_flip;
                    self.back_screen.stepos_suite.backscreen_flip_keep_running = next;
                    if next {
                        "已开启翻转手机继续运行".into()
                    } else {
                        "已关闭翻转手机继续运行".into()
                    }
                } else if key == "backscreen_stay_15" {
                    self.back_screen.stepos_suite.backscreen_stay_on_secs = 15;
                    "已将背屏常亮时长设为 15 秒".into()
                } else if key == "backscreen_stay_30" {
                    self.back_screen.stepos_suite.backscreen_stay_on_secs = 30;
                    "已将背屏常亮时长设为 30 秒".into()
                } else if key == "backscreen_stay_60" {
                    self.back_screen.stepos_suite.backscreen_stay_on_secs = 60;
                    "已将背屏常亮时长设为 60 秒".into()
                } else if key == "open_alipay_coupons" {
                    "已触发端侧快捷指令：打开支付宝领券中心".into()
                } else if key == "capcut_handoff" {
                    "已触发跨端影像流转：将相册素材移交剪映 AI 成片".into()
                } else if key == "pull_remote_agent_files" {
                    "已同步拉取手机端智能体产出文件到桌面".into()
                } else if key == "fill_otp_one_by_one" {
                    "正在向手机当前输入框输入验证码…".into()
                } else {
                    format!("已执行快捷指令：{key}")
                };

                if self.demo_mode {
                    self.back_screen.stepos_suite.last_action_note = optimistic_note.clone();
                    self.error = false;
                    self.status = optimistic_note.clone();
                    self.tools_feedback = Some((self.tool_page, optimistic_note, false));
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = optimistic_note.clone();
                self.status = optimistic_note;
                self.error = false;
                let request = Instant::now();
                let serial = device.serial.clone();
                self.back_screen.mcp_request = Some(request);
                let key_for_apply = key.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        if is_privacy {
                            let note = stepx_companion::toolbox::set_stepos_privacy_setting(
                                &adb, &device, &key, privacy_target,
                            )?;
                            let fresh = stepx_companion::toolbox::inspect_stepos_ai_suite(&adb, &device)
                                .map_err(|error| anyhow::anyhow!("设置已返回，但最新状态读取失败，请刷新核对：{error:#}"))?;
                            let observed = match key.as_str() {
                                "toggle_delete_protect" => fresh.capability_evidence.delete_protect_enabled,
                                "toggle_global_camera" => fresh.capability_evidence.camera_allowed,
                                "toggle_global_mic" => fresh.capability_evidence.microphone_allowed,
                                "toggle_global_location" => fresh.capability_evidence.location_enabled,
                                _ => fresh.capability_evidence.screen_perception,
                            };
                            anyhow::ensure!(observed == privacy_target,
                                "设置已返回，但最新读取未确认目标状态；请在手机核对后刷新");
                            Ok((note, Some(fresh)))
                        } else {
                            stepx_companion::toolbox::trigger_stepos_mcp_action(
                                &adb, &device, &key, &payload,
                            ).map(|note| (note, None))
                        }
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if is_privacy && me.back_screen.mcp_request != Some(request) {
                        return;
                    }
                    if me.back_screen.mcp_request == Some(request) {
                        me.back_screen.mcp_request = None;
                    }
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    let rollback = |me: &mut Self| {
                        // A failed or interrupted setting receipt is not fresh
                        // evidence; keep the cached data but require a new read.
                        if is_privacy {
                            let evidence = &mut me.back_screen.stepos_suite.capability_evidence;
                            match key_for_apply.as_str() {
                                "toggle_delete_protect" => evidence.delete_protect_enabled = None,
                                "toggle_global_camera" => evidence.camera_allowed = None,
                                "toggle_global_mic" => evidence.microphone_allowed = None,
                                "toggle_global_location" => evidence.location_enabled = None,
                                "toggle_screen_perception" => evidence.screen_perception = None,
                                _ => {}
                            }
                        }
                        match key_for_apply.as_str() {
                            "toggle_dnd" | "set_dnd_on" | "set_dnd_off" => {
                                me.back_screen.stepos_suite.controls.dnd_on = prev_dnd;
                            }
                            "toggle_wifi" => {
                                me.back_screen.stepos_suite.controls.wifi_on = prev_wifi;
                            }
                            "toggle_bluetooth" => {
                                me.back_screen.stepos_suite.controls.bluetooth_on = prev_bt;
                            }
                            "backscreen_toggle_flip_keep" => {
                                me.back_screen.stepos_suite.backscreen_flip_keep_running = prev_flip;
                            }
                            "backscreen_stay_15" | "backscreen_stay_30" | "backscreen_stay_60" => {
                                me.back_screen.stepos_suite.backscreen_stay_on_secs = prev_stay;
                            }
                            _ => {}
                        }
                    };
                    match result {
                        Ok(Ok((note, privacy_report))) => {
                            if let Some(fresh) = privacy_report {
                                let current = &mut me.back_screen.stepos_suite;
                                current.capability_evidence = fresh.capability_evidence;
                                current.global_camera_enabled = fresh.global_camera_enabled;
                                current.global_mic_enabled = fresh.global_mic_enabled;
                                current.global_location_enabled = fresh.global_location_enabled;
                                current.delete_protect_enabled = fresh.delete_protect_enabled;
                                current.screen_perception_ready = fresh.screen_perception_ready;
                                current.gui_accessibility_enabled = fresh.gui_accessibility_enabled;
                                current.notification_listener_enabled = fresh.notification_listener_enabled;
                                current.mcp_permission_granted = fresh.mcp_permission_granted;
                                current.mcp_services = fresh.mcp_services;
                            }
                            if matches!(key_for_apply.as_str(), "toggle_dnd" | "set_dnd_on" | "set_dnd_off") {
                                me.back_screen.stepos_suite.controls.dnd_on =
                                    note.contains("已开启");
                            } else if key_for_apply == "toggle_wifi" {
                                me.back_screen.stepos_suite.controls.wifi_on =
                                    !note.contains("已关闭");
                            } else if key_for_apply == "toggle_bluetooth" {
                                me.back_screen.stepos_suite.controls.bluetooth_on =
                                    !note.contains("已关闭");
                            } else if key_for_apply == "backscreen_toggle_flip_keep" {
                                me.back_screen.stepos_suite.backscreen_flip_keep_running =
                                    note.contains("已开启");
                            } else if key_for_apply == "backscreen_stay_15" {
                                me.back_screen.stepos_suite.backscreen_stay_on_secs = 15;
                            } else if key_for_apply == "backscreen_stay_30" {
                                me.back_screen.stepos_suite.backscreen_stay_on_secs = 30;
                            } else if key_for_apply == "backscreen_stay_60" {
                                me.back_screen.stepos_suite.backscreen_stay_on_secs = 60;
                            } else if let Some(sig_raw) =
                                key_for_apply.strip_prefix("backscreen_sig:")
                            {
                                let stripped = sig_raw.trim_start_matches(|c: char| {
                                    c.is_whitespace()
                                        || (c as u32) >= 0x1F000
                                        || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                                });
                                let sig = if stripped.trim().is_empty() {
                                    "专注中"
                                } else {
                                    stripped.trim()
                                };
                                me.back_screen.stepos_suite.backscreen_signature = sig.to_string();
                                me.back_screen.frame = bs::synthesize_chinese_signature_frame(
                                    sig,
                                    me.back_screen.config.invert,
                                );
                            } else if key_for_apply == "query_express_number" {
                                if let Some((carrier, number)) =
                                    stepx_companion::home_hub::extract_express_from_text_flexible(
                                        &note, true,
                                    )
                                {
                                    me.back_screen.stepos_suite.phone_clipboard =
                                        format!("{carrier} {number}");
                                }
                                me.sync_live_express_parcels();
                            }
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            rollback(me);
                            me.back_screen.stepos_suite.last_action_note =
                                format!("操作未完成：{err}");
                            me.error = true;
                            me.status = format!("操作未完成：{err}");
                            me.tools_feedback = Some((me.tool_page, me.status.clone(), true));
                        }
                        Err(_) => {
                            rollback(me);
                            me.error = true;
                            me.status = "手机操作回执中断，结果未确认；请刷新手机状态".into();
                            me.tools_feedback = Some((me.tool_page, me.status.clone(), true));
                        }
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::CalendarCreatePreset(preset) => {
                let preset_idx = *preset;
                let custom_title = self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string();
                let fallback_title = if custom_title.is_empty() {
                    self.fields[F_OV_TEXT].text.trim().to_string()
                } else {
                    custom_title
                };
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再创建手机日历日程".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = "正在向手机日历写入新日程…".into();
                self.status = "正在向手机日历写入新日程…".into();
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::create_phone_calendar_preset(
                            &adb,
                            &device,
                            preset_idx,
                            &fallback_title,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((_id, events, note))) => {
                            me.fields[F_PHONE_SCHEDULE_TITLE].clear();
                            me.back_screen.stepos_suite.checked = true;
                            me.back_screen.stepos_suite.calendar_events = events;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("创建日程未完成：{err}");
                            me.error = true;
                            me.status = format!("创建日程未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::CalendarDelete(event_id) => {
                let id = *event_id;
                let removed_ev = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == id)
                    .cloned();
                let removed_title = removed_ev.as_ref().map(|e| e.title.clone());
                self.back_screen
                    .stepos_suite
                    .calendar_events
                    .retain(|e| e.id != id);
                if self.back_screen.editing_calendar_event_id == Some(id) {
                    self.back_screen.editing_calendar_event_id = None;
                }
                self.remove_calendar_event_state(id);
                if self.prefs.auto_sync_calendar {
                    rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
                }
                if self.demo_mode {
                    let note = match removed_title {
                        Some(t) => format!("已删除日程：「{t}」"),
                        None => "已移除该日程".into(),
                    };
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                if id <= 0 {
                    let note = match removed_title {
                        Some(t) => format!("已移除本地日程：「{t}」"),
                        None => "已移除该本地日程".into(),
                    };
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    return Ok(());
                }
                self.back_screen.stepos_suite.last_action_note = "正在从手机日历移除日程…".into();
                self.status = "正在从手机日历移除日程…".into();
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        let (mut events, note) =
                            stepx_companion::toolbox::delete_phone_calendar_event(
                                &adb, &device, id,
                            )?;
                        if let Some(ref rev) = removed_ev {
                            let matching_ids: Vec<i64> = events
                                .iter()
                                .filter(|e| {
                                    e.id > 0
                                        && e.title
                                            .trim()
                                            .eq_ignore_ascii_case(rev.title.trim())
                                        && (e.dtstart - rev.dtstart).abs() < 60_000
                                })
                                .map(|e| e.id)
                                .collect();
                            for mid in matching_ids {
                                if let Ok((updated, _)) =
                                    stepx_companion::toolbox::delete_phone_calendar_event(
                                        &adb, &device, mid,
                                    )
                                {
                                    events = updated;
                                }
                            }
                        }
                        Ok((events, note))
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((events, note))) => {
                            me.back_screen.stepos_suite.calendar_events = events;
                            if me.back_screen.editing_calendar_event_id == Some(id) {
                                me.back_screen.editing_calendar_event_id = None;
                            }
                            me.remove_calendar_event_state(id);
                            rewrite_local_calendar_ics(&me.back_screen.stepos_suite.calendar_events);
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("移除日程未完成：{err}");
                            me.error = true;
                            me.status = format!("移除日程未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::AlarmCreatePreset(preset) => {
                let preset_idx = *preset;
                let fallback_label = {
                    let l = self.fields[F_ALARM_LABEL].text.trim().to_string();
                    if !l.is_empty() || self.page == Page::Home {
                        l
                    } else {
                        let sched = self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string();
                        if !sched.is_empty() {
                            sched
                        } else {
                            self.fields[F_OV_TEXT].text.trim().to_string()
                        }
                    }
                };
                use chrono::Timelike;
                let now = chrono::Local::now();
                let (hour, minutes, default_label) = match preset_idx {
                    0 => {
                        let target = now + chrono::Duration::minutes(15);
                        (target.hour(), target.minute(), "15分钟专注提醒")
                    }
                    1 => {
                        let target = now + chrono::Duration::minutes(30);
                        (target.hour(), target.minute(), "30分钟专注提醒")
                    }
                    _ => (8, 0, "晨间早起闹钟"),
                };
                let label = if fallback_label.is_empty() {
                    default_label.to_string()
                } else {
                    fallback_label.clone()
                };
                if self.demo_mode {
                    let next_id = self
                        .back_screen
                        .stepos_suite
                        .alarms
                        .iter()
                        .map(|a| a.id)
                        .max()
                        .unwrap_or(100)
                        + 1;
                    self.back_screen.stepos_suite.alarms.push(
                        stepx_companion::toolbox::PhoneAlarmItem {
                            id: next_id,
                            hour: hour as u8,
                            minutes: minutes as u8,
                            enabled: true,
                            days_of_week: 0,
                            vibrate: true,
                            message: label.clone(),
                        },
                    );
                    self.back_screen.stepos_suite.checked = true;
                    if self.back_screen.stepos_suite_synced_at.is_none() {
                        self.back_screen.stepos_suite_synced_at = Some(std::time::Instant::now());
                    }
                    let note = format!("已添加并开启 {:02}:{:02} 闹钟：「{label}」", hour, minutes);
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再创建手机闹钟".into();
                    self.error = true;
                    return Ok(());
                };
                let next_id = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .map(|a| a.id)
                    .max()
                    .unwrap_or(100)
                    + 1;
                self.back_screen.stepos_suite.alarms.push(
                    stepx_companion::toolbox::PhoneAlarmItem {
                        id: next_id,
                        hour: hour as u8,
                        minutes: minutes as u8,
                        enabled: true,
                        days_of_week: 0,
                        vibrate: true,
                        message: label.clone(),
                    },
                );
                self.back_screen.stepos_suite.checked = true;
                if self.back_screen.stepos_suite_synced_at.is_none() {
                    self.back_screen.stepos_suite_synced_at = Some(std::time::Instant::now());
                }
                let optimistic_note =
                    format!("已添加 {:02}:{:02} 闹钟：「{label}」（正在同步至手机…）", hour, minutes);
                self.back_screen.stepos_suite.last_action_note = optimistic_note.clone();
                self.status = optimistic_note;
                self.error = false;
                let serial = device.serial.clone();
                let device_for_cleanup = device.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::create_phone_alarm_preset(
                            &adb,
                            &device,
                            preset_idx,
                            &fallback_label,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((created_id, mut alarms, note))) => {
                            let was_deleted = !me
                                .back_screen
                                .stepos_suite
                                .alarms
                                .iter()
                                .any(|a| a.id == next_id);
                            me.fields[F_ALARM_LABEL].clear();
                            if me.page != Page::Home {
                                me.fields[F_PHONE_SCHEDULE_TITLE].clear();
                            }
                            me.back_screen.stepos_suite.checked = true;
                            me.back_screen.stepos_suite_synced_at = Some(std::time::Instant::now());
                            if was_deleted && created_id > 0 {
                                alarms.retain(|a| a.id != created_id);
                                me.back_screen.stepos_suite.alarms = alarms;
                                let dev = device_for_cleanup.clone();
                                std::thread::spawn(move || {
                                    if let Ok(adb) = Adb::discover() {
                                        let _ = stepx_companion::toolbox::delete_phone_alarm(
                                            &adb,
                                            &dev,
                                            created_id,
                                        );
                                    }
                                });
                            } else {
                                me.back_screen.stepos_suite.alarms = alarms;
                                me.back_screen.stepos_suite.last_action_note = note.clone();
                                me.error = false;
                                me.status = note.clone();
                                me.tools_feedback = Some((me.tool_page, note, false));
                            }
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("创建闹钟未完成：{err}");
                            me.error = true;
                            me.status = format!("创建闹钟未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::AlarmToggle(alarm_id, enabled) => {
                let id = *alarm_id;
                let target_enabled = *enabled;
                if let Some(alarm) = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter_mut()
                    .find(|a| a.id == id)
                {
                    alarm.enabled = target_enabled;
                }
                if self.demo_mode {
                    let note = if target_enabled {
                        "已开启闹钟".to_string()
                    } else {
                        "已关闭闹钟".to_string()
                    };
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::toggle_phone_alarm(
                            &adb,
                            &device,
                            id,
                            target_enabled,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((alarms, note))) => {
                            me.back_screen.stepos_suite.alarms = alarms;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("切换闹钟未完成：{err}");
                            me.error = true;
                            me.status = format!("切换闹钟未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::AlarmDelete(alarm_id) => {
                let id = *alarm_id;
                let removed_alarm = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .find(|a| a.id == id)
                    .cloned();
                if self.back_screen.editing_alarm_id == Some(id) {
                    self.back_screen.editing_alarm_id = None;
                }
                self.back_screen.stepos_suite.alarms.retain(|a| a.id != id);
                if self.demo_mode {
                    let note = "已移除该闹钟".to_string();
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        let (mut alarms, note) =
                            stepx_companion::toolbox::delete_phone_alarm(&adb, &device, id)?;
                        if let Some(ref ra) = removed_alarm {
                            let matching_ids: Vec<i64> = alarms
                                .iter()
                                .filter(|a| {
                                    a.id > 0
                                        && a.hour == ra.hour
                                        && a.minutes == ra.minutes
                                        && a.message.trim() == ra.message.trim()
                                })
                                .map(|a| a.id)
                                .collect();
                            for mid in matching_ids {
                                if let Ok((updated, _)) =
                                    stepx_companion::toolbox::delete_phone_alarm(&adb, &device, mid)
                                {
                                    alarms = updated;
                                }
                            }
                        }
                        Ok((alarms, note))
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((alarms, note))) => {
                            if me.back_screen.editing_alarm_id == Some(id) {
                                me.back_screen.editing_alarm_id = None;
                            }
                            me.back_screen.stepos_suite.alarms = alarms;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("删除闹钟未完成：{err}");
                            me.error = true;
                            me.status = format!("删除闹钟未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::SetBrightnessPercent(percent) => {
                let target_pct = (*percent).clamp(5, 100);
                if !self.demo_mode && self.selected.is_none() {
                    self.status = "请先连接手机后再调节屏幕亮度".into();
                    self.error = true;
                    return Ok(());
                }
                let prev_pct = self.back_screen.stepos_suite.controls.brightness_percent;
                self.back_screen.stepos_suite.controls.brightness_percent = target_pct;
                let note = format!("已将手机屏幕亮度调节为 {target_pct}%");
                self.back_screen.stepos_suite.last_action_note = note.clone();
                self.status = note;
                self.error = false;
                if self.demo_mode {
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                let serial = device.serial.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::set_phone_brightness_percent(
                            &adb, &device, target_pct,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((pct, note))) => {
                            me.back_screen.stepos_suite.controls.brightness_percent = pct;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.controls.brightness_percent = prev_pct;
                            me.back_screen.stepos_suite.last_action_note =
                                format!("调节亮度未完成：{err}");
                            me.error = true;
                            me.status = format!("调节亮度未完成：{err}");
                        }
                        Err(_) => {
                            me.back_screen.stepos_suite.controls.brightness_percent = prev_pct;
                        }
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::CalendarPrevMonth => {
                let state = &self.back_screen;
                let (year, month) = if state.calendar_month <= 1 {
                    (state.calendar_year - 1, 12)
                } else {
                    (state.calendar_year, state.calendar_month - 1)
                };
                self.set_calendar_focus(year, month, state.selected_calendar_day);
            }
            BackScreenAction::CalendarNextMonth => {
                let state = &self.back_screen;
                let (year, month) = if state.calendar_month >= 12 {
                    (state.calendar_year + 1, 1)
                } else {
                    (state.calendar_year, state.calendar_month + 1)
                };
                self.set_calendar_focus(year, month, state.selected_calendar_day);
            }
            BackScreenAction::CalendarToday => {
                use chrono::Datelike;
                let today = chrono::Local::now().date_naive();
                self.set_calendar_focus(today.year(), today.month(), today.day());
            }
            BackScreenAction::CalendarSelectDate(y, m, d) => {
                self.set_calendar_focus(*y, (*m).clamp(1, 12), *d);
            }
            BackScreenAction::CalendarOpenOccurrence(id, start) => {
                if let Some(event) = self.back_screen.stepos_suite.calendar_events.iter()
                    .find(|event| event.id == *id && event.dtstart == *start) {
                    let (year, month, day) = event.date_ymd();
                    self.set_calendar_focus(year, month, day);
                    self.page = Page::Calendar;
                    self.back_screen.editing_calendar_event_id = None;
                    self.focus = None;
                }
            }
            BackScreenAction::ToggleAutoSyncCalendar => {
                let next = !self.prefs.auto_sync_calendar;
                self.prefs.auto_sync_calendar = next;
                self.preferences.auto_sync_calendar = next;
                let _ = self.preferences.save();
                let note = if next {
                    "已开启电脑系统日历与手机日历自动双向同步"
                } else {
                    "已关闭日历自动双向同步（切换为手动管理）"
                };
                self.status = note.into();
                self.settings.feedback = note.into();
                self.back_screen.stepos_suite.last_action_note = note.into();
                if self.selected.is_some() {
                    self.back_screen.stepos_suite_loading = false;
                    self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
                }
            }
            BackScreenAction::ToggleAutoSyncClipboard => {
                let next = !self.prefs.auto_sync_clipboard;
                self.prefs.auto_sync_clipboard = next;
                self.preferences.auto_sync_clipboard = next;
                let _ = self.preferences.save();
                let note = if next {
                    "已开启跨端剪贴板自动双向共享：电脑与手机复制内容将自动互通"
                } else {
                    "已关闭跨端剪贴板自动双向共享"
                };
                self.status = note.into();
                self.settings.feedback = note.into();
                self.back_screen.stepos_suite.last_action_note = note.into();
            }
            BackScreenAction::CalendarCreateOnSelectedDate(preset_time) => {
                let raw_title = self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string();
                let mut time_str = preset_time
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| self.fields[F_CALENDAR_TIME].text.trim().to_string());
                let title = if raw_title.is_empty() {
                    "桌面协同日程".to_string()
                } else if preset_time.is_none() {
                    if let Some((first_tok, rest)) = raw_title.split_once(char::is_whitespace) {
                        let tok = first_tok.trim();
                        let rest_trimmed = rest.trim();
                        if !rest_trimmed.is_empty()
                            && tok.chars().next().is_some_and(|c| c.is_ascii_digit())
                            && (tok.contains(':') || tok.contains('：'))
                        {
                            time_str = tok.to_string();
                            rest_trimmed.to_string()
                        } else {
                            raw_title
                        }
                    } else {
                        raw_title
                    }
                } else {
                    raw_title
                };
                let clean_time = normalize_calendar_time_input(&time_str);
                let y = self.back_screen.calendar_year;
                let m = self.back_screen.calendar_month;
                let d = self.back_screen.selected_calendar_day;
                let auto_sync = self.prefs.auto_sync_calendar;
                let editing_old_id = self.back_screen.editing_calendar_event_id.take();
                let prev_event = editing_old_id.and_then(|old_id| {
                    self.back_screen
                        .stepos_suite
                        .calendar_events
                        .iter()
                        .find(|e| e.id == old_id)
                        .cloned()
                });
                let preserved_all_day = if clean_time.contains("全天") {
                    true
                } else if clean_time.is_empty() {
                    prev_event.as_ref().map(|e| e.all_day).unwrap_or(false)
                } else {
                    false
                };
                let (dtstart, mut dtend) = if preserved_all_day {
                    let (start_0, _) =
                        stepx_companion::toolbox::parse_event_time_range(y, m, d, "00:00")?;
                    let dur = prev_event
                        .as_ref()
                        .map(|p| (p.dtend - p.dtstart).max(86_400_000))
                        .unwrap_or(86_400_000);
                    (start_0, start_0 + dur)
                } else {
                    stepx_companion::toolbox::parse_event_time_range(y, m, d, &clean_time)?
                };
                if let Some(prev) = &prev_event {
                    let has_explicit_range = clean_time.contains('-');
                    let prev_dur = prev.dtend - prev.dtstart;
                    if !preserved_all_day
                        && prev_dur > 0
                        && (!has_explicit_range || clean_time == prev.hm_label())
                    {
                        dtend = dtstart + prev_dur;
                    }
                }
                let preserved_location = prev_event
                    .as_ref()
                    .map(|e| e.location.clone())
                    .unwrap_or_default();
                let preserved_description = prev_event
                    .as_ref()
                    .map(|e| e.description.clone())
                    .filter(|desc| !desc.trim().is_empty())
                    .unwrap_or_else(|| "由 StepX 桌面端直接同步创建".into());
                if let Some(old_id) = editing_old_id {
                    self.back_screen
                        .stepos_suite
                        .calendar_events
                        .retain(|e| e.id != old_id);
                }
                let local_id = editing_old_id.unwrap_or_else(|| {
                    self.back_screen
                        .stepos_suite
                        .calendar_events
                        .iter()
                        .map(|e| e.id)
                        .max()
                        .unwrap_or(1000)
                        + 1
                });
                let local_ev = stepx_companion::toolbox::PhoneCalendarEvent {
                    recurring: Some(false),
                    id: local_id,
                    title: title.clone(),
                    dtstart,
                    dtend,
                    all_day: preserved_all_day,
                    location: preserved_location.clone(),
                    description: preserved_description.clone(),
                    source: if auto_sync {
                        "双端已同步".into()
                    } else {
                        "电脑日历".into()
                    },
                };
                self.back_screen.stepos_suite.calendar_events.push(local_ev);
                self.back_screen
                    .stepos_suite
                    .calendar_events
                    .sort_by_key(|e| e.dtstart);
                self.back_screen.stepos_suite.checked = true;
                if self.back_screen.stepos_suite_synced_at.is_none() {
                    self.back_screen.stepos_suite_synced_at = Some(std::time::Instant::now());
                }
                if auto_sync {
                    rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
                }
                self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                if self.page == Page::Home {
                    self.fields[F_CALENDAR_TIME].set("15:00");
                } else {
                    self.fields[F_CALENDAR_TIME].clear();
                }
                if self.demo_mode {
                    let msg = if editing_old_id.is_some() {
                        format!("已保存日程修改：「{title}」（{y}-{m:02}-{d:02}）")
                    } else {
                        format!("已在 {y}-{m:02}-{d:02} 添加日程：「{title}」")
                    };
                    self.apply(Outcome::StepOsCalendarUpdated(
                        self.back_screen.stepos_suite.calendar_events.clone(),
                        msg,
                    ));
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = if editing_old_id.is_some() {
                    "正在保存并更新手机日历日程…".into()
                } else {
                    format!("已在 {y}-{m:02}-{d:02} 添加日程：「{title}」（正在同步至手机…）")
                };
                self.status = self.back_screen.stepos_suite.last_action_note.clone();
                let serial = device.serial.clone();
                let device_for_cleanup = device.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        if let Some(old_id) = editing_old_id {
                            let _ = stepx_companion::toolbox::delete_phone_calendar_event(
                                &adb, &device, old_id,
                            );
                        }
                        create_phone_calendar_event_preserving_details(
                            &adb,
                            &device,
                            &title,
                            dtstart,
                            dtend,
                            preserved_all_day,
                            &preserved_location,
                            &preserved_description,
                            auto_sync,
                            y,
                            m,
                            d,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((new_id, mut events, msg))) => {
                            let was_deleted = editing_old_id.is_none()
                                && !me
                                    .back_screen
                                    .stepos_suite
                                    .calendar_events
                                    .iter()
                                    .any(|e| e.id == local_id);
                            if was_deleted && new_id > 0 {
                                events.retain(|e| e.id != new_id);
                                me.back_screen.stepos_suite.calendar_events = events;
                                if me.prefs.auto_sync_calendar {
                                    rewrite_local_calendar_ics(
                                        &me.back_screen.stepos_suite.calendar_events,
                                    );
                                }
                                let dev = device_for_cleanup.clone();
                                std::thread::spawn(move || {
                                    if let Ok(adb) = Adb::discover() {
                                        let _ =
                                            stepx_companion::toolbox::delete_phone_calendar_event(
                                                &adb, &dev, new_id,
                                            );
                                    }
                                });
                            } else {
                                if let Some(old_id) = editing_old_id {
                                    if old_id != new_id && new_id > 0 {
                                        me.migrate_calendar_event_state(old_id, new_id);
                                    }
                                }
                                me.apply(Outcome::StepOsCalendarUpdated(events, msg));
                            }
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("保存日程未完成：{err}");
                            me.error = true;
                            me.status = format!("保存日程未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::AlarmSelectQuickTime(t) => {
                self.fields[F_ALARM_TIME].set(*t);
                self.status = format!("已选择闹钟时间：{t}");
            }
            BackScreenAction::AlarmCreateCustom => {
                let time_str = self.fields[F_ALARM_TIME].text.trim().to_string();
                let label = {
                    let l = self.fields[F_ALARM_LABEL].text.trim().to_string();
                    if l.is_empty() && self.page != Page::Home {
                        self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string()
                    } else {
                        l
                    }
                };
                let clean_label = if label.is_empty() {
                    "StepX 桌面提醒".to_string()
                } else {
                    label.clone()
                };
                let (hour, minutes) = stepx_companion::toolbox::parse_alarm_time_str(&time_str)?;
                let editing_old_id = self.back_screen.editing_alarm_id.take();
                let prev_alarm = editing_old_id.and_then(|edit_id| {
                    self.back_screen
                        .stepos_suite
                        .alarms
                        .iter()
                        .find(|a| a.id == edit_id)
                        .cloned()
                });
                let preserved_days_of_week = prev_alarm.as_ref().map(|a| a.days_of_week).unwrap_or(0);
                let preserved_vibrate = prev_alarm.as_ref().map(|a| a.vibrate).unwrap_or(true);
                let local_alarm_id = if let Some(edit_id) = editing_old_id {
                    if let Some(existing) = self
                        .back_screen
                        .stepos_suite
                        .alarms
                        .iter_mut()
                        .find(|a| a.id == edit_id)
                    {
                        existing.hour = hour;
                        existing.minutes = minutes;
                        existing.message = clean_label.clone();
                        existing.enabled = true;
                        existing.days_of_week = preserved_days_of_week;
                        existing.vibrate = preserved_vibrate;
                    } else {
                        self.back_screen.stepos_suite.alarms.push(
                            stepx_companion::toolbox::PhoneAlarmItem {
                                id: edit_id,
                                hour,
                                minutes,
                                enabled: true,
                                days_of_week: preserved_days_of_week,
                                vibrate: preserved_vibrate,
                                message: clean_label.clone(),
                            },
                        );
                    }
                    edit_id
                } else {
                    let next_id = self
                        .back_screen
                        .stepos_suite
                        .alarms
                        .iter()
                        .map(|a| a.id)
                        .max()
                        .unwrap_or(100)
                        + 1;
                    self.back_screen.stepos_suite.alarms.push(
                        stepx_companion::toolbox::PhoneAlarmItem {
                            id: next_id,
                            hour,
                            minutes,
                            enabled: true,
                            days_of_week: 0,
                            vibrate: true,
                            message: clean_label.clone(),
                        },
                    );
                    next_id
                };
                self.back_screen.stepos_suite.checked = true;
                if self.back_screen.stepos_suite_synced_at.is_none() {
                    self.back_screen.stepos_suite_synced_at = Some(std::time::Instant::now());
                }
                if self.demo_mode {
                    self.fields[F_ALARM_LABEL].clear();
                    if self.page != Page::Home {
                        self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                    }
                    let msg = if editing_old_id.is_some() {
                        format!(
                            "已保存闹钟修改：{:02}:{:02}「{clean_label}」",
                            hour, minutes
                        )
                    } else {
                        format!(
                            "已添加并开启 {:02}:{:02} 闹钟：「{clean_label}」",
                            hour, minutes
                        )
                    };
                    self.apply(Outcome::StepOsAlarmsUpdated(
                        self.back_screen.stepos_suite.alarms.clone(),
                        msg,
                    ));
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = if editing_old_id.is_some() {
                    "正在保存并更新手机闹钟…".into()
                } else {
                    "正在向手机时钟写入闹钟…".into()
                };
                self.status = self.back_screen.stepos_suite.last_action_note.clone();
                let serial = device.serial.clone();
                let device_for_cleanup = device.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        if let Some(old_id) = editing_old_id {
                            let _ =
                                stepx_companion::toolbox::delete_phone_alarm(&adb, &device, old_id);
                        }
                        create_phone_alarm_preserving_details(
                            &adb,
                            &device,
                            hour,
                            minutes,
                            &clean_label,
                            preserved_days_of_week,
                            preserved_vibrate,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    if me.selected.as_ref().map(|d| d.serial.as_str()) != Some(serial.as_str()) {
                        return;
                    }
                    match result {
                        Ok(Ok((created_id, mut alarms, msg))) => {
                            let was_deleted = editing_old_id.is_none()
                                && !me
                                    .back_screen
                                    .stepos_suite
                                    .alarms
                                    .iter()
                                    .any(|a| a.id == local_alarm_id);
                            me.fields[F_ALARM_LABEL].clear();
                            if me.page != Page::Home {
                                me.fields[F_PHONE_SCHEDULE_TITLE].clear();
                            }
                            if was_deleted && created_id > 0 {
                                alarms.retain(|a| a.id != created_id);
                                me.back_screen.stepos_suite.alarms = alarms;
                                let dev = device_for_cleanup.clone();
                                std::thread::spawn(move || {
                                    if let Ok(adb) = Adb::discover() {
                                        let _ = stepx_companion::toolbox::delete_phone_alarm(
                                            &adb,
                                            &dev,
                                            created_id,
                                        );
                                    }
                                });
                            } else {
                                me.apply(Outcome::StepOsAlarmsUpdated(alarms, msg));
                            }
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("创建闹钟未完成：{err}");
                            me.error = true;
                            me.status = format!("创建闹钟未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GalleryRefresh => {
                if self.back_screen.gallery_loading {
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    let (_, mut exp_items) =
                        stepx_companion::toolbox::load_expansion_gallery_items();
                    exp_items.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
                    self.back_screen.gallery_checked = true;
                    if !exp_items.is_empty() {
                        let count = exp_items.len();
                        self.apply(Outcome::PhoneGallery(
                            exp_items,
                            format!(
                                "已从电脑扩容库加载 {count} 张归档照片（连接手机后可同步手机端相册）"
                            ),
                        ));
                    } else {
                        self.status = "请先连接手机后再读取手机相册".into();
                    }
                    return Ok(());
                };
                self.back_screen.gallery_loading = true;
                self.status = "正在读取手机相册与截图缩略图…".into();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::list_phone_gallery(&adb, &device)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                    me.back_screen.gallery_loading = false;
                    me.back_screen.gallery_checked = true;
                    match result {
                        Ok(Ok(items)) => {
                            let msg = format!("已加载手机相册 {} 张照片", items.len());
                            me.apply(Outcome::PhoneGallery(items, msg));
                        }
                        Ok(Err(err)) => {
                            me.error = true;
                            me.status = format!("读取手机相册未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GalleryCategory(cat) => {
                if self.back_screen.gallery_category != *cat {
                    self.back_screen.gallery_category = *cat;
                    self.back_screen.gallery_selected = 0;
                    self.trigger_subtab_switch();
                }
            }
            BackScreenAction::GallerySelect(idx) => {
                self.back_screen.gallery_selected = *idx;
                self.sync_gallery_image_previews();
                if let Some(item) = self.back_screen.gallery_items.get(*idx) {
                    let needs_pull = item
                        .local_thumb_path
                        .as_deref()
                        .is_none_or(|p| !std::path::Path::new(p).is_file());
                    if needs_pull {
                        if let Some(device) = self.selected.clone() {
                            let remote = item.remote_path.clone();
                            let name = item.name.clone();
                            let sel_idx = *idx;
                            let (tx, rx) = async_channel::bounded(1);
                            std::thread::spawn(move || {
                                let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                                let _ = std::fs::create_dir_all(&cache_dir);
                                let safe_name: String = name
                                    .chars()
                                    .map(|c| {
                                        if c.is_ascii_alphanumeric()
                                            || c == '.'
                                            || c == '-'
                                            || c == '_'
                                        {
                                            c
                                        } else {
                                            '_'
                                        }
                                    })
                                    .collect();
                                let local_file =
                                    cache_dir.join(format!("sel_{sel_idx}_{safe_name}"));
                                let res = Adb::discover().and_then(|adb| {
                                    storage::pull(&adb, &device, &remote, &local_file)?;
                                    Ok(local_file)
                                });
                                let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                            });
                            ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                                if let Ok(Ok(local_file)) = result {
                                    let local_str = local_file.to_string_lossy().into_owned();
                                    if let Some(slot) =
                                        me.back_screen.gallery_items.get_mut(sel_idx)
                                    {
                                        slot.local_thumb_path = Some(local_str.clone());
                                    }
                                    me.work.insert_image_preview(local_str, local_file);
                                    ctx.notify();
                                }
                            });
                        }
                    }
                }
            }
            BackScreenAction::GalleryZoomIn => {
                let next = match self.back_screen.gallery_zoom_pct {
                    0..=49 => 50,
                    50..=99 => 100,
                    100..=149 => 150,
                    150..=199 => 200,
                    200..=299 => 300,
                    _ => 400,
                };
                self.back_screen.gallery_zoom_pct = next;
                self.status = format!("照片预览已放大至 {next}%");
                self.error = false;
            }
            BackScreenAction::GalleryZoomOut => {
                let next = match self.back_screen.gallery_zoom_pct {
                    301.. => 300,
                    201..=300 => 200,
                    151..=200 => 150,
                    101..=150 => 100,
                    _ => 50,
                };
                self.back_screen.gallery_zoom_pct = next;
                self.status = format!("照片预览已缩小至 {next}%");
                self.error = false;
            }
            BackScreenAction::GalleryZoomSet(pct) => {
                let next = (*pct).clamp(50, 400);
                self.back_screen.gallery_zoom_pct = next;
                self.status = format!("已切换照片预览缩放比例：{next}%");
                self.error = false;
            }
            BackScreenAction::GalleryToggleLightbox => {
                self.back_screen.gallery_lightbox = !self.back_screen.gallery_lightbox;
                self.trigger_subtab_switch();
                self.sync_gallery_image_previews();
                let sel = self.back_screen.gallery_selected;
                let _ = self.do_backscreen_action(&BackScreenAction::GallerySelect(sel), ctx);
                self.status = if self.back_screen.gallery_lightbox {
                    format!(
                        "已展开顶部大图暗房检视（缩放 {}%，支持旋转与翻页）",
                        self.back_screen.gallery_zoom_pct
                    )
                } else {
                    "已收起顶部大图检视，返回分栏相册视图".into()
                };
                self.error = false;
            }
            BackScreenAction::GalleryPrevPhoto | BackScreenAction::GalleryNextPhoto => {
                let indices = self.filtered_gallery_indices();
                if indices.is_empty() {
                    self.status = "当前筛选下暂无可切换的照片".into();
                    return Ok(());
                }
                let cur = self.back_screen.gallery_selected;
                let pos = indices.iter().position(|&i| i == cur).unwrap_or(0);
                let next_pos = if matches!(action, BackScreenAction::GalleryPrevPhoto) {
                    if pos == 0 {
                        indices.len() - 1
                    } else {
                        pos - 1
                    }
                } else {
                    (pos + 1) % indices.len()
                };
                let next_idx = indices[next_pos];
                self.do_backscreen_action(&BackScreenAction::GallerySelect(next_idx), ctx)?;
                if let Some(item) = self.back_screen.gallery_items.get(next_idx) {
                    self.status = format!(
                        "已切换至第 {}/{} 张照片：{}",
                        next_pos + 1,
                        indices.len(),
                        item.name
                    );
                    self.error = false;
                }
            }
            BackScreenAction::GalleryRotateRight(remote_path) => {
                let prev_deg = self
                    .back_screen
                    .gallery_rotations
                    .get(remote_path)
                    .copied()
                    .unwrap_or(0);
                let next_deg = (prev_deg + 90) % 360;
                self.back_screen
                    .insert_gallery_rotation(remote_path.clone(), next_deg);
                if let Some(item_idx) = self
                    .back_screen
                    .gallery_items
                    .iter()
                    .position(|i| &i.remote_path == remote_path)
                {
                    let current_thumb = self.back_screen.gallery_items[item_idx]
                        .local_thumb_path
                        .clone()
                        .filter(|p| std::path::Path::new(p).is_file());
                    if let Some(thumb_str) = current_thumb {
                        let thumb_path = PathBuf::from(&thumb_str);
                        let img = image::open(&thumb_path)
                            .map_err(|e| anyhow::anyhow!("读取本地预览图进行旋转失败：{e}"))?;
                        let rotated = img.rotate90();
                        let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let rot_path = cache_dir.join(format!("rot_{}_{}.png", item_idx, next_deg));
                        rotated
                            .save(&rot_path)
                            .map_err(|e| anyhow::anyhow!("保存旋转后的预览图失败：{e}"))?;
                        let rot_str = rot_path.to_string_lossy().into_owned();
                        let remote_path_ref = std::path::Path::new(remote_path);
                        if remote_path_ref.is_file() {
                            save_image_matching_path_format(&rotated, remote_path_ref)
                                .map_err(|e| anyhow::anyhow!("写回旋转图片到原文件失败：{e}"))?;
                        } else if !self.demo_mode && remote_path.starts_with('/') {
                            if let Some(device) = self.selected.clone() {
                                let remote_for_push = remote_path.clone();
                                let ext = remote_path_ref
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("jpg");
                                let local_for_push = cache_dir
                                    .join(format!("rot_push_{}_{}.{}", item_idx, next_deg, ext));
                                let _ = save_image_matching_path_format(&rotated, &local_for_push);
                                std::thread::spawn(move || {
                                    if let Ok(adb) = Adb::discover() {
                                        if let Ok(transport) = device.resolve_active_transport(&adb)
                                        {
                                            let _ = adb.transfer(
                                                &transport,
                                                "push",
                                                &local_for_push,
                                                std::path::Path::new(&remote_for_push),
                                            );
                                            let _ = adb.shell(
                                                &transport,
                                                &[
                                                    "am",
                                                    "broadcast",
                                                    "-a",
                                                    "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                                                    "-d",
                                                    &format!("file://{remote_for_push}"),
                                                ],
                                            );
                                        }
                                    }
                                });
                            }
                        }
                        self.back_screen.gallery_items[item_idx].local_thumb_path =
                            Some(rot_str.clone());
                        self.work.insert_image_preview(rot_str, rot_path);
                        self.status = format!(
                            "已顺时针旋转 90°（当前 {}°）：{}",
                            next_deg, self.back_screen.gallery_items[item_idx].name
                        );
                        self.error = false;
                    } else if let Some(device) = self.selected.clone() {
                        let remote = remote_path.clone();
                        let name = self.back_screen.gallery_items[item_idx].name.clone();
                        self.status = format!("正在从手机拉取并旋转 {name} ({next_deg}°)…");
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                            let _ = std::fs::create_dir_all(&cache_dir);
                            let safe_name: String = name
                                .chars()
                                .map(|c| {
                                    if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'
                                    {
                                        c
                                    } else {
                                        '_'
                                    }
                                })
                                .collect();
                            let raw_file = cache_dir.join(format!("sel_{item_idx}_{safe_name}"));
                            let res = Adb::discover().and_then(|adb| {
                                storage::pull(&adb, &device, &remote, &raw_file)?;
                                let img = image::open(&raw_file)?;
                                let rotated = rotate_dynamic_image_by_deg(img, next_deg);
                                let rot_file = cache_dir
                                    .join(format!("sel_{item_idx}_{safe_name}_rot{next_deg}.png"));
                                rotated.save(&rot_file)?;
                                save_image_matching_path_format(&rotated, &raw_file)?;
                                if let Ok(transport) = device.resolve_active_transport(&adb) {
                                    let _ = adb.transfer(
                                        &transport,
                                        "push",
                                        &raw_file,
                                        std::path::Path::new(&remote),
                                    );
                                    let _ = adb.shell(
                                        &transport,
                                        &[
                                            "am",
                                            "broadcast",
                                            "-a",
                                            "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                                            "-d",
                                            &format!("file://{remote}"),
                                        ],
                                    );
                                }
                                Ok(rot_file)
                            });
                            let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                        });
                        ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                            match result {
                                Ok(Ok(rot_file)) => {
                                    let rot_str = rot_file.to_string_lossy().into_owned();
                                    if let Some(slot) =
                                        me.back_screen.gallery_items.get_mut(item_idx)
                                    {
                                        slot.local_thumb_path = Some(rot_str.clone());
                                    }
                                    me.work.insert_image_preview(rot_str, rot_file);
                                    me.status = format!("已完成照片旋转（{next_deg}°）");
                                    me.error = false;
                                }
                                Ok(Err(err)) => {
                                    me.error = true;
                                    me.status = format!("旋转照片失败：{err}");
                                }
                                Err(_) => {}
                            }
                            ctx.notify();
                        });
                    } else {
                        self.status = format!("已记录旋转角度 {next_deg}°（连接手机后生效）");
                        self.error = false;
                    }
                }
            }
            BackScreenAction::GallerySetGridCols(cols) => {
                let c = (*cols).clamp(2, 4);
                if self.back_screen.gallery_grid_cols != c {
                    self.back_screen.gallery_grid_cols = c;
                    self.trigger_subtab_switch();
                }
                let label = match c {
                    2 => "2列大图",
                    4 => "4列紧凑",
                    _ => "3列标准",
                };
                self.status = format!("已切换相册缩略图网格：{label}");
                self.error = false;
            }
            BackScreenAction::GalleryCopyPath(text) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()));
                self.status = format!("已复制路径到剪贴板：{text}");
                self.error = false;
            }
            BackScreenAction::PresetFilter(filter) => {
                if self.back_screen.preset_filter != *filter {
                    self.back_screen.preset_filter = *filter;
                    self.trigger_subtab_switch();
                }
            }
            BackScreenAction::GalleryCapturePhoneScreenshot => {
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再截取手机画面".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.gallery_loading = true;
                self.status = "正在截取手机当前画面并存入相册…".into();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::capture_phone_screenshot_to_gallery(&adb, &device)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                    me.back_screen.gallery_loading = false;
                    me.back_screen.gallery_checked = true;
                    match result {
                        Ok(Ok((items, note))) => {
                            me.back_screen.gallery_selected = 0;
                            me.apply(Outcome::PhoneGallery(items, note));
                        }
                        Ok(Err(err)) => {
                            me.error = true;
                            me.status = format!("截取手机画面失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GalleryDelete(path) => {
                let remote_path = path.clone();
                if self
                    .back_screen
                    .gallery_items
                    .iter()
                    .any(|i| i.remote_path == remote_path && i.category == "expansion")
                {
                    self.page = Page::Expansion;
                    self.error = false;
                    self.status =
                        "该照片已归档至电脑扩容库，已为您跳转至「电脑扩容」进行管理或还原".into();
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再删除相册图片".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.gallery_loading = true;
                self.status = "正在从手机相册删除图片…".into();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::delete_phone_gallery_item(
                            &adb,
                            &device,
                            &remote_path,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                    me.back_screen.gallery_loading = false;
                    match result {
                        Ok(Ok(items)) => {
                            me.apply(Outcome::PhoneGallery(
                                items,
                                "已从手机相册删除该图片".into(),
                            ));
                        }
                        Ok(Err(err)) => {
                            me.error = true;
                            me.status = format!("删除相册图片未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GallerySaveToDesktop(path) => {
                let remote_path = path.clone();
                let rot_deg = self
                    .back_screen
                    .gallery_rotations
                    .get(&remote_path)
                    .copied()
                    .unwrap_or(0);
                let matched_item = self
                    .back_screen
                    .gallery_items
                    .iter()
                    .find(|i| i.remote_path == remote_path);
                let is_expansion = matched_item.is_some_and(|i| i.category == "expansion");
                let cached_local = matched_item
                    .and_then(|i| i.local_thumb_path.as_deref())
                    .map(PathBuf::from)
                    .filter(|p| p.is_file());
                let file_name = matched_item
                    .map(|i| i.name.trim())
                    .filter(|n| !n.is_empty())
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        std::path::Path::new(&remote_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| "phone_photo.jpg".to_string());
                let base_dir = if let Some(dir) = self
                    .settings
                    .value
                    .download_directory
                    .as_ref()
                    .filter(|d| !d.as_os_str().is_empty())
                {
                    dir.clone()
                } else {
                    let home = std::env::var_os("HOME")
                        .or_else(|| std::env::var_os("USERPROFILE"))
                        .map(PathBuf::from)
                        .unwrap_or_else(std::env::temp_dir);
                    let pics = home.join("Pictures");
                    if pics.is_dir() {
                        pics
                    } else {
                        home.join("Downloads")
                    }
                };
                if let Some(local_src) =
                    cached_local.filter(|_| is_expansion || rot_deg > 0 || self.selected.is_none())
                {
                    let _ = std::fs::create_dir_all(&base_dir);
                    let out_name = if rot_deg > 0 {
                        let stem = std::path::Path::new(&file_name)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("photo");
                        format!("{stem}_rot{rot_deg}.png")
                    } else {
                        file_name
                    };
                    let dest = base_dir.join(&out_name);
                    match std::fs::copy(&local_src, &dest) {
                        Ok(_) => {
                            self.error = false;
                            self.status = format!("已保存照片到电脑：{}", dest.display());
                        }
                        Err(e) => {
                            self.error = true;
                            self.status = format!("保存照片失败：{e}");
                        }
                    }
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再保存图片到电脑".into();
                    self.error = true;
                    return Ok(());
                };
                let dest = base_dir.join(&file_name);
                self.status = format!("正在将 {file_name} 保存到电脑…");
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = std::fs::create_dir_all(&base_dir);
                    let res = Adb::discover().and_then(|adb| {
                        storage::pull(&adb, &device, &remote_path, &dest)?;
                        Ok(dest)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok(saved_path)) => {
                            me.error = false;
                            me.status = format!("已保存手机照片到电脑：{}", saved_path.display());
                        }
                        Ok(Err(err)) => {
                            me.error = true;
                            me.status = format!("保存照片到电脑失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GalleryOpenOnDesktop(path) => {
                let remote_path = path.clone();
                let cached_local = self
                    .back_screen
                    .gallery_items
                    .iter()
                    .find(|i| i.remote_path == remote_path)
                    .and_then(|i| i.local_thumb_path.as_deref())
                    .map(PathBuf::from)
                    .filter(|p| p.is_file());
                if let Some(local_path) = cached_local {
                    match work_state::open_work_path(&local_path) {
                        Ok(()) => {
                            self.error = false;
                            self.status = format!("已在电脑打开预览：{}", local_path.display());
                        }
                        Err(e) => {
                            self.error = true;
                            self.status = format!("打开图片预览失败：{e:#}");
                        }
                    }
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再打开图片预览".into();
                    self.error = true;
                    return Ok(());
                };
                let file_name = std::path::Path::new(&remote_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("preview.jpg")
                    .to_string();
                self.status = format!("正在拉取 {file_name} 并在电脑打开…");
                let (tx, rx) = async_channel::bounded(1);
                let remote_for_cache = remote_path.clone();
                std::thread::spawn(move || {
                    let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                    let _ = std::fs::create_dir_all(&cache_dir);
                    let local_path = cache_dir.join(&file_name);
                    let res = Adb::discover().and_then(|adb| {
                        storage::pull(&adb, &device, &remote_path, &local_path)?;
                        work_state::open_work_path(&local_path)?;
                        Ok(local_path)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok(local_path)) => {
                            if let Some(item) = me
                                .back_screen
                                .gallery_items
                                .iter_mut()
                                .find(|i| i.remote_path == remote_for_cache)
                            {
                                item.local_thumb_path =
                                    Some(local_path.to_string_lossy().to_string());
                            }
                            me.error = false;
                            me.status = format!("已在电脑打开预览：{}", local_path.display());
                        }
                        Ok(Err(err)) => {
                            me.error = true;
                            me.status = format!("打开图片预览失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GallerySendToAmoo(path) => {
                let remote_path = path.clone();
                let matched_item = self
                    .back_screen
                    .gallery_items
                    .iter()
                    .find(|i| i.remote_path == remote_path);
                let real_name = matched_item
                    .map(|i| i.name.trim())
                    .filter(|n| !n.is_empty())
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        std::path::Path::new(&remote_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| "gallery_photo.jpg".to_string());
                let cached_local = matched_item
                    .and_then(|i| i.local_thumb_path.as_deref())
                    .map(PathBuf::from)
                    .filter(|p| p.is_file());
                if let Some(local_path) = cached_local {
                    let attach_path = if local_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|n| n == real_name)
                    {
                        local_path
                    } else {
                        let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let staged = cache_dir.join(&real_name);
                        if std::fs::copy(&local_path, &staged).is_ok() {
                            staged
                        } else {
                            local_path
                        }
                    };
                    let _ = self.add_amoo_paths(&[attach_path]);
                    self.page = Page::Amoo;
                    if self.fields[F_AMOO].text.trim().is_empty() {
                        self.fields[F_AMOO].set("请帮我分析这张来自手机相册的图片");
                    }
                    self.focus = Some(F_AMOO);
                    self.status = format!("已将照片「{real_name}」添入 Amoo 对话附件");
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再将图片发送给 Amoo".into();
                    self.error = true;
                    return Ok(());
                };
                let file_name = std::path::Path::new(&remote_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("gallery_photo.jpg")
                    .to_string();
                self.status = format!("正在拉取 {file_name} 并带入 Amoo 对话…");
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                    let _ = std::fs::create_dir_all(&cache_dir);
                    let local_path = cache_dir.join(&file_name);
                    let res = Adb::discover().and_then(|adb| {
                        storage::pull(&adb, &device, &remote_path, &local_path)?;
                        Ok(local_path)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok(local_path)) => {
                            let _ = me.add_amoo_paths(&[local_path]);
                            me.page = Page::Amoo;
                            if me.fields[F_AMOO].text.trim().is_empty() {
                                me.fields[F_AMOO].set("请帮我分析这张来自手机相册的图片");
                            }
                            me.focus = Some(F_AMOO);
                            me.error = false;
                            me.status = "已将手机相册图片添入 Amoo 对话附件".into();
                        }
                        Ok(Err(err)) => {
                            me.error = true;
                            me.status = format!("带入 Amoo 对话失败：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GalleryImportFromDesktop => {
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再导入电脑图片".into();
                    self.error = true;
                    return Ok(());
                };
                self.choose_paths(
                    stepx_companion::file_dialog::FileDialog::new()
                        .add_filter("图片", &["png", "jpg", "jpeg", "webp"]),
                    DialogKind::OpenFiles,
                    move |app, local_paths, ctx| {
                        app.ensure_file_dialog_device(&Some(device.serial.clone()))?;
                        anyhow::ensure!(
                            !app.back_screen.gallery_loading,
                            "相册正在更新，请完成后重新导入"
                        );
                        let count = local_paths.len();
                        let first_name = local_paths[0]
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("image.png")
                            .to_string();
                        let summary = if count == 1 {
                            first_name
                        } else {
                            format!("{first_name} 等 {count} 张图片")
                        };
                        app.back_screen.gallery_loading = true;
                        app.status = format!("正在导入 {summary} 到手机相册 (/sdcard/Pictures)…");
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().and_then(|adb| {
                                let transport = device.resolve_active_transport(&adb)?;
                                for local_path in &local_paths {
                                    storage::push(&adb, &device, local_path, "/sdcard/Pictures")?;
                                    if let Some(fname) = local_path.file_name().and_then(|n| n.to_str()) {
                                        let _ = adb.shell(
                                            &transport,
                                            &[
                                                "am",
                                                "broadcast",
                                                "-a",
                                                "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                                                "-d",
                                                &format!("file:///sdcard/Pictures/{fname}"),
                                            ],
                                        );
                                    }
                                }
                                let items = stepx_companion::toolbox::list_phone_gallery(&adb, &device)?;
                                Ok((items, summary))
                            });
                            let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                        });
                        ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                            me.back_screen.gallery_loading = false;
                            me.back_screen.gallery_checked = true;
                            match result {
                                Ok(Ok((items, summary))) => {
                                    me.back_screen.gallery_selected = 0;
                                    me.apply(Outcome::PhoneGallery(
                                        items,
                                        format!("已将电脑图片 {summary} 导入手机相册"),
                                    ));
                                }
                                Ok(Err(err)) => {
                                    me.error = true;
                                    me.status = format!("导入图片到手机相册失败：{err}");
                                }
                                Err(_) => {}
                            }
                            ctx.notify();
                        });

                        Ok(())
                    },
                );
            }
            BackScreenAction::McpQuickOpenCalendar => {
                self.handle_action(&Action::Page(Page::Calendar), ctx);
            }
            BackScreenAction::McpQuickOpenAlarm => {
                self.handle_action(&Action::Page(Page::Alarms), ctx);
            }
            BackScreenAction::McpToggleDnd => {
                self.do_backscreen_action(
                    &BackScreenAction::TriggerMcpQuickAction("toggle_dnd".into()),
                    ctx,
                )?;
            }
            BackScreenAction::ApplyPreset(preset) => {
                self.do_backscreen_action(&BackScreenAction::SelectPreset(*preset), ctx)?;
                self.do_backscreen_action(&BackScreenAction::Play, ctx)?;
            }
            BackScreenAction::CalendarToggleDone(event_id) => {
                self.ensure_calendar_state_scope();
                anyhow::ensure!(self.back_screen.calendar_state_scope.is_some(), "请连接原手机后更新日程状态");
                let now_ms = chrono::Local::now().timestamp_millis();
                if !self.back_screen.completed_calendar_events.remove(event_id) {
                    self.back_screen.completed_calendar_events.insert(*event_id);
                    let ev_title = self
                        .back_screen
                        .stepos_suite
                        .calendar_events
                        .iter()
                        .find(|e| e.id == *event_id)
                        .map(|e| e.title.clone())
                        .unwrap_or_else(|| format!("日程 #{event_id}"));
                    let spec = stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                        format!("cal:{event_id}"),
                        stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                        ev_title,
                    )
                    .with_aliases([
                        format!("schedule:{event_id}"),
                        format!("conflict:conflict-cal-{event_id}"),
                    ]);
                    self.reminder_governance.manual_expire(spec, now_ms);
                    self.status = "已将日程标记为已完成".into();
                } else {
                    self.reminder_governance
                        .restore_reminder(&format!("cal:{event_id}"), now_ms);
                    self.reminder_governance
                        .restore_reminder(&format!("schedule:{event_id}"), now_ms);
                    self.status = "已取消日程完成标记".into();
                }
                self.back_screen.save_completed_calendar_events();
                self.sync_calendar_event_state_to_description(*event_id);
                self.error = false;
            }
            BackScreenAction::CalendarEditEvent(event_id) => {
                if let Some(ev) = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == *event_id)
                    .cloned()
                {
                    let (y, m, d) = ev.date_ymd();
                    self.back_screen.calendar_year = y;
                    self.back_screen.calendar_month = m;
                    self.back_screen.selected_calendar_day = d;
                    self.back_screen.editing_calendar_event_id = Some(*event_id);
                    self.fields[F_PHONE_SCHEDULE_TITLE].set(&ev.title);
                    self.fields[F_CALENDAR_TIME].set(&editable_calendar_time_label(&ev));
                    self.focus_field(F_PHONE_SCHEDULE_TITLE, ctx);
                    self.status =
                        format!("正在编辑日程「{}」，修改后点击保存修改或按 Enter", ev.title);
                }
            }
            BackScreenAction::CalendarCancelEdit => {
                self.back_screen.editing_calendar_event_id = None;
                self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                self.status = "已取消编辑日程".into();
                self.error = false;
            }
            BackScreenAction::CalendarSendReminderToPhone(event_id) => {
                if let Some(ev) = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == *event_id)
                    .cloned()
                {
                    let summary = format!("【日程提醒】{}（{}）", ev.title, ev.hm_label());
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(summary.clone()));
                    self.fields[F_PHONE_SCHEDULE_TITLE].set(&ev.title);
                    self.fields[F_CALENDAR_TIME].set(&ev.hm_label());
                    let subtitle = format!("{} · 桌面与手机双端日程提醒", ev.hm_label());
                    if let Some(device) = self.selected.clone() {
                        self.do_backscreen_action(&BackScreenAction::PushDesktopClipboard, ctx)?;
                        let serial = device.serial.clone();
                        let title = ev.title.clone();
                        let sub = subtitle.clone();
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
                                let req = stepx_companion::push::PhoneLiveCapsuleRequest {
                                    tag: "stepx_schedule".into(),
                                    pill_text: "📅 日程提醒".into(),
                                    title,
                                    subtitle: sub,
                                    badge_text: "日程协同".into(),
                                    live_type: 6,
                                    countdown_secs: None,
                                    ongoing: true,
                                };
                                let _ = stepx_companion::push::post_phone_live_capsule(
                                    &adb, &serial, &req,
                                );
                            }
                        });
                        self.status = format!(
                            "已将日程提醒「{}」同步到手机顶部实况胶囊与剪贴板",
                            ev.title
                        );
                    } else {
                        self.status = format!("已复制日程提醒「{}」", ev.title);
                    }
                    self.back_screen.active_phone_capsule = Some((
                        "stepx_schedule".into(),
                        "📅 日程提醒".into(),
                        ev.title.clone(),
                        Instant::now(),
                    ));
                }
            }
            BackScreenAction::GalleryExportFiltered => {
                let indices = self.filtered_gallery_indices();
                if indices.is_empty() {
                    self.status = "当前筛选下没有可导出的照片".into();
                    self.error = true;
                    return Ok(());
                }
                let items: Vec<_> = indices
                    .into_iter()
                    .filter_map(|i| self.back_screen.gallery_items.get(i).cloned())
                    .collect();
                let base_dir = self
                    .settings
                    .value
                    .download_directory
                    .as_ref()
                    .filter(|d| !d.as_os_str().is_empty())
                    .cloned()
                    .unwrap_or_else(|| {
                        let home = std::env::var_os("HOME")
                            .or_else(|| std::env::var_os("USERPROFILE"))
                            .map(PathBuf::from)
                            .unwrap_or_else(std::env::temp_dir);
                        let pics = home.join("Pictures");
                        if pics.is_dir() {
                            pics.join("StepX_Export")
                        } else {
                            home.join("Downloads").join("StepX_Export")
                        }
                    });
                let device = self.selected.clone();
                let count = items.len();
                self.status = format!(
                    "正在导出当前筛选的 {count} 张照片到 {}…",
                    base_dir.display()
                );
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = std::fs::create_dir_all(&base_dir);
                    let adb = Adb::discover().ok();
                    let mut saved = 0usize;
                    for item in items {
                        let dest = base_dir.join(&item.name);
                        if let Some(local) = item
                            .local_thumb_path
                            .as_deref()
                            .filter(|p| std::path::Path::new(p).is_file())
                        {
                            if std::fs::copy(local, &dest).is_ok() {
                                saved += 1;
                                continue;
                            }
                        }
                        if let (Some(adb), Some(dev)) = (&adb, &device) {
                            if storage::pull(adb, dev, &item.remote_path, &dest).is_ok() {
                                saved += 1;
                            }
                        }
                    }
                    let _ = work_state::open_work_path(&base_dir);
                    let _ = tx.send_blocking((saved, base_dir));
                });
                ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                    if let Ok((saved, dir)) = result {
                        me.error = false;
                        me.status = format!("已导出 {saved} 张照片到电脑：{}", dir.display());
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::GalleryOpenExportFolder => {
                let base_dir = self
                    .settings
                    .value
                    .download_directory
                    .as_ref()
                    .filter(|d| !d.as_os_str().is_empty())
                    .cloned()
                    .unwrap_or_else(|| {
                        let home = std::env::var_os("HOME")
                            .or_else(|| std::env::var_os("USERPROFILE"))
                            .map(PathBuf::from)
                            .unwrap_or_else(std::env::temp_dir);
                        let pics = home.join("Pictures");
                        if pics.is_dir() {
                            pics
                        } else {
                            home.join("Downloads")
                        }
                    });
                let _ = std::fs::create_dir_all(&base_dir);
                let _ = work_state::open_work_path(&base_dir);
                self.status = format!("已打开电脑相册保存目录：{}", base_dir.display());
                self.error = false;
            }
            BackScreenAction::LocalSynthesize => {
                let prompt = self.fields[F_BACK_PROMPT].text.trim().to_string();
                let label_or_prompt = if prompt.is_empty() {
                    "STEPX".to_string()
                } else {
                    prompt
                };
                let (title, frame) = bs::synthesize_prompt_frame(&label_or_prompt);
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Image(frame);
                self.back_screen.title = title.clone();
                self.back_screen.origin = "本地极速点阵合成".into();
                self.back_screen.description = format!(
                    "已根据「{label_or_prompt}」合成 17×23 背屏点阵，可在右侧点击像素微调或直接应用到背屏。"
                );
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                self.back_screen.reset_preview();
                self.back_screen.status =
                    format!("已生成背屏点阵「{title}」，可点击像素编辑或应用到手机");
                self.status = self.back_screen.status.clone();
                self.error = false;
            }
            BackScreenAction::TogglePixel(x, y) => {
                if *x < bs::WIDTH && *y < bs::HEIGHT && !bs::is_backscreen_corner(*x, *y) {
                    if self.back_screen.frame.len() != bs::SLOTS {
                        self.back_screen.refresh_frame();
                    }
                    let mut pixels = if self.back_screen.frame.len() == bs::SLOTS
                        && self.back_screen.frame.iter().any(|&b| b > 0)
                    {
                        self.back_screen.frame.clone()
                    } else if self.page == Page::Home {
                        let custom = self.fields[F_BACK_MARQUEE].text.trim();
                        let suite_sig = self.back_screen.stepos_suite.backscreen_signature.trim();
                        let raw_sig = if !custom.is_empty() && custom != "STEPX AI" {
                            custom
                        } else if !suite_sig.is_empty() {
                            suite_sig
                        } else {
                            "开会勿扰"
                        };
                        let stripped = raw_sig.trim_start_matches(|c: char| {
                            c.is_whitespace()
                                || (c as u32) >= 0x1F000
                                || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                        });
                        let sig = if stripped.is_empty() {
                            "开会勿扰"
                        } else {
                            stripped
                        };
                        bs::synthesize_chinese_signature_frame(sig, self.back_screen.config.invert)
                    } else if self.back_screen.frame.len() == bs::SLOTS {
                        self.back_screen.frame.clone()
                    } else {
                        vec![0; bs::SLOTS]
                    };
                    let idx = *y * bs::WIDTH + *x;
                    pixels[idx] = if pixels[idx] >= 128 { 0 } else { 255 };
                    self.back_screen.native_feature = None;
                    self.back_screen.source = bs::Source::Image(pixels.clone());
                    self.back_screen.frame = pixels;
                    self.back_screen.origin = "手绘 DIY 点阵".into();
                    self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                    self.back_screen.status = format!("已点亮/熄灭背屏坐标 ({}, {})", x + 1, y + 1);
                }
            }
            BackScreenAction::InvertFrame => {
                if self.back_screen.frame.len() != bs::SLOTS {
                    self.back_screen.refresh_frame();
                }
                let mut pixels = self.back_screen.frame.clone();
                for y in 0..bs::HEIGHT {
                    for x in 0..bs::WIDTH {
                        if !bs::is_backscreen_corner(x, y) {
                            let idx = y * bs::WIDTH + x;
                            pixels[idx] = 255u8.saturating_sub(pixels[idx]);
                        }
                    }
                }
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Image(pixels.clone());
                self.back_screen.frame = pixels;
                self.back_screen.origin = "手绘 DIY 点阵".into();
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                self.back_screen.status = "已反相当前背屏点阵画布".into();
            }
            BackScreenAction::ShiftFrame(dx, dy) => {
                if self.back_screen.frame.len() != bs::SLOTS {
                    self.back_screen.refresh_frame();
                }
                let old = self.back_screen.frame.clone();
                let mut next = vec![0u8; bs::SLOTS];
                for y in 0..bs::HEIGHT {
                    for x in 0..bs::WIDTH {
                        if bs::is_backscreen_corner(x, y) {
                            continue;
                        }
                        let sx = x as i32 - *dx;
                        let sy = y as i32 - *dy;
                        if (0..bs::WIDTH as i32).contains(&sx)
                            && (0..bs::HEIGHT as i32).contains(&sy)
                        {
                            next[y * bs::WIDTH + x] = old[sy as usize * bs::WIDTH + sx as usize];
                        }
                    }
                }
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Image(next.clone());
                self.back_screen.frame = next;
                self.back_screen.origin = "手绘 DIY 点阵".into();
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
            }
            BackScreenAction::ClearDiyFrame => {
                let blank = vec![0u8; bs::SLOTS];
                self.back_screen.native_feature = None;
                self.back_screen.source = bs::Source::Image(blank.clone());
                self.back_screen.frame = blank;
                self.back_screen.title = "自定义手绘图案".into();
                self.back_screen.origin = "手绘 DIY 点阵".into();
                self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                self.back_screen.status = "已清空画布，可直接点击右侧 17×23 灯珠绘制图案".into();
            }
            BackScreenAction::SetThresholdValue(val) => {
                self.back_screen.image_options.threshold = *val;
                self.back_screen.reconvert()?;
            }
            BackScreenAction::RotateImage90 => {
                self.back_screen.image_options.rotation_deg =
                    (self.back_screen.image_options.rotation_deg + 90) % 360;
                self.back_screen.reconvert()?;
            }
            BackScreenAction::FlipImageH => {
                self.back_screen.image_options.flip_h = !self.back_screen.image_options.flip_h;
                self.back_screen.reconvert()?;
            }
            BackScreenAction::CalendarStartCreateOnDate(year, month, day) => {
                self.page = Page::Calendar;
                let y = *year;
                let m = (*month).clamp(1, 12);
                let d = (*day).clamp(1, days_in_calendar_month(y, m));
                self.back_screen.calendar_year = y;
                self.back_screen.calendar_month = m;
                self.back_screen.selected_calendar_day = d;
                self.back_screen.editing_calendar_event_id = None;
                self.focus_field(F_PHONE_SCHEDULE_TITLE, ctx);
                self.status = format!("已选中 {y}-{m:02}-{d:02}，请输入日程标题后点击添加");
                self.error = false;
            }
            BackScreenAction::CalendarQuickAddOnDate {
                year,
                month,
                day,
                title,
                time,
            } => {
                self.page = Page::Calendar;
                let y = *year;
                let m = (*month).clamp(1, 12);
                let d = (*day).clamp(1, days_in_calendar_month(y, m));
                self.back_screen.calendar_year = y;
                self.back_screen.calendar_month = m;
                self.back_screen.selected_calendar_day = d;
                self.back_screen.editing_calendar_event_id = None;
                self.fields[F_PHONE_SCHEDULE_TITLE].set(title);
                self.fields[F_CALENDAR_TIME].set(time);
                self.do_backscreen_action(
                    &BackScreenAction::CalendarCreateOnSelectedDate(None),
                    ctx,
                )?;
            }
            BackScreenAction::CalendarClearDay(year, month, day) => {
                let target_ymd = (*year, *month, *day);
                let ids: Vec<i64> = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .filter(|ev| ev.date_ymd() == target_ymd)
                    .map(|ev| ev.id)
                    .collect();
                if ids.is_empty() {
                    self.status = format!("{year}-{month:02}-{day:02} 当日暂无日程可清空");
                    self.error = false;
                    return Ok(());
                }
                let count = ids.len();
                self.back_screen
                    .stepos_suite
                    .calendar_events
                    .retain(|ev| ev.date_ymd() != target_ymd);
                for id in &ids {
                    if self.back_screen.editing_calendar_event_id == Some(*id) {
                        self.back_screen.editing_calendar_event_id = None;
                    }
                    self.remove_calendar_event_state(*id);
                }
                if self.prefs.auto_sync_calendar {
                    rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
                }
                let note = format!("已清空 {year}-{month:02}-{day:02} 的 {count} 项日程");
                self.back_screen.stepos_suite.last_action_note = note.clone();
                self.status = note;
                self.error = false;
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let ids_to_del = ids;
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().map(|adb| {
                                let mut latest_events = None;
                                for id in ids_to_del {
                                    if let Ok((evs, _)) =
                                        stepx_companion::toolbox::delete_phone_calendar_event(
                                            &adb, &device, id,
                                        )
                                    {
                                        latest_events = Some(evs);
                                    }
                                }
                                latest_events
                            });
                            let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                        });
                        ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                            if let Ok(Ok(Some(events))) = result {
                                me.back_screen.stepos_suite.calendar_events = events;
                                ctx.notify();
                            }
                        });
                    }
                }
            }
            BackScreenAction::CalendarDuplicateToTomorrow(event_id) => {
                let Some(ev) = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == *event_id)
                    .cloned()
                else {
                    return Ok(());
                };
                let new_start = ev.dtstart + 86_400_000;
                let new_end = ev.dtend + 86_400_000;
                let new_id = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .map(|e| e.id)
                    .max()
                    .unwrap_or(1000)
                    + 1;
                let mut dup = ev.clone();
                dup.id = new_id;
                dup.dtstart = new_start;
                dup.dtend = new_end;
                let (ny, nm, nd) = dup.date_ymd();
                let title = dup.title.clone();
                let desc = dup.description.clone();
                self.back_screen.stepos_suite.calendar_events.push(dup);
                self.back_screen
                    .stepos_suite
                    .calendar_events
                    .sort_by_key(|e| e.dtstart);
                if self.prefs.auto_sync_calendar {
                    let _ = stepx_companion::toolbox::save_desktop_calendar_event(
                        &title, new_start, new_end, &desc,
                    );
                }
                self.back_screen.calendar_year = ny;
                self.back_screen.calendar_month = nm;
                self.back_screen.selected_calendar_day = nd;
                let note = format!("已将日程「{title}」克隆到明天");
                self.back_screen.stepos_suite.last_action_note = note.clone();
                self.status = note;
                self.error = false;
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let title_bg = title.clone();
                        let loc_bg = ev.location.clone();
                        let desc_bg = if desc.is_empty() {
                            "由 StepX 桌面端克隆创建".to_string()
                        } else {
                            desc
                        };
                        let all_day_bg = ev.all_day;
                        let auto_sync = self.prefs.auto_sync_calendar;
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().and_then(|adb| {
                                create_phone_calendar_event_preserving_details(
                                    &adb, &device, &title_bg, new_start, new_end, all_day_bg,
                                    &loc_bg, &desc_bg, auto_sync, ny, nm, nd,
                                )
                            });
                            let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                        });
                        ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                            if let Ok(Ok((_id, events, _))) = result {
                                me.back_screen.stepos_suite.calendar_events = events;
                                ctx.notify();
                            }
                        });
                    }
                }
            }
            BackScreenAction::CalendarPostponeOneDay(event_id) => {
                let mut updated_info: Option<(
                    i64,
                    String,
                    String,
                    String,
                    bool,
                    i64,
                    i64,
                    (i32, u32, u32),
                )> = None;
                if let Some(ev) = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter_mut()
                    .find(|e| e.id == *event_id)
                {
                    ev.dtstart += 86_400_000;
                    ev.dtend += 86_400_000;
                    updated_info = Some((
                        ev.id,
                        ev.title.clone(),
                        ev.location.clone(),
                        ev.description.clone(),
                        ev.all_day,
                        ev.dtstart,
                        ev.dtend,
                        ev.date_ymd(),
                    ));
                }
                let Some((old_id, title, loc, desc, all_day, new_start, new_end, (ny, nm, nd))) =
                    updated_info
                else {
                    return Ok(());
                };
                self.back_screen
                    .stepos_suite
                    .calendar_events
                    .sort_by_key(|e| e.dtstart);
                if self.prefs.auto_sync_calendar {
                    rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
                }
                self.back_screen.calendar_year = ny;
                self.back_screen.calendar_month = nm;
                self.back_screen.selected_calendar_day = nd;
                let note = format!("已将日程「{title}」顺延至 {ny}-{nm:02}-{nd:02}");
                self.back_screen.stepos_suite.last_action_note = note.clone();
                self.status = note;
                self.error = false;
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let title_bg = title.clone();
                        let loc_bg = loc.clone();
                        let desc_bg = if desc.is_empty() {
                            "由 StepX 桌面端顺延更新".to_string()
                        } else {
                            desc
                        };
                        let auto_sync = self.prefs.auto_sync_calendar;
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().and_then(|adb| {
                                let _ = stepx_companion::toolbox::delete_phone_calendar_event(
                                    &adb, &device, old_id,
                                );
                                create_phone_calendar_event_preserving_details(
                                    &adb, &device, &title_bg, new_start, new_end, all_day, &loc_bg,
                                    &desc_bg, auto_sync, ny, nm, nd,
                                )
                            });
                            let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                        });
                        ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                            if let Ok(Ok((new_id, events, _))) = result {
                                if old_id != new_id && new_id > 0 {
                                    me.migrate_calendar_event_state(old_id, new_id);
                                }
                                me.back_screen.stepos_suite.calendar_events = events;
                                ctx.notify();
                            }
                        });
                    }
                }
            }
            BackScreenAction::CalendarPrefillFromText { title, time_hint } => {
                self.page = Page::Calendar;
                self.back_screen.editing_calendar_event_id = None;
                let flat: String = title
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut chars = flat.chars();
                let head: String = chars.by_ref().take(60).collect();
                let clean_title = if head.is_empty() {
                    "消息提取日程".to_string()
                } else if chars.next().is_some() {
                    format!("{head}…")
                } else {
                    head
                };
                self.fields[F_PHONE_SCHEDULE_TITLE].set(&clean_title);
                self.fields[F_CALENDAR_TIME].set(if time_hint.trim().is_empty() {
                    "09:30-10:00"
                } else {
                    time_hint.trim()
                });
                self.focus_field(F_PHONE_SCHEDULE_TITLE, ctx);
                self.status =
                    format!("已从消息提取日程草稿「{clean_title}」，确认日期与时间后点击添加即可");
                self.error = false;
            }
            BackScreenAction::AlarmEdit(alarm_id) => {
                if let Some(alarm) = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .find(|a| a.id == *alarm_id)
                    .cloned()
                {
                    self.page = Page::Alarms;
                    self.back_screen.editing_alarm_id = Some(*alarm_id);
                    self.fields[F_ALARM_TIME].set(&alarm.time_label());
                    self.fields[F_ALARM_LABEL].set(alarm.display_title());
                    self.focus_field(F_ALARM_LABEL, ctx);
                    self.status = format!(
                        "正在修改闹钟 {}（{}），修改后点击「保存闹钟修改」",
                        alarm.time_label(),
                        alarm.display_title()
                    );
                    self.error = false;
                }
            }
            BackScreenAction::AlarmCancelEdit => {
                self.back_screen.editing_alarm_id = None;
                self.fields[F_ALARM_LABEL].clear();
                self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                self.status = "已取消修改闹钟".into();
                self.error = false;
            }
            BackScreenAction::AlarmShiftMinutes(alarm_id, delta_mins) => {
                let mut shifted_info: Option<(i64, String, u8, u8, u32, bool)> = None;
                if let Some(alarm) = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter_mut()
                    .find(|a| a.id == *alarm_id)
                {
                    let total_mins = (alarm.hour as i32 * 60 + alarm.minutes as i32 + *delta_mins)
                        .rem_euclid(1440);
                    alarm.hour = (total_mins / 60) as u8;
                    alarm.minutes = (total_mins % 60) as u8;
                    alarm.enabled = true;
                    shifted_info = Some((
                        alarm.id,
                        alarm.display_title().to_string(),
                        alarm.hour,
                        alarm.minutes,
                        alarm.days_of_week,
                        alarm.vibrate,
                    ));
                }
                if let Some((old_id, title, new_h, new_m, days_of_week, vibrate)) = shifted_info {
                    let note = format!("已将闹钟「{title}」调整至 {new_h:02}:{new_m:02}");
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    if !self.demo_mode {
                        if let Some(device) = self.selected.clone() {
                            let (tx, rx) = async_channel::bounded(1);
                            std::thread::spawn(move || {
                                let res = Adb::discover().and_then(|adb| {
                                    let _ = stepx_companion::toolbox::delete_phone_alarm(
                                        &adb, &device, old_id,
                                    );
                                    create_phone_alarm_preserving_details(
                                        &adb,
                                        &device,
                                        new_h,
                                        new_m,
                                        &title,
                                        days_of_week,
                                        vibrate,
                                    )
                                });
                                let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                            });
                            ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                                if let Ok(Ok((_id, alarms, _))) = result {
                                    me.back_screen.stepos_suite.alarms = alarms;
                                    ctx.notify();
                                }
                            });
                        }
                    }
                }
            }
            BackScreenAction::AlarmToggleAll(enable) => {
                let target = *enable;
                let ids: Vec<i64> = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .map(|a| a.id)
                    .collect();
                for a in &mut self.back_screen.stepos_suite.alarms {
                    a.enabled = target;
                }
                let note = if target {
                    "已开启全部闹钟".to_string()
                } else {
                    "已关闭全部闹钟".to_string()
                };
                self.back_screen.stepos_suite.last_action_note = note.clone();
                self.status = note;
                self.error = false;
                if !self.demo_mode && !ids.is_empty() {
                    if let Some(device) = self.selected.clone() {
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().map(|adb| {
                                let mut latest_alarms = None;
                                for id in ids {
                                    if let Ok((alarms, _)) =
                                        stepx_companion::toolbox::toggle_phone_alarm(
                                            &adb, &device, id, target,
                                        )
                                    {
                                        latest_alarms = Some(alarms);
                                    }
                                }
                                latest_alarms
                            });
                            let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                        });
                        ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                            if let Ok(Ok(Some(alarms))) = result {
                                me.back_screen.stepos_suite.alarms = alarms;
                                ctx.notify();
                            }
                        });
                    }
                }
            }
            BackScreenAction::ToggleLiveCapsulePopover => {
                self.back_screen.live_capsule_popover_open =
                    !self.back_screen.live_capsule_popover_open;
                if self.back_screen.live_capsule_popover_open {
                    self.dismiss_search_for_header();
                    self.header_actions_open = false;
                    self.auto_sync.details_open = false;
                    self.notification_center.drawer_open = false;
                }
            }
            BackScreenAction::CloseLiveCapsulePopover => {
                self.back_screen.live_capsule_popover_open = false;
            }
            BackScreenAction::SetLiveCapsuleFilter(filter) => {
                self.back_screen.live_capsule_filter = *filter;
                self.back_screen
                    .live_capsule_scroll
                    .scroll_to(facloud_ui::units::Pixels::new(0.0));
            }
            BackScreenAction::CompleteCapsuleItem {
                id,
                pickup_code,
                sms_address,
                im_thread_id,
                notif_key,
            } => {
                self.complete_capsule_item(
                    id,
                    pickup_code.as_deref(),
                    sms_address.as_deref(),
                    im_thread_id.as_deref(),
                    notif_key.as_deref(),
                );
            }
            BackScreenAction::OpenCapsuleTarget {
                sms_address,
                im_thread_id,
                open_calendar,
                open_alarms,
            } => {
                self.back_screen.live_capsule_popover_open = false;
                if *open_calendar {
                    self.page = Page::Calendar;
                    self.status = "已从胶囊打开手机日历与今日议程".into();
                    self.error = false;
                } else if *open_alarms {
                    self.page = Page::Alarms;
                    self.status = "已从胶囊打开手机闹钟管理".into();
                    self.error = false;
                } else if let Some(tid) = im_thread_id.as_ref().filter(|s| !s.trim().is_empty()) {
                    self.page = Page::Sms;
                    self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
                    self.push.im_channel = stepx_companion::push::ImChannel::All;
                    self.apply_select_im_channel(stepx_companion::push::ImChannel::All);
                    self.push.selected_im_thread = Some(tid.clone());
                    self.selected_thread = None;
                    if let Some(thread) = self
                        .push
                        .snapshot
                        .threads
                        .iter_mut()
                        .find(|t| t.thread_id == *tid)
                    {
                        thread.unread_count = 0;
                    }
                    self.sync_notification_center();
                    self.status = "已从胶囊直达对应消息会话".into();
                    self.error = false;
                } else if let Some(addr) = sms_address.as_ref().filter(|s| !s.trim().is_empty()) {
                    self.page = Page::Sms;
                    self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
                    self.push.im_channel = stepx_companion::push::ImChannel::Sms;
                    self.apply_select_im_channel(stepx_companion::push::ImChannel::Sms);
                    self.selected_thread = Some(addr.clone());
                    self.push.selected_im_thread = None;
                    self.mark_sms_thread_read(addr);
                    self.status = format!("已从胶囊定位到短信会话：{addr}");
                    self.error = false;
                } else {
                    self.page = Page::Sms;
                    self.status = "已从胶囊打开通信中枢".into();
                    self.error = false;
                }
            }
            BackScreenAction::OpenFulfillment { id } => {
                self.open_fulfillment_capsule(id, ctx);
            }
            BackScreenAction::PushPhoneLiveCapsule {
                tag,
                pill_text,
                title,
                subtitle,
                countdown_secs,
                sync_backscreen,
            } => {
                self.back_screen.active_phone_capsule = Some((
                    tag.clone(),
                    pill_text.clone(),
                    title.clone(),
                    Instant::now(),
                ));
                if *sync_backscreen && countdown_secs.unwrap_or(0) == 0 {
                    let sig: String = pill_text
                        .chars()
                        .filter(|c| {
                            !c.is_whitespace()
                                && (*c as u32) < 0x1F000
                                && !matches!(*c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                        })
                        .take(6)
                        .collect();
                    if !sig.is_empty() {
                        self.fields[F_BACK_MARQUEE].set(&sig);
                        self.back_screen.stepos_suite.backscreen_signature = sig.clone();
                        self.matrix_marquee_on = true;
                        let frame = bs::synthesize_chinese_signature_frame(
                            &sig,
                            self.back_screen.config.invert,
                        );
                        self.back_screen.native_feature = None;
                        self.back_screen.source = bs::Source::Image(frame.clone());
                        self.back_screen.frame = frame;
                        self.back_screen.title = format!("背屏状态 · {sig}");
                        self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                    }
                }
                self.back_screen.status =
                    format!("正在同步到手机顶部实况胶囊：{} · {}", pill_text, title);
                self.status = self.back_screen.status.clone();
                self.error = false;
                if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                    let serial = device.serial.clone();
                    let tag_c = tag.clone();
                    let pill_c = pill_text.clone();
                    let title_c = title.clone();
                    let sub_c = subtitle.clone();
                    let cd_secs = *countdown_secs;
                    let sync_bs = *sync_backscreen;
                    let brightness = self.back_screen.config.brightness;
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let res = Adb::discover().and_then(|adb| {
                            let req = stepx_companion::push::PhoneLiveCapsuleRequest {
                                tag: tag_c.clone(),
                                pill_text: pill_c.clone(),
                                title: title_c.clone(),
                                subtitle: sub_c.clone(),
                                badge_text: "双端协同".into(),
                                live_type: if cd_secs.unwrap_or(0) > 0 { 11 } else { 1 },
                                countdown_secs: cd_secs,
                                ongoing: true,
                            };
                            stepx_companion::push::post_phone_live_capsule(
                                &adb, &serial, &req,
                            )?;
                            if sync_bs {
                                if let Some(secs) = cd_secs.filter(|s| *s > 0) {
                                    let _ = bs::start_pomodoro_countdown(
                                        &adb,
                                        &device,
                                        secs.clamp(60, 3600) as u32,
                                        brightness,
                                    );
                                } else {
                                    let sig: String = pill_c
                                        .chars()
                                        .filter(|c| {
                                            !c.is_whitespace()
                                                && (*c as u32) < 0x1F000
                                                && !matches!(*c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                                        })
                                        .take(6)
                                        .collect();
                                    let sig = if sig.is_empty() {
                                        "StepX".to_string()
                                    } else {
                                        sig
                                    };
                                    let _ = stepx_companion::toolbox::trigger_stepos_mcp_action(
                                        &adb,
                                        &device,
                                        &format!("backscreen_sig:{sig}"),
                                        &sig,
                                    );
                                }
                            }
                            Ok(format!(
                                "已同步到手机顶部实况胶囊{}：{} · {}",
                                if sync_bs { "与背屏点阵" } else { "" },
                                pill_c,
                                title_c
                            ))
                        });
                        let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                    });
                    let pill_done = pill_text.clone();
                    let title_done = title.clone();
                    ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                        match result {
                            Ok(Ok(msg)) => {
                                me.back_screen.status = msg.clone();
                                me.status = msg;
                                me.error = false;
                            }
                            Ok(Err(err)) => {
                                me.back_screen.status = format!(
                                    "顶部实况胶囊已更新状态（设备回执提示：{err}）：{} · {}",
                                    pill_done, title_done
                                );
                                me.status = me.back_screen.status.clone();
                            }
                            Err(_) => {}
                        }
                        ctx.notify();
                    });
                }
            }
            BackScreenAction::DismissPhoneLiveCapsule(tag) => {
                if self
                    .back_screen
                    .active_phone_capsule
                    .as_ref()
                    .is_some_and(|(active_tag, _, _, _)| active_tag == tag)
                {
                    self.back_screen.active_phone_capsule = None;
                }
                self.back_screen.status = format!("已收起手机顶部实况胶囊（{tag}）");
                self.status = self.back_screen.status.clone();
                self.error = false;
                if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                    let serial = device.serial.clone();
                    let tag_c = tag.clone();
                    std::thread::spawn(move || {
                        if let Ok(adb) = Adb::discover() {
                            let _ = stepx_companion::push::dismiss_phone_live_capsule(
                                &adb, &serial, &tag_c,
                            );
                        }
                    });
                }
            }
            BackScreenAction::ClearAllPhoneLiveCapsules => {
                self.back_screen.active_phone_capsule = None;
                self.back_screen.status = "已清空手机顶部全部 StepX 实况胶囊".into();
                self.status = self.back_screen.status.clone();
                self.error = false;
                if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                    let serial = device.serial.clone();
                    std::thread::spawn(move || {
                        if let Ok(adb) = Adb::discover() {
                            let _ = stepx_companion::push::clear_all_phone_live_capsules(
                                &adb, &serial,
                            );
                        }
                    });
                }
            }
        }
        Ok(())
    }
    fn begin_backscreen_apply(&mut self) -> Result<()> {
        anyhow::ensure!(!self.back_screen.generating, "请等待 Amoo 生成完成");
        let device = self
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
        if let Some(playback) = &self.back_screen.playback {
            self.back_screen.pending_play = true;
            playback.cancel();
            self.back_screen.applied_status = "正在结束原动画，随后应用所选图案…".into();
            return Ok(());
        }
        let adb = Adb::discover()?;
        let source = self.back_screen.source.clone();
        let config = self.back_screen.config.clone();
        let native = self.back_screen.native_feature;
        let title = self.back_screen.title.clone();
        let revision = self.back_screen.revision;
        let companion_running =
            self.back_screen.applied_to_phone && self.back_screen.companion_mode != "native";
        let (sender, receiver) = mpsc::channel();
        self.back_screen.active_device = Some(device.serial.clone());
        self.back_screen.companion_rx = Some(receiver);
        self.back_screen.companion_busy = true;
        self.back_screen.applied_status = "正在应用到手机背屏…".into();
        std::thread::spawn(move || {
            let result = (|| -> Result<CompanionEvent> {
                if let Some(feature) = native {
                    if companion_running {
                        let _ = bs::release_custom_frame_owner(&adb, &device);
                    }
                    let message = bs::configure_native(&adb, &device, feature)?;
                    return Ok(CompanionEvent::Applied {
                        title,
                        revision,
                        message,
                        mode: "native".into(),
                    });
                }
                if matches!(source, bs::Source::App { .. }) {
                    if companion_running {
                        let _ = bs::release_custom_frame_owner(&adb, &device);
                    }
                    let playback = bs::start(adb, device, source, config)?;
                    return Ok(CompanionEvent::TemporaryStarted {
                        playback,
                        title,
                        revision,
                    });
                }
                let message = bs::apply_via_firmware_mcp(&adb, &device, &source, &config, &title)?;
                Ok(CompanionEvent::Applied {
                    title,
                    revision,
                    message,
                    mode: "animation".into(),
                })
            })();
            let _ = sender
                .send(result.unwrap_or_else(|error| CompanionEvent::Failed(format!("{error:#}"))));
        });
        Ok(())
    }
    fn start_companion_action(&mut self, action: BackScreenAction) -> Result<()> {
        let device = self
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
        anyhow::ensure!(
            !self.back_screen.playing,
            "请先停止临时联动，再切换手机游戏"
        );
        let adb = Adb::discover()?;
        let (sender, receiver) = mpsc::channel();
        self.back_screen.companion_checked_device = Some(device.serial.clone());
        self.back_screen.active_device = Some(device.serial.clone());
        self.back_screen.companion_rx = Some(receiver);
        self.back_screen.companion_busy = true;
        self.back_screen.companion_status = match action {
            BackScreenAction::CompanionInstall => "正在安装 StepX伴侣，请保持 USB 连接…",
            BackScreenAction::GameArm => "正在启用三敲启动…",
            BackScreenAction::GameStart => "正在启动重力对齐…",
            BackScreenAction::CompanionStop => "正在停止并恢复原显示…",
            _ => "正在检测 StepX伴侣…",
        }
        .into();
        std::thread::spawn(move || {
            let result = (|| -> Result<CompanionEvent> {
                Ok(match action {
                    BackScreenAction::CompanionInstall => {
                        CompanionEvent::Inspected(ca::install(&adb, &device, |progress| {
                            let _ = sender.send(CompanionEvent::Progress(progress.into()));
                        })?)
                    }
                    BackScreenAction::GameArm => CompanionEvent::Game {
                        message: ca::arm_game(&adb, &device)?,
                        mode: "standby".into(),
                    },
                    BackScreenAction::GameStart => CompanionEvent::Game {
                        message: ca::start_game(&adb, &device)?,
                        mode: "game".into(),
                    },
                    BackScreenAction::CompanionStop => {
                        CompanionEvent::Stopped(bs::stop_via_firmware_mcp(&adb, &device)?)
                    }
                    _ => CompanionEvent::Inspected(ca::inspect(&adb, &device)?),
                })
            })();
            let _ = sender
                .send(result.unwrap_or_else(|error| CompanionEvent::Failed(format!("{error:#}"))));
        });
        Ok(())
    }
    fn start_backscreen_setting<F>(&mut self, operation: F) -> Result<()>
    where
        F: FnOnce(&Adb, &Device) -> Result<String> + Send + 'static,
    {
        let device = self
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
        let adb = Adb::discover()?;
        let (sender, receiver) = mpsc::channel();
        self.back_screen.active_device = Some(device.serial.clone());
        self.back_screen.native_rx = Some(receiver);
        self.back_screen.applying_native = true;
        self.back_screen.status = "正在保存手机背屏设置…".into();
        std::thread::spawn(move || {
            let _ = sender.send(operation(&adb, &device).map_err(|error| format!("{error:#}")));
        });
        Ok(())
    }
    pub(super) fn cancel_backscreen(&mut self) {
        self.back_screen.cancel_pomodoro_commands();
        if let Some(playback) = self.back_screen.playback.take() {
            playback.cancel();
        }
        if let Some(generation) = self.back_screen.generation.take() {
            generation.cancel();
        }
        self.back_screen.playing = false;
        self.back_screen.generating = false;
        self.back_screen.active_device = None;
        self.back_screen.native_rx = None;
        self.back_screen.applying_native = false;
        self.back_screen.companion_rx = None;
        self.back_screen.companion_busy = false;
        self.back_screen.companion_checked_device = None;
        self.back_screen.companion_installed = None;
        self.back_screen.last_synced_autonomous_cfg = None;
        self.back_screen.applied_to_phone = false;
        self.back_screen.applied_revision = None;
        self.back_screen.applied_title.clear();
        self.back_screen.applied_status = "等待连接手机后读取状态".into();
        self.back_screen.pending_play = false;
        self.back_screen.image_rx = None;
        self.back_screen.converting_image = false;
        self.back_screen.generator_status = "设备连接已切换，原创作已取消".into();
        self.back_screen.status = "设备连接已切换；如已断开，停止后的硬件恢复结果无法读取".into();
    }
    pub(super) fn poll_backscreen(&mut self, preview_visible: bool) -> bool {
        self.back_screen
            .set_preview_visible(preview_visible && !self.initializing);
        if self.back_screen.pomodoro_device.as_ref().is_some_and(|serial| {
            self.selected.as_ref().map(|device| &device.serial) != Some(serial)
        }) {
            self.back_screen.cancel_pomodoro_commands();
            self.back_screen.status =
                "设备连接已变化，待执行的背屏计时操作已取消；手机状态待重新确认".into();
            return true;
        }
        if !self.back_screen.needs_poll() {
            return false;
        }
        if self.page == Page::BackScreen
            && self.back_screen.tab == 2
            && !self.back_screen.gallery_loaded
        {
            self.back_screen.load_gallery();
        }
        if self
            .back_screen
            .active_device
            .as_ref()
            .is_some_and(|serial| {
                self.selected.as_ref().map(|device| &device.serial) != Some(serial)
            })
        {
            self.cancel_backscreen();
            return true;
        }
        let mut changed = false;
        if let Some(result) = self.back_screen.gallery_rx.as_ref().map(|rx| rx.try_recv()) {
            match result {
                Ok(images) => {
                    self.back_screen.recent_images = images;
                    self.back_screen.gallery_rx = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.back_screen.gallery_rx = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.selected.is_none() && self.back_screen.companion_checked_device.is_some() {
            self.back_screen.companion_checked_device = None;
            self.back_screen.companion_installed = None;
            self.back_screen.last_synced_autonomous_cfg = None;
            self.back_screen.applied_to_phone = false;
            self.back_screen.companion_status = "连接手机后检测 StepX伴侣".into();
            self.back_screen.applied_status = "手机连接已断开，运行状态待检测".into();
            changed = true;
        }
        if !self.demo_mode
            && self.page == Page::BackScreen
            && !self.back_screen.companion_busy
            && !self.back_screen.playing
            && !self.back_screen.generating
            && !self.back_screen.applying_native
            && self.selected.as_ref().is_some_and(|device| {
                self.back_screen.companion_checked_device.as_ref() != Some(&device.serial)
            })
        {
            if self.back_screen.companion_checked_device.is_some() {
                self.back_screen.companion_mode.clear();
                self.back_screen.applied_title.clear();
                self.back_screen.applied_to_phone = false;
                self.back_screen.applied_revision = None;
            }
            if let Err(error) = self.start_companion_action(BackScreenAction::CompanionInspect) {
                self.back_screen.companion_status = format!("{error:#}");
            }
            changed = true;
        }
        let mut companion_events = Vec::new();
        if let Some(receiver) = &self.back_screen.companion_rx {
            loop {
                match receiver.try_recv() {
                    Ok(event) => companion_events.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if companion_events.is_empty()
                            || companion_events
                                .iter()
                                .all(|event| matches!(event, CompanionEvent::Progress(_)))
                        {
                            companion_events.push(CompanionEvent::Failed(
                                "StepX伴侣操作未返回结果，请重新检测".into(),
                            ));
                        }
                        break;
                    }
                }
            }
        }
        for event in companion_events {
            changed = true;
            if let CompanionEvent::Progress(message) = event {
                self.back_screen.companion_status = message;
                continue;
            }
            self.back_screen.companion_busy = false;
            self.back_screen.companion_rx = None;
            self.back_screen.active_device = None;
            match event {
                CompanionEvent::Inspected(status) => {
                    self.back_screen.companion_installed = Some(status.installed);
                    self.back_screen.companion_permission = status.permission_granted;
                    let phone_message = status.message.clone();
                    self.back_screen.companion_status = if status
                        .version
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit())
                    {
                        format!("v{} · {}", status.version, status.message)
                    } else {
                        status.message
                    };
                    if status.running || self.back_screen.companion_mode != "native" {
                        self.back_screen.companion_mode = status.mode;
                        self.back_screen.applied_to_phone = status.running;
                        self.back_screen.applied_title = status.applied_title;
                        self.back_screen.applied_status = phone_message;
                    }
                }
                CompanionEvent::Applied {
                    title,
                    revision,
                    message,
                    mode,
                } => {
                    self.record_notice("背屏已应用", format!("{title} · {message}"));
                    self.back_screen.applied_title = title;
                    self.back_screen.applied_revision = Some(revision);
                    self.back_screen.applied_to_phone = true;
                    self.back_screen.applied_status = message;
                    self.back_screen.companion_mode = mode;
                    self.back_screen.status = "已应用，可以继续预览其他图案".into();
                }
                CompanionEvent::TemporaryStarted {
                    playback,
                    title,
                    revision,
                } => {
                    self.back_screen.playback = Some(playback);
                    self.back_screen.playing = true;
                    self.back_screen.applied_title = title;
                    self.back_screen.applied_revision = Some(revision);
                    self.back_screen.applied_elapsed_ms = 0;
                    self.back_screen.frames_sent = 0;
                    self.back_screen.companion_mode = "temporary".into();
                    self.back_screen.active_device =
                        self.selected.as_ref().map(|device| device.serial.clone());
                }
                CompanionEvent::Game { message, mode } => {
                    self.record_notice("背屏游戏", message.clone());
                    self.back_screen.companion_status = message.clone();
                    self.back_screen.applied_status = message;
                    self.back_screen.applied_to_phone = true;
                    self.back_screen.applied_revision = None;
                    self.back_screen.applied_title = if mode == "standby" {
                        "重力对齐 · 三敲待命"
                    } else {
                        "重力对齐"
                    }
                    .into();
                    self.back_screen.companion_mode = mode;
                }
                CompanionEvent::Stopped(message) => {
                    self.back_screen.applied_to_phone = false;
                    self.back_screen.applied_revision = None;
                    self.back_screen.applied_title.clear();
                    self.back_screen.applied_status = message.clone();
                    self.back_screen.companion_status = message;
                    self.back_screen.companion_mode.clear();
                }
                CompanionEvent::Failed(message) => {
                    self.record_notice("背屏操作未完成", message.clone());
                    self.back_screen.error = Some(message.clone());
                    self.back_screen.status = message.clone();
                    self.back_screen.applied_status = "本次操作未完成，请检查连接后重试".into();
                    self.back_screen.companion_status = message;
                }
                CompanionEvent::Progress(_) => {}
            }
        }
        if let Some(receiver) = &self.back_screen.image_rx {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("图片转换未返回结果，请重试".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.back_screen.converting_image = false;
                self.back_screen.image_rx = None;
                changed = true;
                match result {
                    Ok((path, origin, frame)) => {
                        self.back_screen.title = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or("图片点阵".into());
                        self.back_screen.image_path = Some(path);
                        self.back_screen.origin = origin;
                        self.back_screen.source = bs::Source::Image(frame);
                        self.back_screen.native_feature = None;
                        self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                        self.back_screen.status = "图片已转换，检查预览后应用到背屏".into();
                        self.back_screen.reset_preview();
                    }
                    Err(error) => {
                        self.back_screen.status = error.clone();
                        self.back_screen.error = Some(error);
                    }
                }
            }
        }
        if let Some(receiver) = &self.back_screen.native_rx {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("手机联动配置未返回结果".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                changed = true;
                self.back_screen.native_rx = None;
                self.back_screen.active_device = None;
                self.back_screen.applying_native = false;
                match result {
                    Ok(message) => self.back_screen.status = message,
                    Err(error) => {
                        self.back_screen.status = error.clone();
                        self.back_screen.error = Some(error);
                    }
                }
            }
        }
        let mut playback_events = Vec::new();
        if let Some(playback) = &self.back_screen.playback {
            loop {
                match playback.events.try_recv() {
                    Ok(event) => playback_events.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !playback_events.iter().any(|event| {
                            matches!(
                                event,
                                bs::PlaybackEvent::Finished { .. } | bs::PlaybackEvent::Failed(_)
                            )
                        }) {
                            playback_events.push(bs::PlaybackEvent::Failed(
                                "背屏播放连接已结束，无法确认手机恢复结果".into(),
                            ));
                        }
                        break;
                    }
                }
            }
        }
        for event in playback_events {
            changed = true;
            match event {
                bs::PlaybackEvent::Started { message } => {
                    self.back_screen.applied_status = message;
                }
                bs::PlaybackEvent::Frame { frame, elapsed_ms } => {
                    self.back_screen.applied_frame = frame;
                    self.back_screen.applied_elapsed_ms = elapsed_ms;
                    self.back_screen.applied_to_phone = true;
                    self.back_screen.frames_sent += 1;
                }
                bs::PlaybackEvent::Finished { cancelled, frames } => {
                    self.back_screen.playing = false;
                    self.back_screen.playback = None;
                    self.back_screen.frames_sent = frames;
                    self.back_screen.active_device = None;
                    self.back_screen.applied_to_phone = false;
                    self.back_screen.applied_revision = None;
                    self.back_screen.applied_status = format!(
                        "{} · 已发送 {} 帧，已恢复原有设置",
                        if cancelled {
                            "已停止"
                        } else {
                            "播放完成"
                        },
                        frames
                    );
                    if self.back_screen.pending_play {
                        self.back_screen.pending_play = false;
                        if let Err(error) = self.begin_backscreen_apply() {
                            self.back_screen.error = Some(format!("{error:#}"));
                        }
                    }
                }
                bs::PlaybackEvent::Failed(error) => {
                    self.back_screen.playing = false;
                    self.back_screen.playback = None;
                    self.back_screen.active_device = None;
                    self.back_screen.error = Some(error.clone());
                    self.back_screen.status = error;
                    self.back_screen.pending_play = false;
                    self.back_screen.applied_to_phone = false;
                    self.back_screen.applied_revision = None;
                }
            }
        }
        let mut generation_events = Vec::new();
        if let Some(generation) = &self.back_screen.generation {
            loop {
                match generation.receiver.try_recv() {
                    Ok(event) => generation_events.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !generation_events.iter().any(|event| {
                            matches!(
                                event,
                                ba::Event::Complete(_)
                                    | ba::Event::Cancelled
                                    | ba::Event::Failed(_)
                            )
                        }) {
                            generation_events
                                .push(ba::Event::Failed("Amoo 创作连接已结束，请重试".into()));
                        }
                        break;
                    }
                }
            }
        }
        for event in generation_events {
            changed = true;
            match event {
                ba::Event::Status(status) => self.back_screen.generator_status = status,
                ba::Event::Complete(generated) => {
                    self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                    self.back_screen.generating = false;
                    self.back_screen.generation = None;
                    self.back_screen.active_device = None;
                    self.back_screen.config.brightness = generated.brightness;
                    self.back_screen.config.fps = generated.fps;
                    self.back_screen.config.duration_secs = generated.duration_secs;
                    self.back_screen.native_feature = None;
                    self.back_screen.load_gallery();
                    let result = if let Some(path) = generated.image_path {
                        self.back_screen.apply_image(path, "Amoo 生成图片")
                    } else if let Some(feature) = generated
                        .native_feature
                        .as_deref()
                        .and_then(native_from_key)
                    {
                        self.back_screen.native_feature = Some(feature);
                        self.back_screen.image_path = None;
                        self.back_screen.title = feature.label().into();
                        self.back_screen.origin = "Amoo 编排 · Android 联动".into();
                        self.back_screen.source = bs::Source::Preset(match feature {
                            bs::NativeFeature::Clock => bs::Preset::Clock,
                            bs::NativeFeature::Camera => bs::Preset::Countdown,
                            bs::NativeFeature::Notifications => bs::Preset::Heart,
                        });
                        self.back_screen.reset_preview();
                        Ok(())
                    } else if let Some(preset) =
                        generated.preset.as_deref().and_then(bs::Preset::from_key)
                    {
                        self.back_screen.source = if let Some(package) = &generated.app_package {
                            bs::Source::App {
                                package: package.clone(),
                                preset,
                            }
                        } else {
                            bs::Source::Preset(preset)
                        };
                        self.back_screen.image_path = None;
                        self.back_screen.title = preset.label().into();
                        self.back_screen.origin = "Amoo 编排".into();
                        self.back_screen.reset_preview();
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Amoo 未返回可播放的图案或图片"))
                    };
                    match result {
                        Ok(()) => {
                            self.back_screen.description = generated.summary.clone();
                            self.back_screen.generator_status = generated.summary;
                            if !self.back_screen.converting_image {
                                self.back_screen.status =
                                    "Amoo 方案已生成，检查预览后应用到手机".into();
                            }
                        }
                        Err(error) => {
                            self.back_screen.error = Some(format!("{error:#}"));
                            self.back_screen.generator_status = format!("{error:#}");
                            self.back_screen.status = "方案转换失败，未发送到背屏".into();
                        }
                    }
                }
                ba::Event::Failed(error) => {
                    self.back_screen.generating = false;
                    self.back_screen.generation = None;
                    self.back_screen.active_device = None;
                    self.back_screen.error = Some(error.clone());
                    self.back_screen.generator_status = error;
                }
                ba::Event::Cancelled => {
                    self.back_screen.generating = false;
                    self.back_screen.generation = None;
                    self.back_screen.active_device = None;
                    self.back_screen.generator_status = "已取消生成".into();
                }
            }
        }
        if self.back_screen.pomodoro_active {
            let remaining = self.back_screen.pomodoro_remaining_secs();
            if remaining == 0 {
                let total = self.back_screen.pomodoro_duration_secs.max(1);
                if matches!(
                    self.back_screen.source,
                    bs::Source::Preset(bs::Preset::Countdown)
                ) {
                    self.back_screen.config.duration_secs = total;
                    self.back_screen.elapsed_ms = u64::from(total) * 1000;
                    self.back_screen.refresh_frame();
                    self.back_screen.applied_frame = self.back_screen.frame.clone();
                }
                self.back_screen.pomodoro_active = false;
                self.back_screen.cancel_pomodoro_commands();
                self.back_screen.pomodoro_started_at = None;
                self.back_screen.pomodoro_last_remaining_secs = None;
                self.back_screen.status = format!(
                    "{} 分钟桌面专注番茄钟已完成",
                    self.back_screen.pomodoro_duration_secs / 60
                );
                changed = true;
            } else if self.back_screen.pomodoro_last_remaining_secs != Some(remaining) {
                self.back_screen.pomodoro_last_remaining_secs = Some(remaining);
                let total = self.back_screen.pomodoro_duration_secs.max(1);
                let elapsed_secs = total.saturating_sub(remaining);
                let elapsed_ms = u64::from(elapsed_secs) * 1000;
                if matches!(
                    self.back_screen.source,
                    bs::Source::Preset(bs::Preset::Countdown)
                ) {
                    self.back_screen.config.duration_secs = total;
                    self.back_screen.elapsed_ms = elapsed_ms;
                    self.back_screen.refresh_frame();
                    self.back_screen.applied_frame = self.back_screen.frame.clone();
                }
                if !self.demo_mode
                    && self.back_screen.pomodoro_request.is_none()
                    && remaining % 5 == 0
                {
                    if let Some(device) = self.selected.clone() {
                        let mut cfg = self.back_screen.config.clone();
                        cfg.duration_secs = total;
                        let frame = bs::frame(
                            &bs::Source::Preset(bs::Preset::Countdown),
                            &cfg,
                            Duration::from_millis(elapsed_ms),
                            &bs::LiveData::default(),
                        );
                        let ticket = self.back_screen.pomodoro_lane.current();
                        let lane = self.back_screen.pomodoro_lane.clone();
                        std::thread::spawn(move || {
                            let _ = lane.run_if_current(ticket, || {
                                if let Ok(adb) = Adb::discover() {
                                    let _ = bs::push_frame(&adb, &device, &frame);
                                }
                            });
                        });
                    }
                }
                changed = true;
            }
        }
        if self.page == Page::BackScreen
            && self.back_screen.preview_running
            && !(self.back_screen.pomodoro_active
                && matches!(
                    self.back_screen.source,
                    bs::Source::Preset(bs::Preset::Countdown)
                ))
        {
            let duration = if self.back_screen.tab == 4 {
                20_000
            } else if let bs::Source::SmartCarousel(slides) = &self.back_screen.source {
                if slides.is_empty() {
                    4_000
                } else {
                    (slides.len() as u64 * bs::SMART_SLIDE_PROGRESS_CELLS as u64 * 1000).max(10_000)
                }
            } else {
                u64::from(self.back_screen.config.duration_secs.max(1)) * 1000
            };
            self.back_screen.elapsed_ms =
                self.back_screen.started.elapsed().as_millis() as u64 % duration;
            let previous = self.back_screen.frame.clone();
            self.back_screen.refresh_frame();
            changed |= self.back_screen.tab == 4 || previous != self.back_screen.frame;
        }
        changed
    }

    pub(super) fn ensure_calendar_state_scope(&mut self) {
        let scope = if self.demo_mode { Some("demo".to_string()) }
            else { self.selected.as_ref().filter(|d| !d.serial.is_empty()).map(|d| format!("phone:{}", d.serial)) };
        if self.back_screen.calendar_state_scope == scope { return; }
        self.back_screen.calendar_state_scope = scope;
        self.back_screen.load_completed_calendar_events();
        self.load_home_checked_checklist_items();
    }

    pub(super) fn calendar_event_action_limit(&self, id: i64) -> Option<&'static str> {
        let mut first_start = None;
        let mut missing_metadata = false;
        for event in self.back_screen.stepos_suite.calendar_events.iter().filter(|event| event.id == id) {
            if event.recurring == Some(true)
                || first_start.is_some_and(|start| start != event.dtstart) {
                return Some("这是重复日程，请在手机日历选择仅本次或整个系列；电脑未修改日程");
            }
            first_start = Some(event.dtstart);
            missing_metadata |= !self.demo_mode && event.recurring.is_none()
                && event.id > 0 && event.source.starts_with("手机");
        }
        missing_metadata.then_some("日程来源尚未提供重复规则，请先刷新日历；也可在手机日历处理，电脑未修改日程")
    }

    pub(super) fn calendar_event_is_completed(&self, event: &stepx_companion::toolbox::PhoneCalendarEvent) -> bool {
        self.back_screen.completed_calendar_events.contains(&event.id)
            && self.calendar_event_action_limit(event.id).is_none()
    }

    pub(super) fn calendar_event_is_suppressed(&self, event: &stepx_companion::toolbox::PhoneCalendarEvent, now_ms: i64) -> bool {
        self.calendar_event_action_limit(event.id).is_none()
            && event.is_suppressed_at(now_ms, &self.reminder_governance)
    }

    pub(super) fn migrate_calendar_event_state(&mut self, old_id: i64, new_id: i64) {
        self.ensure_calendar_state_scope();
        if self.back_screen.completed_calendar_events.remove(&old_id) {
            self.back_screen.completed_calendar_events.insert(new_id);
            self.back_screen.save_completed_calendar_events();
        }
        let prefix = format!("{old_id}:");
        let matching: Vec<String> = self
            .home_checked_checklist_items
            .iter()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        if !matching.is_empty() {
            for k in matching {
                self.home_checked_checklist_items.remove(&k);
                if let Some((_, item)) = k.split_once(':') {
                    self.home_checked_checklist_items
                        .insert(format!("{new_id}:{item}"));
                }
            }
            if let Err(error) = self.save_home_checked_checklist_items() {
                self.back_screen.stepos_suite_error = Some(format!("日程已更新，但清单状态迁移未保存：{error:#}"));
            }
        }
    }

    pub(super) fn remove_calendar_event_state(&mut self, event_id: i64) {
        self.ensure_calendar_state_scope();
        if self.back_screen.completed_calendar_events.remove(&event_id) {
            self.back_screen.save_completed_calendar_events();
        }
        let prefix = format!("{event_id}:");
        let before = self.home_checked_checklist_items.len();
        self.home_checked_checklist_items
            .retain(|k| !k.starts_with(&prefix));
        if self.home_checked_checklist_items.len() != before {
            if let Err(error) = self.save_home_checked_checklist_items() {
                self.back_screen.stepos_suite_error = Some(format!("日程已移除，但清单状态清理未保存：{error:#}"));
            }
        }
    }

    pub(super) fn reconcile_calendar_event_states_from_descriptions(&mut self) {
        self.ensure_calendar_state_scope();
        let mut completed_changed = false;
        let mut checklist_changed = false;
        for ev in &self.back_screen.stepos_suite.calendar_events {
            // Legacy metadata has no occurrence identity. Retain it on disk,
            // but never import one series-wide done flag into all its dates.
            if self.calendar_event_action_limit(ev.id).is_some() { continue; }
            for line in ev.description.lines() {
                let trimmed = line.trim();
                let Some(rest) = trimmed.strip_prefix(CALENDAR_SYNC_META_PREFIX) else {
                    continue;
                };
                let body = rest.strip_suffix('】').unwrap_or(rest);
                let Some((status, items)) = body.split_once("｜已勾清单=") else { continue; };
                let Some(done) = status.strip_prefix("日程状态=").and_then(|s| match s.trim() {
                    "已完成" => Some(true), "待办" => Some(false), _ => None,
                }) else { continue; };
                let items = items.trim();
                // JSON preserves punctuation and the delimiter inside a real
                // checklist label. Legacy delimiter-separated phone metadata
                // remains readable, including all items rather than only one.
                let parsed: Option<Vec<String>> = if items.starts_with('[') { serde_json::from_str(items).ok() }
                    else { Some(items.split('｜').filter(|s| !s.trim().is_empty() && s.trim() != "无").map(str::to_string).collect()) };
                let Some(items) = parsed else { continue; };
                completed_changed |= if done { self.back_screen.completed_calendar_events.insert(ev.id) }
                    else { self.back_screen.completed_calendar_events.remove(&ev.id) };
                let prefix = format!("{}:", ev.id);
                let next: std::collections::BTreeSet<_> = items.iter().filter(|s| !s.trim().is_empty())
                    .map(|item| Self::checklist_item_state_key(ev.id, item)).collect();
                if self.home_checked_checklist_items.iter().filter(|key| key.starts_with(&prefix)).count() != next.len()
                    || next.iter().any(|key| !self.home_checked_checklist_items.contains(key)) {
                    self.home_checked_checklist_items.retain(|key| !key.starts_with(&prefix));
                    self.home_checked_checklist_items.extend(next);
                    checklist_changed = true;
                }
            }
        }
        if completed_changed {
            self.back_screen.save_completed_calendar_events();
        }
        if checklist_changed {
            if let Err(error) = self.save_home_checked_checklist_items() {
                self.back_screen.stepos_suite_error = Some(format!("手机日程已读取，但清单状态未保存：{error:#}"));
            }
        }
    }

    pub(super) fn sync_calendar_event_state_to_description(&mut self, event_id: i64) {
        if let Some(reason) = self.calendar_event_action_limit(event_id) {
            self.status = reason.into();
            self.error = true;
            return;
        }
        self.ensure_calendar_state_scope();
        if self.back_screen.calendar_state_scope.is_none() { return; }
        let event_done = self
            .back_screen
            .completed_calendar_events
            .contains(&event_id);
        let prefix = format!("{event_id}:");
        let mut checked_items: Vec<String> = self
            .home_checked_checklist_items
            .iter()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
            .collect();
        checked_items.sort();

        let Some(ev) = self
            .back_screen
            .stepos_suite
            .calendar_events
            .iter_mut()
            .find(|e| e.id == event_id)
        else {
            return;
        };

        let base_lines: Vec<&str> = ev
            .description
            .lines()
            .filter(|line| !line.trim().starts_with(CALENDAR_SYNC_META_PREFIX))
            .collect();
        let base_desc = base_lines.join("\n").trim().to_string();

        let new_desc = {
            let meta_line = format!(
                "{CALENDAR_SYNC_META_PREFIX}日程状态={}｜已勾清单={}】",
                if event_done { "已完成" } else { "待办" },
                serde_json::to_string(&checked_items).expect("checklist strings serialize")
            );
            if base_desc.is_empty() {
                meta_line
            } else {
                format!("{base_desc}\n{meta_line}")
            }
        };

        ev.description = new_desc.clone();
        if self.prefs.auto_sync_calendar {
            rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
        }
        if !self.demo_mode && event_id > 0 {
            if let Some(device) = self.selected.clone() {
                std::thread::spawn(move || {
                    if let Ok(adb) = Adb::discover() {
                        let _ = stepx_companion::toolbox::update_phone_calendar_event_description(&adb, &device, event_id, &new_desc);
                    }
                });
            }
        }
    }

    pub(super) fn active_smart_preview_slides(&self) -> Vec<bs::SmartDisplaySlide> {
        let kind = self.back_screen.smart_display_preview_kind;
        let count = self.back_screen.smart_display_preview_count.max(1);
        let otp_code = self.back_screen.smart_display_preview_otp.trim().to_string();
        match self.back_screen.smart_display_preview_mode {
            0 => self.back_screen.smart_display_live_slides.clone(),
            1 => vec![bs::SmartDisplaySlide::AppNotification {
                kind,
                count,
                label: format!("{} · {}条", kind.label(), count),
            }],
            2 => self.back_screen.smart_display_live_slides.clone(),
            3 => {
                if otp_code.is_empty() {
                    Vec::new()
                } else {
                    vec![bs::SmartDisplaySlide::OtpCode {
                        code: otp_code.clone(),
                        label: format!("验证码 {otp_code}"),
                    }]
                }
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn apply_smart_preview_to_canvas(&mut self) {
        let slides = self.active_smart_preview_slides();
        self.back_screen.native_feature = None;
        self.back_screen.image_path = None;
        self.back_screen.source = bs::Source::SmartCarousel(slides.clone());
        self.back_screen.config.fps = if slides.is_empty() { 6 } else { 1 };
        self.back_screen.config.duration_secs = if slides.is_empty() {
            15
        } else {
            (slides.len() as u32 * bs::SMART_SLIDE_PROGRESS_CELLS as u32).max(10)
        };
        if slides.is_empty() {
            self.back_screen.title = "旋转太空人".into();
            self.back_screen.description = "无通知时手机自主待机播放（断开电脑仍生效）".into();
            self.back_screen.origin = "智能显示 · 手机自主联动".into();
            self.back_screen.status = "手机伴侣已自主接管背屏通知轮播，也可点击预览应用到手机".into();
        } else if slides.len() == 1 {
            match &slides[0] {
                bs::SmartDisplaySlide::AppNotification { kind, count, .. } => {
                    let c = (*count).max(1);
                    self.back_screen.title =
                        format!("智能显示 · {} ({}条新消息)", kind.label(), c);
                    self.back_screen.description =
                        format!("{} · {} 条（手机端自主驱动背屏，断开电脑仍生效）", kind.label(), c);
                    self.back_screen.origin = "智能显示 · 手机自主联动".into();
                    self.back_screen.status = "手机伴侣已自主接管背屏通知轮播，也可点击预览应用到手机".into();
                }
                bs::SmartDisplaySlide::OtpCode { code, .. } => {
                    self.back_screen.title = format!("验证码 {code}");
                    self.back_screen.description = "10 秒倒计时（手机端自主驱动背屏）".into();
                    self.back_screen.origin = "智能显示 · 手机自主联动".into();
                    self.back_screen.status = "手机伴侣已自主接管背屏通知轮播，也可点击预览应用到手机".into();
                }
            }
        } else {
            let badges: Vec<String> = slides.iter().map(|s| s.short_badge()).collect();
            self.back_screen.title = format!("智能显示 · 轮播 {} 项", slides.len());
            self.back_screen.description =
                format!("{}（手机端自主驱动，断开电脑仍生效）", badges.join(" · "));
            self.back_screen.origin = "智能显示 · 手机自主联动".into();
            self.back_screen.status = "手机伴侣已自主接管背屏通知轮播，也可点击预览应用到手机".into();
        }
        self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
        self.back_screen.reset_preview();
        self.back_screen.preview_running = true;
    }

    pub(super) fn push_current_smart_preview_to_phone(&mut self) {
        if self.demo_mode {
            return;
        }
        let Some(device) = self.selected.clone() else {
            return;
        };
        let Ok(adb) = Adb::discover() else {
            return;
        };
        let slides = self.active_smart_preview_slides();
        let brightness = self.back_screen.config.brightness;
        let invert = self.back_screen.config.invert;
        let title = self.back_screen.title.clone();
        self.back_screen.smart_display_last_pushed_slides = Some(slides.clone());
        self.back_screen.smart_display_active_on_phone = !slides.is_empty();
        self.back_screen.applied_to_phone = true;
        self.back_screen.applied_title = title.clone();
        self.back_screen.applied_status = self.back_screen.description.clone();
        std::thread::spawn(move || {
            let _ = bs::push_smart_carousel_to_phone(&adb, &device, &slides, brightness, invert);
        });
    }

    pub(super) fn compute_smart_backscreen_slides(
        &mut self,
    ) -> (Vec<bs::SmartDisplaySlide>, String) {
        use super::notification_center_state::{
            NotificationAlertMode, NotificationPopupWidget, NotificationVisualKind,
            UnifiedNotificationItem,
        };
        if self.notification_center.alert_mode == NotificationAlertMode::Muted
            || self.push.prefs.scope == stepx_companion::desktop_notify::AlertScope::Muted
            || self
                .notification_center
                .is_snoozed(chrono::Utc::now().timestamp_millis())
        {
            self.back_screen.smart_display_preview_otp.clear();
            return (Vec::new(), "无未读通知 · 旋转太空人".to_string());
        }

        for item in &self.notification_center.items {
            if item.unread {
                self.back_screen
                    .smart_display_seen_unread_ids
                    .insert(item.id.clone());
                self.back_screen.smart_display_cleared_ids.remove(&item.id);
            } else if self
                .back_screen
                .smart_display_seen_unread_ids
                .contains(&item.id)
            {
                self.back_screen
                    .smart_display_cleared_ids
                    .insert(item.id.clone());
            }
        }

        let prefs = &self.push.prefs;
        let read_ids = &self.notification_center.read_ids;
        let is_eligible_item = |item: &UnifiedNotificationItem,
                                cleared: &std::collections::HashSet<String>,
                                dismissed: &std::collections::HashSet<String>,
                                cap_dismissed: &std::collections::HashSet<String>|
         -> bool {
            if prefs.effective_popup_mode(&item.package_name)
                == stepx_companion::desktop_notify::PopupMode::Silent
            {
                return false;
            }
            !read_ids.contains(&item.id)
                && !cleared.contains(&item.id)
                && !dismissed.contains(&item.id)
                && !cap_dismissed.contains(&item.id)
                && !matches!(
                    item.visual,
                    NotificationVisualKind::AppUpdate
                        | NotificationVisualKind::TaskReady
                        | NotificationVisualKind::DeviceLink
                        | NotificationVisualKind::CompanionInfo
                )
        };

        // Strictly use genuinely unread items (`item.unread == true`). Never fabricate from read history!
        let active_items: Vec<&UnifiedNotificationItem> = self
            .notification_center
            .items
            .iter()
            .filter(|item| {
                item.unread
                    && is_eligible_item(
                        item,
                        &self.back_screen.smart_display_cleared_ids,
                        &self.notification_center.dismissed_ids,
                        &self.back_screen.dismissed_capsule_ids,
                    )
            })
            .collect();

        let classify_item = |item: &UnifiedNotificationItem| -> bs::SmartMessageIconKind {
            match item.visual {
                NotificationVisualKind::WeChat => {
                    if item.source_badge == "企微" || item.package_name == "com.tencent.wework" {
                        bs::SmartMessageIconKind::WeWork
                    } else {
                        bs::SmartMessageIconKind::WeChat
                    }
                }
                NotificationVisualKind::Feishu => bs::SmartMessageIconKind::Feishu,
                NotificationVisualKind::DingTalk => bs::SmartMessageIconKind::DingTalk,
                NotificationVisualKind::SmsOtp => bs::SmartMessageIconKind::SmsOrOtp,
                NotificationVisualKind::MissedCall => bs::SmartMessageIconKind::PhoneCall,
                NotificationVisualKind::Express => bs::SmartMessageIconKind::Delivery,
                NotificationVisualKind::CalendarAlarm => bs::SmartMessageIconKind::GeneralMessage,
                _ => bs::SmartMessageIconKind::classify_from_source(
                    &item.source_badge,
                    &item.title,
                ),
            }
        };

        let mut candidates: Vec<(bs::SmartMessageIconKind, String, Option<String>)> = Vec::new();
        let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. Collect from genuinely unread notification_center items
        for item in &active_items {
            let kind = classify_item(item);
            let otp = match &item.popup_widget {
                NotificationPopupWidget::OtpCode { code, .. } if !code.trim().is_empty() => {
                    Some(code.trim().to_string())
                }
                _ => stepx_companion::push::extract_otp_code(&format!(
                    "{} {}",
                    item.title, item.summary
                )),
            };
            seen_keys.insert(format!("{:?}::{}", kind, item.title.trim()));
            candidates.push((kind, item.title.clone(), otp));
        }

        // 2. Also collect from genuinely active, unread, non-dismissed, non-ongoing phone statusbar notifications
        if !cfg!(test) {
            for notif in &self.push.snapshot.notifications {
                if !notif.active
                    || notif.dismissed
                    || notif.read
                    || notif.removed_time_ms > 0
                    || notif.is_ongoing
                    || self.push.prefs.effective_popup_mode(&notif.package_name)
                        == stepx_companion::desktop_notify::PopupMode::Silent
                {
                    continue;
                }
                let raw_key = format!("raw-notif:{}", notif.key);
                if self.back_screen.smart_display_cleared_ids.contains(&raw_key) {
                    continue;
                }
                let pkg = notif.package_name.trim();
                if matches!(
                    pkg,
                    "android"
                        | "com.android.systemui"
                        | "com.stepx.companion.app"
                        | "com.android.vending"
                        | "com.android.chrome"
                        | "com.android.alarmclock"
                        | "com.stepos.appstore"
                        | "com.stepos.ailauncher"
                ) || stepx_companion::push::is_system_or_lockscreen_title(&notif.title)
                {
                    continue;
                }
                let kind = bs::SmartMessageIconKind::classify_from_source(
                    &format!("{} {}", notif.package_name, notif.app_name),
                    &format!("{} {}", notif.title, notif.text),
                );
                let otp = notif
                    .otp_code
                    .clone()
                    .filter(|c| !c.trim().is_empty())
                    .or_else(|| {
                        stepx_companion::push::extract_otp_code(&format!(
                            "{} {}",
                            notif.title, notif.text
                        ))
                    });
                let dedup = format!("{:?}::{}", kind, notif.title.trim());
                if seen_keys.insert(dedup) {
                    candidates.push((kind, notif.title.clone(), otp));
                }
            }
        }

        let slides = bs::build_prioritized_smart_slides(&candidates);
        self.back_screen.smart_display_preview_otp = slides
            .iter()
            .find_map(|s| match s {
                bs::SmartDisplaySlide::OtpCode { code, .. } => Some(code.clone()),
                _ => None,
            })
            .unwrap_or_default();

        let summary = if slides.is_empty() {
            "无未读通知 · 旋转太空人".to_string()
        } else {
            let badges: Vec<String> = slides.iter().map(|s| s.short_badge()).collect();
            badges.join(" · ")
        };

        (slides, summary)
    }

    pub(super) fn compute_smart_backscreen_target(
        &self,
    ) -> Option<(bs::SmartMessageIconKind, u32, String)> {
        use super::notification_center_state::{NotificationAlertMode, NotificationVisualKind};
        if self.notification_center.alert_mode == NotificationAlertMode::Muted
            || self.push.prefs.scope == stepx_companion::desktop_notify::AlertScope::Muted
            || self
                .notification_center
                .is_snoozed(chrono::Utc::now().timestamp_millis())
        {
            return None;
        }
        if self.back_screen.smart_display_live_slides.is_empty() {
            let unread_count = self
                .notification_center
                .items
                .iter()
                .filter(|item| {
                    item.unread
                        && !self.notification_center.read_ids.contains(&item.id)
                        && self.push.prefs.effective_popup_mode(&item.package_name)
                            != stepx_companion::desktop_notify::PopupMode::Silent
                        && !matches!(
                            item.visual,
                            NotificationVisualKind::AppUpdate
                                | NotificationVisualKind::TaskReady
                                | NotificationVisualKind::DeviceLink
                                | NotificationVisualKind::CompanionInfo
                        )
                })
                .count() as u32;
            if unread_count == 0 {
                return None;
            }
            return Some((
                self.back_screen.smart_display_live_kind,
                unread_count,
                self.back_screen.smart_display_live_summary.clone(),
            ));
        }
        Some((
            self.back_screen.smart_display_live_kind,
            self.back_screen.smart_display_live_count.max(1),
            self.back_screen.smart_display_live_summary.clone(),
        ))
    }

    pub(super) fn sync_smart_backscreen_display(&mut self) {
        let (slides, summary) = self.compute_smart_backscreen_slides();
        self.back_screen.smart_display_live_slides = slides.clone();
        if !slides.is_empty() {
            let total_count: u32 = slides
                .iter()
                .map(|s| match s {
                    bs::SmartDisplaySlide::AppNotification { count, .. } => (*count).max(1),
                    bs::SmartDisplaySlide::OtpCode { .. } => 1,
                })
                .sum();
            let primary_kind = slides
                .iter()
                .find_map(|s| match s {
                    bs::SmartDisplaySlide::AppNotification { kind, .. } => Some(*kind),
                    bs::SmartDisplaySlide::OtpCode { .. } => None,
                })
                .unwrap_or(bs::SmartMessageIconKind::SmsOrOtp);
            self.back_screen.smart_display_live_kind = primary_kind;
            self.back_screen.smart_display_live_count = total_count.max(1);
            self.back_screen.smart_display_live_summary = summary.clone();
        } else {
            self.back_screen.smart_display_live_count = 0;
            self.back_screen.smart_display_live_summary = if self.back_screen.smart_display_enabled
                && self.back_screen.auto_notify_flash
            {
                summary.clone()
            } else {
                "已关闭".into()
            };
        }

        let autonomous_cfg = (
            self.back_screen.smart_display_enabled,
            self.back_screen.auto_notify_flash,
            self.back_screen.config.brightness,
            self.back_screen.config.invert,
        );
        if !self.demo_mode {
            if let Some(device) = self.selected.clone() {
                if self.back_screen.last_synced_autonomous_cfg != Some(autonomous_cfg) {
                    self.back_screen.last_synced_autonomous_cfg = Some(autonomous_cfg);
                    if let Ok(adb) = Adb::discover() {
                        std::thread::spawn(move || {
                            let _ = bs::sync_phone_autonomous_smart_backscreen(
                                &adb,
                                &device,
                                autonomous_cfg.0,
                                autonomous_cfg.1,
                                autonomous_cfg.2,
                                autonomous_cfg.3,
                            );
                        });
                    }
                }
            }
        }

        if !self.back_screen.smart_display_enabled || !self.back_screen.auto_notify_flash {
            if self.back_screen.smart_display_active_on_phone {
                self.back_screen.smart_display_active_on_phone = false;
                self.back_screen.smart_display_last_pushed = None;
                self.back_screen.smart_display_last_pushed_slides = None;
            }
            return;
        }
        if self.back_screen.pomodoro_active || self.back_screen.playing {
            return;
        }

        if !slides.is_empty() {
            let primary_kind = self.back_screen.smart_display_live_kind;
            let total_count = self.back_screen.smart_display_live_count;
            if self.back_screen.tab == 5 && self.back_screen.smart_display_preview_mode == 0 {
                self.back_screen.smart_display_preview_kind = primary_kind;
                self.apply_smart_preview_to_canvas();
            }
            if self.back_screen.smart_display_last_pushed_slides.as_ref() == Some(&slides) {
                return;
            }
            self.back_screen.smart_display_last_pushed = Some((primary_kind, total_count));
            self.back_screen.smart_display_last_pushed_slides = Some(slides.clone());
            self.back_screen.smart_display_active_on_phone = true;
            self.back_screen.applied_to_phone = true;
            self.back_screen.applied_title = if slides.len() == 1 {
                match &slides[0] {
                    bs::SmartDisplaySlide::AppNotification { kind, count, .. } => {
                        format!("智能显示 · {} ({}条新消息)", kind.label(), (*count).max(1))
                    }
                    bs::SmartDisplaySlide::OtpCode { code, .. } => {
                        format!("验证码 {code}")
                    }
                }
            } else {
                format!(
                    "智能显示 · 轮播 {} 项 ({}条新消息)",
                    slides.len(),
                    total_count
                )
            };
            self.back_screen.applied_status =
                format!("{summary} · 手机端伴侣自主驱动背屏（断开电脑仍生效）");
        } else {
            if self.back_screen.tab == 5 && self.back_screen.smart_display_preview_mode == 0 {
                self.apply_smart_preview_to_canvas();
            }
            if self.back_screen.smart_display_active_on_phone {
                self.back_screen.smart_display_active_on_phone = false;
                self.back_screen.smart_display_last_pushed = None;
                self.back_screen.smart_display_last_pushed_slides = None;
                self.back_screen.applied_title = "旋转太空人".into();
                self.back_screen.applied_status =
                    "无通知自动待机 · 手机端自主接管（断开电脑仍生效）".into();
            }
        }
    }
}

pub(super) const CALENDAR_SYNC_META_PREFIX: &str =
    "【双端日程协同状态同步记录｜由桌面伴侣自动维护此状态标记行请勿手动删除或拆分｜跨端日程与清单完成度实时同步元数据标记行｜";

pub(super) fn persist_calendar_state(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| anyhow::anyhow!("日程状态路径无效"))?;
    std::fs::create_dir_all(parent)?;
    anyhow::ensure!(!parent.is_symlink() && !path.is_symlink(), "日程状态路径不能是链接");
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

fn editable_calendar_time_label(ev: &stepx_companion::toolbox::PhoneCalendarEvent) -> String {
    use chrono::TimeZone;
    if ev.all_day {
        return "全天".to_string();
    }
    let start_hm = ev.hm_label();
    if ev.dtend > ev.dtstart {
        let end_hm = chrono::Local
            .timestamp_millis_opt(ev.dtend)
            .single()
            .map(|dt| dt.format("%H:%M").to_string())
            .unwrap_or_default();
        if !end_hm.is_empty() && end_hm != start_hm {
            return format!("{start_hm}-{end_hm}");
        }
    }
    start_hm
}

fn normalize_calendar_time_input(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "全天" || trimmed.eq_ignore_ascii_case("allday") {
        return trimmed.to_string();
    }
    // Strip optional date/day prefix such as "今天 14:00–15:30" or "10月31日 09:30"
    if let Some((_, tail)) = trimmed.rsplit_once(' ') {
        if tail.contains(':') || tail.contains('：') {
            return tail.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn rotate_dynamic_image_by_deg(img: image::DynamicImage, deg: u16) -> image::DynamicImage {
    match deg % 360 {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    }
}

fn save_image_matching_path_format(
    img: &image::DynamicImage,
    path: &std::path::Path,
) -> image::ImageResult<()> {
    let is_jpeg = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
        .unwrap_or(false);
    if is_jpeg {
        img.to_rgb8().save(path)
    } else {
        img.save(path)
    }
}

fn parse_bridge_calendar_events(
    val: &serde_json::Value,
) -> Option<Vec<stepx_companion::toolbox::PhoneCalendarEvent>> {
    val.get("calendar_events")?.as_array()?;
    Some(stepx_companion::toolbox::parse_bridge_calendar_events(val))
}

fn parse_bridge_alarms(
    val: &serde_json::Value,
) -> Option<Vec<stepx_companion::toolbox::PhoneAlarmItem>> {
    let arr = val.get("alarms")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let message = item
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if message.contains("StepX自动化验证")
            || message.contains("StepX E2E")
            || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&message)
        {
            continue;
        }
        out.push(stepx_companion::toolbox::PhoneAlarmItem {
            id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
            hour: item.get("hour").and_then(|v| v.as_u64()).unwrap_or(8) as u8,
            minutes: item.get("minutes").and_then(|v| v.as_u64()).unwrap_or(0) as u8,
            enabled: item
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            days_of_week: item
                .get("days_of_week")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            vibrate: item
                .get("vibrate")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            message,
        });
    }
    Some(out)
}

fn create_phone_calendar_event_preserving_details(
    adb: &Adb,
    device: &Device,
    title: &str,
    start_ms: i64,
    end_ms: i64,
    all_day: bool,
    location: &str,
    description: &str,
    sync_desktop_calendar: bool,
    y: i32,
    m: u32,
    d: u32,
) -> anyhow::Result<(
    i64,
    Vec<stepx_companion::toolbox::PhoneCalendarEvent>,
    String,
)> {
    let clean_title = if title.trim().is_empty() {
        "StepX 桌面新建日程"
    } else {
        title.trim()
    };
    let clean_desc = if description.trim().is_empty() {
        "由 StepX 桌面端直接同步创建"
    } else {
        description.trim()
    };
    let clean_loc = location.trim();

    let desktop_saved = if sync_desktop_calendar {
        stepx_companion::toolbox::save_desktop_calendar_event(
            clean_title,
            start_ms,
            end_ms,
            clean_desc,
        )
        .is_ok()
    } else {
        false
    };

    let req = serde_json::json!({
        "op": "calendar_create",
        "title": clean_title,
        "description": clean_desc,
        "location": clean_loc,
        "start_ms": start_ms,
        "end_ms": end_ms,
        "all_day": all_day,
    });
    let (created_id, mut events) = match stepx_companion::bridge::call_bridge(
        adb,
        device,
        &req,
        Duration::from_secs(10),
    ) {
        Ok(val)
            if val
                .get("created_id")
                .or_else(|| val.get("id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                > 0
                || val.get("calendar_events").is_some() =>
        {
            let id = val
                .get("created_id")
                .or_else(|| val.get("id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let evs = parse_bridge_calendar_events(&val).unwrap_or_else(|| {
                stepx_companion::toolbox::inspect_stepos_ai_suite(adb, device)
                    .map(|s| s.calendar_events)
                    .unwrap_or_default()
            });
            (id, evs)
        }
        _ => {
            let (id, evs, _) = stepx_companion::toolbox::create_phone_calendar_event(
                adb,
                device,
                clean_title,
                start_ms,
                end_ms,
                clean_desc,
            )?;
            (id, evs)
        }
    };

    if created_id > 0 && (!clean_loc.is_empty() || all_day) {
        if let Ok(transport) = device.resolve_active_transport(adb) {
            let mut args = vec![
                "content".to_string(),
                "update".to_string(),
                "--uri".to_string(),
                format!("content://com.android.calendar/events/{created_id}"),
            ];
            if !clean_loc.is_empty() {
                let safe_loc = clean_loc.replace('\'', "'\\''");
                args.push("--bind".to_string());
                args.push(format!("eventLocation:s:'{safe_loc}'"));
            }
            if all_day {
                args.push("--bind".to_string());
                args.push("allDay:i:1".to_string());
            }
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = adb.shell(&transport, &arg_refs);
        }
    }

    if let Some(ev) = events.iter_mut().find(|e| {
        (created_id > 0 && e.id == created_id)
            || (e.title == clean_title && (e.dtstart - start_ms).abs() < 60_000)
    }) {
        ev.all_day = all_day;
        if ev.location.trim().is_empty() && !clean_loc.is_empty() {
            ev.location = clean_loc.to_string();
        }
        if ev.description.trim().is_empty() && !clean_desc.is_empty() {
            ev.description = clean_desc.to_string();
        }
    } else {
        events.push(stepx_companion::toolbox::PhoneCalendarEvent {
            recurring: Some(false),
            id: if created_id > 0 {
                created_id
            } else {
                start_ms / 1000
            },
            title: clean_title.to_string(),
            dtstart: start_ms,
            dtend: end_ms,
            all_day,
            location: clean_loc.to_string(),
            description: clean_desc.to_string(),
            source: if sync_desktop_calendar {
                "双端已同步".to_string()
            } else {
                "手机日历".to_string()
            },
        });
        events.sort_by_key(|e| e.dtstart);
    }

    let note = if desktop_saved {
        format!("已在手机与电脑日历同步保存 {y}-{m:02}-{d:02} 日程：「{clean_title}」")
    } else {
        format!("已在手机日历保存 {y}-{m:02}-{d:02} 日程：「{clean_title}」")
    };
    Ok((created_id, events, note))
}

fn create_phone_alarm_preserving_details(
    adb: &Adb,
    device: &Device,
    hour: u8,
    minutes: u8,
    label: &str,
    days_of_week: u32,
    vibrate: bool,
) -> anyhow::Result<(i64, Vec<stepx_companion::toolbox::PhoneAlarmItem>, String)> {
    let clean_label = if label.trim().is_empty() {
        "StepX 桌面提醒"
    } else {
        label.trim()
    };

    let req = serde_json::json!({
        "op": "alarm_create",
        "hour": hour.clamp(0, 23),
        "minutes": minutes.clamp(0, 59),
        "days_of_week": days_of_week,
        "vibrate": vibrate,
        "label": clean_label,
    });
    let (created_id, mut alarms) = match stepx_companion::bridge::call_bridge(
        adb,
        device,
        &req,
        Duration::from_secs(10),
    ) {
        Ok(val)
            if val
                .get("created_id")
                .or_else(|| val.get("id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                > 0
                || val.get("alarms").is_some() =>
        {
            let id = val
                .get("created_id")
                .or_else(|| val.get("id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let items = parse_bridge_alarms(&val).unwrap_or_else(|| {
                stepx_companion::toolbox::inspect_stepos_ai_suite(adb, device)
                    .map(|s| s.alarms)
                    .unwrap_or_default()
            });
            (id, items)
        }
        _ => {
            let (id, items, _) = stepx_companion::toolbox::create_phone_alarm(
                adb,
                device,
                hour,
                minutes,
                clean_label,
            )?;
            (id, items)
        }
    };

    if created_id > 0 && (!vibrate || days_of_week != 0) {
        if let Ok(transport) = device.resolve_active_transport(adb) {
            for uri in [
                format!("content://com.android.alarmclock/alarms/{created_id}"),
                format!("content://com.android.deskclock/alarms/{created_id}"),
            ] {
                let mut args = vec![
                    "content".to_string(),
                    "update".to_string(),
                    "--uri".to_string(),
                    uri,
                ];
                if !vibrate {
                    args.push("--bind".to_string());
                    args.push("vibrate:i:0".to_string());
                }
                if days_of_week != 0 {
                    args.push("--bind".to_string());
                    args.push(format!("daysofweek:i:{days_of_week}"));
                }
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let _ = adb.shell(&transport, &arg_refs);
            }
        }
    }

    if let Some(existing) = alarms.iter_mut().find(|a| {
        (created_id > 0 && a.id == created_id)
            || (a.hour == hour.clamp(0, 23) && a.minutes == minutes.clamp(0, 59))
    }) {
        existing.days_of_week = days_of_week;
        existing.vibrate = vibrate;
        existing.enabled = true;
        if existing.message.trim().is_empty() {
            existing.message = clean_label.to_string();
        }
    } else {
        alarms.push(stepx_companion::toolbox::PhoneAlarmItem {
            id: if created_id > 0 { created_id } else { 1 },
            hour: hour.clamp(0, 23),
            minutes: minutes.clamp(0, 59),
            enabled: true,
            days_of_week,
            vibrate,
            message: clean_label.to_string(),
        });
        alarms.sort_by_key(|a| (a.hour as u16) * 60 + (a.minutes as u16));
    }

    Ok((
        created_id,
        alarms,
        format!(
            "已在手机时钟创建 {:02}:{:02} 闹钟：「{clean_label}」",
            hour.clamp(0, 23),
            minutes.clamp(0, 59)
        ),
    ))
}

fn native_from_key(key: &str) -> Option<bs::NativeFeature> {
    match key {
        "notifications" => Some(bs::NativeFeature::Notifications),
        "clock" => Some(bs::NativeFeature::Clock),
        "camera" => Some(bs::NativeFeature::Camera),
        _ => None,
    }
}
pub(super) fn native_description(feature: bs::NativeFeature) -> &'static str {
    match feature {
        bs::NativeFeature::Notifications => {
            "启用手机通知背屏显示，实际效果由 Android 通知到达时触发。"
        }
        bs::NativeFeature::Clock => "由手机系统显示时钟，断开桌面后仍由手机管理。",
        bs::NativeFeature::Camera => "启用系统相机背屏联动，拍摄提示由手机相机与固件触发。",
    }
}

fn recent_images() -> Vec<PathBuf> {
    let Some(base) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            stepx_companion::resources::user_home()
                .map(|home| PathBuf::from(home).join(".local/share"))
        })
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(base.join("stepx-companion/backscreen/generated")) else {
        return Vec::new();
    };
    let mut images: Vec<_> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
                return None;
            }
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            if !["png", "jpg", "jpeg", "webp", "gif"].contains(&extension.as_str()) {
                return None;
            }
            Some((metadata.modified().unwrap_or(std::time::UNIX_EPOCH), path))
        })
        .collect();
    images.sort_by(|a, b| b.0.cmp(&a.0));
    images.into_iter().take(24).map(|(_, path)| path).collect()
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn recurring_calendar_actions_cannot_modify_a_whole_series_or_reuse_legacy_done_flags() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                view.demo_mode = true;
                let start = chrono::Local::now().timestamp_millis() + 3_600_000;
                let series = stepx_companion::toolbox::PhoneCalendarEvent {
                    id: 71, title: "周期项目周会".into(), dtstart: start,
                    dtend: start + 3_600_000, recurring: Some(true), ..Default::default()
                };
                let ordinary = stepx_companion::toolbox::PhoneCalendarEvent {
                    id: 72, title: "单次评审".into(), recurring: Some(false), ..series.clone()
                };
                view.back_screen.stepos_suite.calendar_events = vec![series.clone(), ordinary];
                view.ensure_calendar_state_scope();
                view.back_screen.completed_calendar_events.insert(series.id);
                view.fields[F_PHONE_SCHEDULE_TITLE].set("保留原草稿");
                let date = series.date_ymd();
                for action in [
                    BackScreenAction::CalendarDelete(71), BackScreenAction::CalendarToggleDone(71),
                    BackScreenAction::CalendarEditEvent(71), BackScreenAction::CalendarPostponeOneDay(71),
                    BackScreenAction::CalendarDuplicateToTomorrow(71),
                    BackScreenAction::CalendarSendReminderToPhone(71),
                    BackScreenAction::CalendarClearDay(date.0, date.1, date.2),
                ] {
                    let before = view.back_screen.stepos_suite.calendar_events.clone();
                    assert!(view.do_backscreen_action(&action, ctx).is_err(), "{action:?}");
                    assert_eq!(view.back_screen.stepos_suite.calendar_events, before);
                    assert_eq!(view.fields[F_PHONE_SCHEDULE_TITLE].text, "保留原草稿");
                }
                assert!(view.back_screen.completed_calendar_events.contains(&71), "legacy storage is preserved");
                assert!(!view.calendar_event_is_completed(&series), "a series id cannot complete future occurrences");
                let spec = stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    "cal:71", stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                    "周期项目周会");
                view.reminder_governance.manual_expire(spec, start - 3_600_000);
                assert!(series.is_valid_unexpired_at(start - 1000,
                    &view.back_screen.completed_calendar_events, &view.reminder_governance));
                assert!(view.do_backscreen_action(&BackScreenAction::CalendarEditEvent(72), ctx).is_ok());
                assert_eq!(view.back_screen.editing_calendar_event_id, Some(72));
                assert_eq!(view.fields[F_PHONE_SCHEDULE_TITLE].text, "单次评审");
            });
        });
    }

    #[test]
    fn duplicate_instances_are_ambiguous_even_when_old_sources_have_no_rule_metadata() {
        let mut view = Companion::new_for_test();
        view.demo_mode = false;
        let event = stepx_companion::toolbox::PhoneCalendarEvent {
            id: 82, title: "项目周会".into(), dtstart: 1794016800000,
            source: "手机日历".into(), ..Default::default()
        };
        view.back_screen.stepos_suite.calendar_events = vec![event.clone()];
        assert!(view.calendar_event_action_limit(82).unwrap().contains("刷新"));
        view.back_screen.stepos_suite.calendar_events.push(stepx_companion::toolbox::PhoneCalendarEvent {
            dtstart: event.dtstart + 7 * 86_400_000, ..event
        });
        assert!(view.calendar_event_action_limit(82).unwrap().contains("重复日程"));
    }
    #[test]
    fn pomodoro_lane_skips_queued_start_and_stop_when_a_newer_request_exists() {
        let lane = PomodoroCommandLane::default();
        let first_start = lane.advance();
        let stop = lane.advance();
        assert!(lane.run_if_current(first_start, || panic!("stale start executed")).is_none());
        assert_eq!(lane.run_if_current(stop, || "stop"), Some("stop"));
        let pending_stop = lane.advance();
        let restart = lane.advance();
        assert!(lane.run_if_current(pending_stop, || panic!("stale stop executed")).is_none());
        assert_eq!(lane.run_if_current(restart, || "restart"), Some("restart"));
    }

    #[test]
    fn pomodoro_lane_serializes_inflight_start_before_stop_even_across_state_reset() {
        let state = BackScreenUiState::default();
        let old_lane = state.pomodoro_lane.clone();
        let start = old_lane.advance();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let start_order = order.clone();
        let starting = std::thread::spawn(move || old_lane.run_if_current(start, || {
            start_order.lock().unwrap().push("start");
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            start_order.lock().unwrap().push("start-finished");
        }));
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // The UI thread can invalidate the previous state and enqueue a stop while the
        // start worker holds the gate; neither state drop nor advance takes that mutex.
        drop(state);
        let next_state = BackScreenUiState::default();
        let next_lane = next_state.pomodoro_lane.clone();
        let stop = next_lane.advance();
        let stop_order = order.clone();
        let stopping = std::thread::spawn(move || next_lane.run_if_current(stop, || {
            stop_order.lock().unwrap().push("stop");
        }));
        release_tx.send(()).unwrap();
        assert_eq!(starting.join().unwrap(), Some(()));
        assert_eq!(stopping.join().unwrap(), Some(()));
        assert_eq!(*order.lock().unwrap(), vec!["start", "start-finished", "stop"]);
    }

    #[test]
    fn pomodoro_device_cancellation_and_state_drop_skip_pending_commands() {
        let mut state = BackScreenUiState::default();
        let lane = state.pomodoro_lane.clone();
        let pending = lane.advance();
        state.pomodoro_request = Some(Instant::now());
        state.pomodoro_device = Some("previous-phone".into());
        assert!(state.needs_poll());
        state.cancel_pomodoro_commands();
        assert!(state.pomodoro_request.is_none());
        assert!(state.pomodoro_device.is_none());
        assert!(lane.run_if_current(pending, || panic!("cancelled command executed")).is_none());
        let pending_after = lane.advance();
        drop(state);
        assert!(lane.run_if_current(pending_after, || panic!("old device command executed")).is_none());
    }

    #[test]
    fn offline_phone_mutations_preserve_snapshot_and_drafts() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = None;
        app.back_screen.stepos_suite.controls.brightness_percent = 73;
        app.back_screen.stepos_suite.controls.dnd_on = true;
        app.back_screen.editing_alarm_id = Some(42);
        app.fields[F_ALARM_LABEL].set("保留闹钟草稿");
        app.fields[F_PHONE_SCHEDULE_TITLE].set("保留日程草稿");
        for action in [
            BackScreenAction::TriggerMcpQuickAction("toggle_dnd".into()),
            BackScreenAction::FillOtpOnPhone("123456".into()),
            BackScreenAction::CalendarCreatePreset(0),
            BackScreenAction::CalendarDelete(42),
            BackScreenAction::CalendarCreateOnSelectedDate(Some("09:00")),
            BackScreenAction::CalendarClearDay(2026, 10, 29),
            BackScreenAction::CalendarDuplicateToTomorrow(42),
            BackScreenAction::CalendarPostponeOneDay(42),
            BackScreenAction::AlarmCreatePreset(0),
            BackScreenAction::AlarmCreateCustom,
            BackScreenAction::AlarmToggle(42, false),
            BackScreenAction::AlarmDelete(42),
            BackScreenAction::AlarmShiftMinutes(42, 10),
            BackScreenAction::AlarmToggleAll(false),
            BackScreenAction::SetBrightnessPercent(15),
            BackScreenAction::McpToggleDnd,
        ] {
            assert!(app.reject_offline_phone_action(&action), "{action:?}");
            assert!(app.error);
            assert!(app.status.contains("尚未执行"));
            assert_eq!(app.back_screen.stepos_suite.controls.brightness_percent, 73);
            assert!(app.back_screen.stepos_suite.controls.dnd_on);
            assert_eq!(app.back_screen.editing_alarm_id, Some(42));
            assert_eq!(app.fields[F_ALARM_LABEL].text, "保留闹钟草稿");
            assert_eq!(app.fields[F_PHONE_SCHEDULE_TITLE].text, "保留日程草稿");
        }
    }

    #[test]
    fn offline_local_timer_and_demo_controls_remain_available() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = None;
        assert!(!app.reject_offline_phone_action(&BackScreenAction::StartPomodoro(25)));
        assert!(!app.reject_offline_phone_action(&BackScreenAction::StopPomodoro));
        assert!(!app.reject_offline_phone_action(&BackScreenAction::AlarmEdit(42)));
        app.demo_mode = true;
        assert!(!app.reject_offline_phone_action(&BackScreenAction::SetBrightnessPercent(15)));
        assert!(!app.reject_offline_phone_action(&BackScreenAction::AlarmCreatePreset(0)));
    }

    #[test]
    fn pomodoro_stop_and_restart_reject_prior_operation_receipts() {
        let mut state = BackScreenUiState::default();
        let start = Instant::now();
        let stop = start + std::time::Duration::from_millis(1);
        let restart = stop + std::time::Duration::from_millis(1);
        state.pomodoro_request = Some(stop);
        assert!(!state.accept_pomodoro_reply(start, "phone-a", Some("phone-a")));
        assert_eq!(state.pomodoro_request, Some(stop));
        state.pomodoro_request = Some(restart);
        assert!(!state.accept_pomodoro_reply(stop, "phone-a", Some("phone-a")));
        assert_eq!(state.pomodoro_request, Some(restart));
        assert!(state.accept_pomodoro_reply(restart, "phone-a", Some("phone-a")));
        assert!(state.pomodoro_request.is_none());
        assert!(!state.accept_pomodoro_reply(restart, "phone-a", Some("phone-a")));
    }

    #[test]
    fn pomodoro_receipts_cannot_cross_device_disconnect_or_state_reset() {
        let request = Instant::now();
        let mut state = BackScreenUiState::default();
        state.pomodoro_request = Some(request);
        assert!(!state.accept_pomodoro_reply(request, "phone-a", Some("phone-b")));
        state.pomodoro_request = Some(request);
        assert!(!state.accept_pomodoro_reply(request, "phone-a", None));
        state = BackScreenUiState::default();
        assert!(!state.accept_pomodoro_reply(request, "phone-a", Some("phone-a")));
    }

    #[test]
    fn unopened_module_does_no_preview_or_gallery_work() {
        let state = BackScreenUiState::default();
        assert!(state.frame.is_empty());
        assert!(!state.preview_running);
        assert!(!state.needs_poll());
        assert!(state.recent_images.is_empty());
        assert!(state.gallery_rx.is_none());
    }
    #[test]
    fn leaving_freezes_preview_and_return_preserves_pause_choice() {
        let mut state = BackScreenUiState::default();
        state.set_preview_visible(true);
        assert_eq!(state.frame.len(), bs::SLOTS);
        assert!(state.preview_running);
        state.elapsed_ms = 1200;
        state.set_preview_visible(false);
        assert!(!state.preview_running);
        assert!(!state.needs_poll());
        state.set_preview_visible(true);
        assert!(state.preview_running);
        assert!(state.started.elapsed().as_millis() >= 1200);
        state.preview_running = false; // Explicit Pause button.
        state.set_preview_visible(false);
        state.set_preview_visible(true);
        assert!(!state.preview_running);
        assert_eq!(state.elapsed_ms, 1200);
    }
    #[test]
    fn hidden_preview_keeps_explicit_device_job_results() {
        let mut state = BackScreenUiState::default();
        let (_tx, rx) = mpsc::channel();
        state.native_rx = Some(rx);
        assert!(state.needs_poll());
        assert!(!state.preview_running);
    }
    #[test]
    fn default_stepos_suite_never_shows_fake_numbers_and_pomodoro_defaults_to_25m() {
        let mut state = BackScreenUiState::default();
        assert!(!state.stepos_suite.checked);
        assert_eq!(state.stepos_suite.wellbeing.screen_time_minutes, 0);
        assert_eq!(state.stepos_suite.wellbeing.unlock_count, 0);
        assert!(state.stepos_suite.wellbeing.top_apps.is_empty());
        assert!(state.stepos_suite.phone_clipboard.is_empty());
        assert_eq!(state.pomodoro_duration_secs, 25 * 60);
        state.pomodoro_active = true;
        state.pomodoro_started_at = Some(Instant::now());
        assert!(state.pomodoro_remaining_secs() >= 1498);
    }
    #[test]
    fn gallery_zoom_lightbox_grid_and_preset_filter_defaults() {
        let state = BackScreenUiState::default();
        assert_eq!(state.gallery_zoom_pct, 100);
        assert!(!state.gallery_lightbox);
        assert_eq!(state.gallery_grid_cols, 3);
        assert!(state.gallery_rotations.is_empty());
        assert_eq!(state.preset_filter, 0);
        assert!(state.editing_calendar_event_id.is_none());
        assert!(state.editing_alarm_id.is_none());
    }

    #[test]
    fn pomodoro_ticks_on_home_page_and_updates_remaining_secs_and_matrix_frame() {
        let mut app = Companion::new_for_test();
        app.page = Page::Home;
        app.back_screen.pomodoro_active = true;
        app.back_screen.pomodoro_duration_secs = 1500;
        app.back_screen.config.duration_secs = 1500;
        app.back_screen.pomodoro_started_at = Some(Instant::now());
        app.back_screen.pomodoro_last_remaining_secs = None;
        app.back_screen.source = bs::Source::Preset(bs::Preset::Countdown);
        assert!(app.poll_backscreen(false));
        let initial_frame = app.back_screen.frame.clone();
        assert_eq!(initial_frame.len(), bs::SLOTS);

        // Advance 65 seconds: Countdown matrix frame (MM:SS) must reflect 23:55 instead of staying at 24:59
        app.back_screen.pomodoro_started_at = Some(Instant::now() - Duration::from_secs(65));
        assert!(
            app.poll_backscreen(false),
            "poll_backscreen must return true when Pomodoro remaining seconds change even on Page::Home"
        );
        let rem = app
            .back_screen
            .pomodoro_last_remaining_secs
            .expect("remaining secs updated");
        assert!(
            (1433..=1435).contains(&rem),
            "expected ~1435 remaining secs, got {rem}"
        );
        assert_eq!(app.live_pomodoro_remaining_secs(), rem);
        assert_eq!(app.back_screen.frame.len(), bs::SLOTS);
        assert_ne!(
            app.back_screen.frame, initial_frame,
            "Countdown matrix frame must advance minutes/seconds as Pomodoro ticks"
        );

        // Even when visiting Page::BackScreen with preview_running = true, Pomodoro elapsed_ms must not be overwritten
        app.page = Page::BackScreen;
        app.back_screen.preview_running = true;
        let frame_before = app.back_screen.frame.clone();
        let elapsed_before = app.back_screen.elapsed_ms;
        app.poll_backscreen(false);
        assert_eq!(app.back_screen.elapsed_ms, elapsed_before);
        assert_eq!(app.back_screen.frame, frame_before);
    }

    #[test]
    fn calendar_done_and_checklist_items_persist_to_disk_and_write_back_to_event_description() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cfg_dir = tempfile::tempdir().unwrap();
        let _cfg_guard = super::super::TestUiConfigDirGuard::set(cfg_dir.path());

        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        let ev_id = 777i64;
        let orig_desc = "准备：投影转换器、架构评审胶片、季度预算表";
        app.back_screen.stepos_suite.calendar_events =
            vec![stepx_companion::toolbox::PhoneCalendarEvent {
                recurring: Some(false),
                id: ev_id,
                title: "季度复盘会议".into(),
                dtstart: 1793325600000,
                dtend: 1793331000000,
                all_day: false,
                location: "3 楼 A 会议室".into(),
                description: orig_desc.into(),
                source: "手机日历".into(),
            }];

        let before_items = stepx_companion::home_hub::extract_schedule_checklist(
            &app.back_screen.stepos_suite.calendar_events[0].description,
        );
        assert_eq!(before_items.len(), 3);

        // Toggle checklist item on Home page
        let key = Companion::checklist_item_state_key(ev_id, &before_items[0]);
        assert!(app.apply_home_action(&Action::HomeToggleChecklistItem(key.clone()), None));
        assert!(app.is_checklist_item_checked(ev_id, &before_items[0]));

        // Toggle event done
        app.back_screen.completed_calendar_events.insert(ev_id);
        app.back_screen.save_completed_calendar_events();
        app.sync_calendar_event_state_to_description(ev_id);

        let updated_desc = app.back_screen.stepos_suite.calendar_events[0]
            .description
            .clone();
        assert!(
            updated_desc.contains("日程状态=已完成"),
            "event description must contain completion state: {updated_desc}"
        );
        assert!(
            updated_desc.contains(&before_items[0]),
            "event description must contain checked item: {updated_desc}"
        );
        let after_items = stepx_companion::home_hub::extract_schedule_checklist(&updated_desc);
        assert_eq!(
            before_items, after_items,
            "sync metadata in description must not corrupt extracted checklist items"
        );

        // Reload in a fresh Companion instance and verify both sets persisted
        let mut reloaded = Companion::new_for_test();
        reloaded.demo_mode = true;
        reloaded.ensure_calendar_state_scope();
        reloaded.back_screen.load_completed_calendar_events();
        reloaded.load_home_checked_checklist_items();
        assert!(reloaded
            .back_screen
            .completed_calendar_events
            .contains(&ev_id));
        assert!(reloaded.is_checklist_item_checked(ev_id, &before_items[0]));

        // Also verify reconcile_calendar_event_states_from_descriptions recovers state from phone event description alone
        let mut from_phone = Companion::new_for_test();
        from_phone.demo_mode = true;
        from_phone.ensure_calendar_state_scope();
        from_phone.back_screen.completed_calendar_events.clear();
        from_phone.home_checked_checklist_items.clear();
        from_phone.back_screen.stepos_suite.calendar_events =
            vec![stepx_companion::toolbox::PhoneCalendarEvent {
                recurring: Some(false),
                id: ev_id,
                title: "季度复盘会议".into(),
                dtstart: 1793325600000,
                dtend: 1793331000000,
                all_day: false,
                location: "3 楼 A 会议室".into(),
                description: updated_desc,
                source: "手机日历".into(),
            }];
        from_phone.reconcile_calendar_event_states_from_descriptions();
        assert!(from_phone
            .back_screen
            .completed_calendar_events
            .contains(&ev_id));
        assert!(from_phone.is_checklist_item_checked(ev_id, &before_items[0]));

        // Preserve the human description and keep an explicit empty-state
        // tombstone, so another desktop can observe cancellation too.
        assert!(app.apply_home_action(&Action::HomeToggleChecklistItem(key), None));
        app.back_screen.completed_calendar_events.remove(&ev_id);
        app.sync_calendar_event_state_to_description(ev_id);
        let cleared = &app.back_screen.stepos_suite.calendar_events[0].description;
        assert_eq!(cleared.lines().next(), Some(orig_desc));
        assert!(cleared.contains("日程状态=待办｜已勾清单=[]"));
        from_phone.back_screen.stepos_suite.calendar_events = app.back_screen.stepos_suite.calendar_events.clone();
        from_phone.reconcile_calendar_event_states_from_descriptions();
        assert!(!from_phone.back_screen.completed_calendar_events.contains(&ev_id));
        assert!(!from_phone.is_checklist_item_checked(ev_id, &before_items[0]));
    }

    #[test]
    fn checklist_save_failure_restores_toggle_and_reports_other_callers() {
        let _guard = TEST_CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tempfile::tempdir().unwrap();
        let _cfg_guard = super::super::TestUiConfigDirGuard::set(cfg.path());
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = None;
        app.ensure_calendar_state_scope();
        assert!(app.save_home_checked_checklist_items().is_err());
        // Demo scope ensures this failure test can never send to a phone.
        app.demo_mode = true;
        app.prefs.auto_sync_calendar = false;
        app.ensure_calendar_state_scope();
        let path = app.home_checked_checklist_items_path().unwrap();
        // A directory at the destination fails atomic persistence even as root.
        std::fs::create_dir_all(&path).unwrap();
        let key = Companion::checklist_item_state_key(7, "检查材料");
        app.back_screen.stepos_suite.calendar_events = vec![stepx_companion::toolbox::PhoneCalendarEvent {
            id: 7, description: "原始日程内容".into(), ..Default::default()
        }];
        for initially_checked in [false, true] {
            app.home_checked_checklist_items.clear();
            app.home_checked_checklist_items.insert("8:其他事项".into());
            if initially_checked { app.home_checked_checklist_items.insert(key.clone()); }
            let before = app.home_checked_checklist_items.clone();
            assert!(app.apply_home_action(&Action::HomeToggleChecklistItem(key.clone()), None));
            assert_eq!(app.home_checked_checklist_items, before);
            assert!(app.error && app.status.contains("未保存"));
            assert_eq!(app.back_screen.stepos_suite.calendar_events[0].description, "原始日程内容",
                "failed persistence must return before description/phone synchronization");
        }
        app.back_screen.stepos_suite_error = None;
        app.migrate_calendar_event_state(7, 9);
        assert!(app.back_screen.stepos_suite_error.as_deref().unwrap().contains("迁移未保存"));
        app.back_screen.stepos_suite_error = None;
        app.remove_calendar_event_state(9);
        assert!(app.back_screen.stepos_suite_error.as_deref().unwrap().contains("清理未保存"));
        app.back_screen.stepos_suite_error = None;
        app.back_screen.stepos_suite.calendar_events[0].description =
            format!("{CALENDAR_SYNC_META_PREFIX}日程状态=待办｜已勾清单=[\"手机清单项\"]】");
        app.reconcile_calendar_event_states_from_descriptions();
        assert!(app.back_screen.stepos_suite_error.as_deref().unwrap().contains("清单状态未保存"));
        assert!(path.is_dir(), "the failed save must not replace the existing destination");
    }

    #[test]
    fn calendar_state_is_scoped_to_phone_and_legacy_owner_is_not_guessed() {
        let _guard = TEST_CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tempfile::tempdir().unwrap();
        let _cfg_guard = super::super::TestUiConfigDirGuard::set(cfg.path());
        let legacy = cfg.path().join("stepx-companion");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("completed-calendar-events.json"), "[7]").unwrap();
        std::fs::write(legacy.join("home-checked-checklist-items.json"), "[\"护照\"]").unwrap();
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device { serial: "calendar-phone-a".into(), ..Default::default() });
        app.ensure_calendar_state_scope();
        assert!(app.back_screen.completed_calendar_events.is_empty());
        assert!(!app.is_checklist_item_checked(7, "护照"));
        app.back_screen.completed_calendar_events.insert(7);
        app.home_checked_checklist_items.insert(Companion::checklist_item_state_key(7, "护照"));
        app.back_screen.save_completed_calendar_events();
        app.save_home_checked_checklist_items().unwrap();
        let phone_a_path = app.back_screen.completed_calendar_events_path();
        app.selected = Some(Device { serial: "calendar-phone-b".into(), ..Default::default() });
        app.ensure_calendar_state_scope();
        assert_ne!(app.back_screen.completed_calendar_events_path(), phone_a_path);
        assert!(!app.back_screen.completed_calendar_events.contains(&7));
        assert!(!app.is_checklist_item_checked(7, "护照"));
        app.home_checked_checklist_items.insert("护照".into());
        assert!(!app.is_checklist_item_checked(8, "护照"), "A bare label cannot mark another event complete");
        app.selected = Some(Device { serial: "calendar-phone-a".into(), ..Default::default() });
        app.ensure_calendar_state_scope();
        assert!(app.back_screen.completed_calendar_events.contains(&7));
        assert!(app.is_checklist_item_checked(7, "护照"));
        app.selected = None;
        app.ensure_calendar_state_scope();
        assert!(app.back_screen.completed_calendar_events.is_empty());
        assert!(app.home_checked_checklist_items.is_empty());
        assert!(app.back_screen.completed_calendar_events_path().is_none());
        assert_eq!(std::fs::read_to_string(legacy.join("completed-calendar-events.json")).unwrap(), "[7]");
        assert_eq!(std::fs::read_to_string(legacy.join("home-checked-checklist-items.json")).unwrap(), "[\"护照\"]");
    }

    #[test]
    fn calendar_phone_metadata_restores_all_labels_and_explicit_clears() {
        let _guard = TEST_CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tempfile::tempdir().unwrap();
        let _cfg_guard = super::super::TestUiConfigDirGuard::set(cfg.path());
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.ensure_calendar_state_scope();
        let items = vec!["GPU: 驱动｜版本", "登记、复核", "对照\"引用\""];
        app.back_screen.completed_calendar_events.insert(7);
        app.home_checked_checklist_items.extend(items.iter().map(|item| Companion::checklist_item_state_key(7, item)));
        app.back_screen.stepos_suite.calendar_events = vec![stepx_companion::toolbox::PhoneCalendarEvent {
            id: 7, description: "准备材料".into(), ..Default::default()
        }];
        app.sync_calendar_event_state_to_description(7);
        app.back_screen.completed_calendar_events.clear();
        app.home_checked_checklist_items.clear();
        app.reconcile_calendar_event_states_from_descriptions();
        for item in &items { assert!(app.is_checklist_item_checked(7, item), "lost label: {item}"); }
        assert!(app.back_screen.completed_calendar_events.contains(&7));
        // Legacy phones use a delimiter-separated list. Every item must survive.
        app.back_screen.stepos_suite.calendar_events[0].description = format!("{CALENDAR_SYNC_META_PREFIX}日程状态=待办｜已勾清单=材料甲｜材料乙｜材料丙】");
        app.reconcile_calendar_event_states_from_descriptions();
        assert!(!app.back_screen.completed_calendar_events.contains(&7));
        for item in ["材料甲", "材料乙", "材料丙"] { assert!(app.is_checklist_item_checked(7, item)); }
        for item in &items { assert!(!app.is_checklist_item_checked(7, item)); }
        app.back_screen.stepos_suite.calendar_events[0].description = format!("{CALENDAR_SYNC_META_PREFIX}日程状态=已完成｜已勾清单=[broken】");
        app.reconcile_calendar_event_states_from_descriptions();
        assert!(!app.back_screen.completed_calendar_events.contains(&7), "Malformed metadata must not overwrite local state");
        assert!(app.is_checklist_item_checked(7, "材料乙"));
        app.back_screen.stepos_suite.calendar_events[0].description = format!("{CALENDAR_SYNC_META_PREFIX}日程状态=待办｜已勾清单=[]】");
        app.reconcile_calendar_event_states_from_descriptions();
        assert!(app.home_checked_checklist_items.is_empty());
    }

    #[test]
    fn gallery_rotation_persists_to_disk_and_calendar_and_alarm_edits_preserve_metadata() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cfg_dir = tempfile::tempdir().unwrap();
        let _cfg_guard = super::super::TestUiConfigDirGuard::set(cfg_dir.path());

        let mut app = Companion::new_for_test();
        app.demo_mode = true;

        // 1. Gallery rotation persists to disk and writes rotated pixels back to both PNG and JPEG files
        let jpg_path = cfg_dir.path().join("sample_photo.jpg");
        let mut raw_img = image::RgbImage::new(4, 2);
        raw_img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        raw_img.save(&jpg_path).unwrap();
        let remote_str = jpg_path.to_string_lossy().into_owned();
        app.back_screen
            .insert_gallery_rotation(remote_str.clone(), 90);

        let opened = image::open(&jpg_path).unwrap();
        let rotated = rotate_dynamic_image_by_deg(opened, 90);
        save_image_matching_path_format(&rotated, &jpg_path)
            .expect("saving rotated RGBA/RGB image to .jpg must succeed");
        let reopened = image::open(&jpg_path).unwrap();
        assert_eq!((reopened.width(), reopened.height()), (2, 4));

        let mut reloaded_bs = BackScreenUiState::default();
        reloaded_bs.load_gallery_rotations();
        assert_eq!(reloaded_bs.gallery_rotations.get(&remote_str), Some(&90));

        // 2. Calendar edit populates editable HH:MM-HH:MM (not date-prefixed time_range_label) and preserves start/end, location, description, all_day
        let (y, m, d) = (2026, 10, 31);
        let (orig_start, orig_end) =
            stepx_companion::toolbox::parse_event_time_range(y, m, d, "14:00-15:30").unwrap();
        let orig_ev = stepx_companion::toolbox::PhoneCalendarEvent {
            recurring: Some(false),
            id: 501,
            title: "客户方案评审".into(),
            dtstart: orig_start,
            dtend: orig_end,
            all_day: false,
            location: "上海研发中心 502".into(),
            description: "准备：合同原件、技术标书、报价明细".into(),
            source: "双端已同步".into(),
        };
        let edit_time_label = editable_calendar_time_label(&orig_ev);
        assert_eq!(edit_time_label, "14:00-15:30");
        let (parsed_start, parsed_end) =
            stepx_companion::toolbox::parse_event_time_range(y, m, d, &edit_time_label).unwrap();
        assert_eq!(parsed_start, orig_start);
        assert_eq!(parsed_end, orig_end);

        // Even if a date-prefixed label is passed, normalize_calendar_time_input strips the date prefix
        let normalized = normalize_calendar_time_input("10月31日 14:00–15:30");
        assert_eq!(normalized, "14:00–15:30");
        let (norm_start, norm_end) =
            stepx_companion::toolbox::parse_event_time_range(y, m, d, &normalized).unwrap();
        assert_eq!(norm_start, orig_start);
        assert_eq!(norm_end, orig_end);

        // Custom 90m duration is preserved when user changes start time to "16:00" without range
        let time_str = "16:00";
        let (dtstart, mut dtend) =
            stepx_companion::toolbox::parse_event_time_range(y, m, d, time_str).unwrap();
        let has_explicit_range = time_str.contains('-');
        let prev_dur = orig_ev.dtend - orig_ev.dtstart;
        if prev_dur > 0 && !has_explicit_range {
            dtend = dtstart + prev_dur;
        }
        assert_eq!(dtend - dtstart, 90 * 60_000);

        // 3. Bridge JSON parsing for calendarCreate and alarmCreate matches Bridge.java's {"created_id": ...} schema
        let cal_json = serde_json::json!({
            "created_id": 888,
            "calendar_events": [{
                "id": 888,
                "title": "客户方案评审",
                "dtstart": orig_start,
                "dtend": orig_end,
                "all_day": false,
                "location": "上海研发中心 502",
                "description": "准备：合同原件",
                "source": "手机日历"
            }]
        });
        let parsed_evs = parse_bridge_calendar_events(&cal_json).unwrap();
        assert_eq!(parsed_evs.len(), 1);
        assert_eq!(parsed_evs[0].id, 888);
        assert_eq!(parsed_evs[0].location, "上海研发中心 502");

        let alarm_json = serde_json::json!({
            "created_id": 302,
            "alarms": [{
                "id": 302,
                "hour": 7,
                "minutes": 30,
                "enabled": true,
                "days_of_week": 31,
                "vibrate": false,
                "message": "工作日早起"
            }]
        });
        let parsed_alarms = parse_bridge_alarms(&alarm_json).unwrap();
        assert_eq!(parsed_alarms.len(), 1);
        assert_eq!(parsed_alarms[0].id, 302);
        assert_eq!(parsed_alarms[0].days_of_week, 0x1f);
        assert!(!parsed_alarms[0].vibrate);
    }

    #[test]
    fn smart_backscreen_display_links_push_bar_single_and_multi_unread_messages() {
        use super::super::notification_center_state::{
            NotificationCategory, NotificationPopupWidget, NotificationPriority,
            NotificationVisualKind, UnifiedNotificationItem,
        };
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.back_screen.smart_display_enabled = true;
        app.back_screen.tab = 5;

        // 0 unread messages -> None
        app.notification_center.items.clear();
        app.sync_smart_backscreen_display();
        assert_eq!(app.back_screen.smart_display_live_count, 0);
        assert!(app.compute_smart_backscreen_target().is_none());

        // 1 unread WeChat message -> single icon (count == 1)
        app.notification_center.items.push(UnifiedNotificationItem {
            id: "im-thread:wx-1".into(),
            category: NotificationCategory::PhoneApps,
            visual: NotificationVisualKind::WeChat,
            priority: NotificationPriority::P0Immediate,
            popup_widget: NotificationPopupWidget::None,
            source_badge: "微信".into(),
            title: "文件传输助手".into(),
            summary: "下午架构评审资料已发".into(),
            time_label: "刚刚".into(),
            sort_ts_ms: 1793772000000,
            unread: true,
            status_tag: Some("微信".into()),
            primary_action: None,
            ..Default::default()
        });
        app.sync_smart_backscreen_display();
        assert_eq!(
            app.back_screen.smart_display_live_kind,
            bs::SmartMessageIconKind::WeChat
        );
        assert_eq!(app.back_screen.smart_display_live_count, 1);
        assert_eq!(
            app.back_screen.smart_display_last_pushed,
            Some((bs::SmartMessageIconKind::WeChat, 1))
        );
        assert!(app.back_screen.applied_title.contains("1条新消息"));

        // Add 2 more unread messages (total 3) -> icon + count 3
        app.notification_center.items.push(UnifiedNotificationItem {
            id: "im-thread:wx-2".into(),
            category: NotificationCategory::PhoneApps,
            visual: NotificationVisualKind::WeChat,
            priority: NotificationPriority::P0Immediate,
            popup_widget: NotificationPopupWidget::None,
            source_badge: "微信".into(),
            title: "产品研发群".into(),
            summary: "@全体成员 请查收背屏智能显示方案".into(),
            time_label: "1分钟前".into(),
            sort_ts_ms: 1793771900000,
            unread: true,
            status_tag: Some("微信".into()),
            primary_action: None,
            ..Default::default()
        });
        app.notification_center.items.push(UnifiedNotificationItem {
            id: "otp:882910".into(),
            category: NotificationCategory::PhoneComm,
            visual: NotificationVisualKind::SmsOtp,
            priority: NotificationPriority::P0Immediate,
            popup_widget: NotificationPopupWidget::None,
            source_badge: "短信".into(),
            title: "验证码 882910".into(),
            summary: "您的登录验证码为 882910".into(),
            time_label: "2分钟前".into(),
            sort_ts_ms: 1793771800000,
            unread: true,
            status_tag: Some("验证码".into()),
            primary_action: None,
            ..Default::default()
        });
        app.sync_smart_backscreen_display();
        assert_eq!(
            app.back_screen.smart_display_live_kind,
            bs::SmartMessageIconKind::WeChat
        );
        assert_eq!(app.back_screen.smart_display_live_count, 3);
        assert_eq!(app.back_screen.smart_display_live_slides.len(), 2);
        assert_eq!(
            app.back_screen.smart_display_last_pushed,
            Some((bs::SmartMessageIconKind::WeChat, 3))
        );
        assert!(app.back_screen.applied_title.contains("3条新消息"));

        // Mark all read -> automatically clears phone smart display state and restores Astronaut animation
        for item in &mut app.notification_center.items {
            item.unread = false;
        }
        app.sync_smart_backscreen_display();
        assert_eq!(app.back_screen.smart_display_live_count, 0);
        assert!(app.back_screen.smart_display_live_slides.is_empty());
        assert!(!app.back_screen.smart_display_active_on_phone);
        assert!(app.back_screen.smart_display_last_pushed.is_none());
        assert!(app.back_screen.applied_title.contains("旋转太空人"));
    }

    #[test]
    fn smart_backscreen_respects_unified_channel_silent_wework_and_calendar_alarm() {
        use super::super::notification_center_state::{
            NotificationAlertMode, NotificationCategory, NotificationPopupWidget,
            NotificationPriority, NotificationVisualKind, UnifiedNotificationItem,
        };
        use stepx_companion::desktop_notify::PopupMode;

        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.back_screen.smart_display_enabled = true;
        app.back_screen.auto_notify_flash = true;
        app.notification_center.items = vec![
            UnifiedNotificationItem {
                id: "im-thread:wework-1".into(),
                package_name: "com.tencent.wework".into(),
                category: NotificationCategory::PhoneApps,
                visual: NotificationVisualKind::WeChat,
                priority: NotificationPriority::P0Immediate,
                popup_widget: NotificationPopupWidget::None,
                source_badge: "企微".into(),
                title: "基础架构部群".into(),
                summary: "请确认发布窗口".into(),
                unread: true,
                ..Default::default()
            },
            UnifiedNotificationItem {
                id: "calendar:101".into(),
                package_name: "com.android.calendar".into(),
                category: NotificationCategory::PhoneComm,
                visual: NotificationVisualKind::CalendarAlarm,
                priority: NotificationPriority::P0Immediate,
                popup_widget: NotificationPopupWidget::None,
                source_badge: "日程提醒".into(),
                title: "季度产品评审".into(),
                summary: "14:00 · 3F会议室".into(),
                unread: true,
                ..Default::default()
            },
        ];

        app.sync_smart_backscreen_display();
        assert_eq!(
            app.back_screen.smart_display_live_kind,
            bs::SmartMessageIconKind::WeWork
        );
        assert_eq!(app.back_screen.smart_display_live_count, 2);

        // Suppressing WeWork channel in Settings removes WeWork from backscreen carousel, leaving CalendarAlarm
        app.push
            .prefs
            .set_app_popup_mode("com.tencent.wework", Some(PopupMode::Silent));
        app.sync_smart_backscreen_display();
        assert_eq!(
            app.back_screen.smart_display_live_kind,
            bs::SmartMessageIconKind::GeneralMessage
        );
        assert_eq!(app.back_screen.smart_display_live_count, 1);

        // Global Muted alert_mode suppresses all backscreen notification slides
        app.notification_center.alert_mode = NotificationAlertMode::Muted;
        app.sync_smart_backscreen_display();
        assert_eq!(app.back_screen.smart_display_live_count, 0);
        assert!(app.back_screen.smart_display_live_slides.is_empty());
    }
}

//! Backscreen editor: edits are local until the user starts playback.
use super::*;
use facloud_ui::clipboard::ClipboardContent;
use stepx_companion::{backscreen as bs, backscreen_amoo as ba, companion_app as ca};

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

fn rewrite_local_calendar_ics(events: &[stepx_companion::toolbox::PhoneCalendarEvent]) {
    let path =
        stepx_companion::resources::user_config().join("stepx-companion/desktop-calendar.ics");
    let _ = std::fs::remove_file(&path);
    let _ = stepx_companion::toolbox::export_merged_calendar_ics(events);
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
    StartPomodoro(u32),
    StopPomodoro,
    InspectStepOsSuite,
    PushDesktopClipboard,
    PullPhoneClipboard,
    TriggerMcpQuickAction(String),
    CalendarCreatePreset(u8),
    CalendarDelete(i64),
    CalendarPrevMonth,
    CalendarNextMonth,
    CalendarToday,
    CalendarSelectDate(i32, u32, u32),
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
    pub pomodoro_active: bool,
    pub pomodoro_started_at: Option<Instant>,
    pub pomodoro_duration_secs: u32,
    pub pomodoro_last_remaining_secs: Option<u32>,
    pub stepos_suite: stepx_companion::toolbox::StepOsAiSuiteReport,
    pub stepos_suite_loading: bool,
    pub calendar_year: i32,
    pub calendar_month: u32,
    pub selected_calendar_day: u32,
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
    pub editing_calendar_event_id: Option<i64>,
    pub editing_alarm_id: Option<i64>,
    pub preset_filter: u8,
    pub last_synced_desktop_clip: String,
    pub last_synced_phone_clip: String,
    pub clipboard_sync_in_flight: bool,
    pub last_clipboard_check: Instant,
    pub clipboard_history: Vec<String>,
    pub mcp_category_filter: u8,
}

impl BackScreenUiState {
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
            pomodoro_active: false,
            pomodoro_started_at: None,
            pomodoro_duration_secs: 25 * 60,
            pomodoro_last_remaining_secs: None,
            stepos_suite: stepx_companion::toolbox::default_stepos_ai_suite_report(),
            stepos_suite_loading: false,
            calendar_year: today.year(),
            calendar_month: today.month(),
            selected_calendar_day: today.day(),
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
            editing_calendar_event_id: None,
            editing_alarm_id: None,
            preset_filter: 0,
            last_synced_desktop_clip: String::new(),
            last_synced_phone_clip: String::new(),
            clipboard_sync_in_flight: false,
            last_clipboard_check: Instant::now(),
            clipboard_history: Vec::new(),
            mcp_category_filter: 0,
        };
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
    fn do_backscreen_action(
        &mut self,
        action: &BackScreenAction,
        ctx: &mut ViewContext<Self>,
    ) -> Result<()> {
        let is_smart_or_stepos = matches!(
            action,
            BackScreenAction::SetMarqueePreset(_)
                | BackScreenAction::ToggleAutoAiBreath
                | BackScreenAction::ToggleAutoNotifyFlash
                | BackScreenAction::StartPomodoro(_)
                | BackScreenAction::StopPomodoro
                | BackScreenAction::InspectStepOsSuite
                | BackScreenAction::PushDesktopClipboard
                | BackScreenAction::PullPhoneClipboard
                | BackScreenAction::TriggerMcpQuickAction(_)
                | BackScreenAction::CalendarCreatePreset(_)
                | BackScreenAction::CalendarDelete(_)
                | BackScreenAction::CalendarPrevMonth
                | BackScreenAction::CalendarNextMonth
                | BackScreenAction::CalendarToday
                | BackScreenAction::CalendarSelectDate(_, _, _)
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
                if self.back_screen.tab != *tab {
                    self.back_screen.tab = *tab;
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
                self.matrix_marquee_on = true;
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
                    device.verify(adb)?;
                    bs::clear_frame(adb, device)?;
                    let state = adb.shell(&device.transport, &["dumpsys", "display_ext"])?;
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
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("选择图片，预览背屏点阵")
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp", "gif"])
                    .show_open_single_file()?
                {
                    self.back_screen.apply_image(path, "本地图片")?;
                    self.back_screen.tab = 2;
                }
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
                let note = if self.back_screen.auto_notify_flash {
                    "已开启：收到验证码或重要消息时背屏闪烁提醒"
                } else {
                    "已关闭：收到验证码或重要消息时背屏闪烁提醒"
                };
                self.back_screen.status = note.into();
                self.status = note.into();
            }
            BackScreenAction::StartPomodoro(seconds) => {
                let raw_secs = if *seconds <= 60 {
                    (*seconds).saturating_mul(60)
                } else {
                    *seconds
                };
                let dur = raw_secs.clamp(60, 3600);
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
                if let Some(device) = self.selected.clone() {
                    let brightness = self.back_screen.config.brightness;
                    let revision = self.back_screen.revision;
                    self.back_screen.status =
                        format!("正在向手机物理背屏同步 {} 分钟专注倒计时…", dur / 60);
                    self.status = self.back_screen.status.clone();
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let res = Adb::discover().and_then(|adb| {
                            bs::start_pomodoro_countdown(&adb, &device, dur, brightness)
                        });
                        let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                    });
                    ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
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
                            Err(_) => {}
                        }
                        ctx.notify();
                    });
                } else {
                    self.back_screen.status = format!(
                        "已开启 {} 分钟桌面专注番茄钟（连接手机后同步物理背屏）",
                        dur / 60
                    );
                    self.status = self.back_screen.status.clone();
                }
            }
            BackScreenAction::StopPomodoro => {
                self.back_screen.pomodoro_active = false;
                self.back_screen.pomodoro_started_at = None;
                self.back_screen.pomodoro_last_remaining_secs = None;
                self.back_screen.status = "已结束专注番茄钟，恢复背屏常规预览".into();
                self.status = self.back_screen.status.clone();
                if let Some(device) = self.selected.clone() {
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let res = Adb::discover()
                            .and_then(|adb| bs::stop_pomodoro_countdown(&adb, &device));
                        let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                    });
                    ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                        if let Ok(Ok(msg)) = result {
                            me.back_screen.applied_to_phone = false;
                            me.back_screen.applied_revision = None;
                            me.back_screen.applied_title.clear();
                            me.back_screen.applied_status = msg.clone();
                            me.back_screen.status = msg.clone();
                            me.status = msg;
                        }
                        ctx.notify();
                    });
                }
            }
            BackScreenAction::InspectStepOsSuite => {
                if self.back_screen.stepos_suite_loading {
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再读取使用统计与剪贴板".into();
                    return Ok(());
                };
                self.back_screen.stepos_suite_loading = true;
                self.back_screen.stepos_suite.last_action_note =
                    "正在读取手机屏幕使用统计、剪贴板与系统服务状态…".into();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::inspect_stepos_ai_suite(&adb, &device)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
                    me.back_screen.stepos_suite_loading = false;
                    match result {
                        Ok(Ok(report)) => {
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
                            let note = report.last_action_note.clone();
                            me.back_screen.stepos_suite = report;
                            if !me.busy {
                                me.status = note.clone();
                            }
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.checked = true;
                            me.back_screen.stepos_suite.last_action_note =
                                format!("读取状态未完成：{err}");
                            if !me.busy {
                                me.status = format!("读取手机统计未完成：{err}");
                            }
                        }
                        Err(_) => {}
                    }
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
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::push_clipboard_to_phone(&adb, &device, &clip_text)
                            .map(|note| (clip_text, note))
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok((pushed_text, note))) => {
                            me.back_screen.record_clipboard_history(&pushed_text);
                            me.back_screen.last_synced_desktop_clip = pushed_text.clone();
                            me.back_screen.last_synced_phone_clip = pushed_text.clone();
                            me.back_screen.stepos_suite.phone_clipboard = pushed_text;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
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
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::push_clipboard_to_phone(&adb, &device, &clip_text)
                            .map(|note| (clip_text, note))
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok((pushed_text, note))) => {
                            me.back_screen.record_clipboard_history(&pushed_text);
                            me.back_screen.last_synced_desktop_clip = pushed_text.clone();
                            me.back_screen.last_synced_phone_clip = pushed_text.clone();
                            me.back_screen.stepos_suite.phone_clipboard = pushed_text;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
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
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::pull_clipboard_from_phone(&adb, &device)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
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
            BackScreenAction::TriggerMcpQuickAction(action_key) => {
                let key = action_key.clone();
                let payload = if key == "open_clip_url_phone" {
                    self.back_screen
                        .stepos_suite
                        .phone_clipboard
                        .trim()
                        .to_string()
                } else {
                    let s = self.fields[F_OV_TEXT].text.trim().to_string();
                    if s.is_empty() {
                        self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string()
                    } else {
                        s
                    }
                };
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再执行快捷指令".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = "正在手机端执行快捷指令…".into();
                self.status = "正在手机端执行快捷指令…".into();
                let key_for_apply = key.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::trigger_stepos_mcp_action(
                            &adb, &device, &key, &payload,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok(note)) => {
                            if key_for_apply == "toggle_dnd" {
                                me.back_screen.stepos_suite.controls.dnd_on =
                                    note.contains("已开启");
                            } else if key_for_apply == "toggle_wifi" {
                                me.back_screen.stepos_suite.controls.wifi_on =
                                    !note.contains("已关闭");
                            } else if key_for_apply == "toggle_bluetooth" {
                                me.back_screen.stepos_suite.controls.bluetooth_on =
                                    !note.contains("已关闭");
                            } else if key_for_apply == "toggle_screen_perception" {
                                me.back_screen.stepos_suite.screen_perception_ready =
                                    note.contains("已开启");
                            }
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("操作未完成：{err}");
                            me.error = true;
                            me.status = format!("操作未完成：{err}");
                        }
                        Err(_) => {}
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
                let removed_title = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.title.clone());
                self.back_screen
                    .stepos_suite
                    .calendar_events
                    .retain(|e| e.id != id);
                if self.back_screen.editing_calendar_event_id == Some(id) {
                    self.back_screen.editing_calendar_event_id = None;
                }
                self.back_screen.completed_calendar_events.remove(&id);
                if self.prefs.auto_sync_calendar {
                    rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
                }
                if self.demo_mode || self.selected.is_none() {
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
                self.back_screen.stepos_suite.last_action_note = "正在从手机日历移除日程…".into();
                self.status = "正在从手机日历移除日程…".into();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::delete_phone_calendar_event(&adb, &device, id)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok((events, note))) => {
                            me.back_screen.stepos_suite.calendar_events = events;
                            if me.back_screen.editing_calendar_event_id == Some(id) {
                                me.back_screen.editing_calendar_event_id = None;
                            }
                            me.back_screen.completed_calendar_events.remove(&id);
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
                let custom_label = {
                    let l = self.fields[F_ALARM_LABEL].text.trim().to_string();
                    if l.is_empty() {
                        self.fields[F_PHONE_SCHEDULE_TITLE].text.trim().to_string()
                    } else {
                        l
                    }
                };
                let fallback_label = if custom_label.is_empty() {
                    self.fields[F_OV_TEXT].text.trim().to_string()
                } else {
                    custom_label
                };
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再创建手机闹钟".into();
                    self.error = true;
                    return Ok(());
                };
                self.back_screen.stepos_suite.last_action_note = "正在向手机时钟写入新闹钟…".into();
                self.status = "正在向手机时钟写入新闹钟…".into();
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
                    match result {
                        Ok(Ok((_id, alarms, note))) => {
                            me.fields[F_ALARM_LABEL].clear();
                            me.fields[F_PHONE_SCHEDULE_TITLE].clear();
                            me.back_screen.stepos_suite.checked = true;
                            me.back_screen.stepos_suite.alarms = alarms;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
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
                if self.demo_mode || self.selected.is_none() {
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
                if self.back_screen.editing_alarm_id == Some(id) {
                    self.back_screen.editing_alarm_id = None;
                }
                self.back_screen.stepos_suite.alarms.retain(|a| a.id != id);
                if self.demo_mode || self.selected.is_none() {
                    let note = "已移除该闹钟".to_string();
                    self.back_screen.stepos_suite.last_action_note = note.clone();
                    self.status = note;
                    self.error = false;
                    return Ok(());
                }
                let Some(device) = self.selected.clone() else {
                    return Ok(());
                };
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        stepx_companion::toolbox::delete_phone_alarm(&adb, &device, id)
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
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
                let target_pct = *percent;
                let Some(device) = self.selected.clone() else {
                    self.status = "请先连接手机后再调节屏幕亮度".into();
                    self.error = true;
                    return Ok(());
                };
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
                    match result {
                        Ok(Ok((pct, note))) => {
                            me.back_screen.stepos_suite.controls.brightness_percent = pct;
                            me.back_screen.stepos_suite.last_action_note = note.clone();
                            me.error = false;
                            me.status = note.clone();
                            me.tools_feedback = Some((me.tool_page, note, false));
                        }
                        Ok(Err(err)) => {
                            me.back_screen.stepos_suite.last_action_note =
                                format!("调节亮度未完成：{err}");
                            me.error = true;
                            me.status = format!("调节亮度未完成：{err}");
                        }
                        Err(_) => {}
                    }
                    ctx.notify();
                });
            }
            BackScreenAction::CalendarPrevMonth => {
                if self.back_screen.calendar_month <= 1 {
                    self.back_screen.calendar_year -= 1;
                    self.back_screen.calendar_month = 12;
                } else {
                    self.back_screen.calendar_month -= 1;
                }
                let max_d = days_in_calendar_month(
                    self.back_screen.calendar_year,
                    self.back_screen.calendar_month,
                );
                self.back_screen.selected_calendar_day =
                    self.back_screen.selected_calendar_day.clamp(1, max_d);
            }
            BackScreenAction::CalendarNextMonth => {
                if self.back_screen.calendar_month >= 12 {
                    self.back_screen.calendar_year += 1;
                    self.back_screen.calendar_month = 1;
                } else {
                    self.back_screen.calendar_month += 1;
                }
                let max_d = days_in_calendar_month(
                    self.back_screen.calendar_year,
                    self.back_screen.calendar_month,
                );
                self.back_screen.selected_calendar_day =
                    self.back_screen.selected_calendar_day.clamp(1, max_d);
            }
            BackScreenAction::CalendarToday => {
                use chrono::Datelike;
                let today = chrono::Local::now().date_naive();
                self.back_screen.calendar_year = today.year();
                self.back_screen.calendar_month = today.month();
                self.back_screen.selected_calendar_day = today.day();
            }
            BackScreenAction::CalendarSelectDate(y, m, d) => {
                let year = *y;
                let month = (*m).clamp(1, 12);
                let max_d = days_in_calendar_month(year, month);
                self.back_screen.calendar_year = year;
                self.back_screen.calendar_month = month;
                self.back_screen.selected_calendar_day = (*d).clamp(1, max_d);
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
                let title = if raw_title.is_empty() {
                    "桌面协同日程".to_string()
                } else {
                    raw_title
                };
                let time_str = preset_time
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| self.fields[F_CALENDAR_TIME].text.trim().to_string());
                let y = self.back_screen.calendar_year;
                let m = self.back_screen.calendar_month;
                let d = self.back_screen.selected_calendar_day;
                let auto_sync = self.prefs.auto_sync_calendar;
                let editing_old_id = self.back_screen.editing_calendar_event_id.take();
                let (dtstart, dtend) =
                    stepx_companion::toolbox::parse_event_time_range(y, m, d, &time_str)?;
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
                    id: local_id,
                    title: title.clone(),
                    dtstart,
                    dtend,
                    all_day: false,
                    location: String::new(),
                    description: "由 StepX 桌面端直接同步创建".into(),
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
                if auto_sync {
                    rewrite_local_calendar_ics(&self.back_screen.stepos_suite.calendar_events);
                }
                if self.demo_mode || self.selected.is_none() {
                    self.fields[F_PHONE_SCHEDULE_TITLE].clear();
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
                    "正在向手机日历写入选中日期日程…".into()
                };
                self.status = self.back_screen.stepos_suite.last_action_note.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        if let Some(old_id) = editing_old_id {
                            let _ = stepx_companion::toolbox::delete_phone_calendar_event(
                                &adb, &device, old_id,
                            );
                        }
                        stepx_companion::toolbox::create_calendar_event_at(
                            &adb, &device, &title, y, m, d, &time_str, auto_sync,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok((_id, events, msg))) => {
                            me.fields[F_PHONE_SCHEDULE_TITLE].clear();
                            me.apply(Outcome::StepOsCalendarUpdated(events, msg));
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
                    if l.is_empty() {
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
                if let Some(edit_id) = editing_old_id {
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
                    } else {
                        self.back_screen.stepos_suite.alarms.push(
                            stepx_companion::toolbox::PhoneAlarmItem {
                                id: edit_id,
                                hour,
                                minutes,
                                enabled: true,
                                days_of_week: 0,
                                vibrate: true,
                                message: clean_label.clone(),
                            },
                        );
                    }
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
                }
                if self.demo_mode || self.selected.is_none() {
                    self.fields[F_ALARM_LABEL].clear();
                    self.fields[F_PHONE_SCHEDULE_TITLE].clear();
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
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        if let Some(old_id) = editing_old_id {
                            let _ =
                                stepx_companion::toolbox::delete_phone_alarm(&adb, &device, old_id);
                        }
                        stepx_companion::toolbox::create_phone_alarm(
                            &adb,
                            &device,
                            hour,
                            minutes,
                            &clean_label,
                        )
                    });
                    let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                });
                ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                    match result {
                        Ok(Ok((_id, alarms, msg))) => {
                            me.fields[F_ALARM_LABEL].clear();
                            me.fields[F_PHONE_SCHEDULE_TITLE].clear();
                            me.apply(Outcome::StepOsAlarmsUpdated(alarms, msg));
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
                        let rotated = image::imageops::rotate90(&img);
                        let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let rot_path = cache_dir.join(format!("rot_{}_{}.png", item_idx, next_deg));
                        rotated
                            .save(&rot_path)
                            .map_err(|e| anyhow::anyhow!("保存旋转后的预览图失败：{e}"))?;
                        let rot_str = rot_path.to_string_lossy().into_owned();
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
                                let rotated = match next_deg {
                                    90 => image::DynamicImage::ImageRgba8(
                                        image::imageops::rotate90(&img),
                                    ),
                                    180 => image::DynamicImage::ImageRgba8(
                                        image::imageops::rotate180(&img),
                                    ),
                                    270 => image::DynamicImage::ImageRgba8(
                                        image::imageops::rotate270(&img),
                                    ),
                                    _ => img,
                                };
                                let rot_file = cache_dir
                                    .join(format!("sel_{item_idx}_{safe_name}_rot{next_deg}.png"));
                                rotated.save(&rot_file)?;
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
                if let Some(local_paths) = native_dialog::FileDialog::new()
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
                    .show_open_multiple_file()
                    .ok()
                    .filter(|v| !v.is_empty())
                {
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
                    self.back_screen.gallery_loading = true;
                    self.status = format!("正在导入 {summary} 到手机相册 (/sdcard/Pictures)…");
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let res = Adb::discover().and_then(|adb| {
                            for local_path in &local_paths {
                                storage::push(&adb, &device, local_path, "/sdcard/Pictures")?;
                                if let Some(fname) = local_path.file_name().and_then(|n| n.to_str())
                                {
                                    let _ = adb.shell(
                                        &device.transport,
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
                            let items =
                                stepx_companion::toolbox::list_phone_gallery(&adb, &device)?;
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
                }
            }
            BackScreenAction::McpQuickOpenCalendar => {
                self.handle_device_action(&Action::OpenApp("com.android.calendar".into()));
            }
            BackScreenAction::McpQuickOpenAlarm => {
                self.handle_device_action(&Action::OpenApp("com.android.deskclock".into()));
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
                if !self.back_screen.completed_calendar_events.remove(event_id) {
                    self.back_screen.completed_calendar_events.insert(*event_id);
                    self.status = "已将日程标记为已完成".into();
                } else {
                    self.status = "已取消日程完成标记".into();
                }
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
                    self.fields[F_CALENDAR_TIME].set(&ev.time_range_label());
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
                    if self.selected.is_some() {
                        self.do_backscreen_action(&BackScreenAction::PushDesktopClipboard, ctx)?;
                        self.status = format!("已将日程提醒「{}」同步到手机剪贴板", ev.title);
                    } else {
                        self.status = format!("已复制日程提醒「{}」", ev.title);
                    }
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
                    let mut pixels = if self.back_screen.frame.len() == bs::SLOTS {
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
                    self.back_screen.completed_calendar_events.remove(id);
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
                        let desc_bg = if desc.is_empty() {
                            "由 StepX 桌面端克隆创建".to_string()
                        } else {
                            desc
                        };
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().and_then(|adb| {
                                stepx_companion::toolbox::create_phone_calendar_event(
                                    &adb, &device, &title_bg, new_start, new_end, &desc_bg,
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
                let mut updated_info: Option<(i64, String, String, i64, i64, (i32, u32, u32))> =
                    None;
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
                        ev.description.clone(),
                        ev.dtstart,
                        ev.dtend,
                        ev.date_ymd(),
                    ));
                }
                let Some((old_id, title, desc, new_start, new_end, (ny, nm, nd))) = updated_info
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
                        let desc_bg = if desc.is_empty() {
                            "由 StepX 桌面端顺延更新".to_string()
                        } else {
                            desc
                        };
                        let (tx, rx) = async_channel::bounded(1);
                        std::thread::spawn(move || {
                            let res = Adb::discover().and_then(|adb| {
                                let _ = stepx_companion::toolbox::delete_phone_calendar_event(
                                    &adb, &device, old_id,
                                );
                                stepx_companion::toolbox::create_phone_calendar_event(
                                    &adb, &device, &title_bg, new_start, new_end, &desc_bg,
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
                let mut shifted_info: Option<(i64, String, u8, u8)> = None;
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
                    ));
                }
                if let Some((old_id, title, new_h, new_m)) = shifted_info {
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
                                    stepx_companion::toolbox::create_phone_alarm(
                                        &adb, &device, new_h, new_m, &title,
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
                        ca::stop(&adb, &device)?;
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
                        ca::stop(&adb, &device)?;
                    }
                    let playback = bs::start(adb, device, source, config)?;
                    return Ok(CompanionEvent::TemporaryStarted {
                        playback,
                        title,
                        revision,
                    });
                }
                let message = if let bs::Source::Preset(
                    preset @ (bs::Preset::Clock | bs::Preset::Battery | bs::Preset::Countdown),
                ) = &source
                {
                    ca::apply_preset(
                        &adb,
                        &device,
                        preset.key(),
                        config.brightness,
                        config.duration_secs,
                    )?
                } else {
                    let count = match &source {
                        bs::Source::Image(_) => 1,
                        bs::Source::Preset(bs::Preset::Astronaut) => u32::from(config.fps) * 4,
                        _ => u32::from(config.fps) * 8,
                    };
                    let mut render = config.clone();
                    render.brightness = 8;
                    let frames: Vec<_> = (0..count)
                        .map(|index| {
                            bs::frame(
                                &source,
                                &render,
                                Duration::from_secs_f64(f64::from(index) / f64::from(config.fps)),
                                &Default::default(),
                            )
                        })
                        .collect();
                    ca::apply_animation(
                        &adb,
                        &device,
                        &frames,
                        config.fps,
                        config.brightness,
                        &title,
                    )?
                };
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
                        CompanionEvent::Stopped(ca::stop(&adb, &device)?)
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
                self.back_screen.pomodoro_active = false;
                self.back_screen.pomodoro_started_at = None;
                self.back_screen.pomodoro_last_remaining_secs = None;
                self.back_screen.status =
                    "25 分钟背屏专注番茄钟已完成，手机背屏恢复默认显示".into();
                changed = true;
            } else if self.page == Page::BackScreen
                && self.back_screen.pomodoro_last_remaining_secs != Some(remaining)
            {
                self.back_screen.pomodoro_last_remaining_secs = Some(remaining);
                changed = true;
            }
        }
        if self.page == Page::BackScreen && self.back_screen.preview_running {
            let duration = if self.back_screen.tab == 4 {
                20_000
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
}

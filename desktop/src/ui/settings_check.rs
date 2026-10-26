//! Native interaction regression: an isolated settings file, no device-side writes.
use super::*;
use facloud_ui::TypedActionView;
#[derive(Default)]
struct Check {
    step: u8,
    last: Option<Instant>,
}
thread_local! {static CHECK:RefCell<Check>=RefCell::new(Check::default());}
impl Companion {
    fn interaction_capture(&self, ctx: &mut ViewContext<Self>, name: &str) {
        let path =
            PathBuf::from(std::env::var("STEPX_INTERACTION_CAPTURE_DIR").unwrap()).join(name);
        ctx.windows()
            .platform_window(ctx.window_id())
            .unwrap()
            .as_ctx()
            .request_frame_capture(Box::new(move |frame| {
                let _ = image::save_buffer(
                    &path,
                    &frame.data,
                    frame.width,
                    frame.height,
                    image::ColorType::Rgba8,
                );
            }));
    }
    pub(super) fn verify_settings(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.demo_mode || !std::env::args().any(|a| a == "--verify-settings") {
            return;
        }
        assert!(std::env::var_os("STEPX_SETTINGS_FILE").is_some());
        CHECK.with(|c|{let mut c=c.borrow_mut();if c.last.is_some_and(|t|t.elapsed()<Duration::from_millis(450)){return;}c.last=Some(Instant::now());
        match c.step {
            0=>{self.discard_settings();self.page=Page::Settings;self.handle_action(&Action::Settings(SettingsAction::Motion),ctx);assert_ne!(self.settings.draft,self.settings.value);assert!(!stepx_companion::settings::load().reduced_motion);},
            1=>{self.handle_action(&Action::Settings(SettingsAction::Apply),ctx);assert!(self.interaction.review.is_some());assert!(!stepx_companion::settings::load().reduced_motion);},
            2=>self.interaction_capture(ctx,"settings-review.png"),
            3=>{self.handle_action(&Action::ReviewCancel,ctx);assert!(self.settings_dirty());self.handle_action(&Action::Page(Page::Overview),ctx);assert!(self.interaction.review.as_ref().unwrap().discard);assert_eq!(self.page,Page::Settings);},
            4=>self.interaction_capture(ctx,"unsaved-review.png"),
            5=>{self.handle_action(&Action::Escape,ctx);assert!(self.settings_dirty());self.handle_action(&Action::Settings(SettingsAction::Apply),ctx);self.handle_action(&Action::ReviewConfirm,ctx);assert!(stepx_companion::settings::load().reduced_motion);assert!(!self.settings_dirty());},
            6=>{self.handle_action(&Action::Settings(SettingsAction::Motion),ctx);self.handle_action(&Action::Settings(SettingsAction::Discard),ctx);assert!(self.settings.value.reduced_motion);assert!(!self.settings_dirty());self.handle_action(&Action::Push(PushAction::TogglePrivacyMask),ctx);assert_ne!(self.settings.notify_draft,self.push.prefs);},
            7=>{self.handle_action(&Action::Settings(SettingsAction::Discard),ctx);self.settings.draft.download_directory=Some(PathBuf::from("/not-a-real-stepx-folder"));self.handle_action(&Action::Settings(SettingsAction::Apply),ctx);self.handle_action(&Action::ReviewConfirm,ctx);assert!(self.settings_dirty());assert!(stepx_companion::settings::load().download_directory.is_none());self.discard_settings();},
            8=>{self.page=Page::Expansion;self.expansion.snapshot.status=serde_json::json!({"permission":true,"policy":{"enabled":false,"photos":true,"videos":false,"documents":false,"retentionDays":7,"desktopId":"test"}});self.expansion.snapshot.online=true;self.expansion.config.folder=None;self.expansion.settings=false;self.handle_action(&Action::Expansion(expansion_state::ExpansionAction::Settings),ctx);self.expansion.draft_folder=Some(std::env::temp_dir());self.handle_action(&Action::Expansion(expansion_state::ExpansionAction::Type("videos".into())),ctx);assert!(self.expansion.dirty());assert!(self.expansion.config.folder.is_none());},
            9=>self.interaction_capture(ctx,"expansion-editor.png"),
            10=>{self.handle_action(&Action::Expansion(expansion_state::ExpansionAction::Confirm),ctx);assert!(self.interaction.review.is_some());assert!(self.expansion.pending.is_none());},
            11=>self.interaction_capture(ctx,"expansion-review.png"),
            12=>{self.handle_action(&Action::ReviewCancel,ctx);let(tx,rx)=mpsc::channel();self.expansion.pending=Some(rx);tx.send(Err("test write failure".into())).unwrap();self.tick_expansion();assert!(self.expansion.dirty());assert!(self.expansion.feedback.contains("test write failure"));self.handle_action(&Action::Expansion(expansion_state::ExpansionAction::Back),ctx);assert!(self.interaction.review.as_ref().unwrap().discard);},
            13=>{self.handle_action(&Action::ReviewConfirm,ctx);assert!(!self.expansion.settings);assert!(!self.expansion.dirty());self.handle_action(&Action::Settings(SettingsAction::Tab(1)),ctx);},
            14=>self.interaction_capture(ctx,"settings-files.png"),
            15=>{self.handle_action(&Action::ReviewHistory,ctx);},
            16=>self.interaction_capture(ctx,"reminders.png"),
            17=>{self.handle_action(&Action::ReviewCancel,ctx);self.page=Page::Settings;self.settings.tab=0;self.handle_action(&Action::Settings(SettingsAction::CloseToTray),ctx);self.handle_action(&Action::Settings(SettingsAction::Apply),ctx);self.handle_action(&Action::ReviewConfirm,ctx);self.handle_action(&Action::CloseWindow,ctx);assert!(self.settings.hidden);},
            18=>{self.page=Page::Overview;self.handle_tray_menu_id("stepx-settings",ctx);assert!(!self.settings.hidden);assert_eq!(self.page,Page::Settings);self.settings.hidden=true;self.handle_tray_menu_id("stepx-compact",ctx);assert!(!self.settings.hidden);assert_eq!(self.push.shell_mode,ShellMode::MiniAi);self.settings.hidden=true;self.handle_tray_menu_id("stepx-show",ctx);assert!(!self.settings.hidden);self.handle_action(&Action::Page(Page::Settings),ctx);assert_eq!(self.push.shell_mode,ShellMode::Full);self.handle_action(&Action::Settings(SettingsAction::Motion),ctx);assert!(self.settings_dirty());assert!(!self.interaction_gate(&Action::QuitApplication,ctx));assert!(self.interaction.review.is_none());self.discard_settings();self.handle_action(&Action::Settings(SettingsAction::Tray),ctx);self.handle_action(&Action::Settings(SettingsAction::Apply),ctx);self.handle_action(&Action::ReviewConfirm,ctx);assert!(self.settings.tray.is_none());assert!(!self.settings.value.close_to_tray);},
            19=>{self.handle_action(&Action::BackScreen(BackScreenAction::Play),ctx);assert!(self.interaction.review.is_some());self.back_screen.config.fps+=1;self.handle_action(&Action::ReviewConfirm,ctx);assert!(self.interaction.history_open);assert!(!self.back_screen.playing);self.handle_action(&Action::ReviewCancel,ctx);},
            20=>{self.fields[F_SMS_BODY].set("测试草稿，不发送");self.apply(Outcome::SmsPrepared(SmsPlan{request_id:"ui-test".into(),number:"12345".into(),body:"测试草稿，不发送".into(),subscription:1,subscription_label:"测试 SIM".into(),parts:1}));assert!(self.interaction.review.is_some());self.handle_action(&Action::ReviewCancel,ctx);assert!(self.pending_sms_plan.is_none());assert_eq!(self.fields[F_SMS_BODY].text,"测试草稿，不发送");},
            21=>{self.prepare_file_transfer(vec![PathBuf::from("/tmp/confirmation-only.txt")],Some("/sdcard/Download".into()));assert!(self.interaction.review.is_some());self.handle_action(&Action::ReviewCancel,ctx);assert!(!self.busy);self.settings.draft.reduced_motion=!self.settings.value.reduced_motion;self.settings.hidden=true;eprintln!("INTERACTION_NATIVE_PASS: staged edits, no premature writes, modal cancel, dirty navigation, confirmed save, failure preservation, notification draft, expansion review, history, tray lifecycle, changed target rejected, SMS cancel, transfer cancel");self.handle_tray_menu_id("stepx-quit",ctx);assert!(self.settings.hidden);assert!(self.interaction.review.is_none());},
            _=>{}
        }c.step+=1;ctx.notify();});
    }
}

use super::{icons::UiIcon, theme::*, widgets::*, *};
use facloud_ui::TypedActionView;
use stepx_companion::phone::{CallControl, LiveCall};

#[derive(Clone, Debug)]
pub(super) enum LiveCallAction {
    Control { id: String, action: CallControl },
    ComputerAnswer { id: String },
    PhoneAudio { id: String },
    CallBack(String),
}

#[derive(Default)]
pub(super) struct LiveCallUi {
    device_serial: String,
    receiver: Option<mpsc::Receiver<(String, anyhow::Result<LiveCall>)>>,
    pub feedback: String,
    pub owner: String,
    observer: Option<mpsc::Receiver<(String, anyhow::Result<LiveCall>)>>,
    observed: Option<Instant>,
    observer_serial: String,
    last_observed_call: Option<(String, LiveCall)>,
    records: Option<mpsc::Receiver<(String, anyhow::Result<Vec<CallRecord>>)>>,
    records_pending: bool,
    pending_callback: Option<(String, String)>,
    audio: Option<stepx_companion::call_audio::CallAudioSession>,
    audio_receiver: Option<mpsc::Receiver<(String, anyhow::Result<(LiveCall, stepx_companion::call_audio::CallAudioSession)>)>>,
    capability_receiver: Option<mpsc::Receiver<(String, anyhow::Result<stepx_companion::call_audio::Capabilities>)>>,
    capability_serial: String,
    capability_checked: Option<Instant>,
    capability: stepx_companion::call_audio::Capabilities,
    audio_requested: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl LiveCallUi {
    fn cancel_audio(&mut self) {
        if let Some(active) = self.audio_requested.take() {
            active.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        // Dropping the receiver also discards a late handshake response and its
        // audio session, so a cancelled route can never reconnect itself.
        self.audio_receiver = None;
        self.audio = None;
    }

    fn select_device(&mut self, serial: &str) {
        if self.device_serial != serial {
            self.cancel_audio();
            let mut next = Self::default();
            next.device_serial = serial.into();
            *self = next;
        }
    }

    fn cancel_stale_audio(&mut self, current: &LiveCall, now: i64) -> bool {
        if self.audio_receiver.is_some()
            && (current.id != self.owner || current.multiple_calls || !current.ongoing(now)) {
            self.cancel_audio();
            self.feedback = "通话已结束或变化，电脑音频连接已取消".into();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    #[test]
    fn cancelling_or_switching_device_drops_pending_audio_before_it_can_attach() {
        for switch in [false, true] {
            let mut state = LiveCallUi::default();
            state.device_serial = "first-phone".into();
            let requested = Arc::new(AtomicBool::new(true));
            state.audio_requested = Some(requested.clone());
            let (tx, rx) = mpsc::channel();
            state.audio_receiver = Some(rx);
            if switch { state.select_device("second-phone"); } else { state.cancel_audio(); }
            assert!(!requested.load(Ordering::Relaxed));
            assert!(state.audio_receiver.is_none());
            assert!(tx.send(("first-phone".into(), Err(anyhow::anyhow!("late result")))).is_err());
        }
    }

    #[test]
    fn disconnected_audio_worker_finishes_instead_of_waiting_forever() {
        let (tx, rx) = mpsc::channel::<()>();
        let mut receiver = Some(rx);
        assert!(completed_worker(&mut receiver).is_none());
        drop(tx);
        assert!(matches!(completed_worker(&mut receiver), Some(Err(_))));
        assert!(receiver.is_none());
    }

    #[test]
    fn phone_hangup_cancels_a_handshake_before_a_late_microphone_start() {
        let mut state = LiveCallUi::default();
        state.owner = "ended-call".into();
        let requested = Arc::new(AtomicBool::new(true));
        state.audio_requested = Some(requested.clone());
        let (_tx, rx) = mpsc::channel();
        state.audio_receiver = Some(rx);
        assert!(state.cancel_stale_audio(&LiveCall { available:true, state:"idle".into(), observed_ms:20_000, ..Default::default() }, 20_000));
        assert!(!requested.load(Ordering::Relaxed));
        assert!(state.audio_receiver.is_none());
    }

    #[test]
    fn late_audio_must_not_revive_an_ended_changed_or_multi_call() {
        let now = 20_000;
        let active = LiveCall { available:true, id:"current".into(), state:"active".into(), observed_ms:now, ..Default::default() };
        assert!(audio_receipt(Some("phone"), "phone", &active, &active, "current", now).is_some());
        assert!(audio_receipt(Some("other"), "phone", &active, &active, "current", now).is_none());
        assert!(audio_receipt(Some("phone"), "phone", &active, &active, "previous", now).is_none());
        for call in [
            LiveCall { state:"idle".into(), ..active.clone() },
            LiveCall { multiple_calls:true, ..active.clone() },
            LiveCall { observed_ms:1, ..active.clone() },
        ] { assert!(audio_receipt(Some("phone"), "phone", &call, &active, "current", now).is_none()); }
    }

    #[test]
    fn fast_answer_receipt_can_advance_old_ringing_but_preserves_newer_caller_details() {
        let ringing = LiveCall { available:true, id:"call".into(), state:"ringing".into(), observed_ms:20_000, ..Default::default() };
        let reply = LiveCall { state:"active".into(), observed_ms:20_100, ..ringing.clone() };
        assert_eq!(audio_receipt(Some("phone"), "phone", &ringing, &reply, "call", 20_100), Some(reply.clone()));
        let latest = LiveCall { name:"更新后的联系人".into(), observed_ms:20_200, ..reply.clone() };
        assert_eq!(audio_receipt(Some("phone"), "phone", &latest, &reply, "call", 20_200), Some(latest));
        let idle = LiveCall { state:"idle".into(), observed_ms:20_200, ..reply.clone() };
        assert!(audio_receipt(Some("phone"), "phone", &idle, &reply, "call", 20_200).is_none());
    }
}

impl Drop for LiveCallUi {
    fn drop(&mut self) {
        if let Some(active) = &self.audio_requested {
            active.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn completed_worker<T>(receiver: &mut Option<mpsc::Receiver<T>>) -> Option<Result<T, mpsc::RecvError>> {
    let result = match receiver.as_ref()?.try_recv() {
        Err(mpsc::TryRecvError::Empty) => return None,
        Ok(value) => Ok(value),
        Err(mpsc::TryRecvError::Disconnected) => Err(mpsc::RecvError),
    };
    *receiver = None;
    Some(result)
}

fn audio_receipt(serial: Option<&str>, response_serial: &str, current: &LiveCall, reply: &LiveCall, id: &str, now: i64) -> Option<LiveCall> {
    if serial != Some(response_serial) || current.id != id || current.multiple_calls
        || !current.ongoing(now) || reply.id != id || reply.state != "active"
        || reply.multiple_calls || !reply.ongoing(now) { return None; }
    // A successful answer can arrive before the next 900 ms observer poll.
    // Use its active receipt only if newer, never revive a newer terminal state
    // or overwrite fresher caller details with the handshake's older snapshot.
    let newest = if reply.observed_ms > current.observed_ms { reply } else { current };
    (newest.state == "active").then(|| newest.clone())
}

impl Companion {
    pub(super) fn handle_live_call(&mut self, action: &LiveCallAction, ctx: &mut ViewContext<Self>) {
        self.live_calls.select_device(self.selected.as_ref().map_or("", |device| device.serial.as_str()));
        match action {
            LiveCallAction::PhoneAudio { id } => {
                if self.push.snapshot.live_call.id != *id { return; }
                self.live_calls.cancel_audio();
                self.live_calls.feedback = "电脑音频已关闭，请在手机继续通话".into();
            }
            LiveCallAction::ComputerAnswer { id } => {
                if self.live_calls.audio_receiver.is_some() || self.live_calls.receiver.is_some() { return; }
                let Some(device) = self.selected.clone() else { return; };
                let call = &self.push.snapshot.live_call;
                if !call.ongoing(chrono::Utc::now().timestamp_millis()) || call.id != *id || !self.live_calls.capability.supported { return; }
                self.live_calls.owner = id.clone();
                self.live_calls.feedback = "正在连接电脑通话音频…".into();
                let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                self.live_calls.audio_requested = Some(active.clone());
                let (tx, rx) = mpsc::channel();
                self.live_calls.audio_receiver = Some(rx);
                let id = id.clone();
                std::thread::spawn(move || {
                    let result = (|| {
                        let adb = Adb::discover()?;
                        let current = stepx_companion::push::fetch_live_call(&adb, &device.resolve_active_transport(&adb)?)?;
                        anyhow::ensure!(active.load(std::sync::atomic::Ordering::Relaxed), "电脑音频连接已取消");
                        let call = if current.id == id && current.state == "ringing" {
                            stepx_companion::phone::control_call(&adb, &device, &id, CallControl::Answer)?
                        } else { current };
                        let audio = stepx_companion::call_audio::CallAudioSession::start_cancellable(&adb, &device, &id, active)?;
                        Ok((call, audio))
                    })();
                    let _ = tx.send((device.serial, result));
                });
            }
            LiveCallAction::CallBack(number) => {
                self.fields[F_PHONE_DIAL].set(number);
                self.handle_action(&Action::Page(Page::Phone), ctx);
                self.settings.hidden = false;
                ctx.windows().show_window_and_focus_app(ctx.window_id());
                if self.page != Page::Phone { return; }
                self.back_screen.live_capsule_popover_open = false;
                self.notification_center.drawer_open = false;
                if self.busy {
                    self.live_calls.pending_callback = self.selected.as_ref().map(|device| (device.serial.clone(), number.clone()));
                    self.status = "回拨号码已填入，通话资料加载后将显示号码与 SIM 确认".into();
                    self.error = false;
                    ctx.notify();
                    return;
                }
                self.handle_action(&Action::PhoneCall(number.clone()), ctx);
            }
            LiveCallAction::Control { id, action } => {
                if self.live_calls.receiver.is_some() { return; }
                let Some(device) = self.selected.clone() else { return; };
                self.live_calls.owner = id.clone();
                let call = &self.push.snapshot.live_call;
                if !call.ongoing(chrono::Utc::now().timestamp_millis()) || &call.id != id {
                    self.live_calls.feedback = "这次通话已结束或变化".into();
                    ctx.notify(); return;
                }
                if *action == CallControl::Hangup { self.live_calls.cancel_audio(); }
                let (tx, rx) = mpsc::channel();
                self.live_calls.receiver = Some(rx);
                self.live_calls.feedback = "正在等待手机确认…".into();
                let id = id.clone();
                let action = *action;
                std::thread::spawn(move || {
                    let result = Adb::discover().and_then(|adb| stepx_companion::phone::control_call(&adb, &device, &id, action));
                    let _ = tx.send((device.serial, result));
                });
            }
        }
        ctx.notify();
    }

    pub(super) fn tick_live_calls(&mut self, ctx: &mut ViewContext<Self>) {
        self.live_calls.select_device(self.selected.as_ref().map_or("", |device| device.serial.as_str()));
        self.tick_call_audio(ctx);
        self.tick_live_call_observer(ctx);
        self.tick_ended_call_records(ctx);
        if !self.busy {
            if let Some((serial, number)) = self.live_calls.pending_callback.take() {
                if self.page == Page::Phone && self.pending_call.is_none()
                    && self.selected.as_ref().is_some_and(|device| device.serial == serial)
                    && self.fields[F_PHONE_DIAL].text == number {
                    self.handle_action(&Action::PhoneCall(number), ctx);
                }
            }
        }
        let Some(receiver) = &self.live_calls.receiver else { return; };
        let result = match receiver.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return,
            result => result,
        };
        self.live_calls.receiver = None;
        match result {
            Ok((serial, result)) if self.selected.as_ref().is_some_and(|device| device.serial == serial) => {
                match result {
                    Ok(call) => {
                        self.live_calls.feedback = if call.state == "idle" { "通话已结束" } else { "手机已确认接听" }.into();
                        // A newer stream event may already describe another call.
                        if self.push.snapshot.live_call.id == self.live_calls.owner {
                            if call.state == "idle" { self.live_calls.records_pending = true; }
                            self.push.snapshot.live_call = call;
                        }
                    }
                    Err(error) => self.live_calls.feedback = error.to_string(),
                }
            }
            _ => self.live_calls.feedback = "设备连接已变化，请核对手机通话状态".into(),
        }
        self.sync_notification_center();
        ctx.notify();
    }

    fn tick_call_audio(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_demo_device() { self.live_calls.cancel_audio(); return; }
        if self.live_calls.cancel_stale_audio(&self.push.snapshot.live_call, chrono::Utc::now().timestamp_millis()) { ctx.notify(); }
        if let Some(result) = completed_worker(&mut self.live_calls.capability_receiver) {
            match result {
              Ok((serial, result)) => {
                if self.selected.as_ref().is_some_and(|device| device.serial == serial) {
                    self.live_calls.capability = result.unwrap_or_else(|error| stepx_companion::call_audio::Capabilities { supported: false, reason: error.to_string() });
                    ctx.notify();
                }
              }
              Err(_) => {
                  self.live_calls.capability = stepx_companion::call_audio::Capabilities { supported:false, reason:"电脑音频检查已中断，可稍后重试".into() };
                  ctx.notify();
              }
            }
        }
        if let Some(device) = self.selected.clone().filter(|device| device.serial != self.live_calls.capability_serial
            || (!self.live_calls.capability.supported && self.live_calls.capability_receiver.is_none()
                && self.live_calls.capability_checked.is_some_and(|at| at.elapsed() > Duration::from_secs(30)))) {
            self.live_calls.capability_serial = device.serial.clone();
            self.live_calls.capability_checked = Some(Instant::now());
            self.live_calls.capability = Default::default();
            let (tx, rx) = mpsc::channel(); self.live_calls.capability_receiver = Some(rx);
            std::thread::spawn(move || { let result = Adb::discover().and_then(|adb| stepx_companion::call_audio::capabilities(&adb, &device)); let _ = tx.send((device.serial, result)); });
        }
        if let Some(result) = completed_worker(&mut self.live_calls.audio_receiver) {
            match result {
              Ok((serial, result)) => {
                if self.selected.as_ref().is_some_and(|device| device.serial == serial) {
                    match result {
                        Ok((call, audio)) => {
                            if let Some(current) = audio_receipt(self.selected.as_ref().map(|device| device.serial.as_str()), &serial, &self.push.snapshot.live_call, &call, &audio.call_id, chrono::Utc::now().timestamp_millis()) {
                                self.push.snapshot.live_call = current;
                                self.live_calls.audio = Some(audio);
                                self.live_calls.feedback = "电脑音频已连接".into();
                            } else {
                                self.live_calls.feedback = "通话已变化，电脑音频已关闭".into();
                            }
                        }
                        Err(error) => self.live_calls.feedback = format!("电脑音频未建立：{error}。请查看手机通话状态"),
                    }
                    self.sync_notification_center(); ctx.notify();
                }
              }
              Err(_) => {
                  self.live_calls.cancel_audio();
                  self.live_calls.feedback = "电脑音频连接已中断，请在手机继续通话或重试".into();
                  self.sync_notification_center(); ctx.notify();
              }
            }
        }
        if let Some(audio) = &mut self.live_calls.audio {
            if !audio.running() || !self.selected.as_ref().is_some_and(|device| device.serial == audio.serial)
                || self.push.snapshot.live_call.id != audio.call_id || self.push.snapshot.live_call.state != "active" {
                self.live_calls.cancel_audio();
                self.live_calls.feedback = if self.push.snapshot.live_call.state == "active" {
                    "电脑音频中断，请用手机继续通话或重新连接电脑"
                } else { "电脑音频连接已结束" }.into();
                self.sync_notification_center(); ctx.notify();
            }
        }
    }

    pub(super) fn live_call_audio_label(&self) -> String {
        if let Some(audio) = &self.live_calls.audio {
            format!("电脑通话 · 麦克风：{}{}", audio.microphone_label(),
                if audio.microphone_muted() { "（已静音）" } else { "" })
        } else if !self.live_calls.capability.supported && !self.live_calls.capability.reason.is_empty() {
            format!("可在手机接听 · {}", self.live_calls.capability.reason)
        } else if self.push.snapshot.live_call.state == "ringing" {
            "选择手机或电脑接听".into()
        } else {
            "手机通话 · 声音通过手机或手机连接的耳机播放".into()
        }
    }
    pub(super) fn call_audio_on_computer(&self) -> bool { self.live_calls.audio.is_some() }

    #[cfg(feature = "ui-capture")]
    pub(super) fn call_audio_diagnostics(&self) -> serde_json::Value {
        let bytes = self.live_calls.audio.as_ref().map(|session| session.bytes());
        serde_json::json!({"call_id":self.push.snapshot.live_call.id,"state":self.push.snapshot.live_call.state,
            "available":self.push.snapshot.live_call.available,"computer_supported":self.live_calls.capability.supported,
            "computer_connected":self.live_calls.audio.is_some(),"audio_bytes":bytes,
            "connecting":self.live_calls.audio_receiver.is_some(),"feedback":self.live_calls.feedback,
            "microphone_label":self.live_calls.audio.as_ref().map(|audio| audio.microphone_label()),
            "microphone_source":self.live_calls.audio.as_ref().map(|audio| audio.microphone_source()),
            "microphone_muted":self.live_calls.audio.as_ref().map(|audio| audio.microphone_muted())})
    }

    fn tick_live_call_observer(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_demo_device() { return; }
        if let Some(receiver) = &self.live_calls.observer {
            match receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                result => {
                    self.live_calls.observer = None;
                    if let Ok((serial, Ok(call))) = result {
                        if self.selected.as_ref().is_some_and(|device| device.serial == serial)
                            && call.observed_ms >= self.push.snapshot.live_call.observed_ms {
                            let changed = call.id != self.push.snapshot.live_call.id
                                || call.state != self.push.snapshot.live_call.state
                                || call.number != self.push.snapshot.live_call.number
                                || call.name != self.push.snapshot.live_call.name
                                || call.multiple_calls != self.push.snapshot.live_call.multiple_calls
                                || call.available != self.push.snapshot.live_call.available;
                            // Normal message snapshots can already contain idle.
                            // Detect call completion against this observer's own
                            // previous state, so they cannot consume the transition.
                            if call.state == "idle" && self.live_calls.last_observed_call.as_ref().is_some_and(|(last_serial, last)|
                                last_serial == &serial && matches!(last.state.as_str(), "ringing" | "active")) {
                                self.live_calls.records_pending = true;
                            }
                            self.live_calls.last_observed_call = Some((serial, call.clone()));
                            self.push.snapshot.live_call = call;
                            if changed { self.sync_notification_center(); ctx.notify(); }
                        }
                    }
                }
            }
        }
        let Some(device) = self.selected.clone() else { return; };
        if self.live_calls.observer_serial == device.serial
            && self.live_calls.observed.is_some_and(|last| last.elapsed() < Duration::from_millis(900)) { return; }
        self.live_calls.observer_serial = device.serial.clone();
        self.live_calls.observed = Some(Instant::now());
        let (tx, rx) = mpsc::channel();
        self.live_calls.observer = Some(rx);
        std::thread::spawn(move || {
            let result = Adb::discover().and_then(|adb| {
                let transport = device.resolve_active_transport(&adb)?;
                stepx_companion::push::fetch_live_call(&adb, &transport)
            });
            let _ = tx.send((device.serial, result));
        });
    }

    fn tick_ended_call_records(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(receiver) = &self.live_calls.records {
            match receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                result => {
                    self.live_calls.records = None;
                    if let Ok((serial, Ok(records))) = result {
                        if self.selected.as_ref().is_some_and(|device| device.serial == serial) {
                            self.call_logs = records;
                            self.sync_notification_center();
                            ctx.notify();
                        }
                    }
                }
            }
        }
        if !self.live_calls.records_pending { return; }
        let Some(device) = self.selected.clone() else { return; };
        self.live_calls.records_pending = false;
        let (tx, rx) = mpsc::channel();
        self.live_calls.records = Some(rx);
        std::thread::spawn(move || {
            let result = Adb::discover().and_then(|adb| device.call_logs(&adb));
            let _ = tx.send((device.serial, result));
        });
    }

    pub(super) fn live_caller_name(&self, call: &LiveCall) -> String {
        if !call.name.trim().is_empty() && call.name != call.number { return call.name.clone(); }
        let normalize = |text: &str| stepx_companion::sms::normalize_address(text);
        if !call.number.is_empty() {
            if let Some(contact) = self.contacts.iter().find(|contact| normalize(&contact.number) == normalize(&call.number)) {
                if !contact.name.trim().is_empty() { return contact.name.clone(); }
            }
            return call.number.clone();
        }
        "未知来电人".into()
    }

    pub(super) fn live_call_controls(&self, id: &str, ringing: bool) -> El {
        if self.push.snapshot.live_call.multiple_calls {
            return self.line("多路通话，请在手机上选择操作", CAPTION, self.p().text2);
        }
        if self.live_calls.receiver.is_some() {
            return self.line("正在等待手机确认…", CAPTION, self.p().text2);
        }
        let action = |action| Action::LiveCall(LiveCallAction::Control { id: id.into(), action });
        let mut buttons = Vec::new();
        if self.live_calls.audio_receiver.is_some() {
            buttons.push(self.button_icon("live-call-audio-cancel", "取消电脑音频", UiIcon::Phone,
                Action::LiveCall(LiveCallAction::PhoneAudio { id:id.into() }), Btn::Secondary));
            buttons.push(self.button_icon("live-call-hangup", "挂断", UiIcon::PhoneOff, action(CallControl::Hangup), Btn::Secondary));
            return self.hstack(S2, buttons);
        }
        if ringing {
            buttons.push(self.button_icon("live-call-answer", "手机接听", UiIcon::Phone, action(CallControl::Answer), Btn::Primary));
        }
        if self.live_calls.capability.supported && self.live_calls.audio.is_none() {
            buttons.push(self.button_icon("live-call-answer-computer", if ringing { "电脑接听" } else { "转到电脑" }, UiIcon::Monitor,
                Action::LiveCall(LiveCallAction::ComputerAnswer { id: id.into() }), Btn::Secondary));
        } else if self.live_calls.audio.is_some() {
            buttons.push(self.button_icon("live-call-audio-phone", "切回手机", UiIcon::Phone, Action::LiveCall(LiveCallAction::PhoneAudio { id:id.into() }), Btn::Secondary));
        }
        buttons.push(self.button_icon("live-call-hangup", if ringing { "挂断" } else { "结束通话" }, UiIcon::PhoneOff, action(CallControl::Hangup), Btn::Secondary));
        self.hstack(S2, buttons)
    }

    pub(super) fn live_call_panel(&self) -> Option<El> {
        let call = &self.push.snapshot.live_call;
        if self.selected.is_some() && self.push.loaded && !call.available {
            return Some(self.panel(vec![
                self.section_heading(UiIcon::Phone, "实时来电尚未就绪"),
                self.text("请检查手机伴侣的电话权限与后台协作状态。通话记录仍可查看。", CAPTION, self.p().text2),
                self.button_icon("live-call-check-companion", "检查手机伴侣", UiIcon::Phone,
                    Action::OpenTool(Some(ToolPage::Companion)), Btn::Secondary),
            ]));
        }
        if !call.ongoing(chrono::Utc::now().timestamp_millis()) { return None; }
        let p = self.p();
        Some(self.panel(vec![
            self.section_heading(UiIcon::Phone, if call.state == "ringing" { "手机来电" } else { "当前通话" }),
            self.heading(self.live_caller_name(call)),
            self.line(if call.number.is_empty() { "手机未提供号码" } else { &call.number }, CAPTION, p.text2),
            self.text(self.live_call_audio_label(), MICRO, p.text2),
            self.live_call_controls(&call.id, call.state == "ringing"),
            self.text(if self.live_calls.owner == call.id { &self.live_calls.feedback } else { "" }, MICRO, p.text2),
        ]))
    }
}

//! UI state and background workers for StepX LAN Cast & Multi-Protocol Screen Projection.

use super::{
    icons::UiIcon,
    theme::*,
    widgets::{Btn, El},
    Action, Companion, Page, ToolPage,
};
use facloud_ui::{
    assets::asset_cache::AssetSource, elements::*, fonts::Weight, ViewContext,
};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use stepx_companion::{
    adb::Adb,
    lan_cast::{
        self, LanCastProtocol, LanCastQuality, LanCastReceiver, LanCastServer, LanCastStats,
        LanDiscoverySnapshot, PhoneMiracastState,
    },
    storage::format_bytes,
};

#[derive(Clone, Debug)]
pub enum LanCastAction {
    ToggleServer,
    StartServer,
    StopServer,
    ScanDevices,
    CycleQuality,
    SetQuality(LanCastQuality),
    ToggleRemoteControl,
    PushToReceiver(String),
    StopReceiverPush(String),
    OpenLocalWebPreview,
    OpenLocalVlcPreview,
    TogglePhoneMiracast(bool),
    OpenPhoneCastSettings,
    ConnectMiracastSink(String),
}

pub enum LanCastWorkerEvent {
    DiscoveryDone(LanDiscoverySnapshot),
    PushResult {
        receiver_id: String,
        receiver_name: String,
        result: Result<String, String>,
    },
    StopPushDone {
        receiver_id: String,
        receiver_name: String,
    },
    MiracastUpdated(Result<PhoneMiracastState, String>),
    Notice(String, bool),
}

pub struct LanCastUiState {
    pub server: Option<LanCastServer>,
    pub auto_started: bool,
    pub quality: LanCastQuality,
    pub remote_control_enabled: bool,
    pub scanning: bool,
    pub pushing_id: Option<String>,
    pub active_pushed_id: Option<String>,
    pub active_pushed_name: Option<String>,
    pub discovery: Option<LanDiscoverySnapshot>,
    pub status_note: Option<(String, bool)>,
    pub last_tick: Instant,
    tx: mpsc::Sender<LanCastWorkerEvent>,
    rx: mpsc::Receiver<LanCastWorkerEvent>,
}

impl Default for LanCastUiState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let host_lan_ip = lan_cast::detect_lan_ipv4();
        Self {
            server: None,
            auto_started: false,
            quality: LanCastQuality::Hd1080p,
            remote_control_enabled: false,
            scanning: false,
            pushing_id: None,
            active_pushed_id: None,
            active_pushed_name: None,
            discovery: Some(LanDiscoverySnapshot {
                host_lan_ip,
                ..Default::default()
            }),
            status_note: None,
            last_tick: Instant::now(),
            tx,
            rx,
        }
    }
}

impl LanCastUiState {
    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    pub fn stats(&self) -> LanCastStats {
        if let Some(srv) = &self.server {
            let mut s = srv.stats();
            if s.pushed_receiver_id.is_none() {
                s.pushed_receiver_id = self.active_pushed_id.clone();
                s.pushed_receiver_name = self.active_pushed_name.clone();
            }
            s
        } else {
            let host_ip = self
                .discovery
                .as_ref()
                .map(|d| d.host_lan_ip.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(lan_cast::detect_lan_ipv4);
            let port = lan_cast::DEFAULT_CAST_PORT;
            LanCastStats {
                running: false,
                port,
                host_ip: host_ip.clone(),
                web_url: String::new(),
                ts_url: String::new(),
                hls_url: String::new(),
                mjpeg_url: String::new(),
                qr_png_path: None,
                active_clients: 0,
                frames_captured: 0,
                bytes_broadcast: 0,
                fps: 0.0,
                phone_width: 1200,
                phone_height: 2670,
                remote_control_enabled: self.remote_control_enabled,
                quality: self.quality,
                pushed_receiver_id: self.active_pushed_id.clone(),
                pushed_receiver_name: self.active_pushed_name.clone(),
            }
        }
    }
}

impl Companion {
    pub(super) fn tick_lan_cast(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.lan_cast.rx.try_recv() {
            changed = true;
            match ev {
                LanCastWorkerEvent::DiscoveryDone(snap) => {
                    self.lan_cast.scanning = false;
                    let count = snap.receivers.len();
                    self.lan_cast.discovery = Some(snap);
                    if self.lan_cast.status_note.is_none() {
                        self.lan_cast.status_note = Some((
                            format!("已完成局域网多协议扫描，发现 {count} 台可投屏/接收终端"),
                            false,
                        ));
                    }
                }
                LanCastWorkerEvent::PushResult {
                    receiver_id,
                    receiver_name,
                    result,
                } => {
                    self.lan_cast.pushing_id = None;
                    match result {
                        Ok(msg) => {
                            self.lan_cast.active_pushed_id = Some(receiver_id.clone());
                            self.lan_cast.active_pushed_name = Some(receiver_name.clone());
                            if let Some(srv) = &self.lan_cast.server {
                                srv.set_pushed_receiver(Some(receiver_id), Some(receiver_name));
                            }
                            self.lan_cast.status_note = Some((msg.clone(), false));
                            self.status = msg;
                            self.error = false;
                        }
                        Err(err) => {
                            self.lan_cast.status_note = Some((err.clone(), true));
                            self.status = err;
                            self.error = true;
                        }
                    }
                }
                LanCastWorkerEvent::StopPushDone {
                    receiver_id,
                    receiver_name,
                } => {
                    if self.lan_cast.active_pushed_id.as_deref() == Some(receiver_id.as_str()) {
                        self.lan_cast.active_pushed_id = None;
                        self.lan_cast.active_pushed_name = None;
                    }
                    if let Some(srv) = &self.lan_cast.server {
                        srv.set_pushed_receiver(None, None);
                    }
                    let msg = format!("已停止向「{receiver_name}」推送投屏画面");
                    self.lan_cast.status_note = Some((msg.clone(), false));
                    self.status = msg;
                }
                LanCastWorkerEvent::MiracastUpdated(res) => match res {
                    Ok(m) => {
                        let enabled = m.enabled;
                        if let Some(disc) = &mut self.lan_cast.discovery {
                            disc.miracast = m;
                        }
                        let msg = if enabled {
                            "已开启手机原生 Miracast / Wi-Fi Display 无线显示扫描".to_string()
                        } else {
                            "已关闭手机原生 Miracast 无线显示".to_string()
                        };
                        self.lan_cast.status_note = Some((msg.clone(), false));
                        self.status = msg;
                    }
                    Err(e) => {
                        self.lan_cast.status_note = Some((e, true));
                    }
                },
                LanCastWorkerEvent::Notice(msg, is_err) => {
                    self.lan_cast.status_note = Some((msg.clone(), is_err));
                    self.status = msg;
                    self.error = is_err;
                }
            }
        }

        if self.lan_cast.is_running()
            && self.page == Page::Tools
            && self.tool_page == Some(ToolPage::Screen)
            && self.lan_cast.last_tick.elapsed() >= Duration::from_millis(500)
        {
            if let (Some(srv), Some(dev)) = (&self.lan_cast.server, &self.selected) {
                srv.set_serial(dev.transport.clone());
            }
            self.lan_cast.last_tick = Instant::now();
            changed = true;
        }
        changed
    }

    pub(super) fn ensure_lan_cast_server_started(&mut self) -> Result<LanCastStats, String> {
        if let Some(srv) = &self.lan_cast.server {
            if let Some(dev) = &self.selected {
                srv.set_serial(dev.transport.clone());
            }
            return Ok(srv.stats());
        }
        let Ok(adb) = Adb::discover() else {
            return Err("未找到 ADB 组件，无法启动局域网投屏".into());
        };
        let Some(dev) = &self.selected else {
            return Err("请先连接 StepX 手机后再开启局域网投屏".into());
        };
        let srv = LanCastServer::start(
            adb.path.clone(),
            dev.transport.clone(),
            self.lan_cast.quality,
            self.lan_cast.remote_control_enabled,
        )
        .map_err(|e| e.to_string())?;
        let stats = srv.stats();
        self.lan_cast.server = Some(srv);
        Ok(stats)
    }

    pub(super) fn trigger_lan_cast_discovery(&mut self) {
        if self.lan_cast.scanning {
            return;
        }
        self.lan_cast.scanning = true;
        let tx = self.lan_cast.tx.clone();
        let adb = Adb::discover().ok();
        let serial = self.selected.as_ref().map(|d| d.transport.clone());
        thread::spawn(move || {
            let snap = lan_cast::discover_lan_receivers(adb.as_ref(), serial.as_deref());
            let _ = tx.send(LanCastWorkerEvent::DiscoveryDone(snap));
        });
    }

    pub(super) fn handle_lan_cast_action(
        &mut self,
        action: &LanCastAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            LanCastAction::ToggleServer => {
                if self.lan_cast.is_running() {
                    self.handle_lan_cast_action(&LanCastAction::StopServer, ctx);
                } else {
                    self.handle_lan_cast_action(&LanCastAction::StartServer, ctx);
                }
            }
            LanCastAction::StartServer => {
                match self.ensure_lan_cast_server_started() {
                    Ok(stats) => {
                        let msg = format!(
                            "局域网投屏网关已启动：{} · MPEG-TS 流 {}",
                            stats.web_url, stats.ts_url
                        );
                        self.lan_cast.status_note = Some((msg.clone(), false));
                        self.status = msg;
                        self.error = false;
                        if self
                            .lan_cast
                            .discovery
                            .as_ref()
                            .is_none_or(|d| d.receivers.is_empty())
                        {
                            self.trigger_lan_cast_discovery();
                        }
                    }
                    Err(err) => {
                        self.lan_cast.status_note = Some((err.clone(), true));
                        self.status = err;
                        self.error = true;
                    }
                }
                ctx.notify();
            }
            LanCastAction::StopServer => {
                if let Some(mut srv) = self.lan_cast.server.take() {
                    srv.stop();
                }
                self.lan_cast.active_pushed_id = None;
                self.lan_cast.active_pushed_name = None;
                let msg = "已停止局域网跨机投屏网关".to_string();
                self.lan_cast.status_note = Some((msg.clone(), false));
                self.status = msg;
                self.error = false;
                ctx.notify();
            }
            LanCastAction::ScanDevices => {
                self.lan_cast.status_note =
                    Some(("正在扫描局域网 DLNA / AirPlay / Cast / Miracast 投屏终端…".into(), false));
                self.trigger_lan_cast_discovery();
                ctx.notify();
            }
            LanCastAction::CycleQuality => {
                let next = self.lan_cast.quality.next();
                self.handle_lan_cast_action(&LanCastAction::SetQuality(next), ctx);
            }
            LanCastAction::SetQuality(q) => {
                self.lan_cast.quality = *q;
                if let Some(srv) = &self.lan_cast.server {
                    srv.set_quality(*q);
                }
                let msg = format!("局域网投屏画质已切换为：{}", q.label());
                self.lan_cast.status_note = Some((msg.clone(), false));
                self.status = msg;
                ctx.notify();
            }
            LanCastAction::ToggleRemoteControl => {
                self.lan_cast.remote_control_enabled = !self.lan_cast.remote_control_enabled;
                if let Some(srv) = &self.lan_cast.server {
                    srv.set_remote_control(self.lan_cast.remote_control_enabled);
                }
                let msg = if self.lan_cast.remote_control_enabled {
                    "已允许局域网接收端反向触控操作手机".to_string()
                } else {
                    "已切换为纯观看模式（禁止局域网远端触控）".to_string()
                };
                self.lan_cast.status_note = Some((msg.clone(), false));
                self.status = msg;
                ctx.notify();
            }
            LanCastAction::PushToReceiver(id) => {
                let stats = match self.ensure_lan_cast_server_started() {
                    Ok(s) => s,
                    Err(e) => {
                        self.lan_cast.status_note = Some((e.clone(), true));
                        self.status = e;
                        self.error = true;
                        ctx.notify();
                        return;
                    }
                };
                let Some(receiver) = self
                    .lan_cast
                    .discovery
                    .as_ref()
                    .and_then(|d| d.receivers.iter().find(|r| &r.id == id))
                    .cloned()
                else {
                    return;
                };
                self.lan_cast.pushing_id = Some(receiver.id.clone());
                let tx = self.lan_cast.tx.clone();
                let adb = Adb::discover().ok();
                let serial = self.selected.as_ref().map(|d| d.transport.clone());
                thread::spawn(move || {
                    let res = lan_cast::push_stream_to_receiver(
                        &receiver,
                        &stats,
                        adb.as_ref(),
                        serial.as_deref(),
                    )
                    .map_err(|e| e.to_string());
                    let _ = tx.send(LanCastWorkerEvent::PushResult {
                        receiver_id: receiver.id,
                        receiver_name: receiver.name,
                        result: res,
                    });
                });
                ctx.notify();
            }
            LanCastAction::StopReceiverPush(id) => {
                let Some(receiver) = self
                    .lan_cast
                    .discovery
                    .as_ref()
                    .and_then(|d| d.receivers.iter().find(|r| &r.id == id))
                    .cloned()
                else {
                    return;
                };
                let tx = self.lan_cast.tx.clone();
                thread::spawn(move || {
                    let _ = lan_cast::stop_receiver_push(&receiver);
                    let _ = tx.send(LanCastWorkerEvent::StopPushDone {
                        receiver_id: receiver.id,
                        receiver_name: receiver.name,
                    });
                });
                ctx.notify();
            }
            LanCastAction::OpenLocalWebPreview => {
                match self.ensure_lan_cast_server_started() {
                    Ok(stats) => {
                        let tx = self.lan_cast.tx.clone();
                        let url = stats.web_url;
                        thread::spawn(move || {
                            let res = lan_cast::open_stream_in_local_player(&url, false);
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(LanCastWorkerEvent::Notice(
                                        format!("已在浏览器打开局域网投屏互动端：{url}"),
                                        false,
                                    ));
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(LanCastWorkerEvent::Notice(e.to_string(), true));
                                }
                            }
                        });
                    }
                    Err(e) => {
                        self.lan_cast.status_note = Some((e, true));
                    }
                }
                ctx.notify();
            }
            LanCastAction::OpenLocalVlcPreview => {
                match self.ensure_lan_cast_server_started() {
                    Ok(stats) => {
                        let tx = self.lan_cast.tx.clone();
                        let url = stats.ts_url;
                        thread::spawn(move || {
                            let res = lan_cast::open_stream_in_local_player(&url, true);
                            match res {
                                Ok(msg) => {
                                    let _ = tx.send(LanCastWorkerEvent::Notice(msg, false));
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(LanCastWorkerEvent::Notice(e.to_string(), true));
                                }
                            }
                        });
                    }
                    Err(e) => {
                        self.lan_cast.status_note = Some((e, true));
                    }
                }
                ctx.notify();
            }
            LanCastAction::TogglePhoneMiracast(enable) => {
                let (Ok(adb), Some(dev)) = (Adb::discover(), self.selected.clone()) else {
                    return;
                };
                let enable = *enable;
                let tx = self.lan_cast.tx.clone();
                thread::spawn(move || {
                    let res = lan_cast::set_phone_miracast_enabled(&adb, &dev.transport, enable)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LanCastWorkerEvent::MiracastUpdated(res));
                });
                ctx.notify();
            }
            LanCastAction::OpenPhoneCastSettings => {
                let (Ok(adb), Some(dev)) = (Adb::discover(), self.selected.clone()) else {
                    return;
                };
                let tx = self.lan_cast.tx.clone();
                thread::spawn(move || {
                    match lan_cast::open_phone_cast_settings(&adb, &dev.transport) {
                        Ok(()) => {
                            let _ = tx.send(LanCastWorkerEvent::Notice(
                                "已在手机端打开系统「无线投屏 (Cast / Miracast)」设置界面".into(),
                                false,
                            ));
                        }
                        Err(e) => {
                            let _ = tx.send(LanCastWorkerEvent::Notice(e.to_string(), true));
                        }
                    }
                });
                ctx.notify();
            }
            LanCastAction::ConnectMiracastSink(addr) => {
                let (Ok(adb), Some(dev)) = (Adb::discover(), self.selected.clone()) else {
                    return;
                };
                let addr = addr.clone();
                let tx = self.lan_cast.tx.clone();
                thread::spawn(move || {
                    match lan_cast::connect_phone_miracast(&adb, &dev.transport, &addr) {
                        Ok(()) => {
                            let _ = tx.send(LanCastWorkerEvent::Notice(
                                format!("已在手机端发起对 Miracast 无线显示器 ({addr}) 的连接"),
                                false,
                            ));
                        }
                        Err(e) => {
                            let _ = tx.send(LanCastWorkerEvent::Notice(e.to_string(), true));
                        }
                    }
                });
                ctx.notify();
            }
        }
    }

    /// Renders the complete Screen Mirroring + Multi-Protocol LAN Cast & Screenshot page (`ToolPage::Screen`).
    pub(super) fn screen_and_lan_cast_tool_page(&self) -> El {
        let p = self.p();
        let stats = self.lan_cast.stats();
        let disc = self.lan_cast.discovery.as_ref();

        // --------------------------------------------------------------------
        // 1. Local High-FPS Window Mirroring (scrcpy) Card
        // --------------------------------------------------------------------
        let mirror_status_pill = if self.mirror.is_some() {
            self.pill_badge("独立窗口投屏运行中", p.success_text, p.success_soft)
        } else {
            self.pill_badge("按需启动", p.text2, p.surface)
        };

        let quality_seg = self.segmented(
            "local-mirror-quality",
            vec![
                (
                    "720p 流畅".into(),
                    self.mirror_options.quality == 0,
                    Action::SetMirrorQuality(0),
                ),
                (
                    "1080p 高清".into(),
                    self.mirror_options.quality == 1,
                    Action::SetMirrorQuality(1),
                ),
                (
                    "原画 120Hz".into(),
                    self.mirror_options.quality >= 2,
                    Action::SetMirrorQuality(2),
                ),
            ],
        );

        use stepx_companion::mirror::MirrorAudioRoute;
        let audio_seg = self.segmented(
            "local-mirror-audio",
            vec![
                (
                    "电脑发声".into(),
                    self.mirror_options.audio_route == MirrorAudioRoute::Pc,
                    Action::SetMirrorAudioRoute(MirrorAudioRoute::Pc),
                ),
                (
                    "手机发声".into(),
                    self.mirror_options.audio_route == MirrorAudioRoute::Phone,
                    Action::SetMirrorAudioRoute(MirrorAudioRoute::Phone),
                ),
                (
                    "双端同播".into(),
                    self.mirror_options.audio_route == MirrorAudioRoute::Both,
                    Action::SetMirrorAudioRoute(MirrorAudioRoute::Both),
                ),
            ],
        );

        let local_mirror_card = self.panel(vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Monitor, 16., p.accent),
                        self.heading("本机高清窗口投屏 (scrcpy 硬件低延迟直连)"),
                        mirror_status_pill,
                    ],
                ),
                self.hstack(
                    S2,
                    vec![
                        if self.mirror.is_some() {
                            self.button(
                                "mirror-control-stop",
                                "结束本机投屏",
                                Action::StopMirror,
                                Btn::Danger,
                            )
                        } else {
                            self.button_icon(
                                "mirror-control-start",
                                "打开本机投屏窗口",
                                UiIcon::Monitor,
                                Action::Mirror,
                                Btn::Primary,
                            )
                        },
                        self.button_icon(
                            "tool-shot",
                            "截取当前画面",
                            UiIcon::Camera,
                            Action::Screenshot,
                            Btn::Secondary,
                        ),
                        if self.screenshot.is_some() {
                            self.button(
                                "tool-save-shot",
                                "保存截图",
                                Action::SaveScreenshot,
                                Btn::Secondary,
                            )
                        } else {
                            self.disabled_button("保存截图")
                        },
                    ],
                ),
            ),
            self.text(
                "在独立窗口查看和操作手机，侧边控制栏提供主页、返回、多任务、音量、声音路由与截图。流畅度取决于手机、画质和连接。",
                CAPTION,
                p.text2,
            ),
            self.between(
                self.hstack(
                    S3,
                    vec![
                        self.line_w("画质档位", CAPTION, p.text2, Weight::Medium),
                        quality_seg,
                        self.line_w("声音输出", CAPTION, p.text2, Weight::Medium),
                        audio_seg,
                    ],
                ),
                self.hstack(
                    S3,
                    vec![
                        self.hstack(
                            6.,
                            vec![
                                self.line("窗口置顶", CAPTION, p.text2),
                                self.toggle_switch(
                                    "mirror-top",
                                    self.mirror_options.always_on_top,
                                    Action::ToggleMirrorAlwaysOnTop,
                                ),
                            ],
                        ),
                        self.hstack(
                            6.,
                            vec![
                                self.line("投屏时手机息屏", CAPTION, p.text2),
                                self.toggle_switch(
                                    "mirror-screen-off",
                                    self.mirror_options.turn_screen_off,
                                    Action::ToggleMirrorTurnScreenOff,
                                ),
                            ],
                        ),
                    ],
                ),
            ),
        ]);

        // --------------------------------------------------------------------
        // 2. Multi-Protocol LAN Cast Gateway Card (DLNA / AirPlay / Web / HLS / MPEG-TS)
        // --------------------------------------------------------------------
        let gateway_badge = if stats.running {
            self.pill_badge(
                format!(
                    "正在广播 · {:.1} FPS · {} 台终端在线",
                    stats.fps, stats.active_clients
                ),
                p.success_text,
                p.success_soft,
            )
        } else {
            self.pill_badge("网关待机", p.text2, p.surface)
        };

        let lan_quality_seg = self.segmented(
            "lan-cast-quality",
            vec![
                (
                    "流畅 · 960".into(),
                    stats.quality == LanCastQuality::Smooth720p,
                    Action::LanCast(LanCastAction::SetQuality(LanCastQuality::Smooth720p)),
                ),
                (
                    "清晰 · 1280".into(),
                    stats.quality == LanCastQuality::Hd1080p,
                    Action::LanCast(LanCastAction::SetQuality(LanCastQuality::Hd1080p)),
                ),
                (
                    "高画质 · 1600".into(),
                    stats.quality == LanCastQuality::UltraNative,
                    Action::LanCast(LanCastAction::SetQuality(LanCastQuality::UltraNative)),
                ),
            ],
        );

        let stream_endpoints_col = self.vstack(
            S2,
            vec![
                self.lan_endpoint_row(
                    "Web 互动投屏控制台 (电脑 / 平板 / 电视浏览器免安装即开即投 + 触控反控)",
                    &stats.web_url,
                    Some((
                        "在浏览器预览",
                        Action::LanCast(LanCastAction::OpenLocalWebPreview),
                    )),
                    "lan-copy-web",
                ),
                self.lan_endpoint_row(
                    "标准 MPEG-TS 实时视频流 (DLNA 电视盒子 / VLC / PotPlayer / mpv 原生直连)",
                    &stats.ts_url,
                    Some((
                        "用 VLC 验证播放",
                        Action::LanCast(LanCastAction::OpenLocalVlcPreview),
                    )),
                    "lan-copy-ts",
                ),
                self.lan_endpoint_row(
                    "Apple HLS 实时流 (AirPlay 接收端 / Apple TV / iPad / Mac Safari 直连)",
                    &stats.hls_url,
                    None,
                    "lan-copy-hls",
                ),
            ],
        );

        let qr_box = if let Some(qr_path) = &stats.qr_png_path {
            if super::widgets::cached_is_file(qr_path) {
                Container::new(
                    Flex::column()
                        .with_spacing(6.)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            ConstrainedBox::new(
                                Image::new(
                                    AssetSource::LocalFile {
                                        path: qr_path.to_string_lossy().into_owned(),
                                    },
                                    CacheOption::BySize,
                                )
                                .contain()
                                .finish(),
                            )
                            .with_width(136.)
                            .with_height(136.)
                            .finish(),
                        )
                        .with_child(self.line("扫码在其他设备上直接观看/反控", MICRO, p.text2))
                        .finish(),
                )
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish()
            } else {
                self.qr_placeholder_box()
            }
        } else {
            self.qr_placeholder_box()
        };

        let endpoints_and_qr = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S3)
            .with_child(Expanded::new(1., stream_endpoints_col).finish())
            .with_child(qr_box)
            .finish();

        let mut gateway_rows = vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Wifi, 16., p.accent),
                        self.heading("局域网投屏"),
                        gateway_badge,
                    ],
                ),
                self.hstack(
                    S2,
                    vec![
                        if stats.running {
                            self.button(
                                "lan-cast-stop",
                                "停止局域网广播",
                                Action::LanCast(LanCastAction::StopServer),
                                Btn::Danger,
                            )
                        } else {
                            self.button_icon(
                                "lan-cast-start",
                                "开启局域网投屏广播",
                                UiIcon::Play,
                                Action::LanCast(LanCastAction::StartServer),
                                Btn::Primary,
                            )
                        },
                        self.button_icon(
                            "lan-cast-open-web",
                            "本机浏览器预览",
                            UiIcon::ExternalLink,
                            Action::LanCast(LanCastAction::OpenLocalWebPreview),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "lan-cast-open-vlc",
                            "VLC 播放验证",
                            UiIcon::Play,
                            Action::LanCast(LanCastAction::OpenLocalVlcPreview),
                            Btn::Secondary,
                        ),
                    ],
                ),
            ),
            self.text(
                "手机连接本电脑后，同一局域网内的浏览器可扫码观看。链接仅在本次投屏期间有效，请只分享给信任的人。远端控制需单独开启；DLNA 与旧版 AirPlay 接收功能为实验性适配，发现设备不代表能播放。",
                CAPTION,
                p.text2,
            ),
            self.between(
                self.hstack(
                    S3,
                    vec![
                        self.line_w("广播画质", CAPTION, p.text2, Weight::Medium),
                        lan_quality_seg,
                        self.mono(
                            format!(
                                "本机局域网 IP: {} · 手机分辨率: {}×{} · 已传输 {}",
                                stats.host_ip,
                                stats.phone_width,
                                stats.phone_height,
                                format_bytes(stats.bytes_broadcast)
                            ),
                            MICRO,
                            p.text3,
                        ),
                    ],
                ),
                self.hstack(
                    8.,
                    vec![
                        self.line("允许持有本次链接的接收端操作手机", CAPTION, p.text2),
                        self.toggle_switch(
                            "lan-cast-remote-ctl",
                            stats.remote_control_enabled,
                            Action::LanCast(LanCastAction::ToggleRemoteControl),
                        ),
                    ],
                ),
            ),
            endpoints_and_qr,
        ];

        if let Some((note, is_err)) = &self.lan_cast.status_note {
            gateway_rows.push(
                Container::new(self.between(
                    self.line(
                        note.clone(),
                        CAPTION,
                        if *is_err { p.danger } else { p.accent },
                    ),
                    if let Some(pushed_name) = &stats.pushed_receiver_name {
                        self.pill_badge(
                            format!("正在投送至：{pushed_name}"),
                            p.success_text,
                            p.success_soft,
                        )
                    } else {
                        Empty::new().finish()
                    },
                ))
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_background_color(if *is_err { p.danger_soft } else { p.accent_soft })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }

        let lan_gateway_card = self.panel(gateway_rows);

        // --------------------------------------------------------------------
        // 3. Discovered LAN Cast Receivers Card (DLNA / AirPlay / Cast / Miracast / LAN Hosts)
        // --------------------------------------------------------------------
        let receivers = disc.map(|d| d.receivers.as_slice()).unwrap_or(&[]);
        let scan_time_hint = disc
            .filter(|d| !d.scanned_at_hms.is_empty())
            .map(|d| {
                format!(
                    "最近扫描 {} · 发现 {} 台局域网设备 (手机 Wi-Fi: {})",
                    d.scanned_at_hms,
                    d.receivers.len(),
                    d.phone_lan_ip.as_deref().unwrap_or("同网段直连")
                )
            })
            .unwrap_or_else(|| "支持自动发现局域网内 DLNA/UPnP、AirPlay、Google Cast、Miracast 与在线终端".into());

        let mut receiver_rows: Vec<El> = Vec::new();
        if receivers.is_empty() {
            receiver_rows.push(
                Container::new(self.between(
                    self.line(
                        if self.lan_cast.scanning {
                            "正在并发探测局域网 SSDP (DLNA/UPnP)、mDNS (AirPlay/Cast) 与无线显示终端…"
                        } else {
                            "暂未扫描局域网接收端，点击右上角「扫描局域网设备」立即探测。"
                        },
                        CAPTION,
                        p.text2,
                    ),
                    self.button_icon(
                        "lan-scan-empty",
                        "立即扫描局域网",
                        UiIcon::Refresh,
                        Action::LanCast(LanCastAction::ScanDevices),
                        Btn::Secondary,
                    ),
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        } else {
            for (pair_idx, pair) in receivers.chunks(2).enumerate() {
                let base_idx = pair_idx * 2;
                let left_el = self.lan_receiver_row(base_idx, &pair[0], &stats);
                let right_el = if pair.len() > 1 {
                    self.lan_receiver_row(base_idx + 1, &pair[1], &stats)
                } else {
                    Empty::new().finish()
                };
                receiver_rows.push(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(S3)
                        .with_child(Expanded::new(1., left_el).finish())
                        .with_child(Expanded::new(1., right_el).finish())
                        .finish(),
                );
            }
        }

        let receivers_card = self.panel(vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Connect, 16., p.accent),
                        self.heading("局域网投屏设备发现与一键推送"),
                        self.pill_badge(
                            format!("{} 台终端", receivers.len()),
                            p.accent,
                            p.accent_soft,
                        ),
                    ],
                ),
                self.button_icon(
                    "lan-scan-btn",
                    if self.lan_cast.scanning {
                        "正在扫描局域网…"
                    } else {
                        "扫描局域网设备"
                    },
                    UiIcon::Refresh,
                    Action::LanCast(LanCastAction::ScanDevices),
                    Btn::Secondary,
                ),
            ),
            self.line(scan_time_hint, CAPTION, p.text2),
            self.vstack(S2, receiver_rows),
        ]);

        // --------------------------------------------------------------------
        // 4. Phone Native Miracast / Wi-Fi Display Card
        // --------------------------------------------------------------------
        let m_state = disc.map(|d| &d.miracast);
        let m_enabled = m_state.is_some_and(|m| m.enabled);
        let m_badge = if let Some(active_name) = m_state.and_then(|m| m.active_display_name.as_ref())
        {
            self.pill_badge(
                format!("已连接：{active_name}"),
                p.success_text,
                p.success_soft,
            )
        } else if m_enabled {
            self.pill_badge("手机 WFD 无线显示已开启", p.accent, p.accent_soft)
        } else {
            self.pill_badge("无线显示待开启", p.text2, p.surface)
        };

        let miracast_card = self.panel(vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Smartphone, 16., p.accent),
                        self.heading("手机原生 Miracast / Wi-Fi Display 无线投屏 (Qualcomm WFD)"),
                        m_badge,
                    ],
                ),
                self.hstack(
                    S2,
                    vec![
                        self.button_icon(
                            "miracast-toggle",
                            if m_enabled {
                                "刷新 Miracast 扫描"
                            } else {
                                "开启手机 Miracast 扫描"
                            },
                            UiIcon::Wifi,
                            Action::LanCast(LanCastAction::TogglePhoneMiracast(true)),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "miracast-open-settings",
                            "打开手机无线投屏界面",
                            UiIcon::Settings,
                            Action::LanCast(LanCastAction::OpenPhoneCastSettings),
                            Btn::Secondary,
                        ),
                    ],
                ),
            ),
            self.text(
                "适用于支持 Miracast / Wi-Fi Direct 协议的智能电视、无线投屏器（微软无线显示适配器、小米/索尼/LG 电视及 Windows「投影到此电脑」），由手机底层硬件直连投屏。",
                CAPTION,
                p.text2,
            ),
        ]);

        let mut page_items = vec![
            lan_gateway_card,
            receivers_card,
            miracast_card,
            local_mirror_card,
        ];

        if let Some((_, path)) = &self.screenshot {
            page_items.push(self.panel(vec![
                self.between(
                    self.heading("最近截图预览"),
                    self.button(
                        "tool-save-shot-preview",
                        "另存为图片…",
                        Action::SaveScreenshot,
                        Btn::Secondary,
                    ),
                ),
                ConstrainedBox::new(
                    Image::new(
                        AssetSource::LocalFile {
                            path: path.to_string_lossy().into_owned(),
                        },
                        CacheOption::BySize,
                    )
                    .contain()
                    .finish(),
                )
                .with_height(280.)
                .finish(),
            ]));
        }

        self.vstack(S4, page_items)
    }

    fn qr_placeholder_box(&self) -> El {
        let p = self.p();
        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(self.ui_icon(UiIcon::Wifi, 24., p.text3))
                    .with_child(self.line("开启广播后生成", CAPTION, p.text2))
                    .with_child(self.line("局域网扫码投屏二维码", MICRO, p.text3))
                    .finish(),
            )
            .with_uniform_padding(S3)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish(),
        )
        .with_width(164.)
        .with_height(172.)
        .finish()
    }

    fn lan_endpoint_row(
        &self,
        title: &str,
        url: &str,
        primary_btn: Option<(&str, Action)>,
        copy_id: &str,
    ) -> El {
        let p = self.p();
        let mut right_btns = Vec::new();
        if let Some((primary_btn_label, primary_action)) = primary_btn {
            right_btns.push(self.button(
                format!("{copy_id}-open"),
                primary_btn_label,
                primary_action,
                Btn::Secondary,
            ));
        }
        if !url.is_empty() { right_btns.push(self.button_icon(
            copy_id,
            "复制",
            UiIcon::Clipboard,
            Action::CopyText(url.to_string(), format!("已复制投屏地址：{url}")),
            Btn::Ghost,
        )); }
        Container::new(self.between(
            self.vstack(
                2.,
                vec![
                    self.line_w(title, CAPTION, p.text, Weight::Medium),
                    self.mono(if url.is_empty() { "开启投屏后生成本次访问链接" } else { url }, CAPTION, p.accent),
                ],
            ),
            self.hstack(S2, right_btns),
        ))
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.surface_border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish()
    }

    fn lan_receiver_row(&self, idx: usize, r: &LanCastReceiver, stats: &LanCastStats) -> El {
        let p = self.p();
        let is_pushed = stats.pushed_receiver_id.as_deref() == Some(r.id.as_str());
        let is_pushing = self.lan_cast.pushing_id.as_deref() == Some(r.id.as_str());

        let (proto_fg, proto_bg) = match r.protocol {
            LanCastProtocol::DlnaUpnp => (p.accent, p.accent_soft),
            LanCastProtocol::AirPlay => (p.success_text, p.success_soft),
            LanCastProtocol::GoogleCast => (p.warning, p.surface),
            LanCastProtocol::Miracast => (p.accent, p.accent_soft),
            LanCastProtocol::WebReceiver => (p.text2, p.surface),
        };

        let left = self.vstack(
            2.,
            vec![
                self.hstack(
                    8.,
                    vec![
                        self.line_w(&r.name, BODY, p.text, Weight::Semibold),
                        self.pill_badge(r.protocol.label(), proto_fg, proto_bg),
                        if is_pushed {
                            self.pill_badge("正在接收投屏", p.success_text, p.success_soft)
                        } else {
                            Empty::new().finish()
                        },
                    ],
                ),
                self.line(
                    format!("地址：{}:{} · {}", r.ip, r.port, r.model),
                    MICRO,
                    p.text2,
                ),
            ],
        );

        let mut actions = Vec::new();
        if is_pushed {
            actions.push(self.button(
                format!("lan-rx-stop-{idx}"),
                "断开投屏",
                Action::LanCast(LanCastAction::StopReceiverPush(r.id.clone())),
                Btn::Danger,
            ));
        } else if is_pushing {
            actions.push(self.disabled_button("正在连接推送…"));
        } else if r.can_direct_push {
            actions.push(self.button_icon(
                format!("lan-rx-push-{idx}"),
                if r.ip == stats.host_ip {
                    "本机打开接收端"
                } else {
                    "一键推送投屏"
                },
                UiIcon::Play,
                Action::LanCast(LanCastAction::PushToReceiver(r.id.clone())),
                Btn::Primary,
            ));
        } else {
            actions.push(self.button_icon(
                format!("lan-rx-copy-{idx}"),
                "复制投屏地址",
                UiIcon::Clipboard,
                Action::CopyText(
                    stats.web_url.clone(),
                    format!("已复制投屏地址 {}，可在 {} 上直接打开", stats.web_url, r.ip),
                ),
                Btn::Secondary,
            ));
        }

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(S2)
                .with_child(Expanded::new(1., left).finish())
                .with_child(self.hstack(S2, actions))
                .finish(),
        )
            .with_horizontal_padding(12.)
            .with_vertical_padding(10.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(if is_pushed {
                p.success
            } else {
                p.surface_border
            }))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
    }
}

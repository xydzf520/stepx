use super::super::{
    icons::UiIcon, is_wireless, theme::*, widgets::*, Action, Companion, Page, PairTab, ToolPage,
    F_CONNECT, F_PAIR_ADDR, F_PAIR_CODE,
};
use facloud_ui::{assets::asset_cache::AssetSource, elements::*, platform::Cursor};

fn remembered_device_is_online(
    serial: &str,
    selected: Option<&stepx_companion::device::Device>,
    transports: &[stepx_companion::device::Transport],
) -> bool {
    transports
        .iter()
        .any(|t| t.state == "device" && t.id == serial)
        || selected.is_some_and(|d| {
            d.serial == serial
                && transports
                    .iter()
                    .any(|t| t.state == "device" && t.id == d.transport)
        })
}

fn guide_can_continue(
    step: u8,
    connected: bool,
    wireless: bool,
    companion: &stepx_companion::companion_app::CompanionStatus,
) -> bool {
    match step {
        0 => true,
        1 => connected,
        2 => connected && companion.installed,
        // This only opens pairing controls. Some firmware cannot report the
        // switch state; do not trap a user who enabled it on the phone. Step 4
        // still requires an actual wireless transport before completion.
        3 => connected,
        4 => connected && wireless,
        _ => false,
    }
}

impl Companion {
    pub(in super::super) fn connect_actions(&self) -> Vec<El> {
        let mut actions = vec![if self.connect_guide_step.is_some() {
            self.button(
                "conn-guide",
                "收起引导",
                Action::ConnectGuide(None),
                Btn::Secondary,
            )
        } else {
            self.button(
                "conn-guide",
                "连接引导",
                Action::ConnectGuide(Some(0)),
                Btn::Secondary,
            )
        }];
        actions.push(self.button(
            "conn-adb-help",
            "如何开启 ADB",
            Action::AdbGuide(Some(0)),
            Btn::Secondary,
        ));
        actions.push(if self.busy {
            self.disabled_button("检测中…")
        } else {
            self.button("conn-refresh", "重新扫描", Action::Refresh, Btn::Secondary)
        });
        if self.selected.is_some() {
            actions.push(self.button(
                "conn-disconnect",
                "断开当前设备",
                Action::Forget,
                Btn::Secondary,
            ));
        }
        actions
    }

    fn connection_guide(&self, step: u8) -> El {
        let p = self.p();
        let titles = [
            "开启 USB 调试",
            "连接并授权电脑",
            "安装 StepX伴侣",
            "开启无线调试",
            "手机扫码配对",
            "确认连接",
        ];
        let mut body = vec![
            self.between(
                self.heading(format!(
                    "新手连接 · {} / 6 · {}",
                    step + 1,
                    titles[step as usize]
                )),
                self.button(
                    "guide-hide",
                    "收起引导",
                    Action::ConnectGuide(None),
                    Btn::Ghost,
                ),
            ),
            self.wrap(
                titles
                    .iter()
                    .enumerate()
                    .map(|(idx, title)| {
                        self.button(
                            format!("guide-step-pill-{idx}"),
                            format!("{}. {title}", idx + 1),
                            Action::ConnectGuide(Some(idx as u8)),
                            if step == idx as u8 {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        )
                    })
                    .collect(),
            ),
        ];
        let connected = self.selected.is_some();
        let companion = &self.companion_status;
        let can_continue = guide_can_continue(
            step,
            connected,
            self.selected
                .as_ref()
                .is_some_and(|d| is_wireless(&d.transport)),
            companion,
        );
        match step {
            0 => {
                body.push(self.connection_reference(1));
                body.push(self.button("guide-find-developer", "找不到开发者选项？看前一步", Action::AdbGuide(Some(0)), Btn::Secondary));
                body.push(self.text("第一次使用建议先连数据线；电脑已包含连接工具。", CAPTION, p.text2));
            }
            1 => {
                body.push(self.connection_reference(2));
                let hint = if connected {
                    "已通过 ADB 验证当前手机，可继续安装。"
                } else if self.devices.iter().any(|d| d.state == "unauthorized") {
                    "已检测到手机，但尚未授权。请在手机确认调试弹窗；若没有弹窗，重插数据线或撤销调试授权后重试。"
                } else if self.devices.iter().any(|d| d.state == "offline") {
                    "手机通道离线。请解锁手机、重插数据线，再检测。"
                } else {
                    "尚未检测到可用连接。检查数据线是否支持传输、USB 调试是否开启；若显示多台设备，请在下面选择目标手机。"
                };
                body.push(self.text(hint, CAPTION, if connected { p.success } else { p.text2 }));
            }
            2 => {
                body.push(self.text("在电脑点下面的安装按钮，再到手机确认安装。", BODY, p.text));
                body.push(self.text(
                    if companion.checked {
                        &companion.message
                    } else {
                        "正在等待伴侣状态检测。"
                    },
                    CAPTION,
                    p.text2,
                ));
                body.push(if !connected || self.busy {
                    self.disabled_button("安装／修复 StepX伴侣")
                } else {
                    self.button(
                        "guide-install",
                        if companion.installed {
                            "更新／修复 StepX伴侣"
                        } else {
                            "安装 StepX伴侣"
                        },
                        Action::CompanionInstall,
                        Btn::Primary,
                    )
                });
                body.push(self.text("手机询问安装、通知或扫码权限时，按当前功能确认。若签名不一致会保留数据并停止覆盖；具体结果会显示在这里。", CAPTION, p.text3));
            }
            3 => {
                let wireless = self
                    .selected
                    .as_ref()
                    .is_some_and(|d| is_wireless(&d.transport));
                let wireless_ready = wireless || companion.wireless_enabled == Some(true);
                body.push(self.connection_reference(3));
                body.push(self.text("手机若询问是否允许当前 Wi-Fi，请在手机上确认。", CAPTION, p.text2));
                if wireless_ready {
                    body.push(self.text(
                        if wireless {
                            "当前已通过无线调试连接，可直接进入下一步或收起引导。"
                        } else {
                            "已检测到手机无线调试开启，可继续下一步完成配对。"
                        },
                        CAPTION,
                        p.success,
                    ));
                } else {
                    if !companion.pairing_ready && !companion.pairing_message.is_empty() {
                        body.push(self.text(&companion.pairing_message, CAPTION, p.warning));
                        body.push(self.button(
                            "guide-system-pair",
                            "改用系统配对码",
                            Action::PairTab(PairTab::Code),
                            Btn::Secondary,
                        ));
                    }
                    body.push(self.text(
                        match companion.wireless_enabled {
                            Some(true) => "已检测到手机无线调试开启，可继续下一步完成配对。",
                            Some(false) => "无线调试尚未开启；在手机确认后点击「检测伴侣状态」。",
                            None => {
                                "请在手机开启「无线调试」，或直接点击「下一步」使用配对码连接。"
                            }
                        },
                        CAPTION,
                        p.text2,
                    ));
                }
                body.push(self.wrap(vec![
                    if connected && companion.installed && !self.busy {
                        self.button(
                            "guide-open-phone",
                            "在手机打开 StepX伴侣",
                            Action::OpenApp(stepx_companion::companion_app::PACKAGE.into()),
                            Btn::Secondary,
                        )
                    } else {
                        self.disabled_button("在手机打开 StepX伴侣")
                    },
                    if connected && !self.busy {
                        self.button(
                            "guide-check",
                            "检测伴侣状态",
                            Action::CompanionInspect,
                            Btn::Secondary,
                        )
                    } else {
                        self.disabled_button("检测伴侣状态")
                    },
                ]));
            }
            4 => {
                body.push(self.connection_reference(3));
                body.push(self.text(if companion.pairing_ready { "在手机伴侣点“扫码连接电脑”，扫描电脑下方的二维码。" } else { "在手机选“使用配对码配对设备”，把弹窗中的地址和六位码填到电脑下方。" }, BODY, p.text));
                body.push(self.text("填配对弹窗里的地址，不是上一页的连接地址；配对成功后会记住这台手机。", CAPTION, p.text2));
            }
            _ => {
                let wireless = self
                    .selected
                    .as_ref()
                    .is_some_and(|d| is_wireless(&d.transport));
                body.push(self.text(if wireless { "手机已通过无线 ADB 验证。保持同一网络和无线调试开启，即可使用 Amoo、文件传输和背屏。" } else if connected { "手机已通过 USB 验证。无线连接尚未确认，请返回扫码步骤，连接真实无线通道后再拔线。" } else { "当前连接已断开，请重新连接后再继续。" }, BODY, p.text));
                body.push(self.wrap(vec![
                    self.button(
                        "guide-home",
                        "返回首页",
                        Action::Page(Page::Home),
                        Btn::Primary,
                    ),
                    self.button(
                        "guide-tools",
                        "管理 StepX伴侣",
                        Action::OpenTool(Some(ToolPage::Companion)),
                        Btn::Secondary,
                    ),
                ]));
            }
        }
        if self.busy {
            let progress = self
                .google_progress
                .try_lock()
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.clone())
                .unwrap_or_else(|| self.status.clone());
            body.push(self.text(progress, CAPTION, p.accent));
        } else if self.error {
            body.push(self.text(&self.status, CAPTION, p.danger));
        }
        let mut navigation = vec![];
        if step > 0 {
            navigation.push(self.button(
                "guide-prev",
                "上一步",
                Action::ConnectGuide(Some(step - 1)),
                Btn::Secondary,
            ));
        }
        if step < 5 {
            navigation.push(if can_continue && !self.busy {
                self.button(
                    "guide-next",
                    if step == 3 {
                        "去配对"
                    } else if step == 0 {
                        "已开启，下一步"
                    } else {
                        "下一步"
                    },
                    Action::ConnectGuide(Some(step + 1)),
                    Btn::Primary,
                )
            } else {
                self.disabled_button("完成此步骤后继续")
            });
        }
        if !navigation.is_empty() {
            body.push(self.wrap(navigation));
        }
        self.sub_enter(self.panel(body))
    }

    fn detected_connection_endpoint(&self) -> Option<String> {
        let selected = self.selected.as_ref()?;
        // Only current, identity-associated endpoints. An IP alone tells us no port.
        self.online_device_transports(selected)
            .into_iter()
            .find(|endpoint| stepx_companion::pairing::validate_endpoint(endpoint).is_ok())
    }

    fn connection_pairing_panel(&self) -> El {
        let p = self.p();
        let detected_endpoint = self.detected_connection_endpoint();
        let connected = self.selected.is_some();
        let wireless = self
            .selected
            .as_ref()
            .is_some_and(|d| is_wireless(&d.transport));
        let mut rows = vec![
            self.section_heading(UiIcon::Smartphone, "无线配对"),
            self.text(
                "Android 11 及以上支持系统无线调试。手机与电脑需要能在同一局域网互相访问。",
                CAPTION,
                p.text2,
            ),
        ];
        if !wireless {
            rows.push(self.button_icon(
                "conn-usb-wireless-bridge",
                "已插数据线？一键开启同局域网无线连接（开启后可拔线）",
                UiIcon::Wifi,
                Action::EnableUsbWirelessBridge,
                Btn::Secondary,
            ));
        }
        if let Some(endpoint) = &detected_endpoint {
            if !wireless {
                rows.push(self.button_icon(
                    "conn-fill-detected-ip",
                    format!("使用已验证的连接地址 {endpoint}"),
                    UiIcon::Wifi,
                    Action::FillDetectedConnectionEndpoint(endpoint.clone()),
                    Btn::Ghost,
                ));
            }
        }
        if wireless {
            rows.push(
                Container::new(self.text(
                    "当前手机已连接。以下入口用于添加另一台手机；可用功能取决于手机权限与固件支持。",
                    CAPTION,
                    p.success_text,
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.success_soft)
                .with_border(Border::all(1.).with_border_color(p.success))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        } else if connected {
            rows.push(
                Container::new(self.text(
                    "当前手机已通过 USB 连接。无线备用需要手机开启无线调试并通过身份核验；下方可配对其他手机。",
                    CAPTION,
                    p.success_text,
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.success_soft)
                .with_border(Border::all(1.).with_border_color(p.success))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }
        rows.push(
            self.segmented(
                "pair-mode",
                [
                    (PairTab::Qr, "扫码配对"),
                    (PairTab::Code, "6 位配对码"),
                    (PairTab::Manual, "IP 直连"),
                ]
                .into_iter()
                .map(|(tab, label)| (label.into(), self.pair_tab == tab, Action::PairTab(tab)))
                .collect(),
            ),
        );
        match self.pair_tab {
            PairTab::Qr => {
                if let Some(qr) = self.qr.as_ref().filter(|q| !q.expired()) {
                    rows.push(
                        ConstrainedBox::new(
                            Image::new(
                                AssetSource::LocalFile {
                                    path: qr.path.to_string_lossy().into_owned(),
                                },
                                CacheOption::BySize,
                            )
                            .contain()
                            .finish(),
                        )
                        .with_width(210.)
                        .with_height(210.)
                        .finish(),
                    );
                    rows.push(self.text(
                        format!("等待手机扫描 · 还有 {} 秒", qr.remaining_secs()),
                        CAPTION,
                        p.text2,
                    ));
                    rows.push(self.text(format!("配对服务：{}", qr.service), MICRO, p.text3));
                    rows.push(self.button(
                        "qr-refresh",
                        "生成新二维码",
                        Action::Qr,
                        Btn::Secondary,
                    ));
                    rows.push(self.text(
                        "可使用 StepX伴侣「扫码连接电脑」或 Android 系统「无线调试 → 使用二维码配对设备」扫描。",
                        CAPTION,
                        p.text2,
                    ));
                } else if connected && self.qr.is_none() {
                    rows.push(self.button("qr-refresh", "生成二维码", Action::Qr, Btn::Secondary));
                    rows.push(self.text(
                        "点击上方「生成二维码」，即可用另一台手机扫码配对连接。",
                        CAPTION,
                        p.text2,
                    ));
                } else {
                    rows.push(self.button("qr-refresh", "生成二维码", Action::Qr, Btn::Primary));
                    rows.push(self.text(
                        "二维码尚未生成或已经过期，请点击上方「生成二维码」后用手机扫描。",
                        CAPTION,
                        p.text2,
                    ));
                    rows.push(self.text(
                        if self.companion_status.pairing_ready {
                            "StepX伴侣 → 连接 → 扫码连接电脑，或使用 Android 系统二维码配对入口。"
                        } else {
                            "可使用 Android 系统「无线调试 → 使用二维码配对设备」扫码，或切换 6 位配对码连接。"
                        },
                        CAPTION,
                        p.text3,
                    ));
                }
            }
            PairTab::Code => {
                rows.push(self.text("手机「开发者选项 → 无线调试 → 使用配对码配对设备」，输入弹窗中的配对端口和 6 位数字。配对端口与连接端口不同。", CAPTION, p.text2));
                rows.push(self.input(
                    F_PAIR_ADDR,
                    "手机显示的 IP:配对端口（唯一设备可留空）",
                    None,
                ));
                rows.push(self.input(F_PAIR_CODE, "6 位配对码", None));
                rows.push(if self.busy {
                    self.disabled_button("配对")
                } else {
                    self.button("pair-code-btn", "配对", Action::PairCode, Btn::Primary)
                });
            }
            PairTab::Manual => {
                rows.push(self.text("仅用于已配对的手机。输入手机无线调试首页显示的 IP 和连接端口，无法用此入口跳过首次配对授权。", CAPTION, p.text2));
                rows.push(self.input(F_CONNECT, "IP:连接端口", None));
                rows.push(if self.busy {
                    self.disabled_button("连接")
                } else {
                    self.button("pair-ip-btn", "连接", Action::Connect, Btn::Primary)
                });
            }
        }
        self.sub_enter(self.panel(rows))
    }

    pub(in super::super) fn connect_page(&self) -> El {
        let p = self.p();
        let mut sections = vec![];
        if self.connection_watch.user_paused {
            sections.push(self.panel(vec![
                self.heading("自动连接已暂停"),
                self.text(
                    "已保留信任信息。选择下方手机并点击连接，即可恢复本次桌面会话。",
                    CAPTION,
                    p.text2,
                ),
            ]));
        }
        if let Some(step) = self.connect_guide_step {
            sections.push(self.connection_guide(step));
        } else if self.selected.is_none() && self.preferences.devices.is_empty() {
            sections.push(self.panel(vec![self.between(
                self.vstack(
                    S1,
                    vec![
                        self.heading("第一次连接？"),
                        self.text(
                            "从 USB 授权开始，完成伴侣安装与无线扫码连接。",
                            CAPTION,
                            p.text2,
                        ),
                    ],
                ),
                self.button(
                    "connect-guide-start",
                    "开始引导",
                    Action::ConnectGuide(Some(0)),
                    Btn::Primary,
                ),
            )]));
        }
        if let Some(sel) = &self.selected {
            let backups = self.online_device_backups(sel);
            let has_dual = self.device_has_online_usb_and_wireless(sel);
            let active_transport = self.online_device_active_transport(sel);
            let active_wireless = is_wireless(&active_transport);
            let primary_desc = format!(
                "{} · {}",
                if active_wireless {
                    "无线 Wi-Fi"
                } else {
                    "USB 有线"
                },
                active_transport
            );
            let standby_desc = if !backups.is_empty() {
                format!(
                    "已验证备用：{} · 当前通道断开后尝试切换",
                    backups.join(" / ")
                )
            } else if !active_wireless {
                "USB 正在使用；无线备用尚未验证，请完成无线连接后再拔线。".into()
            } else {
                "已通过无线连接；手机与电脑保持同一网络即可继续同步。".into()
            };
            let mut dual_actions = Vec::new();
            if let Some(standby) = backups.first() {
                let switch_label = if is_wireless(standby) {
                    "切换至无线通道"
                } else {
                    "切换至 USB 通道"
                };
                dual_actions.push(self.button(
                    "conn-switch-preferred-transport",
                    switch_label,
                    Action::SwitchPreferredTransport(standby.clone()),
                    Btn::Secondary,
                ));
            } else if !active_wireless {
                dual_actions.push(self.button(
                    "conn-enable-usb-wireless-standby",
                    "启用无线备用",
                    Action::EnableUsbWirelessBridge,
                    Btn::Primary,
                ));
            }
            dual_actions.push(self.button(
                "conn-heal-connection",
                "检查连接",
                Action::HealConnection,
                Btn::Secondary,
            ));
            dual_actions.push(self.button_icon(
                "conn-ai-diagnosis",
                "AI 诊断",
                UiIcon::Sparkles,
                Action::RunDoctorHarness {
                    topic: stepx_companion::doctor_harness::DoctorTopic::CompanionSelfHeal,
                    query: "检查当前手机连接、伴侣后台与权限，给出依据和可执行修复建议".into(),
                },
                Btn::Secondary,
            ));
            sections.push(self.panel(vec![
                self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.content_icon(UiIcon::Smartphone, 22., p.text2),
                            self.heading(&sel.model),
                            self.device_connection_icons("connection-primary", sel),
                            self.pill_badge(
                                if has_dual {
                                    "双通道已连接"
                                } else if !backups.is_empty() {
                                    "备用地址已连接"
                                } else if active_wireless {
                                    "已连接"
                                } else {
                                    "已连接"
                                },
                                p.success_text,
                                p.success_soft,
                            ),
                        ],
                    ),
                    self.wrap(dual_actions),
                ),
                self.text(primary_desc, BODY, p.text),
                self.text(standby_desc, CAPTION, p.text2),
            ]));
        }
        let mut channels = vec![self.heading(format!("检测到的设备 · {}", self.devices.len()))];
        if self.devices.is_empty() {
            channels.push(self.text(
                "尚未发现设备。连接 USB 并授权，或使用右侧无线配对。",
                BODY,
                p.text2,
            ));
        }
        let range = self.list_page_range("connected-device-transports",self.devices.len(),30);
        channels.push(self.list_page_controls("connected-device-transports",self.devices.len(),30));
        for (i, device) in self.devices.iter().enumerate().skip(range.start).take(range.len()) {
            let selected = self.selected.as_ref().is_some_and(|d| {
                self.online_device_transports(d)
                    .iter()
                    .any(|id| id == &device.id || device.backup_transports().contains(id))
            });
            let online = device.state == "device";
            let wireless_ch = is_wireless(&device.id);
            let note = match device.state.as_str() {
                "device" if selected => "当前使用 · 已验证",
                "device" => "ADB 在线 · 可连接验证",
                "unauthorized" => "等待手机授权 USB 调试",
                "offline" => "通道离线，请解锁或重新连接",
                _ => "通道不可用",
            };
            channels.push(self.divider());
            let row_inner = self.between(
                self.vstack(
                    S1,
                    vec![
                        self.hstack(
                            S2,
                            vec![
                                self.content_icon(UiIcon::Smartphone, 20., p.text2),
                                self.heading(if device.model.is_empty() {
                                    "Android 设备"
                                } else {
                                    &device.model
                                }),
                                self.status_icon(
                                    format!("conn-transport-{i}"),
                                    if wireless_ch {
                                        "无线连接"
                                    } else {
                                        "USB 有线连接"
                                    },
                                    if wireless_ch {
                                        UiIcon::Wifi
                                    } else {
                                        UiIcon::Usb
                                    },
                                    p.text2,
                                ),
                            ],
                        ),
                        self.text(&device.id, CAPTION, p.text3),
                        self.text(note, CAPTION, if selected { p.success } else { p.text2 }),
                    ],
                ),
                if selected {
                    self.status_icon(
                        format!("conn-online-{i}"),
                        "当前设备已连接",
                        UiIcon::Check,
                        p.success_text,
                    )
                } else if online && !self.busy {
                    self.button(
                        format!("conn-select-{i}"),
                        "连接",
                        Action::Select(device.id.clone()),
                        Btn::Secondary,
                    )
                } else if device.state == "unauthorized" {
                    self.button(
                        format!("conn-auth-help-{i}"),
                        "查看授权指引",
                        Action::AdbGuide(Some(0)),
                        Btn::Secondary,
                    )
                } else {
                    self.disabled_button("等待就绪")
                },
            );
            let dev_id_dbl = device.id.clone();
            let dev_id_menu = device.id.clone();
            let dev_model_menu = device.model.clone();
            channels.push(
                Hoverable::new(
                    self.mouse_state(format!("conn-device-row-{i}")),
                    move |mouse| {
                        let bg = if mouse.is_clicked() {
                            p.accent_soft
                        } else if mouse.is_mouse_over_element() {
                            p.hover
                        } else {
                            facloud_ui::color::ColorU::transparent_black()
                        };
                        Container::new(row_inner)
                            .with_horizontal_padding(S2)
                            .with_vertical_padding(S1)
                            .with_background_color(bg)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                    },
                )
                .with_defer_events_to_children()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    if online && !selected {
                        ctx.dispatch_typed_action(Action::Select(dev_id_dbl.clone()));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::ConnectDevice {
                            id: dev_id_menu.clone(),
                            model: dev_model_menu.clone(),
                            online,
                            selected,
                        },
                        at,
                    ))
                })
                .finish(),
            );
        }
        let mut remembered = vec![self.section_heading(UiIcon::Shield, "已信任的手机")];
        if self.preferences.devices.is_empty() {
            remembered.push(self.text(
                "成功验证设备后，会在此保存型号和序列号。",
                CAPTION,
                p.text2,
            ));
        }
        let multi_remembered = self.preferences.devices.len() > 1;
        let range = self.list_page_range("remembered-devices",self.preferences.devices.len(),30);
        remembered.push(self.list_page_controls("remembered-devices",self.preferences.devices.len(),30));
        for (idx, device) in self.preferences.devices.iter().enumerate().skip(range.start).take(range.len()) {
            let is_preferred =
                self.preferences.selected_serial.as_deref() == Some(device.serial.as_str());
            let is_online =
                remembered_device_is_online(&device.serial, self.selected.as_ref(), &self.devices);
            let mut left = vec![self.text(&device.model, BODY, p.text)];
            if is_online {
                left.push(self.status_icon(
                    format!("conn-trusted-online-{idx}"),
                    "当前在线",
                    UiIcon::Check,
                    p.success_text,
                ));
            }
            if is_preferred {
                left.push(self.status_icon(
                    format!("conn-trusted-default-{idx}"),
                    "默认连接设备",
                    UiIcon::Pin,
                    p.accent,
                ));
            }
            let mut right_actions = vec![];
            if multi_remembered && !is_preferred {
                right_actions.push(self.button(
                    format!("conn-pref-{idx}"),
                    "设为默认",
                    Action::SetPreferredDevice(device.serial.clone()),
                    Btn::Ghost,
                ));
            }
            right_actions.push(self.button(
                format!("conn-forget-{idx}"),
                "移除",
                Action::RemoveRememberedDevice(device.serial.clone()),
                Btn::Ghost,
            ));
            remembered.push(self.divider());
            let rem_inner = self.between(
                self.vstack(
                    2.,
                    vec![
                        self.hstack(S2, left),
                        self.mono(&device.serial, MICRO, p.text3),
                    ],
                ),
                self.hstack(S1, right_actions),
            );
            let serial_dbl = device.serial.clone();
            let serial_menu = device.serial.clone();
            let model_menu = device.model.clone();
            remembered.push(
                Hoverable::new(
                    self.mouse_state(format!("conn-remembered-row-{idx}")),
                    move |mouse| {
                        let bg = if mouse.is_clicked() {
                            p.accent_soft
                        } else if mouse.is_mouse_over_element() {
                            p.hover
                        } else {
                            facloud_ui::color::ColorU::transparent_black()
                        };
                        Container::new(rem_inner)
                            .with_horizontal_padding(S2)
                            .with_vertical_padding(S1)
                            .with_background_color(bg)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                    },
                )
                .with_defer_events_to_children()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::SetPreferredDevice(serial_dbl.clone()))
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::RememberedDevice {
                            serial: serial_menu.clone(),
                            model: model_menu.clone(),
                            is_preferred,
                        },
                        at,
                    ))
                })
                .finish(),
            );
        }
        remembered.push(self.text(
            "相同网络内会尝试重连已配对的设备；手机关闭无线调试后需要重新开启。",
            CAPTION,
            p.text3,
        ));
        if self.error && self.connect_guide_step.is_none() {
            sections.push(self.panel(vec![self.text(&self.status, BODY, p.danger)]));
        }
        if self.doctor_running {
            sections.push(self.panel(vec![self.lead(
                S2,
                self.content_icon(UiIcon::Activity, 16., p.text2),
                self.text("正在检查手机与电脑；资料同步继续运行", CAPTION, p.text2),
            )]));
        } else if let Some(report) = self.doctor_report.as_ref().filter(|r| {
            self.selected.is_some()
                && r.topic == stepx_companion::doctor_harness::DoctorTopic::CompanionSelfHeal
        }) {
            sections.push(self.panel(vec![
                self.between(
                    self.section_heading(UiIcon::Activity, "连接诊断"),
                    self.line(&report.model_label, MICRO, p.text2),
                ),
                self.text(&report.headline, BODY, p.text),
                self.text(
                    report.ai_conclusion.as_deref().unwrap_or(&report.summary),
                    CAPTION,
                    p.text2,
                ),
                self.text(
                    if report.phone_token_used {
                        "手机账号已返回诊断建议，修复仍按实测结果核验。"
                    } else {
                        "当前显示本地实测结果；未取得手机 AI 的有效答复。"
                    },
                    MICRO,
                    p.text3,
                ),
            ]));
        }
        if self.selected.is_some() {
            sections.push(self.render_device_sync_panel());
        }
        sections.push(self.panel(vec![self.between(
            self.vstack(
                S1,
                vec![
                    self.heading("设备管理"),
                    self.text(
                        format!(
                            "{} 台已信任手机 · 可添加手机或管理已保存的连接",
                            self.preferences.devices.len()
                        ),
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            self.wrap(vec![
                self.button_icon(
                    "conn-manage-toggle",
                    if self.connection_manage_open {
                        "收起设备"
                    } else {
                        "管理设备"
                    },
                    UiIcon::Settings,
                    Action::ConnectionManage(!self.connection_manage_open),
                    Btn::Secondary,
                ),
                self.button_icon(
                    "conn-pairing-toggle",
                    if self.connection_pairing_open {
                        "收起配对"
                    } else {
                        "添加手机"
                    },
                    UiIcon::Plus,
                    Action::ConnectionPairing(!self.connection_pairing_open),
                    Btn::Secondary,
                ),
            ]),
        )]));
        if self.selected.is_none() || self.connection_manage_open {
            sections.push(self.responsive_columns(
                900.,
                S4,
                S4,
                CrossAxisAlignment::Start,
                vec![(1., self.panel(channels)), (1., self.panel(remembered))],
            ));
        }
        if self.selected.is_none()
            || self.connection_pairing_open
            || self.connect_guide_step.is_some()
        {
            sections.push(self.connection_pairing_panel());
        }
        self.vstack(S4, sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stepx_companion::companion_app::CompanionStatus;
    #[test]
    fn connection_address_uses_real_port_and_keeps_ipv6_brackets() {
        let mut app = Companion::new_for_test();
        let mut phone = stepx_companion::device::Device {
            serial: "ENDPOINT_NO_GUESS".into(),
            transport: "10.0.1.2".into(),
            ..Default::default()
        };
        app.selected = Some(phone.clone());
        assert!(
            app.detected_connection_endpoint().is_none(),
            "bare IP must not become port 5555"
        );
        phone.transport = "[fd00::3]:45281".into();
        app.selected = Some(phone);
        assert_eq!(
            app.detected_connection_endpoint().as_deref(),
            Some("[fd00::3]:45281")
        );
    }

    #[test]
    fn disconnect_preserves_trust_and_blocks_automatic_reselection() {
        let mut app = Companion::new_for_test();
        let serial = "PAUSED_CONNECTION_TEST".to_string();
        app.preferences.remember(serial.clone(), "PZ261A".into());
        app.preferences.selected_serial = Some(serial.clone());
        app.selected = Some(stepx_companion::device::Device {
            serial: serial.clone(),
            transport: serial.clone(),
            ..Default::default()
        });
        app.handle_device_action(&Action::Forget);
        assert!(app.selected.is_none());
        assert!(app.connection_watch.user_paused);
        assert!(app.preferences.devices.iter().any(|d| d.serial == serial));
        assert_eq!(
            app.preferences.selected_serial.as_deref(),
            Some(serial.as_str())
        );
        app.auto_select();
        assert!(app.selected.is_none());
        // Results from work started before disconnect must not resume this client.
        app.apply(super::super::super::Outcome::Paired(
            "192.0.2.1:45281".into(),
        ));
        app.apply(super::super::super::Outcome::Qr(Some(
            "192.0.2.1:45281".into(),
        )));
        assert!(app.connection_watch.user_paused);
        assert!(app.pairing_hosts.is_empty());
        app.handle_device_action(&Action::PairCode);
        app.handle_device_action(&Action::ConnectEndpoint("invalid-address".into()));
        assert!(
            app.connection_watch.user_paused,
            "invalid input must not resume auto-connect"
        );
    }

    #[test]
    fn replacement_phone_does_not_mark_same_model_history_online() {
        use stepx_companion::device::{parse_devices, Device};
        let phone = Device {
            serial: "NEW_PHONE".into(),
            transport: "192.0.2.44:43111".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        let online = parse_devices(
            "List of devices attached\n192.0.2.44:43111 device model:PZ261A product:pandora\n",
        );
        assert!(remembered_device_is_online(
            "NEW_PHONE",
            Some(&phone),
            &online
        ));
        assert!(!remembered_device_is_online(
            "OLD_PHONE",
            Some(&phone),
            &online
        ));
        assert!(!remembered_device_is_online("NEW_PHONE", Some(&phone), &[]));
    }
    #[test]
    fn onboarding_requires_actual_install_permissions_and_wireless_connection() {
        let mut phone = CompanionStatus::default();
        assert!(guide_can_continue(0, false, false, &phone));
        assert!(!guide_can_continue(1, false, false, &phone));
        assert!(guide_can_continue(1, true, false, &phone));
        assert!(!guide_can_continue(2, true, false, &phone));
        phone.installed = true;
        assert!(guide_can_continue(2, true, false, &phone));
        phone.pairing_permission_granted = true;
        assert!(guide_can_continue(2, true, false, &phone));
        assert!(!guide_can_continue(2, false, false, &phone));
        assert!(guide_can_continue(3, true, false, &phone));
        phone.wireless_enabled = Some(false);
        assert!(guide_can_continue(3, true, false, &phone));
        assert!(!guide_can_continue(3, false, false, &phone));
        phone.wireless_enabled = Some(true);
        assert!(guide_can_continue(3, true, false, &phone));
        phone.pairing_ready = true;
        assert!(guide_can_continue(3, true, false, &phone));
        assert!(!guide_can_continue(4, true, false, &phone));
        assert!(!guide_can_continue(4, false, true, &phone));
        assert!(guide_can_continue(4, true, true, &phone));
        assert!(!guide_can_continue(5, true, true, &phone));
    }
}

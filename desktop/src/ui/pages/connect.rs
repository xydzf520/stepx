use super::super::{
    icons::UiIcon, is_wireless, theme::*, widgets::*, Action, Companion, Page, PairTab, ToolPage,
    F_CONNECT, F_PAIR_ADDR, F_PAIR_CODE,
};
use facloud_ui::{assets::asset_cache::AssetSource, elements::*, platform::Cursor};

fn guide_can_continue(
    step: u8,
    connected: bool,
    wireless: bool,
    companion: &stepx_companion::companion_app::CompanionStatus,
) -> bool {
    match step {
        0 => true,
        1 => connected,
        2 => connected && companion.installed && companion.pairing_permission_granted,
        3 => connected && companion.pairing_ready && companion.wireless_enabled == Some(true),
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
            "连接成功",
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
                body.push(self.text("在手机「设置 → 关于手机 → 版本信息」连续点击「版本号」7 次，按系统提示完成验证。返回设置，搜索「开发者选项」，开启「USB 调试」。不同系统的入口名称可能略有不同。", BODY, p.text));
                body.push(self.text("首次安装伴侣需要已授权的 USB 连接。也可先用下方系统配对码建立无线连接使用现有功能，但安装与权限适配仍需 USB。未安装的伴侣无法直接扫码取得权限。", CAPTION, p.text2));
            }
            1 => {
                body.push(self.text("用支持数据传输的 USB 线连接手机和电脑，保持手机解锁。在「允许 USB 调试」弹窗核对这台电脑并允许。", BODY, p.text));
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
                body.push(self.text("安装手机界面与当前会话的扫码系统服务，并配置文件收发、背屏所需权限。安装会核验签名、权限和手机的服务接收结果；熄屏任务另以真实连接回执确认。", BODY, p.text));
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
                body.push(self.text("安装时配置相机、通知、通话状态和电池优化豁免。已适配的 StepX 172 固件支持开机恢复：重启后首次手动解锁，已配对电脑在同一局域网自动连接。新电脑、新 Wi-Fi 仍需首次授权；安装结果会说明未完成的能力。", CAPTION, p.text3));
            }
            3 => {
                body.push(self.text(if companion.pairing_ready { "手机和电脑连接同一局域网。在手机 StepX伴侣「连接」页开启无线调试，确认 Android 的当前 Wi-Fi 信任提示。不要使用访客网络或阻止设备互访的网络。" } else { "手机和电脑连接同一局域网。在手机「设置 → 开发者选项 → 无线调试」手动开启，确认系统的 Wi-Fi 信任提示。伴侣扫码服务未通过核验时，可直接使用下方系统配对码完成连接。" }, BODY, p.text));
                if !companion.pairing_ready {
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
                        Some(true) => "已检测到无线调试开启。请确认手机和电脑在同一网络后继续。",
                        Some(false) => "无线调试尚未开启；在手机确认后点击「检测伴侣状态」。",
                        None => "尚未读到无线调试状态，请先完成手机端操作并重新检测。",
                    },
                    CAPTION,
                    p.text2,
                ));
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
                body.push(self.text(if companion.pairing_ready { "在手机 StepX伴侣「连接」页点「扫码连接电脑」扫描下方二维码。也可以使用系统开发者选项中的「使用二维码配对设备」。" } else { "伴侣扫码服务尚未通过核验。请使用手机系统「开发者选项 → 无线调试 → 使用二维码配对设备」，或切换下方的 6 位配对码。" }, BODY, p.text));
                body.push(self.text("首次配对后会记住这台手机，同一局域网内自动重连。手机开启「连接管理 → 自动连接」，电脑保持 StepX Companion 运行即可。新 Wi-Fi 的系统信任授权仍需首次确认。", CAPTION, p.text2));
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
                        Action::Page(Page::Overview),
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
                    if step == 0 {
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

    fn detected_phone_ip(&self) -> Option<String> {
        let wifi_ip = self.cached_device_metrics().wifi_ipv4;
        if !wifi_ip.is_empty() && wifi_ip != "未知" && wifi_ip != "未检测" {
            return Some(wifi_ip);
        }
        if let Some(ip) = self.pairing_hosts.first() {
            return Some(ip.to_string());
        }
        if let Some(svc) = self.mdns_services.first() {
            if let Some(host) = svc.endpoint.split(':').next() {
                if !host.is_empty() {
                    return Some(host.to_string());
                }
            }
        }
        None
    }

    fn connection_pairing_panel(&self) -> El {
        let p = self.p();
        let detected_ip = self.detected_phone_ip();
        let mut rows = vec![
            self.heading("无线配对"),
            self.text(
                "Android 11 及以上支持系统无线调试。手机与电脑需要能在同一局域网互相访问。",
                CAPTION,
                p.text2,
            ),
            self.button_icon(
                "conn-usb-wireless-bridge",
                "已插数据线？一键开启同局域网无线连接（开启后可拔掉数据线）",
                UiIcon::Wifi,
                Action::EnableUsbWirelessBridge,
                Btn::Secondary,
            ),
        ];
        if let Some(ip) = &detected_ip {
            rows.push(self.button_icon(
                "conn-fill-detected-ip",
                format!("填入检测到的手机 IP ({ip}:5555)"),
                UiIcon::Bolt,
                Action::FillDetectedPhoneIp(ip.clone()),
                Btn::Ghost,
            ));
        }
        if self.selected.is_some() {
            rows.push(
                Container::new(self.text(
                    "当前手机已通过 USB 连接，系统会自动预热并维护同局域网无线备用通道，无需手动扫码配对；如需手动绑定新设备可使用下方配对方式。",
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
                } else {
                    rows.push(self.text(
                        "二维码尚未生成或已经过期，请点击生成二维码。",
                        BODY,
                        p.text2,
                    ));
                }
                rows.push(self.text(if self.companion_status.pairing_ready { "StepX伴侣 → 连接 → 扫码连接电脑，或使用 Android 系统二维码配对入口。" } else { "伴侣扫码服务尚未通过核验。请使用 Android 系统「无线调试 → 使用二维码配对设备」；没有此入口时请选择 6 位配对码。" }, CAPTION, p.text2));
                rows.push(self.button("qr-refresh", "生成新二维码", Action::Qr, Btn::Secondary));
                rows.push(self.text("扫描无响应：确认同一网络、关闭访客隔离，检查电脑防火墙，刷新二维码；也可切换配对码填写手机显示的地址。", CAPTION, p.text3));
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
        let mut channels = vec![self.heading(format!("检测到的设备 · {}", self.devices.len()))];
        if self.devices.is_empty() {
            channels.push(self.text(
                "尚未发现设备。连接 USB 并授权，或使用右侧无线配对。",
                BODY,
                p.text2,
            ));
        }
        for (i, device) in self.devices.iter().enumerate() {
            let selected = self
                .selected
                .as_ref()
                .is_some_and(|d| d.transport == device.id);
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
                                self.ui_icon(
                                    if wireless_ch {
                                        UiIcon::Wifi
                                    } else {
                                        UiIcon::Smartphone
                                    },
                                    20.,
                                    p.text2,
                                ),
                                self.heading(if device.model.is_empty() {
                                    "Android 设备"
                                } else {
                                    &device.model
                                }),
                                self.pill_badge(
                                    if wireless_ch { "无线" } else { "USB" },
                                    p.text2,
                                    p.surface,
                                ),
                            ],
                        ),
                        self.text(&device.id, CAPTION, p.text3),
                        self.text(note, CAPTION, if selected { p.success } else { p.text2 }),
                    ],
                ),
                if selected {
                    self.pill_badge("已连接", p.success_text, p.success_soft)
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
        let mut remembered = vec![self.heading("已信任的手机")];
        if self.preferences.devices.is_empty() {
            remembered.push(self.text(
                "成功验证设备后，会在此保存型号和序列号。",
                CAPTION,
                p.text2,
            ));
        }
        let multi_remembered = self.preferences.devices.len() > 1;
        for (idx, device) in self.preferences.devices.iter().enumerate() {
            let is_preferred =
                self.preferences.selected_serial.as_deref() == Some(device.serial.as_str());
            let is_online = self.selected.as_ref().is_some_and(|sel| {
                sel.serial == device.serial
                    || (!device.model.is_empty() && sel.model == device.model)
            }) || self.devices.iter().any(|d| {
                d.state == "device"
                    && (d.id == device.serial
                        || (!device.model.is_empty() && d.model == device.model))
            });
            let mut left = vec![self.text(&device.model, BODY, p.text)];
            if is_online {
                left.push(self.pill_badge("当前在线", p.success_text, p.success_soft));
            }
            if is_preferred {
                left.push(self.pill_badge("默认", p.accent, p.accent_soft));
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
        sections.push(
            Flex::row()
                .with_spacing(S4)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Expanded::new(
                        1.,
                        self.vstack(S4, vec![self.panel(channels), self.panel(remembered)]),
                    )
                    .finish(),
                )
                .with_child(Expanded::new(1., self.connection_pairing_panel()).finish())
                .finish(),
        );
        self.vstack(S4, sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stepx_companion::companion_app::CompanionStatus;
    #[test]
    fn onboarding_requires_actual_install_permissions_and_wireless_connection() {
        let mut phone = CompanionStatus::default();
        assert!(guide_can_continue(0, false, false, &phone));
        assert!(!guide_can_continue(1, false, false, &phone));
        assert!(guide_can_continue(1, true, false, &phone));
        assert!(!guide_can_continue(2, true, false, &phone));
        phone.installed = true;
        assert!(!guide_can_continue(2, true, false, &phone));
        phone.pairing_permission_granted = true;
        assert!(guide_can_continue(2, true, false, &phone));
        assert!(!guide_can_continue(2, false, false, &phone));
        assert!(!guide_can_continue(3, true, false, &phone));
        phone.wireless_enabled = Some(false);
        assert!(!guide_can_continue(3, true, false, &phone));
        phone.wireless_enabled = Some(true);
        assert!(!guide_can_continue(3, true, false, &phone));
        phone.pairing_ready = true;
        assert!(guide_can_continue(3, true, false, &phone));
        assert!(!guide_can_continue(4, true, false, &phone));
        assert!(!guide_can_continue(4, false, true, &phone));
        assert!(guide_can_continue(4, true, true, &phone));
        assert!(!guide_can_continue(5, true, true, &phone));
    }
}

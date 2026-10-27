use super::super::{
    icons::UiIcon, settings_state::SettingsAction as S, studio_state::StudioAction, theme::*,
    widgets::*, Action, BackScreenAction, Companion, Page, PushAction,
};
use facloud_ui::{elements::*, fonts::Weight};
use stepx_companion::{
    desktop_notify::{AlertScope, PopupMode},
    settings::Appearance,
};
impl Companion {
    fn setting_button(&self, id: &str, label: &str, action: S) -> El {
        self.button(id, label, Action::Settings(action), Btn::Secondary)
    }
    fn setting_button_icon(&self, id: &str, label: &str, icon: UiIcon, a: S) -> El {
        self.button_icon(id, label, icon, Action::Settings(a), Btn::Secondary)
    }
    fn setting_row(&self, title: &str, note: &str, control: El) -> El {
        let p = self.p();
        self.hstack(
            S5,
            vec![
                Expanded::new(
                    1.,
                    self.vstack(
                        S2,
                        vec![
                            self.line_w(title, SUBHEAD, p.text, Weight::Medium),
                            self.text(note, CAPTION, p.text2),
                        ],
                    ),
                )
                .finish(),
                control,
            ],
        )
    }
    fn setting_toggle(&self, id: &str, on: bool, action: Action) -> El {
        let p = self.p();
        let state = self.mouse_state(id.to_owned());
        let toggle = Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = if self.settings.value.reduced_motion {
                if hovered {
                    1.
                } else {
                    0.
                }
            } else {
                mouse.hover_progress().clamp(0., 1.)
            };
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let base_track = if on { p.success } else { p.border_strong };
            let hover_track = if on { p.accent } else { p.text2 };
            let track_bg = if mouse.is_clicked() {
                hover_track
            } else {
                facloud_ui::color::ColorU::new(
                    mix(base_track.r, hover_track.r),
                    mix(base_track.g, hover_track.g),
                    mix(base_track.b, hover_track.b),
                    mix(base_track.a, hover_track.a),
                )
            };
            let thumb_w = if mouse.is_clicked() { 20. } else { 18. };
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(if on {
                        MainAxisAlignment::End
                    } else {
                        MainAxisAlignment::Start
                    })
                    .with_child(
                        ConstrainedBox::new(
                            Container::new(Empty::new().finish())
                                .with_background_color(rgb(255, 255, 255))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
                                .finish(),
                        )
                        .with_width(thumb_w)
                        .with_height(18.)
                        .finish(),
                    )
                    .finish(),
            )
            .with_uniform_padding(3.)
            .with_background_color(track_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish()
        })
        .with_cursor(facloud_ui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        ConstrainedBox::new(toggle)
            .with_width(42.)
            .with_height(24.)
            .finish()
    }
    fn settings_dir_line(&self, id: impl Into<String>, path_str: String, content: El) -> El {
        let open_path = std::path::PathBuf::from(&path_str);
        Hoverable::new(self.mouse_state(id.into()), move |_| content)
            .with_defer_events_to_children()
            .with_cursor(facloud_ui::platform::Cursor::PointingHand)
            .on_double_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::Studio(StudioAction::OpenPath(open_path.clone())))
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::LocalDirectory(path_str.clone()),
                    at,
                ))
            })
            .finish()
    }
    fn settings_quick_dir_card(&self, prefix: &str) -> El {
        let p = self.p();
        let cfg_dir = stepx_companion::resources::user_config()
            .join("stepx-companion")
            .display()
            .to_string();
        let state_dir = stepx_companion::resources::user_state()
            .display()
            .to_string();
        self.panel(vec![
            self.heading("配置与数据目录快捷操作"),
            self.text(
                "快速复制本机配置路径，或直接打开配置、数据与日志目录（用于备份、迁移或问题排查）。",
                CAPTION,
                p.text2,
            ),
            self.settings_dir_line(
                format!("{prefix}-cfg-dir-line"),
                cfg_dir.clone(),
                self.text(format!("配置目录：{cfg_dir}"), CAPTION, p.text3),
            ),
            self.settings_dir_line(
                format!("{prefix}-state-dir-line"),
                state_dir.clone(),
                self.text(format!("数据与日志目录：{state_dir}"), CAPTION, p.text3),
            ),
            self.wrap(vec![
                self.setting_button_icon(
                    &format!("{prefix}-open-cfg-dir"),
                    "打开配置目录",
                    UiIcon::Folder,
                    S::OpenConfigDir,
                ),
                self.setting_button_icon(
                    &format!("{prefix}-open-state-dir"),
                    "打开数据与日志目录",
                    UiIcon::Folder,
                    S::OpenStateDir,
                ),
                self.button_icon(
                    format!("{prefix}-copy-cfg-dir"),
                    "复制配置目录路径",
                    UiIcon::Clipboard,
                    Action::CopyText(cfg_dir, "已复制配置目录路径".into()),
                    Btn::Ghost,
                ),
                self.button_icon(
                    format!("{prefix}-copy-state-dir"),
                    "复制数据目录路径",
                    UiIcon::Clipboard,
                    Action::CopyText(state_dir, "已复制数据与日志目录路径".into()),
                    Btn::Ghost,
                ),
            ]),
        ])
    }
    pub(in super::super) fn settings_page(&self) -> El {
        let p = self.p();
        let tab = if self.page == Page::About {
            4
        } else {
            self.settings.tab.min(4)
        };
        let s = &self.settings.draft;
        let labels = ["通用", "文件与存储", "生成文件", "通知", "关于"];
        let icons = [
            UiIcon::Settings,
            UiIcon::Folder,
            UiIcon::Briefcase,
            UiIcon::Bell,
            UiIcon::Info,
        ];
        let nav = self.vstack(
            S2,
            (0..5)
                .map(|i| {
                    self.nav_item_icon(
                        &format!("settings-tab-{i}"),
                        icons[i],
                        labels[i],
                        None,
                        tab == i as u8,
                        Action::Settings(S::Tab(i as u8)),
                    )
                })
                .collect(),
        );
        let mut cards = vec![];
        match tab {
            0 => {
                let modes = [
                    (Appearance::System, "跟随系统"),
                    (Appearance::Light, "浅色"),
                    (Appearance::Dark, "深色"),
                ];
                cards.push(self.panel(vec![
                        self.heading("外观与动画"),
                        self.setting_row(
                            "界面颜色",
                            "选择你习惯的显示方式",
                            self.hstack(
                                S2,
                                modes
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, (v, t))| {
                                        self.button(
                                            format!("settings-theme-{i}"),
                                            t,
                                            Action::Settings(S::Appearance(v)),
                                            if s.appearance == v {
                                                Btn::Primary
                                            } else {
                                                Btn::Secondary
                                            },
                                        )
                                    })
                                    .collect(),
                            ),
                        ),
                        self.divider(),
                        self.setting_row(
                            "减少动态效果",
                            "关闭页面过渡，让加载提示保持静态",
                            self.setting_toggle(
                                "settings-motion",
                                s.reduced_motion,
                                Action::Settings(S::Motion),
                            ),
                        ),
                    ]));
                cards.push(self.panel(vec![
                    self.heading("跨端同步与剪贴板互通"),
                    self.setting_row(
                        "自动双向共享电脑与手机剪贴板",
                        "开启后，在电脑或手机复制文字时自动双向同步剪贴板，直接粘贴即可使用",
                        self.setting_toggle(
                            "settings-toggle-auto-sync-clipboard",
                            self.prefs.auto_sync_clipboard,
                            Action::BackScreen(BackScreenAction::ToggleAutoSyncClipboard),
                        ),
                    ),
                    self.divider(),
                    self.setting_row(
                        "自动双向同步电脑系统日历与手机日历",
                        "开启后，日历页面与后台会定期将电脑系统日历日程与手机日历双向合并同步",
                        self.setting_toggle(
                            "settings-toggle-auto-sync-calendar",
                            self.prefs.auto_sync_calendar,
                            Action::BackScreen(BackScreenAction::ToggleAutoSyncCalendar),
                        ),
                    ),
                ]));
                cards.push(self.panel(vec![
                    self.heading("窗口与托盘"),
                    self.setting_row(
                        "显示托盘图标",
                        "收起窗口后，也能快速打开伴侣",
                        self.setting_toggle(
                            "settings-tray",
                            s.tray_icon,
                            Action::Settings(S::Tray),
                        ),
                    ),
                    self.divider(),
                    self.setting_row(
                        "点关闭后留在后台",
                        "继续接收文件和通知；在托盘菜单中退出程序",
                        self.setting_toggle(
                            "settings-close",
                            s.close_to_tray,
                            Action::Settings(S::CloseToTray),
                        ),
                    ),
                    self.text("完整窗口与简洁窗口的大小会分别记住。", CAPTION, p.text3),
                ]));
                let mut shortcut_actions = vec![self.setting_button(
                    "settings-shortcut-default",
                    "恢复默认",
                    S::ResetShortcut,
                )];
                if self.shortcuts.recording {
                    shortcut_actions.push(self.setting_button(
                        "settings-shortcut-cancel",
                        "取消录入",
                        S::CancelShortcut,
                    ));
                }
                cards.push(self.panel(vec![
                    self.heading("简洁对话快捷键"),
                    self.setting_row(
                        "快速打开 Amoo 对话",
                        "呼出简洁窗口并定位输入框，会话和草稿保持不变。",
                        self.setting_button(
                            "settings-shortcut-record",
                            &if self.shortcuts.recording {
                                "按下组合键…".into()
                            } else {
                                format!("{}  · 更改", s.compact_shortcut.label())
                            },
                            S::RecordShortcut,
                        ),
                    ),
                    self.text(
                        if self.shortcuts.recording {
                            "Ctrl / Command + Alt / Shift + 字母、数字或 F1–F12；Esc 取消。"
                        } else if self.shortcuts.note.is_empty() {
                            "程序运行时可呼出简洁对话；系统不支持时仅在窗口内生效。"
                        } else {
                            &self.shortcuts.note
                        },
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(shortcut_actions),
                    self.text(
                        "更改后点击页面底部“确认保存”，确认后生效。",
                        CAPTION,
                        p.text3,
                    ),
                ]));
                cards.push(self.panel(vec![
                    self.heading("设备与模型入口"),
                    self.setting_row(
                        "连接手机",
                        "USB、无线连接和新手引导",
                        self.button(
                            "settings-connect",
                            "管理连接",
                            Action::Page(Page::Connect),
                            Btn::Secondary,
                        ),
                    ),
                    self.divider(),
                    self.setting_row(
                        "Studio 模型服务",
                        "用于本地创建应用、技能和智能体",
                        self.setting_button("settings-model", "配置模型", S::Models),
                    ),
                ]));
                cards.push(self.studio_lab_settings());
            }
            1 => {
                let path = s
                    .download_directory
                    .clone()
                    .unwrap_or_else(stepx_companion::settings::downloads);
                let path_str = path.display().to_string();
                cards.push(self.panel(vec![
                    self.heading("下载与接收"),
                    self.setting_row(
                        "保存到",
                        "从手机接收的文件，以及另存为时的默认位置",
                        self.setting_button("settings-download", "更改", S::Downloads),
                    ),
                    self.settings_dir_line(
                        "settings-download-dir-line",
                        path_str.clone(),
                        self.text(path_str.clone(), BODY, p.text),
                    ),
                    self.text("更改后，已有文件仍保留在原位置。", CAPTION, p.text3),
                    self.wrap(vec![
                        self.setting_button(
                            "settings-download-open",
                            "打开文件夹",
                            S::OpenDownloads,
                        ),
                        self.button_icon(
                            "settings-download-copy",
                            "复制目录路径",
                            UiIcon::Clipboard,
                            Action::CopyText(path_str, "已复制下载目录路径".into()),
                            Btn::Ghost,
                        ),
                    ]),
                ]));
                let exp_folder_opt = self
                    .expansion
                    .config
                    .folder
                    .as_ref()
                    .map(|p| p.display().to_string());
                let exp_row = self.setting_row(
                    "保存位置",
                    exp_folder_opt.as_deref().unwrap_or("尚未设置"),
                    self.button(
                        "settings-expansion",
                        "设置扩容",
                        Action::Settings(S::Expansion),
                        Btn::Secondary,
                    ),
                );
                let exp_row = if let Some(exp_str) = exp_folder_opt {
                    self.settings_dir_line("settings-expansion-dir-row", exp_str, exp_row)
                } else {
                    exp_row
                };
                cards.push(self.panel(vec![
                    self.heading("手机存储扩容"),
                    self.text("将手机原文件保存到电脑，手机保留访问入口。", BODY, p.text2),
                    exp_row,
                    self.text(
                        "自动迁移的文件类型和规则，在“存储扩容”中设置。",
                        CAPTION,
                        p.text3,
                    ),
                ]));
                cards.push(self.settings_quick_dir_card("settings-storage"));
            }
            2 => {
                let work_dir_opt = s.work_directory.as_ref().map(|p| p.display().to_string());
                let work_dir_line = if let Some(ref work_dir_str) = work_dir_opt {
                    self.settings_dir_line(
                        "settings-work-dir-line",
                        work_dir_str.clone(),
                        self.text(work_dir_str.clone(), BODY, p.text),
                    )
                } else {
                    self.text("还没有选择，会使用默认工作文件夹", BODY, p.text)
                };
                let mut work_panel = vec![
                    self.heading("项目文件"),
                    self.setting_row(
                        "工作文件夹",
                        "Amoo 生成文件时读取资料、保存结果的位置",
                        self.setting_button("settings-work", "更改", S::WorkDirectory),
                    ),
                    work_dir_line,
                    self.text(
                        "更换目录不会移动旧项目，会保留当前对话和草稿。",
                        CAPTION,
                        p.text3,
                    ),
                ];
                if let Some(work_dir_str) = work_dir_opt {
                    work_panel.push(self.wrap(vec![self.button_icon(
                        "settings-work-copy",
                        "复制工作目录路径",
                        UiIcon::Clipboard,
                        Action::CopyText(work_dir_str, "已复制工作目录路径".into()),
                        Btn::Ghost,
                    )]));
                }
                cards.push(self.panel(work_panel));
                let mut sync_panel = vec![
                    self.heading("手机也能拿到成果"),
                    self.setting_row(
                        "桌面任务完成后自动发送到手机",
                        "仅发送生成的交付文件，项目源码和临时文件留在电脑",
                        self.setting_toggle(
                            "settings-sync",
                            s.sync_work_outputs,
                            Action::Settings(S::SyncOutputs),
                        ),
                    ),
                    self.text(
                        "默认关闭。开启后，通过当前手机连接传输，在伴侣 App 的“文件”中查看；断线后下次连接继续。",
                        CAPTION,
                        p.text2,
                    ),
                ];
                if !self.settings.sync_status.is_empty() {
                    sync_panel.push(self.text(&self.settings.sync_status, CAPTION, p.text3));
                }
                if self.settings.sync_status.contains("待发送") && s.sync_work_outputs {
                    sync_panel.push(self.wrap(vec![self.setting_button(
                        "settings-sync-retry",
                        "重试未完成的传输",
                        S::RetrySync,
                    )]));
                }
                cards.push(self.panel(sync_panel));
                cards.push(self.studio_lab_settings());
            }
            3 => {
                cards.push(self.panel(vec![
                        self.heading("哪些通知提醒我"),
                        self.wrap(
                            AlertScope::ALL
                                .into_iter()
                                .enumerate()
                                .map(|(i, v)| {
                                    self.button(
                                        format!("settings-scope-{i}"),
                                        v.label(),
                                        Action::Push(PushAction::SetAlertScope(v)),
                                        if self.settings.notify_draft.scope == v {
                                            Btn::Primary
                                        } else {
                                            Btn::Secondary
                                        },
                                    )
                                })
                                .collect(),
                        ),
                        self.text("只影响电脑上的提醒，不会更改手机通知。", CAPTION, p.text3),
                    ]));
                cards.push(self.panel(vec![
                        self.heading("提醒方式"),
                        self.wrap(
                            PopupMode::ALL
                                .into_iter()
                                .enumerate()
                                .map(|(i, v)| {
                                    self.button(
                                        format!("settings-popup-{i}"),
                                        v.label(),
                                        Action::Push(PushAction::SetPopupMode(v)),
                                        if self.settings.notify_draft.popup_mode == v {
                                            Btn::Primary
                                        } else {
                                            Btn::Secondary
                                        },
                                    )
                                })
                                .collect(),
                        ),
                    ]));
                cards.push(self.panel(vec![
                    self.heading("隐私与便捷操作"),
                    self.setting_row(
                        "隐藏通知正文",
                        "提醒中只显示来源，保护私人内容",
                        self.setting_toggle(
                            "settings-privacy",
                            self.settings.notify_draft.privacy_mask,
                            Action::Push(PushAction::TogglePrivacyMask),
                        ),
                    ),
                    self.divider(),
                    self.setting_row(
                        "自动复制验证码",
                        "识别到验证码后复制到电脑剪贴板",
                        self.setting_toggle(
                            "settings-otp",
                            self.settings.notify_draft.auto_copy_otp,
                            Action::Push(PushAction::ToggleAutoCopyOtp),
                        ),
                    ),
                ]));
            }
            _ => {
                if self.settings.changelog {
                    let mut lines = include_str!("../../../../CHANGELOG.md")
                        .lines()
                        .skip_while(|l| !l.starts_with("## "));
                    let mut items =
                        vec![self
                            .heading(lines.next().unwrap_or("更新日志").trim_start_matches("## "))];
                    for line in lines
                        .take_while(|l| !l.starts_with("## "))
                        .filter(|l| !l.trim().is_empty())
                    {
                        items.push(self.text(
                            format!("• {}", line.trim_start_matches("- ")),
                            BODY,
                            p.text2,
                        ));
                    }
                    items.push(self.wrap(vec![self.setting_button(
                        "settings-log-back",
                        "返回关于",
                        S::Changelog,
                    )]));
                    cards.push(self.panel(items));
                } else {
                    cards.push(self.about_page());
                    cards.push(self.desktop_update_card());
                    cards.push(self.settings_quick_dir_card("settings-about"));
                }
            }
        }
        let notes = [
            "跨端剪贴板与日历开关立即生效；启动与保活选项修改后可在底部确认保存。",
            "管理文件保存的位置。",
            "让工作资料与交付成果各归其位。",
            "选择电脑如何接收手机提醒。",
            "版本、项目信息与联系方式。",
        ];
        let mut content = vec![
            self.line_w(labels[tab as usize], 24., p.text, Weight::Bold),
            self.text(notes[tab as usize], BODY, p.text2),
        ];
        content.extend(cards);
        if !self.settings.feedback.is_empty() {
            content.push(self.text(
                &self.settings.feedback,
                CAPTION,
                if self.error { p.danger } else { p.text2 },
            ));
        }
        let body = self.sub_enter(
            Align::new(
                ConstrainedBox::new(self.vstack(S4, content))
                    .with_max_width(800.)
                    .finish(),
            )
            .top_left()
            .finish(),
        );
        let footer_opt = if self.settings_dirty() {
            Some(self.edit_footer(
                format!("{} 项更改未保存", self.settings_changes().len()),
                "先确认变更，再保存生效",
                true,
                true,
                Action::Settings(S::Discard),
                Action::Settings(S::Apply),
                "确认保存",
            ))
        } else {
            None
        };
        let mut right_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Expanded::new(1., self.scrollable(self.scroll.clone(), body)).finish());
        if let Some(footer) = footer_opt {
            right_col = right_col.with_child(footer);
        }
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                self.fixed_w(
                    180.,
                    Container::new(nav)
                        .with_horizontal_padding(S3)
                        .with_vertical_padding(S4)
                        .with_border(Border::right(1.).with_border_color(p.border))
                        .finish(),
                ),
            )
            .with_child(Expanded::new(1., right_col.finish()).finish())
            .finish()
    }
}

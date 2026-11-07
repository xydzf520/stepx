use super::super::{
    icons::UiIcon,
    notification_center_state::{NotificationAlertMode, NotificationCenterAction},
    settings_state::{SettingsAction as S, AGENT_SETTINGS_TAB},
    studio_state::StudioAction,
    theme::*,
    widgets::*,
    Action, BackScreenAction, Companion, Page, PushAction, F_CAPSULE_PROMPT, F_HOME_AGENT_PROMPT,
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
    pub(in crate::ui) fn setting_row(&self, title: &str, note: &str, control: El) -> El {
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
            self.section_heading(UiIcon::Folder, "配置与数据目录快捷操作"),
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
            self.settings.tab.min(AGENT_SETTINGS_TAB)
        };
        let s = &self.settings.draft;
        let labels = ["通用", "文件与存储", "生成文件", "通知", "关于", "Agent"];
        let icons = [
            UiIcon::Settings,
            UiIcon::Folder,
            UiIcon::Briefcase,
            UiIcon::Bell,
            UiIcon::Info,
            UiIcon::Sparkles,
        ];
        let nav = self.vstack(
            S2,
            [0, 1, 2, 3, usize::from(AGENT_SETTINGS_TAB), 4].into_iter()
                .map(|i| {
                    self.nav_item_icon(
                        &format!("settings-tab-{i}"),
                        icons[i],
                        labels[i],
                        crate::ui::shell::SidebarBadge::None,
                        tab == i as u8,
                        Action::Settings(S::Tab(i as u8)),
                    )
                })
                .collect(),
        );
        let mut cards = vec![];
        if tab==1 {
            cards.push(self.panel(vec![
                self.section_heading(UiIcon::Folder,"本机数据与备份"),
                self.text(self.interaction.storage_status.as_deref().unwrap_or("后台自动维护：每天校验备份，保留最近 7 份。日志保留 30 天，过期预览和无引用暂存自动回收。"),BODY,p.text2),
                self.setting_row("维护与备份","原文件、成果、模型和待发送文件不会被缓存清理删除。手动备份会一直保留。",
                    self.button_icon("storage-maintain",if self.interaction.storage_rx.is_some(){"维护中…"}else{"备份并维护"},UiIcon::Refresh,Action::StorageMaintain,Btn::Secondary)),
            ]));
        }
        match tab {
            0 => {
                cards.push(self.voice_settings_card());
                cards.push(self.panel(vec![
                    self.section_heading(UiIcon::Tools, "排障日志"),
                    self.setting_row(
                        "操作结果与失败原因",
                        "查看最近 30 天的本机日志，可重试读取失败的操作。",
                        self.button_icon(
                            "settings-diagnostics",
                            "查看日志",
                            UiIcon::Clock,
                            Action::ReviewHistory,
                            Btn::Secondary,
                        ),
                    ),
                ]));
                let modes = [
                    (Appearance::System, "跟随系统"),
                    (Appearance::Light, "浅色"),
                    (Appearance::Dark, "深色"),
                ];
                cards.push(self.panel(vec![
                        self.section_heading(UiIcon::Image, "外观与动画"),
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
                    self.section_heading(UiIcon::Clipboard, "跨端同步与剪贴板互通"),
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
                    self.section_heading(UiIcon::Monitor, "窗口与托盘"),
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
                    self.section_heading(UiIcon::Keyboard, "简洁对话快捷键"),
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
                    self.section_heading(UiIcon::Cpu, "设备与模型入口"),
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
                        "AI 来源与用量",
                        "首页成稿、工作任务与应用创作共用手机账号或所选独立 API",
                        self.setting_button("settings-model", "管理 AI 来源", S::Models),
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
                    self.section_heading(UiIcon::Folder, "下载与接收"),
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
                    self.section_heading(UiIcon::Storage, "手机存储扩容"),
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
                    self.section_heading(UiIcon::Folder, "项目文件"),
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
                    self.section_heading(UiIcon::Package, "手机也能拿到成果"),
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
                let nc_mode = self.notification_center.alert_mode;
                let nc_unread = self.notification_raw_unread_count();
                let global_mode = self.settings.notify_draft.popup_mode;
                if self.notification_center.is_currently_snoozed() {
                    let until = self.notification_center.snooze_until_ms.unwrap_or_default();
                    let time = chrono::DateTime::from_timestamp_millis(until)
                        .map(|t| {
                            t.with_timezone(&chrono::Local)
                                .format("%m-%d %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default();
                    cards.push(self.panel(vec![self.setting_row(
                        "弹窗已暂停",
                        &format!("{time} 自动恢复，消息仍正常同步"),
                        self.button(
                            "settings-popup-resume",
                            "立即恢复",
                            Action::NotificationCenter(NotificationCenterAction::ClearSnooze),
                            Btn::Secondary,
                        ),
                    )]));
                }

                cards.push(self.panel(vec![
                    self.between(
                        self.section_heading(UiIcon::Chat, "新消息弹窗模式 · 全局默认"),
                        self.button_icon(
                            "settings-popup-preview-latest",
                            "预览已保存设置",
                            UiIcon::Bell,
                            Action::NotificationCenter(
                                NotificationCenterAction::ReplayLatestPopups,
                            ),
                            Btn::Secondary,
                        ),
                    ),
                    self.text(
                        if self.settings.notify_draft.scope == AlertScope::Muted {
                            "电脑消息提醒当前已静音。选择下方任一提醒模式即可恢复聊天与验证码提醒。"
                        } else {
                            "选择收到手机新消息时的电脑桌面提醒方式；提醒来源仍按下方范围设置过滤。"
                        },
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(
                        PopupMode::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| {
                                self.button(
                                    format!("settings-popup-{i}"),
                                    v.label(),
                                    Action::Push(PushAction::SetPopupMode(v)),
                                    if global_mode == v {
                                        Btn::Primary
                                    } else {
                                        Btn::Secondary
                                    },
                                )
                            })
                            .collect(),
                    ),
                    self.text(
                        format!(
                            "所选全局模式：{} — {}",
                            global_mode.label(),
                            global_mode.description()
                        ),
                        CAPTION,
                        p.accent,
                    ),
                    self.divider(),
                    self.setting_row(
                        "1. 不弹窗",
                        "新消息仅在「消息」与通知中心内静默记录，不弹出桌面窗口",
                        self.button(
                            "settings-popup-row-silent",
                            if global_mode == PopupMode::Silent {
                                "已选择"
                            } else {
                                "设为默认"
                            },
                            Action::Push(PushAction::SetPopupMode(PopupMode::Silent)),
                            if global_mode == PopupMode::Silent {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    ),
                    self.setting_row(
                        "2. 隐藏内容",
                        "电脑右下角的 StepX 弹窗，仅显示来源；隐藏正文和图片",
                        self.button(
                            "settings-popup-row-banner",
                            if global_mode == PopupMode::Banner {
                                "已选择"
                            } else {
                                "设为默认"
                            },
                            Action::Push(PushAction::SetPopupMode(PopupMode::Banner)),
                            if global_mode == PopupMode::Banner {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    ),
                    self.setting_row(
                        "3. 显示内容",
                        "显示消息摘要和对应图片；支持后台回复时显示输入框",
                        self.button(
                            "settings-popup-row-full",
                            if global_mode == PopupMode::FullReply {
                                "已选择"
                            } else {
                                "设为默认"
                            },
                            Action::Push(PushAction::SetPopupMode(PopupMode::FullReply)),
                            if global_mode == PopupMode::FullReply {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    ),
                ]));

                let mut app_entries: Vec<(String, String)> = vec![
                    ("com.tencent.mm".into(), "微信".into()),
                    ("com.ss.android.lark".into(), "飞书".into()),
                    ("com.alibaba.android.rimet".into(), "钉钉".into()),
                    ("com.tencent.wework".into(), "企业微信".into()),
                    ("com.android.mms".into(), "短信与验证码".into()),
                    ("com.android.dialer".into(), "电话与未接来电".into()),
                    ("com.android.calendar".into(), "日历日程提醒".into()),
                    ("com.android.deskclock".into(), "闹钟与倒计时提醒".into()),
                    ("com.cainiao.wireless".into(), "快递与外卖实况".into()),
                    ("com.tencent.mobileqq".into(), "QQ".into()),
                    ("com.xingin.xhs".into(), "小红书".into()),
                    ("com.taobao.taobao".into(), "淘宝".into()),
                    ("com.jingdong.app.mall".into(), "京东".into()),
                    ("com.sankuai.meituan".into(), "美团".into()),
                    ("com.ss.android.ugc.aweme".into(), "抖音".into()),
                    ("tv.danmaku.bili".into(), "哔哩哔哩".into()),
                    ("com.eg.android.AlipayGphone".into(), "支付宝".into()),
                    ("com.whatsapp".into(), "WhatsApp".into()),
                    ("org.telegram.messenger".into(), "Telegram".into()),
                ];
                let mut seen_pkgs: std::collections::HashSet<String> =
                    app_entries.iter().map(|(pkg, _)| pkg.clone()).collect();
                for t in &self.push.snapshot.threads {
                    let pkg = stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
                        t.package_name.trim(),
                    );
                    if !pkg.is_empty() && seen_pkgs.insert(pkg.clone()) {
                        let label = if !t.app_name.trim().is_empty() {
                            t.app_name.trim().to_string()
                        } else {
                            pkg.clone()
                        };
                        app_entries.push((pkg, label));
                    }
                }
                for n in &self.push.snapshot.notifications {
                    let pkg = stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
                        n.package_name.trim(),
                    );
                    if !pkg.is_empty() && seen_pkgs.insert(pkg.clone()) {
                        let label = if !n.app_name.trim().is_empty() {
                            n.app_name.trim().to_string()
                        } else {
                            pkg.clone()
                        };
                        app_entries.push((pkg, label));
                    }
                }
                for raw_pkg in self.settings.notify_draft.app_popup_modes.keys() {
                    let pkg = stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
                        raw_pkg.trim(),
                    );
                    if !pkg.is_empty() && seen_pkgs.insert(pkg.clone()) {
                        app_entries.push((pkg.clone(), pkg));
                    }
                }

                let override_count = self.settings.notify_draft.app_popup_modes.len();
                let mut per_app_children: Vec<El> = vec![
                    self.between(
                        self.section_heading(UiIcon::Apps, "各应用独立弹窗设置（支持全部应用）"),
                        self.pill_badge(
                            if override_count > 0 {
                                format!("{override_count} 个独立配置")
                            } else {
                                "全部跟随全局".into()
                            },
                            if override_count > 0 { p.accent } else { p.text2 },
                            if override_count > 0 {
                                p.accent_soft
                            } else {
                                p.tag_bg
                            },
                        ),
                    ),
                    self.text(
                        "可为各应用单独选择提醒方式；完整消息窗口仅在手机提供后台回复入口时显示发送框。隐藏正文设置优先于完整消息模式。",
                        CAPTION,
                        p.text2,
                    ),
                ];

                let range = self.list_page_range("notification-app-settings",app_entries.len(),30);
                per_app_children.push(self.list_page_controls("notification-app-settings",app_entries.len(),30));
                for (idx, (pkg, app_label)) in app_entries.into_iter().enumerate().skip(range.start).take(range.len()) {
                    if idx > 0 {
                        per_app_children.push(self.divider());
                    }
                    let override_mode = self.settings.notify_draft.app_popup_mode_override(&pkg);
                    let effective_mode = self.settings.notify_draft.effective_popup_mode(&pkg);
                    let status_desc = match override_mode {
                        Some(m) => format!("独立设置：{} · {}", m.short_label(), pkg),
                        None => format!(
                            "跟随全局（当前：{}） · {}",
                            effective_mode.short_label(),
                            pkg
                        ),
                    };
                    let buttons = self.wrap(vec![
                        self.button(
                            format!("settings-app-popup-inherit-{idx}"),
                            "跟随全局",
                            Action::Push(PushAction::SetAppPopupMode {
                                package_name: pkg.clone(),
                                mode: None,
                            }),
                            if override_mode.is_none() {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            format!("settings-app-popup-silent-{idx}"),
                            PopupMode::Silent.short_label(),
                            Action::Push(PushAction::SetAppPopupMode {
                                package_name: pkg.clone(),
                                mode: Some(PopupMode::Silent),
                            }),
                            if override_mode == Some(PopupMode::Silent) {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            format!("settings-app-popup-banner-{idx}"),
                            PopupMode::Banner.short_label(),
                            Action::Push(PushAction::SetAppPopupMode {
                                package_name: pkg.clone(),
                                mode: Some(PopupMode::Banner),
                            }),
                            if override_mode == Some(PopupMode::Banner) {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            format!("settings-app-popup-full-{idx}"),
                            PopupMode::FullReply.short_label(),
                            Action::Push(PushAction::SetAppPopupMode {
                                package_name: pkg.clone(),
                                mode: Some(PopupMode::FullReply),
                            }),
                            if override_mode == Some(PopupMode::FullReply) {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    ]);
                    per_app_children.push(self.setting_row(&app_label, &status_desc, buttons));
                }
                cards.push(self.panel(per_app_children));

                cards.push(self.panel(vec![
                    self.between(
                        self.section_heading(UiIcon::Chat, "消息中心与桌面弹窗"),
                        self.pill_badge(
                            if nc_unread > 0 {
                                format!("{nc_unread} 未读")
                            } else {
                                "0 未读".into()
                            },
                            if nc_unread > 0 && nc_mode.allows_badge() {
                                p.danger
                            } else {
                                p.text2
                            },
                            if nc_unread > 0 && nc_mode.allows_badge() {
                                p.danger_soft
                            } else {
                                p.tag_bg
                            },
                        ),
                    ),
                    self.text(
                        "统一收纳手机应用推送、未接来电与短信验证码、应用更新、跨端协同任务及系统状态。",
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button_icon(
                            "settings-nc-mode-popup",
                            "弹窗与红点",
                            UiIcon::Bell,
                            Action::NotificationCenter(NotificationCenterAction::SetAlertMode(
                                NotificationAlertMode::PopupAndBadge,
                            )),
                            if nc_mode == NotificationAlertMode::PopupAndBadge {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            "settings-nc-mode-badge",
                            "仅显示红点",
                            Action::NotificationCenter(NotificationCenterAction::SetAlertMode(
                                NotificationAlertMode::BadgeOnly,
                            )),
                            if nc_mode == NotificationAlertMode::BadgeOnly {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button_icon(
                            "settings-nc-mode-muted",
                            "关闭提醒",
                            UiIcon::Mute,
                            Action::NotificationCenter(NotificationCenterAction::SetAlertMode(
                                NotificationAlertMode::Muted,
                            )),
                            if nc_mode == NotificationAlertMode::Muted {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    ]),
                    self.text(nc_mode.description(), CAPTION, p.text3),
                    self.divider(),
                    self.wrap(vec![
                        self.button_icon(
                            "settings-nc-open-drawer",
                            "打开消息中心",
                            UiIcon::Bell,
                            Action::NotificationCenter(NotificationCenterAction::ToggleDrawer),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "settings-nc-mark-all-read",
                            "全部已读",
                            UiIcon::Check,
                            Action::NotificationCenter(NotificationCenterAction::MarkAllRead),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "settings-nc-clear",
                            "清理已读",
                            UiIcon::Trash,
                            Action::NotificationCenter(
                                NotificationCenterAction::ClearReadAndDismissed,
                            ),
                            Btn::Ghost,
                        ),
                    ]),
                ]));
                cards.push(self.panel(vec![
                        self.section_heading(UiIcon::Bell, "哪些通知提醒我"),
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
                    self.section_heading(UiIcon::Shield, "隐私与便捷操作"),
                    self.setting_row(
                        "弹窗防窥保护（仅在桌面弹窗中隐藏消息正文，进入消息页正常显示）",
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
            AGENT_SETTINGS_TAB => {
                cards.push(self.render_agent_workbench());
                cards.push(self.panel(vec![
                    self.section_heading(UiIcon::Layers, "何时调用 Agent"),
                    self.setting_row(
                        "首页指令自动接手",
                        "明确的搜索、复盘或设备指令走本地。其余请求用对应场景整理成草稿。",
                        self.setting_toggle(
                            "settings-home-agent",
                            s.home_agent_enabled,
                            Action::Settings(S::HomeAgent),
                        ),
                    ),
                    self.text("本地直达示例\n搜索 项目评审 PDF → 打开真实搜索结果\n日程 2026-11-09 14:30 项目评审 → 预填日历，核对后添加\n打开日历、日总结、专注25分钟 → 使用现有入口", CAPTION, p.text2),
                    self.text("找资料给搜索词，拟回复给可复制草稿，讨论提取结论与待办，日程保留已知时间，方案给建议和检查清单。缺资料时会提示补充。", CAPTION, p.text2),
                    self.between(
                        self.heading("工作台提示词"),
                        self.setting_button("settings-home-agent-reset", "恢复默认", S::ResetHomeAgentPrompt),
                    ),
                    self.input(F_HOME_AGENT_PROMPT, "输入 Agent 的表达偏好与要求", None),
                    self.setting_button_icon("agent-model-settings", "模型设置", UiIcon::Settings, S::Models),
                ]));
                cards.push(self.panel(vec![
                    self.section_heading(UiIcon::Package, "实况胶囊"),
                    self.setting_row(
                        "Agent 复核物流方向",
                        "本地规则先判断。开启后只发送提取的状态短语，沿用已选模型来源。",
                        self.setting_toggle(
                            "settings-capsule-agent",
                            s.capsule_agent_enabled,
                            Action::Settings(S::CapsuleAgent),
                        ),
                    ),
                    self.between(
                        self.heading("Agent 提示词"),
                        self.button(
                            "settings-capsule-prompt-reset",
                            "恢复默认",
                            Action::Settings(S::ResetCapsulePrompt),
                            Btn::Ghost,
                        ),
                    ),
                    self.input(F_CAPSULE_PROMPT, "输入物流方向判断规则", None),
                    self.text(
                        if self.settings.value.capsule_agent_enabled && !self.fulfillment.status.is_empty() {
                            self.fulfillment.status.as_str()
                        } else {
                            "已送达的提醒在查看后收起，未查看时最多保留 10 分钟。"
                        },
                        CAPTION,
                        p.text2,
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
                            format!("• {}", line.trim_start_matches("- ").replace("**", "")),
                            BODY,
                            p.text2,
                        ));
                    }
                    items.push(self.wrap(vec![
                        self.button(
                            "settings-log-open-whats-new",
                            "弹出版本更新卡片",
                            Action::DesktopUpdate(
                                super::super::desktop_update_state::DesktopUpdateAction::OpenWhatsNew,
                            ),
                            Btn::Primary,
                        ),
                        self.setting_button(
                            "settings-log-back",
                            "返回关于",
                            S::Changelog,
                        ),
                    ]));
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
            "规则先判断，Agent 处理需要理解的内容。先看草稿，再决定下一步。",
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

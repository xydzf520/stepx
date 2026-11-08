use super::super::{
    backscreen_state::BackScreenAction, icons::UiIcon, theme::*, update_state::UpdateAction,
    widgets::*, Action, Companion, Page, ToolOperation, ToolPage, F_OV_TEXT,
    F_PHONE_SCHEDULE_TITLE,
};
use facloud_ui::{elements::*, fonts::Weight};
use stepx_companion::{storage::format_bytes, toolbox::FileInfo};

impl Companion {
    fn storage_actions(&self) -> Vec<El> {
        if self.busy {
            return vec![self.disabled_button("正在处理…")];
        }
        vec![
            self.button(
                "tools-browse-files",
                "浏览手机文件",
                Action::Page(Page::Files),
                Btn::Secondary,
            ),
            self.button(
                "tools-settings",
                "手机系统存储设置",
                Action::OpenStorageSettings,
                Btn::Secondary,
            ),
            self.button(
                "tools-scan",
                "重新扫描",
                Action::ScanStorage,
                if self.tools_pending.is_some() {
                    Btn::Secondary
                } else {
                    Btn::Primary
                },
            ),
        ]
    }

    fn tool_file_row(
        &self,
        index: usize,
        file: &FileInfo,
        operation: Option<ToolOperation>,
        keep: Option<&str>,
    ) -> El {
        let p = self.p();
        let name = file.path.rsplit('/').next().unwrap_or(&file.path);
        let mut details = vec![
            self.line_w(name, BODY, p.text, Weight::Medium),
            self.line(&file.path, CAPTION, p.text2),
        ];
        if let Some(keep) = keep {
            details.push(self.line(format!("保留：{keep}"), CAPTION, p.text3));
        }
        let menu_op = operation.clone();
        let action = match operation {
            Some(operation)
                if !self.busy
                    && (file.bytes <= 256 * 1024 * 1024
                        || matches!(operation, ToolOperation::DeleteFile(_))) =>
            {
                self.button(
                    format!("tool-op-{index}"),
                    operation.label(),
                    Action::PrepareToolOperation(operation),
                    Btn::Secondary,
                )
            }
            Some(_) => self.disabled_button(if self.busy {
                "处理中"
            } else {
                "超过 256 MiB"
            }),
            None => self.button(
                format!("tool-locate-{index}"),
                "查看文件",
                Action::LocateToolFile(file.path.clone()),
                Btn::Secondary,
            ),
        };
        self.list_row_with_menu(
            format!("tool-file-row-{index}"),
            self.between(
                self.vstack(S1, details),
                self.hstack(
                    S4,
                    vec![
                        self.mono(format_bytes(file.bytes), CAPTION, p.text2),
                        action,
                    ],
                ),
            ),
            false,
            None,
            Some(Action::LocateToolFile(file.path.clone())),
            Some(crate::ui::context_menu::Target::ToolFile {
                path: file.path.clone(),
                operation: menu_op,
            }),
        )
    }

    fn input_method_tool_page(&self) -> El {
        let p = self.p();
        let r = &self.input_method;
        self.panel(vec![
            self.heading("小企鹅中文输入法"),
            self.text("开源中文输入法，支持拼音、中文候选与中英文切换。",BODY,p.text2),
            self.divider(),
            self.line(if !r.checked {"尚未读取手机状态"} else if r.active {"默认输入法：小企鹅 Fcitx5"} else {"默认输入法：其他输入法"},BODY,p.text),
            self.text(if !r.checked {"读取后显示真实安装与默认状态。"} else if !r.sogou_installed {"搜狗：当前用户未安装"} else if r.sogou_disabled {"搜狗：已停用"} else {"搜狗：已安装，可在切换后停用"},CAPTION,p.text2),
            self.wrap(if self.busy {vec![self.disabled_button("正在处理，请稍候…")]} else {vec![
                self.button("ime-install","一键安装并使用",Action::SwitchInputMethodFcitx5(true),Btn::Primary),
                self.button("ime-refresh","刷新状态",Action::InspectInputMethod,Btn::Secondary),
                self.button("ime-settings","手机输入法设置",Action::OpenInputSettings,Btn::Secondary),
                self.button("ime-restore","恢复搜狗…",Action::SwitchInputMethodFcitx5(false),Btn::Ghost),
            ]}),
            self.text("确认后应用到当前手机：校验安装包 → 启用小企鹅 → 核对默认输入法 → 停用搜狗。已有输入法和词库不删除，系统安全输入法保留。",CAPTION,p.text2),
            self.link_muted("ime-source","官方项目与开源说明",Action::StudioOpenUrl("https://github.com/fcitx5-android/fcitx5-android".into())),
        ])
    }

    fn storage_tool_page(&self) -> El {
        let p = self.p();
        let mut sections = Vec::new();
        if self.error {
            sections.push(self.panel(vec![self.text(&self.status, BODY, p.danger)]));
        }
        if let Some(report) = &self.storage_report {
            let metrics = [
                ("手机可用", format_bytes(report.available_bytes)),
                ("数据分区总量", format_bytes(report.total_bytes)),
                ("重复副本大小", format_bytes(report.duplicate_bytes)),
            ]
            .into_iter()
            .map(|(label, value)| {
                Expanded::new(
                    1.,
                    self.vstack(
                        S2,
                        vec![
                            self.line(label, CAPTION, p.text2),
                            self.line_w(value, METRIC, p.text, Weight::Semibold),
                        ],
                    ),
                )
                .finish()
            })
            .collect();
            sections.push(self.panel(vec![
                self.hstack(S5, metrics),
                self.text(
                    "重复副本大小是文件内容大小；实际可用空间以处理后的手机读数为准。",
                    CAPTION,
                    p.text3,
                ),
            ]));
        }
        if let Some(result) = &self.tools_last_result {
            sections.push(self.panel(vec![self.text(result, CAPTION, p.text)]));
        }

        let (system_title, system_note) = match &self.system_compression {
            None => (
                "应用透明压缩 · 待检测",
                "检查当前固件是否支持保持原路径、直接使用的系统压缩。",
            ),
            Some(s) if !s.kernel_support => {
                ("应用透明压缩 · 内核不支持", "当前固件缺少系统压缩能力。")
            }
            Some(s) if !s.volume_support => (
                "应用透明压缩 · 当前不可用",
                "当前数据分区未开启压缩特性，仅提升应用权限无法启用。",
            ),
            Some(_) => (
                "应用透明压缩 · 需要适配",
                "检测到存储支持；应用更新、写入和还原兼容性尚未验证。",
            ),
        };
        sections.push(self.panel(vec![self.between(
            self.vstack(
                S1,
                vec![
                    self.heading(system_title),
                    self.text(system_note, CAPTION, p.text2),
                ],
            ),
            if self.busy {
                self.disabled_button("检测能力")
            } else {
                self.button(
                    "tools-probe",
                    "检测能力",
                    Action::ProbeSystemCompression,
                    Btn::Secondary,
                )
            },
        )]));
        sections.push(self.panel(vec![self.between(
            self.vstack(
                S1,
                vec![
                    self.heading("应用缓存"),
                    self.text(
                        "由手机系统清理可再生成的缓存，应用后续使用时会重新生成。",
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            if self.busy {
                self.disabled_button("清理缓存")
            } else {
                self.button(
                    "tools-cache",
                    "清理缓存",
                    Action::PrepareToolOperation(ToolOperation::ClearCache),
                    Btn::Secondary,
                )
            },
        )]));

        if let Some(operation) = &self.tools_pending {
            let explanation = match operation {
                ToolOperation::ClearCache => "请求手机系统清理所有应用可回收的缓存。缓存的图片等内容可能需要重新加载；不执行清除应用数据或卸载。".to_string(),
                ToolOperation::Compress(_) => "压缩并校验后，以 .gz 文件替换原文件，释放手机空间。使用前需要还原；不会备份到电脑。".to_string(),
                ToolOperation::Restore(_) => "还原后移除 .gz 文件，会占用更多手机空间；已有同名文件时不覆盖。".to_string(),
                ToolOperation::DeleteFile(_) => "永久删除此文件以释放手机空间。此操作不经过回收站，无法撤销。".to_string(),
                ToolOperation::DeleteDuplicate { keep, .. } => format!("重新核对两份内容一致后，永久删除此副本。保留：{}。此操作不经过回收站。", keep.path),
            };
            sections.push(self.panel(vec![
                    self.heading(format!("确认{}", operation.label())),
                    self.text(
                        operation
                            .file()
                            .map(|f| f.path.as_str())
                            .unwrap_or("手机内部存储 · 应用缓存"),
                        BODY,
                        p.text,
                    ),
                    self.text(explanation, CAPTION, p.text2),
                    self.hstack(
                        S2,
                        vec![
                            self.button(
                                "tool-cancel",
                                "取消",
                                Action::CancelToolOperation,
                                Btn::Secondary,
                            ),
                            self.button(
                                "tool-confirm",
                                "确认处理",
                                Action::ConfirmToolOperation,
                                if matches!(
                                    operation,
                                    ToolOperation::DeleteDuplicate { .. }
                                        | ToolOperation::DeleteFile(_)
                                ) {
                                    Btn::DangerSolid
                                } else {
                                    Btn::Primary
                                },
                            ),
                        ],
                    ),
                ]));
        }
        sections.push(
            self.hstack(
                S2,
                ["文档压缩", "重复文件", "大文件"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, label)| {
                        self.button(
                            format!("tools-tab-{i}"),
                            if self.tools_tab as usize == i {
                                format!("{label} · 当前")
                            } else {
                                label.to_string()
                            },
                            Action::ToolTab(i as u8),
                            Btn::Secondary,
                        )
                    })
                    .collect(),
            ),
        );
        let Some(report) = &self.storage_report else {
            sections.push(self.panel(vec![
                self.heading(if self.busy {
                    "正在读取手机存储"
                } else {
                    "等待扫描"
                }),
                self.text(
                    "扫描下载、文档、相册、图片、视频和音乐目录。应用私有数据不在扫描范围。",
                    CAPTION,
                    p.text2,
                ),
            ]));
            return self.vstack(S4, sections);
        };
        let scope = format!(
            "{} · 已扫描 {} 个文件 · {} · {} 个目录项无法读取 · {} 个文件未完成重复校验",
            super::format_time(report.scanned_at_ms),
            report.scanned_files,
            if report.limited {
                "达到扫描上限，结果不完整"
            } else {
                "公开目录扫描完成"
            },
            report.unreadable,
            report.hash_skipped
        );
        sections.push(self.text(scope, CAPTION, p.text2));
        sections.push(self.text(
            match self.tools_tab {
                0 => "仅压缩文本文档，每个文件最多 256 MiB。压缩无收益时保留原文件。",
                1 => "按内容校验重复项，每组保留第一份。删除前会再次校验，并显示具体路径。",
                _ => "按大小列出前 20 个文件。确认后永久删除，不经过回收站。",
            },
            CAPTION,
            p.text3,
        ));

        let mut entries: Vec<(&FileInfo, Option<ToolOperation>, Option<&str>)> = vec![];
        match self.tools_tab {
            0 => {
                for f in &report.documents {
                    let op = if f.path.ends_with(".gz") {
                        ToolOperation::Restore(f.clone())
                    } else {
                        ToolOperation::Compress(f.clone())
                    };
                    entries.push((f, Some(op), None));
                }
            }
            1 => {
                for group in &report.duplicates {
                    if let Some(keep) = group.first() {
                        for f in group.iter().skip(1) {
                            entries.push((
                                f,
                                Some(ToolOperation::DeleteDuplicate {
                                    file: f.clone(),
                                    keep: keep.clone(),
                                }),
                                Some(&keep.path),
                            ));
                        }
                    }
                }
            }
            _ => {
                for f in &report.largest {
                    entries.push((f, Some(ToolOperation::DeleteFile(f.clone())), None));
                }
            }
        }
        if entries.is_empty() {
            sections.push(self.panel(vec![self.text(
                match self.tools_tab {
                    0 => "扫描范围内没有可压缩或还原的文本文档。",
                    1 => "已校验的文件中没有发现重复副本。",
                    _ => "扫描范围内没有文件。",
                },
                BODY,
                p.text2,
            )]));
        } else {
            let page_count = entries.len().div_ceil(10);
            let page = self.tools_result_page.min(page_count - 1);
            sections.push(
                self.list(
                    entries
                        .iter()
                        .enumerate()
                        .skip(page * 10)
                        .take(10)
                        .map(|(i, (f, op, keep))| self.tool_file_row(i, f, op.clone(), *keep))
                        .collect(),
                ),
            );
            let mut pagination = vec![self.line(
                format!(
                    "共 {} 项 · 第 {} / {} 页",
                    entries.len(),
                    page + 1,
                    page_count
                ),
                CAPTION,
                p.text2,
            )];
            if page > 0 {
                pagination.push(self.button(
                    "tools-prev",
                    "上一页",
                    Action::ToolResultPage(page - 1),
                    Btn::Ghost,
                ));
            }
            if page + 1 < page_count {
                pagination.push(self.button(
                    "tools-next",
                    "下一页",
                    Action::ToolResultPage(page + 1),
                    Btn::Ghost,
                ));
            }
            sections.push(self.hstack(S3, pagination));
        }
        self.vstack(S4, sections)
    }
}

impl Companion {
    pub(in super::super) fn tools_actions(&self) -> Vec<El> {
        if self.tool_page.is_none()
            || (self.selected.is_none() && self.tool_page != Some(ToolPage::Weather))
        {
            return vec![];
        }
        if self.busy {
            return vec![self.disabled_button("正在处理…")];
        }
        match self.tool_page {
            Some(ToolPage::Weather) => {
                let mut actions = vec![self.button_icon(
                    "weather-hdr-refresh",
                    "刷新天气",
                    UiIcon::Refresh,
                    Action::WeatherRefresh,
                    Btn::Secondary,
                )];
                if self.weather_snapshot.from_live_api || self.demo_mode {
                    actions.push(self.button(
                        "weather-hdr-amoo",
                        "带入当前天气问 Amoo",
                        Action::WeatherBringToAmoo(None),
                        Btn::Primary,
                    ));
                }
                actions
            }
            Some(ToolPage::Storage) => self.storage_actions(),
            Some(ToolPage::Google) => vec![self.button(
                "google-check",
                "检测状态",
                Action::GoogleSuiteInspect,
                Btn::Secondary,
            )],
            Some(ToolPage::Companion) => vec![self.button(
                "companion-check",
                "检测状态",
                Action::CompanionInspect,
                Btn::Secondary,
            )],
            Some(ToolPage::Neo) => {
                vec![self.button("neo-check", "检测状态", Action::NeoInspect, Btn::Secondary)]
            }
            Some(ToolPage::Services) => vec![self.button(
                "services-check",
                "刷新状态",
                Action::InspectServices,
                Btn::Secondary,
            )],
            Some(ToolPage::Device) => {
                vec![self.button("device-check", "重新检测", Action::Summary, Btn::Secondary)]
            }
            Some(ToolPage::Clipboard) => vec![
                self.button(
                    "clip-hdr-pull",
                    "拉取手机剪贴板",
                    Action::BackScreen(BackScreenAction::PullPhoneClipboard),
                    Btn::Secondary,
                ),
                self.button(
                    "clip-hdr-push",
                    "推送电脑剪贴板",
                    Action::BackScreen(BackScreenAction::PushDesktopClipboard),
                    Btn::Primary,
                ),
            ],
            Some(ToolPage::McpHub) => vec![self.button_icon(
                "mcp-hdr-refresh",
                "刷新服务状态",
                UiIcon::Refresh,
                Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                Btn::Secondary,
            )],
            Some(ToolPage::AiSafety) => vec![
                self.button_icon(
                    "ai-hdr-refresh",
                    "刷新状态",
                    UiIcon::Refresh,
                    Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                    Btn::Secondary,
                ),
                self.button(
                    "ai-hdr-grant",
                    "一键修复 AI 权限",
                    Action::GrantDeepAmooPerms,
                    Btn::Primary,
                ),
            ],
            Some(ToolPage::Wellbeing) => vec![
                self.button_icon(
                    "wb-hdr-refresh",
                    "刷新统计",
                    UiIcon::Refresh,
                    Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                    Btn::Secondary,
                ),
                self.button(
                    "wb-hdr-alphame",
                    "打开 AlphaMe",
                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                        "open_alphame".into(),
                    )),
                    Btn::Primary,
                ),
            ],
            Some(ToolPage::Screen) => vec![
                self.button_icon(
                    "screen-hdr-scan-lan",
                    "扫描局域网设备",
                    UiIcon::Refresh,
                    Action::LanCast(super::super::LanCastAction::ScanDevices),
                    Btn::Secondary,
                ),
                if self.lan_cast.is_running() {
                    self.button(
                        "screen-hdr-stop-lan",
                        "停止局域网广播",
                        Action::LanCast(super::super::LanCastAction::StopServer),
                        Btn::Danger,
                    )
                } else {
                    self.button_icon(
                        "screen-hdr-start-lan",
                        "开启局域网投屏",
                        UiIcon::Wifi,
                        Action::LanCast(super::super::LanCastAction::StartServer),
                        Btn::Primary,
                    )
                },
            ],
            _ => vec![],
        }
    }

    fn utility_button(&self, id: &str, label: &str, action: Action, primary: bool) -> El {
        if self.busy || self.selected.is_none() {
            self.disabled_button(label)
        } else {
            self.button(
                id,
                label,
                action,
                if primary {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            )
        }
    }

    fn mcp_quick_btn(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        cmd: impl Into<String>,
        style: Btn,
    ) -> El {
        self.button(
            id,
            label,
            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(cmd.into())),
            style,
        )
    }

    fn tool_spectrum_bar(&self, slot: &str, segments: &[(f32, facloud_ui::color::ColorU)]) -> El {
        use stepx_companion::home_visuals::{source_spectrum_bar, Paint};
        let p = self.p();
        let paint =
            |c: facloud_ui::color::ColorU| Paint::rgb(c.r, c.g, c.b).with_alpha(c.a as f32 / 255.0);
        let segs: Vec<(f32, Paint)> = segments.iter().map(|&(w, c)| (w, paint(c))).collect();
        let svg = source_spectrum_bar(
            520.0,
            12.0,
            &segs,
            paint(p.text3).with_alpha(0.16),
            paint(p.card).with_alpha(1.0),
        );
        self.svg_canvas(slot, svg, None, 12., true)
    }

    fn tool_card(&self, tool: ToolPage) -> El {
        let p = self.p();
        let (symbol, state) = match tool {
            ToolPage::InputMethod => (
                "⌨",
                if self.input_method.active {
                    "小企鹅已启用"
                } else {
                    "安装中文输入法"
                }
                .into(),
            ),
            ToolPage::Storage => (
                "▤",
                self.storage_report
                    .as_ref()
                    .map(|s| format!("可用 {}", format_bytes(s.available_bytes)))
                    .unwrap_or("进入后扫描".into()),
            ),
            ToolPage::Google => (
                "🌐",
                if !self.google_suite.checked {
                    "待检测"
                } else if self.google_suite.compatibility_active {
                    "已安装 · 兼容补丁已应用"
                } else if self.google_suite.installed {
                    "已安装 · 查看状态"
                } else {
                    "待安装 / 完善"
                }
                .into(),
            ),
            ToolPage::Companion => (
                "📱",
                if self.companion_status.installed {
                    format!("已安装 · {}", self.companion_status.version)
                } else if self.companion_status.checked {
                    "等待安装".into()
                } else {
                    "进入后检测".into()
                },
            ),
            ToolPage::Neo => (
                "✦",
                if self.neo_status.selected {
                    "手机执行已接入"
                } else if self.neo_status.installed {
                    "已安装 · 待配置"
                } else {
                    "更新伴侣即可使用"
                }
                .into(),
            ),
            ToolPage::Apk => ("⊞", "选择本地文件".into()),
            ToolPage::Services => (
                "≡",
                if self.system_services.is_empty() {
                    "进入后检测".into()
                } else {
                    format!("{} 项服务", self.system_services.len())
                },
            ),
            ToolPage::Screen => (
                "□",
                if self.lan_cast.is_running() {
                    "局域网投屏广播中"
                } else if self.mirror.is_some() {
                    "正在投屏"
                } else {
                    "本机 / 局域网投屏"
                }
                .into(),
            ),
            ToolPage::Clipboard => (
                "⇄",
                if self.prefs.auto_sync_clipboard {
                    "自动双向同步中".into()
                } else if !self.back_screen.stepos_suite.phone_clipboard.is_empty() {
                    "已同步剪贴板".into()
                } else {
                    "跨端秒传".into()
                },
            ),
            ToolPage::McpHub => {
                let suite = &self.back_screen.stepos_suite;
                let discovered = suite.mcp_services.iter().filter(|s| s.discovered).count();
                (
                    "⚡",
                    if suite.capability_evidence.mcp_directory_read == Some(true) {
                        format!("已发现 {discovered} 项 · 调用待核验")
                    } else {
                        "服务目录待检测".into()
                    },
                )
            }
            ToolPage::AiSafety => {
                let suite = &self.back_screen.stepos_suite;
                (
                    "🛡",
                    if suite.aiengine_running {
                        format!(
                            "AI 引擎运行中 · {} 项已注册能力",
                            suite.aiengine_ability_count
                        )
                    } else {
                        "AI 引擎待检测".into()
                    },
                )
            }
            ToolPage::Wellbeing => {
                let wb = &self.back_screen.stepos_suite.wellbeing;
                (
                    "◉",
                    if wb.screen_time_minutes > 0 || wb.unlock_count > 0 {
                        format!(
                            "今日 {} · 解锁 {} 次",
                            stepx_companion::toolbox::format_duration_minutes(
                                wb.screen_time_minutes
                            ),
                            wb.unlock_count
                        )
                    } else {
                        "进入后读取统计".into()
                    },
                )
            }
            ToolPage::Weather => (
                "☀",
                format!(
                    "{} {}°C · {}",
                    self.weather_snapshot.city,
                    self.weather_snapshot.temp_c,
                    self.weather_snapshot.condition
                ),
            ),
            ToolPage::Device => ("◈", "读取手机状态".into()),
            ToolPage::Updates => ("↻", self.updates.title().to_owned()),
        };
        self.hover_panel_interactive(
            format!("tool-card-{tool:?}"),
            self.vstack(
                S3,
                vec![
                    self.between(
                        self.hstack(
                            S3,
                            vec![
                                self.section_icon_badge(
                                    super::super::icons::symbol(symbol).unwrap_or(UiIcon::Tools),
                                    18.,
                                    32.,
                                ),
                                self.heading(tool.title()),
                            ],
                        ),
                        self.line(
                            if self.selected.is_none()
                                && !matches!(tool, ToolPage::Updates | ToolPage::Weather)
                            {
                                "需要连接手机".into()
                            } else {
                                state
                            },
                            MICRO,
                            p.text3,
                        ),
                    ),
                    self.text(tool_card_blurb(tool), BODY, p.text2),
                    self.between(
                        Empty::new().finish(),
                        self.button(
                            format!("tool-open-{tool:?}"),
                            "打开",
                            Action::OpenTool(Some(tool)),
                            Btn::Secondary,
                        ),
                    ),
                ],
            ),
            Some(Action::OpenTool(Some(tool))),
            Some(Action::OpenTool(Some(tool))),
            Some(crate::ui::context_menu::Target::ToolCard(tool)),
        )
    }

    pub(in super::super) fn tools_page(&self) -> El {
        let p = self.p();
        let mut parts = vec![];
        if let Some(tool) = self.tool_page {
            parts.push(self.hstack(
                S2,
                vec![
                    self.button(
                        "tools-back",
                        "← 全部工具",
                        Action::OpenTool(None),
                        Btn::Ghost,
                    ),
                    self.line(format!("/ {}", tool.title()), CAPTION, p.text3),
                ],
            ));
        }
        if self.selected.is_none()
            && !matches!(self.tool_page, Some(ToolPage::Updates | ToolPage::Weather))
        {
            parts.push(self.panel(vec![self.between(
                self.text("连接手机后即可使用；可以先浏览工具。", BODY, p.text2),
                self.button(
                    "tools-connect",
                    "连接设备",
                    Action::Page(Page::Connect),
                    Btn::Primary,
                ),
            )]));
        }
        if self.busy && self.tool_page != Some(ToolPage::InputMethod) {
            let progress = self
                .google_progress
                .try_lock()
                .ok()
                .filter(|s| {
                    !s.is_empty()
                        && matches!(self.job_tool, Some(ToolPage::Google | ToolPage::Companion))
                })
                .map(|s| s.clone())
                .unwrap_or_else(|| self.status.clone());
            parts.push(self.panel(vec![self.text(progress, CAPTION, p.accent)]));
        } else if let Some((tool, message, error)) = &self.tools_feedback {
            if self.tool_page != Some(ToolPage::InputMethod)
                && (!matches!(self.tool_page, Some(ToolPage::Screen | ToolPage::Google)) || *error)
                && (self.tool_page.is_none() || self.tool_page == *tool)
            {
                let label = format!(
                    "{} · {}",
                    tool.map(ToolPage::title).unwrap_or("小工具"),
                    message
                );
                parts.push(self.panel(vec![self.text(
                    label,
                    CAPTION,
                    if *error { p.danger } else { p.text2 },
                )]));
            }
        }
        if let Some(action) = self.tool_confirmation.as_deref() {
            let message = match action {
                Action::NeoConnectAmoo => "向手机 Amoo 安装后台操作技能（同名技能自动备份更新），安装后可在新建对话中委派任务。",
                Action::NeoExecutionMode(true) => "切换为「后台虚拟屏 · 系统 MDI」执行：任务在手机后台虚拟屏运行，保持解锁即可，结束自动释放。",
                Action::NeoExecutionMode(false) => "切换为「手机前台」执行：任务直接操作当前屏幕，执行期间可随时暂停或手动接管。",
                Action::NeoRestore => "停用 GUI Agent 任务路由并撤销无障碍与可选输入法配置，保留伴侣其他服务与数据。",
                Action::GoogleSuiteInstall => "安装或修复 StepX伴侣与 Google 组件，并应用 Play 兼容补丁；首次安装后需按提示重启。",
                Action::GoogleSuiteMaintain => "修复 Play 商店配置、服务权限与登录入口，并启用组件更新后自动恢复；保留账号与应用数据。",
                Action::GoogleSuiteRollback => "恢复兼容补丁修改的设置并重启 Google 服务；保留账号与已安装应用。",
                Action::ToggleSystemService(_, true) => "将重新启用此服务，它可能恢复后台运行。",
                _ => "将停用此服务，相关功能会停止；可在此处重新启用。不会卸载应用或删除数据。",
            };
            let target = match action {
                Action::NeoConnectAmoo | Action::NeoExecutionMode(_) | Action::NeoRestore => {
                    "伴侣内置 GUI Agent · 当前连接的手机".into()
                }
                Action::ToggleSystemService(package, _) => self
                    .system_services
                    .iter()
                    .find(|s| &s.package == package)
                    .map(|s| format!("{} · {}", s.name, s.package))
                    .unwrap_or_else(|| package.clone()),
                Action::GoogleSuiteMaintain | Action::GoogleSuiteRollback => {
                    "Google Play 兼容补丁 · 当前连接的手机".into()
                }
                _ => "Google Play + StepX伴侣 · 当前连接的手机".into(),
            };
            parts.push(self.panel(vec![
                self.heading("确认操作"),
                self.text(target, BODY, p.text),
                self.text(message, BODY, p.text2),
                self.hstack(
                    S2,
                    vec![
                        self.button(
                            "utility-cancel",
                            "取消",
                            Action::CancelUtility,
                            Btn::Secondary,
                        ),
                        self.utility_button(
                            "utility-confirm",
                            "确认继续",
                            Action::ConfirmUtility,
                            true,
                        ),
                    ],
                ),
            ]));
        }
        match self.tool_page {
            None => {
                parts.push(self.heading("工具集合"));
                parts.push(
                    self.segmented(
                        "tool-categories",
                        ["全部", "空间整理", "应用与服务", "设备协同"]
                            .into_iter()
                            .enumerate()
                            .map(|(i, title)| {
                                (
                                    format!(
                                        "{title} · {}",
                                        ToolPage::ALL
                                            .iter()
                                            .filter(|t| i == 0 || t.category() == i as u8)
                                            .count()
                                    ),
                                    i as u8 == self.tool_category,
                                    Action::ToolCategory(i as u8),
                                )
                            })
                            .collect(),
                    ),
                );
                let tools: Vec<_> = ToolPage::ALL
                    .into_iter()
                    .filter(|t| self.tool_category == 0 || t.category() == self.tool_category)
                    .collect();
                for row in tools.chunks(2) {
                    let mut cards: Vec<El> = row
                        .iter()
                        .map(|t| Expanded::new(1., self.tool_card(*t)).finish())
                        .collect();
                    if row.len() == 1 {
                        cards.push(Expanded::new(1., Empty::new().finish()).finish());
                    }
                    parts.push(self.hstack(S4, cards));
                }
            }
            Some(ToolPage::Weather) => parts.push(self.weather_tool_page()),
            Some(ToolPage::Updates) => parts.push(self.updates_tool_page()),
            Some(ToolPage::Companion) => parts.push(self.companion_tool_page()),
            Some(ToolPage::Neo) => parts.push(self.neo_tool_page()),
            Some(ToolPage::Clipboard) => parts.push(self.clipboard_tool_page()),
            Some(ToolPage::McpHub) => parts.push(self.mcp_hub_tool_page()),
            Some(ToolPage::AiSafety) => parts.push(self.ai_safety_tool_page()),
            Some(ToolPage::Wellbeing) => parts.push(self.wellbeing_tool_page()),
            Some(_) if self.selected.is_none() => {}
            Some(ToolPage::InputMethod) => parts.push(self.input_method_tool_page()),
            Some(ToolPage::Storage) => parts.push(self.storage_tool_page()),
            Some(ToolPage::Google) => parts.push(self.google_tool_page()),
            Some(ToolPage::Apk) => parts.push(self.panel(vec![
                self.heading("安装本地 APK"),
                self.text(
                    "选择电脑上的单个 APK 安装或更新应用（不支持 APKS / XAPK 拆分包）。",
                    BODY,
                    p.text2,
                ),
                self.hstack(
                    S2,
                    vec![
                        self.utility_button("tool-apk", "选择 APK…", Action::InstallApk, true),
                        self.button(
                            "tool-app-list",
                            "已安装应用",
                            Action::Page(Page::Apps),
                            Btn::Secondary,
                        ),
                    ],
                ),
            ])),
            Some(ToolPage::Screen) => {
                parts.push(self.screen_and_lan_cast_tool_page());
            }
            Some(ToolPage::Services) => {
                parts.push(self.panel(vec![self.text("仅列出已适配的服务。停用会影响对应功能，请逐项选择；当前输入法不允许停用。", BODY, p.text2),
                    self.utility_button("tool-ime-settings", "手机输入法设置", Action::OpenInputSettings, false)]));
                if self.system_services.is_empty() {
                    parts.push(self.panel(vec![self.text(
                        "尚无检测结果，请点击刷新状态。",
                        BODY,
                        p.text2,
                    )]));
                }
                for (i, service) in self.system_services.iter().enumerate() {
                    let can_toggle = service.safe_to_remove
                        && !service.status_note.contains("当前默认")
                        && service.status_note != "未安装";
                    parts.push(self.panel(vec![
                        self.between(
                            self.vstack(
                                S1,
                                vec![
                                    self.heading(&service.name),
                                    self.line(&service.status_note, CAPTION, p.text2),
                                ],
                            ),
                            if can_toggle {
                                self.utility_button(
                                    &format!("tool-service-{i}"),
                                    if service.enabled { "停用" } else { "启用" },
                                    Action::PrepareUtility(Box::new(Action::ToggleSystemService(
                                        service.package.clone(),
                                        !service.enabled,
                                    ))),
                                    false,
                                )
                            } else {
                                self.disabled_button("不可停用")
                            },
                        ),
                        self.text(&service.package, CAPTION, p.text3),
                    ]));
                }
            }
            Some(ToolPage::Device) => {
                let report = self
                    .selected
                    .as_ref()
                    .map(|d| {
                        format!(
                            "型号：{}\nAndroid：{}\n连接：{}\n\n{}",
                            d.model, d.android, d.transport, self.summary
                        )
                    })
                    .unwrap_or_default();
                parts.push(self.panel(vec![
                    self.heading("设备状态"),
                    self.text(
                        if self.summary.is_empty() {
                            "尚无检测结果，请点击重新检测。"
                        } else {
                            &report
                        },
                        BODY,
                        p.text2,
                    ),
                ]));
                if !self.summary.is_empty() {
                    parts.push(self.button(
                        "tool-copy-report",
                        "复制检测结果",
                        Action::CopyText(report, "已复制设备检测结果".into()),
                        Btn::Secondary,
                    ));
                }
            }
        }
        self.vstack(S4, parts)
    }

    fn updates_tool_page(&self) -> El {
        use stepx_companion::ota::{OtaStatus, VersionRelation};
        let p = self.p();
        let u = &self.updates;
        let mut summary = vec![self.between(
            self.vstack(
                S1,
                vec![
                    self.heading("手机系统更新"),
                    self.text(
                        "官方检查会打开手机更新界面，由系统确认可用版本。",
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            if u.busy || u.transfer.is_some() {
                self.disabled_button("检查中…")
            } else if self.selected.is_none() {
                self.button(
                    "ota-connect",
                    "连接手机",
                    Action::Page(Page::Connect),
                    Btn::Primary,
                )
            } else {
                self.hstack(
                    S2,
                    vec![
                        self.button(
                            "ota-channels",
                            "测试渠道",
                            Action::Updates(UpdateAction::Check),
                            Btn::Secondary,
                        ),
                        self.button(
                            "ota-check",
                            "官方检查",
                            Action::Updates(UpdateAction::CheckOfficial),
                            Btn::Primary,
                        ),
                    ],
                )
            },
        )];
        if let Some(phone) = &u.phone {
            summary.push(self.divider());
            summary.push(self.text(format!("当前系统  {}", phone.firmware), BODY, p.text));
            summary.push(self.line(
                format!("安全补丁  {}", phone.security_patch),
                CAPTION,
                p.text2,
            ));
        } else if let Some(device) = &self.selected {
            summary.push(self.text(
                format!("已连接 {}，点击检查以读取系统版本。", device.model),
                BODY,
                p.text2,
            ));
        } else {
            summary.push(self.text(
                "连接 StepX 手机后即可检测，无需配置桌面软件更新源。",
                BODY,
                p.text2,
            ));
        }
        if let Some(progress) = &u.progress {
            summary.push(self.text(progress, BODY, p.accent));
        }
        for error in [&u.error, &u.phone_error].into_iter().flatten() {
            summary.push(self.text(error, BODY, p.danger));
        }
        if let Some(notice) = &u.notice {
            summary.push(self.text(notice, CAPTION, p.text2));
        }
        if let Some((phase, done, total)) = &u.transfer_progress {
            summary.push(self.divider());
            summary.push(self.between(
                self.text(phase, BODY, p.text),
                self.button(
                    "ota-cancel-transfer",
                    "取消",
                    Action::Updates(UpdateAction::CancelDownload),
                    Btn::Ghost,
                ),
            ));
            summary.push(self.progress(
                if *total > 0 {
                    Some((done.saturating_mul(100) / total).min(100) as u8)
                } else {
                    None
                },
                p.accent,
            ));
            if *total > 0 {
                summary.push(self.line(
                    format!("{} / {}", format_bytes(*done), format_bytes(*total)),
                    CAPTION,
                    p.text2,
                ));
            }
        }
        let mut sections = vec![self.panel(summary)];
        if let Some(staged) = &u.staged {
            let can_install = u.report.as_ref().is_some_and(|r| {
                r.eligibility_verified
                    && r.channels
                        .iter()
                        .filter(|c| c.channel == "current")
                        .flat_map(|c| &c.releases)
                        .any(|release| {
                            release.slot == "new"
                                && release.relation == VersionRelation::Newer
                                && release.need_update
                                && release.exists_update
                                && release.version == staged.package.version
                                && release.sha256 == staged.package.firmware().sha256
                                && release.md5 == staged.package.firmware().md5
                                && !release.mandatory_clear
                                && release.size_bytes == staged.package.size_bytes
                        })
            });
            let mut ready = vec![
                self.heading("手机上的更新包已就绪"),
                self.text(&staged.package.version, BODY, p.text),
                self.line(format_bytes(staged.package.size_bytes), CAPTION, p.text2),
                self.text(
                    format!("手机位置：{}", staged.remote_path),
                    CAPTION,
                    p.text2,
                ),
                self.button(
                    "ota-copy-path",
                    "复制手机路径",
                    Action::CopyText(staged.remote_path.clone(), "已复制手机固件路径".into()),
                    Btn::Ghost,
                ),
            ];
            if can_install {
                ready.push(if u.busy {
                    self.disabled_button("正在处理…")
                } else {
                    self.button(
                        "ota-prepare-install",
                        "准备安装",
                        Action::Updates(UpdateAction::PrepareInstall),
                        Btn::Primary,
                    )
                });
                if u.confirm_install {
                    ready.push(self.divider());
                    ready.push(self.text(format!("即将升级当前连接的手机至 {}。安装会交给手机官方校验器，请求保留数据且不自动重启。", staged.package.version), BODY, p.text));
                    ready.push(self.text("请保持连接，完成正在手机上进行的工作。若官方策略要求清除数据，请改在手机更新器中核对处理。", CAPTION, p.text2));
                    ready.push(self.hstack(
                        S2,
                        vec![
                            self.button(
                                "ota-install-confirm",
                                "确认提交系统安装",
                                Action::Updates(UpdateAction::ConfirmInstall),
                                Btn::Primary,
                            ),
                            self.button(
                                "ota-install-cancel",
                                "暂不安装",
                                Action::Updates(UpdateAction::CancelInstall),
                                Btn::Secondary,
                            ),
                        ],
                    ));
                }
            } else {
                ready.push(self.text(
                    "此包已完成下载验证；本次尚未确认可用于这台手机的升级，不会重刷同版或降级。",
                    BODY,
                    p.text2,
                ));
            }
            sections.push(self.panel(ready));
        }
        if let Some(report) = &u.report {
            let mut results = vec![self.between(
                self.heading(u.title()),
                self.line(format!("检测时间  {}", report.checked_at), CAPTION, p.text3),
            )];
            for channel in &report.channels {
                let color = match channel.status {
                    OtaStatus::Available => p.success,
                    OtaStatus::AccessDenied
                    | OtaStatus::Failed
                    | OtaStatus::DeviceContextMissing => p.warning,
                    _ => p.text2,
                };
                results.push(self.between(
                    self.line(channel.label(), BODY, p.text),
                    self.line(channel.status_label(), BODY, color),
                ));
                if u.details && !channel.message.is_empty() {
                    results.push(self.text(&channel.message, CAPTION, p.text2));
                }
                if let Some(plan) = &channel.plan {
                    results.push(self.text(format!("发布计划：{}", plan.note), CAPTION, p.text2));
                }
            }
            results.push(self.divider());
            if report.install_status.internal_permission == Some(false) {
                results.push(self.text("此手机未开放桌面后台安装权限。下载包可校验并传到手机，安装须由手机官方更新器确认。", CAPTION, p.text2));
            }
            results.push(self.text(
                "未返回新版本不代表不存在内部构建；测试渠道以官方开放资格为准。",
                CAPTION,
                p.text3,
            ));
            results.push(self.hstack(
                S2,
                vec![self.button(
                    "ota-details",
                    if u.details {
                        "收起检测详情"
                    } else {
                        "检测详情"
                    },
                    Action::Updates(UpdateAction::ToggleDetails),
                    Btn::Ghost,
                )],
            ));
            if u.details {
                results.push(self.text(&report.limitation, CAPTION, p.text3));
                results.push(self.text(
                    format!(
                        "服务环境：{} · 本机信息：{}",
                        report.environment,
                        if report.identity_complete {
                            "已读取"
                        } else {
                            "部分信息不可读"
                        }
                    ),
                    CAPTION,
                    p.text2,
                ));
                for channel in &report.channels {
                    results.push(self.text(
                        format!(
                                "{} · HTTP {} · 服务返回码 {}",
                                channel.label(),
                                channel
                                    .http_status
                                    .map(|n| n.to_string())
                                    .unwrap_or("—".into()),
                                channel
                                    .server_code
                                    .map(|n| n.to_string())
                                    .unwrap_or("—".into())
                            ),
                        CAPTION,
                        p.text3,
                    ));
                }
            }
            sections.push(self.panel(results));
            let total = report.channels.iter().flat_map(|channel|channel.releases.iter()).map(|release|&release.version).collect::<std::collections::HashSet<_>>().len();
            let range = self.list_page_range("ota-release-list",total,20);
            sections.push(self.list_page_controls("ota-release-list",total,20));
            let mut shown = std::collections::HashSet::new();
            let mut row_index = 0;
            for (ci, channel) in report.channels.iter().enumerate() {
                for (ri, release) in channel.releases.iter().enumerate() {
                    if !shown.insert(release.version.clone()) {
                        continue;
                    }
                    let visible = range.contains(&row_index);
                    row_index += 1;
                    if !visible {continue;}
                    let state = match release.relation {
                        VersionRelation::Current => "与本机同版",
                        VersionRelation::Newer => "高于本机版本",
                        VersionRelation::Older => "低于本机版本",
                        VersionRelation::Unknown => "版本关系待确认",
                    };
                    let mut card = vec![
                        self.between(
                            self.heading(format!("{} · {}", channel.label(), state)),
                            self.line(&release.channel_label, CAPTION, p.text3),
                        ),
                        self.text(&release.version, BODY, p.text),
                        self.line(
                            format!(
                                "{} · {}",
                                release.release_date,
                                format_bytes(release.size_bytes)
                            ),
                            CAPTION,
                            p.text2,
                        ),
                    ];
                    if release.mandatory_clear {
                        card.push(self.text(
                            "官方标记：此升级需要清除数据，桌面不会自动提交安装。",
                            BODY,
                            p.warning,
                        ));
                    }
                    if release.mandatory_upgrade {
                        card.push(self.line("官方标记：必需更新", CAPTION, p.text2));
                    }
                    if !release.release_notes.is_empty() {
                        card.push(self.work_rich_text(
                            &release.release_notes,
                            &format!("ota-notes-{ci}-{ri}"),
                        ));
                    }
                    if release.has_download()
                        && (release.sha256.is_some() || release.md5.is_some())
                        && release.size_bytes > 0
                    {
                        let already_staged = u
                            .staged
                            .as_ref()
                            .is_some_and(|pkg| pkg.package.version == release.version);
                        card.push(if u.busy || u.transfer.is_some() || already_staged {
                            self.disabled_button(if already_staged {
                                "已传到手机"
                            } else {
                                "正在处理…"
                            })
                        } else {
                            self.button(
                                format!("ota-download-{ci}-{ri}"),
                                "下载到手机",
                                Action::Updates(UpdateAction::Download(ci, ri)),
                                Btn::Secondary,
                            )
                        });
                    }
                    sections.push(self.panel(card));
                }
            }
        }
        if self.selected.is_some() {
            let mut cleanup = vec![self.between(
                self.vstack(
                    S1,
                    vec![
                        self.heading("OTA 暂存空间"),
                        self.text(
                            "查看和清理此工具传到手机的固件包，可在重启桌面后继续清理。",
                            CAPTION,
                            p.text2,
                        ),
                    ],
                ),
                if u.busy || u.transfer.is_some() {
                    self.disabled_button("管理暂存包")
                } else {
                    self.button(
                        "ota-cleanup-inspect",
                        "管理暂存包",
                        Action::Updates(UpdateAction::PrepareCleanup),
                        Btn::Secondary,
                    )
                },
            )];
            if let Some(usage) = &u.cleanup {
                cleanup.push(self.text(
                    format!(
                        "手机上有 {} 个暂存包，占用 {}。",
                        usage.count,
                        format_bytes(usage.bytes)
                    ),
                    BODY,
                    p.text,
                ));
                if usage.count > 0 {
                    cleanup.push(self.text(
                        "清理前请确认手机官方更新器没有正在安装此包。",
                        CAPTION,
                        p.warning,
                    ));
                    cleanup.push(self.hstack(
                        S2,
                        vec![
                            self.button(
                                "ota-cleanup-confirm",
                                "清理这些暂存包",
                                Action::Updates(UpdateAction::ConfirmCleanup),
                                Btn::Secondary,
                            ),
                            self.button(
                                "ota-cleanup-cancel",
                                "保留",
                                Action::Updates(UpdateAction::CancelCleanup),
                                Btn::Ghost,
                            ),
                        ],
                    ));
                }
            }
            sections.push(self.panel(cleanup));
            sections.push(self.panel(vec![self.between(
                self.vstack(
                    S1,
                    vec![
                        self.heading("手机官方更新器"),
                        self.text("查看系统升级策略、报名资格与安装结果。", CAPTION, p.text2),
                    ],
                ),
                if u.busy || u.transfer.is_some() {
                    self.disabled_button("在手机查看")
                } else {
                    self.button(
                        "ota-phone-open",
                        "在手机查看",
                        Action::Updates(UpdateAction::PhoneOpen),
                        Btn::Secondary,
                    )
                },
            )]));
        }
        self.vstack(S4, sections)
    }

    fn neo_tool_page(&self) -> El {
        let p = self.p();
        let neo = &self.neo_status;
        let mut rows = vec![self.panel(vec![
            self.heading("StepX伴侣 · 内置 GUI Agent"),
            self.text("GUI Agent 集成在 StepX伴侣内，在手机上看屏、规划与执行，断开电脑后也可使用。Amoo 可以委派任务，伴侣提供暂停、继续和停止控制。", BODY, p.text2),
            self.text("选择内置 GUI Agent 的完整 StepX伴侣 APK。安装会核对签名与内置组件，保留伴侣数据，并启用本模块的无障碍服务；操作哪些应用由你在手机确认。", CAPTION, p.text3),
            self.hstack(S2, vec![
                self.utility_button("neo-install", if neo.companion_installed { "更新 StepX伴侣…" } else { "安装 StepX伴侣…" }, Action::NeoInstall, true),
                self.button("neo-ime", if self.neo_enable_ime { "增强中文输入：开启" } else { "增强中文输入：可选" }, Action::NeoIme(!self.neo_enable_ime), Btn::Secondary),
            ]),
            self.text("增强中文输入会启用 Agent 输入法；任务结束恢复原键盘。普通文字输入仍可使用无障碍。", CAPTION, p.text3),
        ])];
        if neo.checked {
            let state = |ready: bool| if ready { "已就绪" } else { "待配置" };
            rows.push(self.panel(vec![
                self.heading(if neo.installed {
                    format!("伴侣内置版本 · {}", neo.version)
                } else {
                    "等待安装内置 GUI Agent 的伴侣版本".into()
                }),
                self.text(&neo.message, BODY, p.text2),
                self.text(
                    format!(
                        "签名核验：{}   无障碍启用：{}   电池豁免：{}",
                        state(neo.signature_verified),
                        state(neo.accessibility_enabled),
                        state(neo.doze_exempt)
                    ),
                    CAPTION,
                    p.text2,
                ),
                self.text(
                    format!(
                        "任务路由：{}   增强输入法：{}   Amoo 技能：{}",
                        if neo.selected { "Neo" } else { "未启用" },
                        if neo.ime_enabled {
                            "已启用"
                        } else {
                            "未启用"
                        },
                        if neo.amoo_skill_installed {
                            "已就绪"
                        } else {
                            "待安装"
                        }
                    ),
                    CAPTION,
                    p.text2,
                ),
            ]));
        }
        if neo.installed {
            rows.push(self.panel(vec![
                self.heading("完成手机配置"),
                self.text("在手机确认模型接口、图片输入能力与允许操作的应用，再从 Amoo 发起任务。锁屏或需要本人验证时，任务会等待你接管。", BODY, p.text2),
                self.hstack(S2, vec![
                    self.utility_button("neo-open", "打开手机配置", Action::NeoOpenPhone, true),
                    self.utility_button("neo-enable", "启用任务路由", Action::NeoActivate, false),
                    self.utility_button("neo-refresh", "重新检测", Action::NeoInspect, false),
                ]),
            ]));
        }
        if neo.installed && neo.selected {
            rows.push(self.panel(vec![
                self.heading("让 Amoo 调用伴侣"),
                self.text("首次使用时，将后台操作技能安装到你的 Amoo 账号。安装结果会在真实对话中显示；成功后新建对话，说“打开京东，我的订单”即可委派。", BODY, p.text2),
                self.text("Amoo 负责委派，伴侣使用你在手机配置的视觉模型完成操作。电脑可断开；不会自动切换到手机前台执行。", CAPTION, p.text3),
                self.utility_button("neo-amoo-connect", "连接 Amoo…", Action::PrepareUtility(Box::new(Action::NeoConnectAmoo)), true),
            ]));
            let background = neo.execution_mode == "background";
            rows.push(self.panel(vec![
                self.heading("任务在哪里执行"),
                self.text(if background { "当前：后台虚拟屏 · 系统 MDI" } else { "当前：手机前台" }, BODY, p.text),
                self.text("后台虚拟屏使用原厂系统服务，模型在手机独立运行；支持的应用可在后台完成任务。同一应用可能迁入虚拟屏，手机需要保持解锁。设置后用于伴侣、电脑与 Amoo 委派的新任务。", CAPTION, p.text2),
                self.hstack(S2, vec![
                    self.utility_button("neo-mdi", "使用后台虚拟屏", Action::PrepareUtility(Box::new(Action::NeoExecutionMode(true))), background),
                    self.utility_button("neo-foreground", "使用手机前台", Action::PrepareUtility(Box::new(Action::NeoExecutionMode(false))), !background),
                ]),
            ]));
        }
        if neo.recovery_available {
            rows.push(self.panel(vec![
                self.heading("恢复设置"),
                self.text(
                    "仅撤销本机添加的 GUI Agent 组件配置。伴侣原有无障碍、后台豁免、文件服务和数据保持原样。",
                    CAPTION,
                    p.text2,
                ),
                self.utility_button(
                    "neo-restore",
                    "撤销组件配置",
                    Action::PrepareUtility(Box::new(Action::NeoRestore)),
                    false,
                ),
            ]));
        }
        self.vstack(S4, rows)
    }

    fn extract_url_from_text(text: &str) -> Option<String> {
        for token in text.split_whitespace() {
            let trimmed = token.trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\''
                        | '“'
                        | '”'
                        | '（'
                        | '）'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '<'
                        | '>'
                        | ','
                        | '，'
                        | '。'
                        | ';'
                )
            });
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Some(trimmed.to_string());
            }
        }
        None
    }

    fn extract_otp_from_text(text: &str) -> Option<String> {
        stepx_companion::toolbox::extract_otp_from_text(text)
    }

    fn extract_express_number_from_text(text: &str) -> Option<(&'static str, String)> {
        stepx_companion::toolbox::extract_express_number_from_text(text)
    }

    fn clipboard_tool_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let auto_sync = self.prefs.auto_sync_clipboard;
        let phone_clip = suite.phone_clipboard.trim();
        let typed_clip = self.fields[F_OV_TEXT].text.trim();
        let active_clip = if !phone_clip.is_empty() {
            phone_clip
        } else {
            typed_clip
        };
        let detected_url = Self::extract_url_from_text(active_clip);
        let detected_otp = if detected_url.is_none() {
            Self::extract_otp_from_text(active_clip)
        } else {
            None
        };
        let detected_express = Self::extract_express_number_from_text(active_clip);

        let mut phone_actions = vec![
            self.button(
                "clip-pull-now",
                "拉取手机剪贴板到电脑",
                Action::BackScreen(BackScreenAction::PullPhoneClipboard),
                Btn::Primary,
            ),
            if !phone_clip.is_empty() {
                self.button(
                    "clip-copy-pc",
                    "复制到电脑剪贴板",
                    Action::CopyText(
                        phone_clip.to_string(),
                        "已将手机剪贴板内容复制到电脑".into(),
                    ),
                    Btn::Secondary,
                )
            } else {
                self.disabled_button("复制到电脑剪贴板")
            },
            if !phone_clip.is_empty() {
                self.button(
                    "clip-to-amoo",
                    "带入 Amoo 对话",
                    Action::SendToAmoo(phone_clip.to_string()),
                    Btn::Secondary,
                )
            } else {
                self.disabled_button("带入 Amoo 对话")
            },
        ];
        if let Some(url) = &detected_url {
            phone_actions.push(self.button(
                "clip-open-url-pc",
                "在电脑浏览器打开链接",
                Action::StudioOpenUrl(url.clone()),
                Btn::Secondary,
            ));
            phone_actions.push(self.button(
                "clip-open-url-phone",
                "在手机浏览器打开链接",
                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                    "open_clip_url_phone".into(),
                )),
                Btn::Ghost,
            ));
        }
        if let Some(otp) = &detected_otp {
            phone_actions.push(self.button(
                "clip-copy-otp",
                format!("仅复制验证码 {otp}"),
                Action::CopyText(otp.clone(), format!("已复制验证码 {otp}")),
                Btn::Secondary,
            ));
            phone_actions.push(self.button(
                "clip-fill-otp-one-by-one",
                format!("分格验证码自动逐位填入手机 ({otp})"),
                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                    "fill_otp_one_by_one".into(),
                )),
                Btn::Primary,
            ));
        }
        if let Some((courier, _code)) = &detected_express {
            phone_actions.push(self.button(
                "clip-query-express",
                format!("在手机一键查快递 ({courier})"),
                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                    "query_express_number".into(),
                )),
                Btn::Primary,
            ));
        }

        let mut smart_badges = vec![self.pill_badge(
            if suite.checked {
                "实时桥接已就绪"
            } else {
                "支持 USB / 无线"
            },
            p.success,
            p.surface,
        )];
        if detected_url.is_some() {
            smart_badges.push(self.pill_badge("识别到网页链接", p.accent, p.accent_soft));
        }
        if let Some(otp) = &detected_otp {
            smart_badges.push(self.pill_badge(
                format!("识别到数字码 {otp}"),
                p.accent,
                p.accent_soft,
            ));
        }
        if let Some((courier, code)) = &detected_express {
            smart_badges.push(self.pill_badge(
                format!("识别到快递单号 · {courier} {code}"),
                p.success,
                p.surface,
            ));
        }

        let mut history_items: Vec<String> = self.back_screen.clipboard_history.clone();
        if !phone_clip.is_empty() && !history_items.iter().any(|h| h == phone_clip) {
            history_items.insert(0, phone_clip.to_string());
        }

        let mut history_rows = vec![
            self.between(
                self.heading("最近跨端剪贴板记录"),
                self.line(
                    format!("共 {} 条缓存记录", history_items.len()),
                    CAPTION,
                    p.text3,
                ),
            ),
            self.text(
                "本次会话中双端同步过的文本、链接或口令会自动保存在下方，支持一键复制到电脑或重新写回手机。",
                CAPTION,
                p.text2,
            ),
        ];
        if history_items.is_empty() {
            history_rows.push(self.text(
                "暂无同步记录。点击上方「拉取手机剪贴板」或「推送到手机」后，历史条目会显示在这里。",
                BODY,
                p.text3,
            ));
        } else {
            for (idx, item) in history_items.iter().enumerate() {
                history_rows.push(self.divider());
                history_rows.push(self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.mono(item, BODY, p.text),
                            self.line(
                                if idx == 0 {
                                    "最新同步内容"
                                } else {
                                    "历史同步记录"
                                },
                                MICRO,
                                p.text3,
                            ),
                        ],
                    ),
                    self.hstack(
                        S2,
                        vec![
                            self.button(
                                format!("clip-hist-copy-{idx}"),
                                "复制到电脑",
                                Action::CopyText(item.clone(), "已复制剪贴板历史条目".into()),
                                Btn::Secondary,
                            ),
                            self.button(
                                format!("clip-hist-push-{idx}"),
                                "推送到手机",
                                Action::BackScreen(BackScreenAction::PushSpecificClipboard(
                                    item.clone(),
                                )),
                                Btn::Ghost,
                            ),
                        ],
                    ),
                ));
            }
        }

        self.vstack(
            S4,
            vec![
                self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.heading("双端剪贴板双向同步舱"),
                                self.pill_badge(
                                    if auto_sync {
                                        "自动双向同步中"
                                    } else {
                                        "手动同步模式"
                                    },
                                    if auto_sync { p.success } else { p.text2 },
                                    p.surface,
                                ),
                            ],
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.button(
                                    "clip-toggle-autosync",
                                    if auto_sync {
                                        "关闭自动双向同步"
                                    } else {
                                        "开启自动双向同步"
                                    },
                                    Action::BackScreen(BackScreenAction::ToggleAutoSyncClipboard),
                                    if auto_sync {
                                        Btn::Secondary
                                    } else {
                                        Btn::Primary
                                    },
                                ),
                                self.button(
                                    "clip-inspect-now",
                                    "刷新状态",
                                    Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                                    Btn::Ghost,
                                ),
                            ],
                        ),
                    ),
                    self.text(
                        "解决电脑与手机之间频繁手动输入长链接、验证码、账号口令和终端指令的痛点。开启自动双向同步后，任意一端复制文本即可在 3 秒内自动流转至另一端。",
                        BODY,
                        p.text2,
                    ),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading("手机系统剪贴板当前内容"),
                        self.hstack(S2, smart_badges),
                    ),
                    Container::new(self.vstack(
                        S1,
                        vec![
                            self.line("StepOS 系统剪贴板实时读数 (IClipboard)", MICRO, p.text3),
                            self.mono(
                                if phone_clip.is_empty() {
                                    "（当前手机剪贴板为空，或尚未读取；点击下方「拉取手机剪贴板到电脑」即可立即读取）"
                                } else {
                                    phone_clip
                                },
                                BODY,
                                if phone_clip.is_empty() { p.text3 } else { p.text },
                            ),
                        ],
                    ))
                    .with_uniform_padding(S3)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                    self.wrap(phone_actions),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading("电脑向手机跨端速传"),
                        self.line("留空时直接读取电脑系统剪贴板推送", CAPTION, p.text3),
                    ),
                    self.input(
                        F_OV_TEXT,
                        "在此输入或粘贴文本、网页链接、口令（留空则直接推送电脑当前复制的内容）…",
                        None,
                    ),
                    self.wrap(vec![
                        self.button(
                            "clip-push-typed",
                            "推送到手机剪贴板",
                            Action::BackScreen(BackScreenAction::PushDesktopClipboard),
                            Btn::Primary,
                        ),
                        self.button(
                            "clip-push-note",
                            "写入手机剪贴板并打开 Amoo 速记",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "note_create".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "clip-typed-fill-otp",
                            "分格验证码自动逐位填入手机 (50ms/位)",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "fill_otp_one_by_one".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "clip-typed-query-express",
                            "识别快递单号并在手机查件",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "query_express_number".into(),
                            )),
                            Btn::Secondary,
                        ),
                        if !self.fields[F_OV_TEXT].text.is_empty() {
                            self.button(
                                "clip-clear-typed",
                                "清空输入框",
                                Action::ClearField(F_OV_TEXT),
                                Btn::Ghost,
                            )
                        } else {
                            Empty::new().finish()
                        },
                    ]),
                ]),
                self.panel(history_rows),
            ],
        )
    }

    pub(crate) fn mcp_service_action_key(id: &str) -> &'static str {
        if id.contains("CalendarAbs") || id.contains("CalendarEvent") {
            "calendar_event"
        } else if id.contains("AlarmMCP") {
            "alarm_15m"
        } else if id.contains("GalleryMCP") {
            "capcut_handoff"
        } else if id.contains("FileManager") {
            "open_filemanager"
        } else if id.contains("packageinstaller") {
            "open_app_manager"
        } else if id.contains("ailauncher") {
            "open_remote_agent_hub"
        } else if id.contains("xmscore") {
            "open_xms_service_numbers"
        } else if id.contains("calculator") {
            "open_calculator"
        } else if id.contains("soundrecorder") {
            "open_soundrecorder"
        } else if id.contains("VoiceAbility") {
            "voice_tts"
        } else if id.contains("settings") {
            "toggle_dnd"
        } else if id.contains("ScreenMCP") {
            "open_screen_perception"
        } else if id.contains("SystemUIMCPService") || id.contains("com.android.systemui/") {
            "open_amoo_voice"
        } else if id.contains("FeedBackMCPService")
            || id.contains("com.stepos.feedback")
            || id.contains("LogReportAbilityService")
            || id.contains("com.stepos.logreport")
        {
            "open_feedback"
        } else if id.contains("SearchManagerMcpService") || id.contains("com.stepos.net.search") {
            "search_files"
        } else if id.contains("com.stepos.safebox") || id.contains("safebox") {
            "open_safebox"
        } else if id.contains("com.stepos.safe/") {
            "open_security_scan"
        } else if id.contains("com.stepos.secureservice") || id.contains("secureservice") {
            "open_permission_manager"
        } else if id.contains("CommonAbility") || id.contains("com.stepos.common.ability.proxy") {
            "open_backscreen"
        } else if id.contains("permissioncontroller") {
            "toggle_global_camera"
        } else if id.contains("agentsafetycenter") {
            "open_safety_ledger"
        } else if id.contains("dialer") {
            "open_dialer"
        } else if id.contains("mms") {
            "open_mms"
        } else if id.contains("TaskRobot") || id.contains("agentbus") {
            "wake_amoo_assistant"
        } else {
            "open_aiengine"
        }
    }

    pub(crate) fn mcp_service_action_label(id: &str) -> &'static str {
        if id.contains("CalendarAbs") || id.contains("CalendarEvent") {
            "› 新建手机日程"
        } else if id.contains("AlarmMCP") {
            "› 设 15 分钟闹钟"
        } else if id.contains("GalleryMCP") {
            "› 剪映AI一键成片"
        } else if id.contains("FileManager") {
            "› 跨端文件寻址"
        } else if id.contains("packageinstaller") {
            "› 应用部署校验"
        } else if id.contains("ailauncher") {
            "› 远程智能体协同"
        } else if id.contains("xmscore") {
            "› 智慧信息服务号"
        } else if id.contains("calculator") {
            "› 打开科学计算器"
        } else if id.contains("soundrecorder") {
            "› 打开录音机纪要"
        } else if id.contains("VoiceAbility") {
            "› 语音合成播报"
        } else if id.contains("settings") {
            "› 切换勿扰模式"
        } else if id.contains("ScreenMCP") {
            "› 端侧视觉看屏"
        } else if id.contains("SystemUIMCPService") || id.contains("com.android.systemui/") {
            "› 唤醒语音助手"
        } else if id.contains("FeedBackMCPService") || id.contains("com.stepos.feedback") {
            "› 提交诊断反馈"
        } else if id.contains("LogReportAbilityService") || id.contains("com.stepos.logreport") {
            "› 故障健康诊断"
        } else if id.contains("SearchManagerMcpService") || id.contains("com.stepos.net.search") {
            "› 知识检索协同"
        } else if id.contains("com.stepos.safebox") || id.contains("safebox") {
            "› 私密存储保护"
        } else if id.contains("com.stepos.safe/") {
            "› 系统防护策略"
        } else if id.contains("com.stepos.secureservice") || id.contains("secureservice") {
            "› 可信执行环境"
        } else if id.contains("CommonAbility") || id.contains("com.stepos.common.ability.proxy") {
            "› 背屏与基础协同"
        } else if id.contains("permissioncontroller") {
            "› 权限自愈守护"
        } else if id.contains("agentsafetycenter") {
            "› 打开安全审计台账"
        } else if id.contains("dialer") {
            "› 通话状态协同"
        } else if id.contains("mms") {
            "› 验证码流转"
        } else if id.contains("TaskRobot") || id.contains("agentbus") {
            "› 自动化任务编排"
        } else {
            "› 打开 AI 引擎"
        }
    }

    fn mcp_hub_tool_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let directory_known = suite.capability_evidence.mcp_directory_read == Some(true);
        let discovered_count = suite
            .mcp_services
            .iter()
            .filter(|s| directory_known && s.discovered)
            .count();
        let ready_count = suite
            .mcp_services
            .iter()
            .filter(|s| directory_known && s.ready)
            .count();
        let cat_filter = self.back_screen.mcp_category_filter;
        let categories = [
            "全部",
            "日程与时间",
            "相册与文件",
            "语音与系统",
            "安全与自动化",
        ];

        let filtered_services: Vec<_> = suite
            .mcp_services
            .iter()
            .filter(|s| cat_filter == 0 || s.category == categories[cat_filter as usize])
            .collect();

        let mut service_rows = Vec::new();
        for chunk in filtered_services.chunks(2) {
            let mut row_cards = Vec::new();
            for item in chunk {
                let act_key = Self::mcp_service_action_key(&item.id);
                let act_label = Self::mcp_service_action_label(&item.id);
                let card = Container::new(self.vstack(
                    S2,
                    vec![
                        self.between(
                            self.hstack(
                                S2,
                                vec![
                                    self.line_w(&item.title, BODY, p.text, Weight::Medium),
                                    self.pill_badge(&item.category, p.accent, p.accent_soft),
                                ],
                            ),
                            self.line(
                                if !directory_known {
                                    "未检测"
                                } else if item.ready {
                                    "调用已核验"
                                } else if item.discovered {
                                    "已发现 · 调用未验证"
                                } else {
                                    "本次未发现"
                                },
                                MICRO,
                                if directory_known && item.ready {
                                    p.success
                                } else {
                                    p.text2
                                },
                            ),
                        ),
                        self.mono(&item.id, MICRO, p.text3),
                        self.between(
                            self.line("com.stepos.mcp.sdk.TOOLS_SERVICE", MICRO, p.text3),
                            self.button(
                                format!("mcp-srv-act-{}", item.id),
                                act_label,
                                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                    act_key.into(),
                                )),
                                Btn::Secondary,
                            ),
                        ),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
                row_cards.push(Expanded::new(1., card).finish());
            }
            if row_cards.len() == 1 {
                row_cards.push(Expanded::new(1., Empty::new().finish()).finish());
            }
            service_rows.push(self.hstack(S3, row_cards));
        }

        let remote_files_summary = if suite.remote_agent_files.is_empty() {
            "手机 /sdcard/Download 与 /sdcard/Documents 近期生成的智能体文档将在刷新状态后显示"
                .to_string()
        } else {
            let names: Vec<String> = suite
                .remote_agent_files
                .iter()
                .take(4)
                .map(|f| {
                    let n = f.path.rsplit('/').next().unwrap_or(&f.path);
                    format!("{n} ({})", format_bytes(f.bytes))
                })
                .collect();
            format!(
                "已识别 {} 份手机端近期文档：{}",
                suite.remote_agent_files.len(),
                names.join(" · ")
            )
        };

        self.vstack(
            S4,
            vec![
                self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.heading("StepOS 原生 MCP 能力中心（AgentBus 总线编排）"),
                                self.pill_badge(
                                    if directory_known {
                                        format!("发现 {discovered_count} 项 · 调用核验 {ready_count} 项")
                                    } else {
                                        "服务目录未检测 · 调用状态未知".into()
                                    },
                                    p.text2,
                                    p.surface,
                                ),
                                if !suite.firmware_profile.is_empty() {
                                    self.pill_badge(
                                        &suite.firmware_profile,
                                        p.accent,
                                        p.accent_soft,
                                    )
                                } else {
                                    Empty::new().finish()
                                },
                            ],
                        ),
                        self.button(
                            "mcp-refresh-top",
                            "刷新总线状态",
                            Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                            Btn::Secondary,
                        ),
                    ),
                    self.text(
                        "这里列出手机声明的 MCP 服务。发现服务不等于已获调用权限，也不代表每项工具已经执行验证；快捷按钮可能打开手机页面或设置，具体结果以手机回执为准。",
                        BODY,
                        p.text2,
                    ),
                    {
                        let mut cat_counts = [0usize; 4];
                        for s in &suite.mcp_services {
                            match s.category.as_str() {
                                "日程与时间" => cat_counts[0] += 1,
                                "相册与文件" => cat_counts[1] += 1,
                                "语音与系统" => cat_counts[2] += 1,
                                "安全与自动化" => cat_counts[3] += 1,
                                _ => {}
                            }
                        }
                        let [c1, c2, c3, c4] = cat_counts;
                        self.vstack(
                            6.,
                            vec![
                                self.tool_spectrum_bar(
                                    "tool-mcp-spectrum",
                                    &[
                                        ((c1 as f32).max(1.0), p.indigo),
                                        ((c2 as f32).max(1.0), p.blue),
                                        ((c3 as f32).max(1.0), p.teal),
                                        ((c4 as f32).max(1.0), p.success),
                                    ],
                                ),
                                self.wrap(vec![
                                    self.pill_badge(format!("日程与时间 {c1}"), p.indigo, p.indigo_soft),
                                    self.pill_badge(format!("相册与文件 {c2}"), p.blue, p.accent_soft),
                                    self.pill_badge(format!("语音与系统 {c3}"), p.teal, p.teal_soft),
                                    self.pill_badge(format!("安全与自动化 {c4}"), p.success, p.success_soft),
                                ]),
                            ],
                        )
                    },
                    self.divider(),
                    self.between(
                        self.line_w(&suite.agentbus_status, CAPTION, p.text, Weight::Medium),
                        if !suite.thermal_summary.is_empty() {
                            self.line(&suite.thermal_summary, CAPTION, p.accent)
                        } else {
                            self.line("支持 USB 与局域网无线直连调用", CAPTION, p.text3)
                        },
                    ),
                ]),
                self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.heading(
                                    "StepOS 12 量产版新生态协同舱（远程智能体 · 剪映成片 · 神券收银台 · 换机助手）",
                                ),
                                self.pill_badge("pandora-12 新增能力", p.accent, p.accent_soft),
                            ],
                        ),
                        self.line(
                            format!(
                                "背屏常亮：{}s · 翻转保持：{}",
                                suite.backscreen_stay_on_secs,
                                if suite.backscreen_flip_keep_running {
                                    "开启"
                                } else {
                                    "关闭"
                                }
                            ),
                            CAPTION,
                            p.text2,
                        ),
                    ),
                    self.text(
                        "围绕 StepOS 12 最新演进的远程智能体中心 (WorkBuddy / Claude / Codex / StepAI)、相册 50 项工具与剪映智能成片 (GalleryMCP v12.0.50)、支付宝/淘宝闪购神券收银台及 XMS 服务号短信治理打造桌面端一键联调面板：",
                        CAPTION,
                        p.text2,
                    ),
                    self.line_w(
                        "1. 远程智能体卡片与跨端文件直拉 (WorkBuddy / Claude / Codex / StepAI)",
                        CAPTION,
                        p.text,
                        Weight::Medium,
                    ),
                    self.wrap(vec![
                        self.button(
                            "mcp-eco-pull-agent-files",
                            "拉取手机远程智能体产出文件到电脑",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "pull_remote_agent_files".into(),
                            )),
                            Btn::Primary,
                        ),
                        self.button(
                            "mcp-eco-workbuddy-card",
                            "创建 WorkBuddy 远程智能体桌面卡片",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "create_workbuddy_card".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-remote-agent-hub",
                            "在手机打开远程智能体中心",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_remote_agent_hub".into(),
                            )),
                            Btn::Secondary,
                        ),
                    ]),
                    self.line(&remote_files_summary, MICRO, p.text3),
                    self.divider(),
                    self.line_w(
                        "2. 剪映 AI 智能成片 · 本地生活神券收银台 · 咪咕音乐桥接",
                        CAPTION,
                        p.text,
                        Weight::Medium,
                    ),
                    self.wrap(vec![
                        self.button(
                            "mcp-eco-capcut",
                            "相册选材交给剪映一键成片",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "capcut_handoff".into(),
                            )),
                            Btn::Primary,
                        ),
                        self.button(
                            "mcp-eco-alipay-coupon",
                            "领淘宝闪购 / 支付宝立减神券",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_alipay_coupons".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-migu-music",
                            "唤起咪咕音乐播放",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_migu_music".into(),
                            )),
                            Btn::Secondary,
                        ),
                    ]),
                    self.divider(),
                    self.line_w(
                        "3. StepOS 12 新增系统工具与背屏 12 汉字签名快控",
                        CAPTION,
                        p.text,
                        Weight::Medium,
                    ),
                    self.wrap(vec![
                        self.button(
                            "mcp-eco-data-migration",
                            "StepOS 换机助手",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_data_migration".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-xms-srv",
                            "XMS 服务号短信治理",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_xms_service_numbers".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-calc",
                            "计算器 MCP",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_calculator".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-soundrecorder",
                            "录音机 MCP",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_soundrecorder".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-lang",
                            "多语言与输入法",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_language_settings".into(),
                            )),
                            Btn::Ghost,
                        ),
                        self.button(
                            "mcp-eco-feedback",
                            "问题反馈",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_feedback".into(),
                            )),
                            Btn::Ghost,
                        ),
                        self.button(
                            "mcp-eco-bs-sig-dnd",
                            "背屏签名：开会勿扰",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "backscreen_sig:开会勿扰".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-bs-sig-focus",
                            "背屏签名：专注中",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "backscreen_sig:专注中".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-bs-flip",
                            if suite.backscreen_flip_keep_running {
                                "翻转保持运行：已开启"
                            } else {
                                "翻转保持运行：已关闭"
                            },
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "backscreen_toggle_flip_keep".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-eco-bs-15",
                            "背屏 15s",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "backscreen_stay_15".into(),
                            )),
                            if suite.backscreen_stay_on_secs == 15 {
                                Btn::Primary
                            } else {
                                Btn::Ghost
                            },
                        ),
                        self.button(
                            "mcp-eco-bs-30",
                            "背屏 30s",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "backscreen_stay_30".into(),
                            )),
                            if suite.backscreen_stay_on_secs == 30 {
                                Btn::Primary
                            } else {
                                Btn::Ghost
                            },
                        ),
                        self.button(
                            "mcp-eco-bs-60",
                            "背屏 60s",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "backscreen_stay_60".into(),
                            )),
                            if suite.backscreen_stay_on_secs == 60 {
                                Btn::Primary
                            } else {
                                Btn::Ghost
                            },
                        ),
                    ]),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading("手机系统状态快捷控制舱"),
                        self.line(
                            format!(
                                "勿扰：{} · Wi-Fi：{} · 蓝牙：{} · 亮度：{}%",
                                if suite.controls.dnd_on { "开启" } else { "关闭" },
                                if suite.controls.wifi_on { "开启" } else { "关闭" },
                                if suite.controls.bluetooth_on { "开启" } else { "关闭" },
                                suite.controls.brightness_percent
                            ),
                            CAPTION,
                            p.accent,
                        ),
                    ),
                    self.text(
                        "通过系统设置与媒体会话 MCP 通道，直接在电脑端切换手机勿扰模式、无线开关、屏幕亮度（支持 12-bit 4095 级高精背光）与媒体音量：",
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button(
                            "mcp-ctrl-dnd",
                            if suite.controls.dnd_on {
                                "勿扰模式：已开启（点击关闭）"
                            } else {
                                "勿扰模式：已关闭（点击开启）"
                            },
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "toggle_dnd".into(),
                            )),
                            if suite.controls.dnd_on {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            "mcp-ctrl-dnd-on",
                            "开启手机勿扰",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "set_dnd_on".into(),
                            )),
                            if suite.controls.dnd_on {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            "mcp-ctrl-dnd-off",
                            "关闭勿扰",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "set_dnd_off".into(),
                            )),
                            if !suite.controls.dnd_on {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            "mcp-ctrl-amoo-voice",
                            "语音唤醒设置",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_amoo_voice".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-ctrl-backscreen-nap",
                            "小憩计时（20 分钟）",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "start_back_screen_nap".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-ctrl-wifi",
                            if suite.controls.wifi_on {
                                "手机 Wi-Fi：已开启（切换）"
                            } else {
                                "手机 Wi-Fi：已关闭（开启）"
                            },
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "toggle_wifi".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-ctrl-bt",
                            if suite.controls.bluetooth_on {
                                "手机蓝牙：已开启（切换）"
                            } else {
                                "手机蓝牙：已关闭（开启）"
                            },
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "toggle_bluetooth".into(),
                            )),
                            Btn::Secondary,
                        ),
                    ]),
                    self.divider(),
                    self.between(
                        self.line("手机屏幕亮度快捷档位", CAPTION, p.text2),
                        self.hstack(
                            S2,
                            [
                                (25u8, "25% 柔和"),
                                (50u8, "50% 标准"),
                                (75u8, "75% 明亮"),
                                (100u8, "100% 户外"),
                            ]
                            .into_iter()
                            .map(|(pct, label)| {
                                self.button(
                                    format!("mcp-bright-{pct}"),
                                    label,
                                    Action::BackScreen(BackScreenAction::SetBrightnessPercent(pct)),
                                    if (suite.controls.brightness_percent as i16 - pct as i16).abs()
                                        <= 12
                                    {
                                        Btn::Primary
                                    } else {
                                        Btn::Secondary
                                    },
                                )
                            })
                            .collect(),
                        ),
                    ),
                    self.between(
                        self.line("手机媒体音量快捷调节", CAPTION, p.text2),
                        self.hstack(
                            S2,
                            vec![
                                self.button(
                                    "mcp-vol-mute",
                                    "静音",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "volume_mute".into(),
                                    )),
                                    Btn::Ghost,
                                ),
                                self.button(
                                    "mcp-vol-down",
                                    "音量 -",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "volume_down".into(),
                                    )),
                                    Btn::Secondary,
                                ),
                                self.button(
                                    "mcp-vol-50",
                                    "50% 适中",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "volume_50".into(),
                                    )),
                                    Btn::Secondary,
                                ),
                                self.button(
                                    "mcp-vol-up",
                                    "音量 +",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "volume_up".into(),
                                    )),
                                    Btn::Secondary,
                                ),
                            ],
                        ),
                    ),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading("跨应用 MCP 快捷动作执行台"),
                        self.line("支持自定义备注/标题参数", CAPTION, p.text3),
                    ),
                    self.input(
                        F_PHONE_SCHEDULE_TITLE,
                        "可选：输入闹钟备注、新建日程标题或速记内容（留空则使用默认说明）…",
                        None,
                    ),
                    self.wrap(vec![
                        self.button(
                            "mcp-act-alarm15",
                            "设 15 分钟后手机闹钟",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "alarm_15m".into(),
                            )),
                            Btn::Primary,
                        ),
                        self.button(
                            "mcp-act-alarm08",
                            "设 08:00 早间闹钟",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "alarm_0800".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-act-cal",
                            "在手机新建日程",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "calendar_event".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-act-todo",
                            "创建手机待办提醒",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "todo_create".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-act-note",
                            "写入速记并唤起 Amoo",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "note_create".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-act-voice",
                            "唤醒手机语音助手",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "voice_tts".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "mcp-act-gallery",
                            "打开手机系统相册",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_gallery".into(),
                            )),
                            Btn::Ghost,
                        ),
                    ]),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading(format!(
                            "StepOS {} 项系统级 MCP 服务矩阵",
                            suite.mcp_services.len()
                        )),
                        self.line(
                            format!("当前显示 {} 项服务", filtered_services.len()),
                            CAPTION,
                            p.text3,
                        ),
                    ),
                    self.segmented(
                        "mcp-cat-tabs",
                        categories
                            .into_iter()
                            .enumerate()
                            .map(|(idx, label)| {
                                let count = suite
                                    .mcp_services
                                    .iter()
                                    .filter(|s| idx == 0 || s.category == label)
                                    .count();
                                (
                                    format!("{label} · {count}"),
                                    cat_filter == idx as u8,
                                    Action::BackScreen(BackScreenAction::McpCategoryFilter(
                                        idx as u8,
                                    )),
                                )
                            })
                            .collect(),
                    ),
                    self.vstack(S3, service_rows),
                ]),
            ],
        )
    }

    fn ai_safety_tool_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let evidence = &suite.capability_evidence;
        let perm_label = |value: Option<bool>| match value {
            Some(true) => "已授权",
            Some(false) => "未授权 / 已关闭",
            None => "尚未读取，状态待确认",
        };
        let perm_color = |value: Option<bool>| match value {
            Some(true) => p.success,
            Some(false) => p.warning,
            None => p.text2,
        };
        let switch_label = |value: Option<bool>| match value {
            Some(true) => "已开启",
            Some(false) => "已关闭",
            None => "待确认",
        };
        let privacy_action = |known: Option<bool>, key: &str| {
            Action::BackScreen(if known.is_some() {
                BackScreenAction::TriggerMcpQuickAction(key.into())
            } else {
                BackScreenAction::InspectStepOsSuite
            })
        };

        self.vstack(
            S4,
            vec![
                self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.heading("StepOS 端侧大模型引擎 (com.stepos.aiengine)"),
                                self.pill_badge(
                                    if suite.aiengine_running {
                                        "AI 引擎运行中"
                                    } else {
                                        "按需唤醒"
                                    },
                                    if suite.aiengine_running {
                                        p.success
                                    } else {
                                        p.text2
                                    },
                                    p.surface,
                                ),
                                if !suite.aiengine_htp_status.is_empty() {
                                    self.pill_badge(
                                        &suite.aiengine_htp_status,
                                        p.accent,
                                        p.accent_soft,
                                    )
                                } else {
                                    Empty::new().finish()
                                },
                            ],
                        ),
                        self.line(
                            format!("已注册 {} 项能力", suite.aiengine_ability_count),
                            CAPTION,
                            p.accent,
                        ),
                    ),
                    self.text(&suite.aiengine_model_summary, BODY, p.text),
                    self.text(
                        "能力目录不代表模型已安装或可离线调用。语音处理位置与可用性以设置中的实际检测为准。",
                        CAPTION,
                        p.text2,
                    ),
                    if !suite.thermal_summary.is_empty() {
                        self.line(&suite.thermal_summary, CAPTION, p.text3)
                    } else {
                        Empty::new().finish()
                    },
                    self.wrap(vec![
                        self.mcp_quick_btn(
                            "ai-open-model-mgr",
                            "在手机打开端侧模型管理",
                            "open_aiengine",
                            Btn::Primary,
                        ),
                        self.mcp_quick_btn(
                            "ai-open-engine-settings",
                            "端侧 AI 引擎设置",
                            "open_aiengine_settings",
                            Btn::Secondary,
                        ),
                        self.button(
                            "ai-refresh-telemetry",
                            "刷新 NPU 与算子状态",
                            Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                            Btn::Ghost,
                        ),
                    ]),
                ]),
                self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.heading(
                                    "手机系统隐私开关与删除保护设置",
                                ),
                                self.pill_badge(
                                    "设置后回读确认",
                                    p.accent,
                                    p.accent_soft,
                                ),
                            ],
                        ),
                        self.line(
                            format!(
                                "相机：{} · 麦克风：{} · 定位：{} · 删除保护设置：{}",
                                switch_label(evidence.camera_allowed),
                                switch_label(evidence.microphone_allowed),
                                switch_label(evidence.location_enabled),
                                switch_label(evidence.delete_protect_enabled)
                            ),
                            CAPTION,
                            p.accent,
                        ),
                    ),
                    self.text(
                        "读取并修改当前手机用户的系统开关。相机和麦克风还受应用权限影响；删除保护设置不代表所有应用的删除都会被拦截。未知状态先读取，不自动改变设置。",
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button(
                            "ai-priv-cam",
                            match evidence.camera_allowed {
                                Some(true) => "关闭系统相机访问",
                                Some(false) => "允许系统相机访问",
                                None => "读取相机开关状态",
                            },
                            privacy_action(evidence.camera_allowed, "toggle_global_camera"),
                            Btn::Secondary,
                        ),
                        self.button(
                            "ai-priv-mic",
                            match evidence.microphone_allowed {
                                Some(true) => "关闭系统麦克风访问",
                                Some(false) => "允许系统麦克风访问",
                                None => "读取麦克风开关状态",
                            },
                            privacy_action(evidence.microphone_allowed, "toggle_global_mic"),
                            Btn::Secondary,
                        ),
                        self.button(
                            "ai-priv-loc",
                            match evidence.location_enabled {
                                Some(true) => "关闭系统定位",
                                Some(false) => "开启系统定位",
                                None => "读取定位开关状态",
                            },
                            privacy_action(evidence.location_enabled, "toggle_global_location"),
                            Btn::Secondary,
                        ),
                        self.button(
                            "ai-priv-del-protect",
                            match evidence.delete_protect_enabled {
                                Some(true) => "关闭删除保护设置",
                                Some(false) => "开启删除保护设置",
                                None => "读取删除保护设置",
                            },
                            privacy_action(evidence.delete_protect_enabled, "toggle_delete_protect"),
                            Btn::Secondary,
                        ),
                    ]),
                    self.divider(),
                    self.between(
                        self.line(
                            "打开手机回收站（可恢复内容与保留期限以手机为准）",
                            CAPTION,
                            p.text2,
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.button(
                                    "ai-priv-gallery-trash",
                                    "打开手机相册回收站",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "open_gallery_trash".into(),
                                    )),
                                    Btn::Secondary,
                                ),
                                self.button(
                                    "ai-priv-file-trash",
                                    "打开文件管理回收站",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "open_file_trash".into(),
                                    )),
                                    Btn::Secondary,
                                ),
                            ],
                        ),
                    ),
                ]),
                self.panel(vec![
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.heading("AI 核心感知与操控权限四维矩阵"),
                                self.text(
                                    "每项状态来自对应授权或开关的读取；尚未读取时不推断已授权。可打开手机设置逐项核对。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.utility_button(
                            "ai-heal-all-perms",
                            "配置伴侣所需权限",
                            Action::GrantDeepAmooPerms,
                            true,
                        ),
                    ),
                    self.divider(),
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.line_w(
                                    "1. 屏幕视觉感知 (ScreenMCPService · 识屏理解授权)",
                                    BODY,
                                    p.text,
                                    Weight::Medium,
                                ),
                                self.text(
                                    "允许端侧模型与 Amoo 在任务执行期间读取当前屏幕画面与布局树，用于看屏问答与 GUI 自动规划。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.line(
                                    perm_label(evidence.screen_perception),
                                    CAPTION,
                                    perm_color(evidence.screen_perception),
                                ),
                                self.button(
                                    "ai-toggle-perception",
                                    match evidence.screen_perception {
                                        Some(true) => "关闭识屏设置",
                                        Some(false) => "开启识屏设置",
                                        None => "读取识屏设置",
                                    },
                                    privacy_action(evidence.screen_perception, "toggle_screen_perception"),
                                    Btn::Secondary,
                                ),
                                self.button(
                                    "ai-open-perception",
                                    "手机设置",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "open_screen_perception".into(),
                                    )),
                                    Btn::Ghost,
                                ),
                            ],
                        ),
                    ),
                    self.divider(),
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.line_w(
                                    "2. StepX 伴侣无障碍服务授权",
                                    BODY,
                                    p.text,
                                    Weight::Medium,
                                ),
                                self.text(
                                    "只核对 StepX 伴侣无障碍组件是否已启用，不代表手机所有自动化执行器都已就绪。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.line(
                                    perm_label(evidence.gui_accessibility),
                                    CAPTION,
                                    perm_color(evidence.gui_accessibility),
                                ),
                                self.button(
                                    "ai-open-operate-phone",
                                    "接管权限设置",
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "open_operate_phone".into(),
                                    )),
                                    Btn::Secondary,
                                ),
                            ],
                        ),
                    ),
                    self.divider(),
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.line_w(
                                    "3. 系统通知实时监听 (NotificationListener · 跨端推送与验证码提取)",
                                    BODY,
                                    p.text,
                                    Weight::Medium,
                                ),
                                self.text(
                                    "用于桌面端实时接收手机通知、快递取件码汇总、验证码秒级提取与背屏通知呼吸闪动。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.line(
                            perm_label(evidence.notification_listener),
                            CAPTION,
                            perm_color(evidence.notification_listener),
                        ),
                    ),
                    self.divider(),
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.line_w(
                                    "4. 原厂工具调用者权限 (ACCESS_MCP_SERVICES)",
                                    BODY,
                                    p.text,
                                    Weight::Medium,
                                ),
                                self.text(
                                    "核对原厂 ability 调用者的系统权限；不代表桌面进程已获授权，具体工具仍需独立调用验证。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.line(
                            perm_label(evidence.mcp_access),
                            CAPTION,
                            perm_color(evidence.mcp_access),
                        ),
                    ),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading("AI 安全审计台账 (com.stepos.agentsafetycenter)"),
                        self.pill_badge(
                            if suite.safety_center_running {
                                "安全台账实时记录中"
                            } else {
                                "按需唤醒"
                            },
                            if suite.safety_center_running {
                                p.success
                            } else {
                                p.text2
                            },
                            p.surface,
                        ),
                    ),
                    self.text(
                        "StepOS AI 安全审计中心会完整记录每一次智能体调用系统能力、读取屏幕或执行跨应用操作的审计轨迹 (LedgerProvider)，并支持按应用粒度随时收回智能体权限。",
                        BODY,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button(
                            "ai-open-safety-ledger",
                            "在手机打开 AI 安全审计台账",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_safety_ledger".into(),
                            )),
                            Btn::Primary,
                        ),
                        self.button(
                            "ai-open-perm-mgr",
                            "打开智能体权限管理中心",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_agent_perm_manager".into(),
                            )),
                            Btn::Secondary,
                        ),
                    ]),
                ]),
            ],
        )
    }

    fn wellbeing_tool_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let wb = &suite.wellbeing;
        let max_mins = wb
            .top_apps
            .iter()
            .map(|(_, _, m)| *m)
            .max()
            .unwrap_or(1)
            .max(1);

        use stepx_companion::home_visuals::{ring_gauge, Paint};
        let paint =
            |c: facloud_ui::color::ColorU| Paint::rgb(c.r, c.g, c.b).with_alpha(c.a as f32 / 255.0);
        let ring_track = paint(p.text3)
            .with_alpha(0.18)
            .over(paint(p.surface).with_alpha(1.0));
        let screen_ratio = (wb.screen_time_minutes as f32 / 360.0).clamp(0.08, 1.0);
        let unlock_ratio = (wb.unlock_count as f32 / 80.0).clamp(0.08, 1.0);
        let apps_ratio = (wb.top_apps.len() as f32 / 8.0).clamp(0.08, 1.0);

        let metrics = [
            (
                "tool-wb-ring-screen",
                "今日手机亮屏时长",
                stepx_companion::toolbox::format_duration_minutes(wb.screen_time_minutes),
                screen_ratio,
                p.blue,
            ),
            (
                "tool-wb-ring-unlock",
                "今日解锁次数",
                format!("{} 次", wb.unlock_count),
                unlock_ratio,
                p.teal,
            ),
            (
                "tool-wb-ring-apps",
                "今日统计活跃应用",
                format!("{} 款应用", wb.top_apps.len()),
                apps_ratio,
                p.indigo,
            ),
        ]
        .into_iter()
        .map(|(slot, label, val, ratio, color)| {
            let ring = self.svg_canvas(
                slot,
                ring_gauge(38., 4.5, ratio, ring_track, paint(color)),
                Some(38.),
                38.,
                false,
            );
            Expanded::new(
                1.,
                self.card_box(
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.line(label, CAPTION, p.text2),
                                self.line_w(val, METRIC, p.text, Weight::Semibold),
                            ],
                        ),
                        ring,
                    ),
                    p.surface,
                    p.border,
                    R_CONTROL,
                    S3,
                    S3,
                ),
            )
            .finish()
        })
        .collect();

        let colors = [p.blue, p.teal, p.indigo, p.warning, p.success];
        let mut app_rows = vec![
            self.between(
                self.heading("今日高频应用使用时长排行"),
                self.line("基于 Android UsageStats 实时统计", CAPTION, p.text3),
            ),
            self.text(
                "直观展示今日前台使用时间最长的手机应用，支持一键在手机打开应用，或对后台耗电/分心应用执行强制停止。",
                CAPTION,
                p.text2,
            ),
            {
                let segs: Vec<(f32, facloud_ui::color::ColorU)> = wb
                    .top_apps
                    .iter()
                    .take(5)
                    .enumerate()
                    .map(|(i, (_, _, m))| ((*m).max(1) as f32, colors[i % colors.len()]))
                    .collect();
                self.tool_spectrum_bar("tool-wellbeing-spectrum", &segs)
            },
        ];

        if wb.top_apps.is_empty() {
            app_rows.push(self.text(
                "今日暂无高频应用统计或正在读取中，请点击右上角「刷新统计」获取最新数据。",
                BODY,
                p.text3,
            ));
        } else {
            let range = self.list_page_range("wellbeing-app-usage",wb.top_apps.len(),30);
            app_rows.push(self.list_page_controls("wellbeing-app-usage",wb.top_apps.len(),30));
            for (idx, (name, pkg, mins)) in wb.top_apps.iter().enumerate().skip(range.start).take(range.len()) {
                let pct = ((*mins as u64 * 100) / max_mins as u64).clamp(6, 100) as u8;
                let total_share = if wb.screen_time_minutes > 0 {
                    ((*mins as u64 * 100) / wb.screen_time_minutes as u64).min(100)
                } else {
                    0
                };
                let bar_color = colors[idx % colors.len()];
                app_rows.push(self.divider());
                app_rows.push(self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.hstack(
                                S2,
                                vec![
                                    self.line_w(
                                        format!("{}. {name}", idx + 1),
                                        BODY,
                                        p.text,
                                        Weight::Medium,
                                    ),
                                    self.pill_badge(
                                        format!(
                                            "{} · 占亮屏 {}%",
                                            stepx_companion::toolbox::format_duration_minutes(
                                                *mins
                                            ),
                                            total_share
                                        ),
                                        bar_color,
                                        p.accent_soft,
                                    ),
                                ],
                            ),
                            self.mono(pkg, MICRO, p.text3),
                        ],
                    ),
                    self.hstack(
                        S2,
                        vec![
                            self.button(
                                format!("wb-mirror-app-{idx}"),
                                "投屏打开",
                                Action::OpenAppInMirror(pkg.clone()),
                                Btn::Secondary,
                            ),
                            self.button(
                                format!("wb-open-app-{idx}"),
                                "在手机打开",
                                Action::OpenApp(pkg.clone()),
                                Btn::Secondary,
                            ),
                            self.button(
                                format!("wb-stop-app-{idx}"),
                                "强制停止",
                                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                    format!("force_stop:{pkg}"),
                                )),
                                Btn::Ghost,
                            ),
                        ],
                    ),
                ));
                app_rows.push(self.progress(Some(pct), bar_color));
            }
        }

        let pomodoro_active = self.back_screen.pomodoro_active;
        let pomodoro_mins = (self.back_screen.pomodoro_duration_secs / 60).max(1);

        self.vstack(
            S4,
            vec![
                self.panel(vec![
                    self.between(
                        self.heading("数字健康与屏幕使用统计"),
                        self.button(
                            "wb-refresh-btn",
                            "刷新使用统计",
                            Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                            Btn::Secondary,
                        ),
                    ),
                    self.hstack(S3, metrics),
                ]),
                self.panel(app_rows),
                self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.heading("跨端专注番茄钟（桌面 + 手机背屏联动）"),
                                self.pill_badge(
                                    if pomodoro_active {
                                        format!("专注倒计时进行中 · {pomodoro_mins} 分钟")
                                    } else {
                                        "背屏进度条联动".into()
                                    },
                                    if pomodoro_active { p.success } else { p.text2 },
                                    p.surface,
                                ),
                            ],
                        ),
                        if pomodoro_active {
                            self.button(
                                "wb-pomo-stop",
                                "结束专注番茄钟",
                                Action::BackScreen(BackScreenAction::StopPomodoro),
                                Btn::Danger,
                            )
                        } else {
                            self.line("开启后手机背屏同步显示倒计时点阵", CAPTION, p.text3)
                        },
                    ),
                    self.text(
                        "启动专注倒计时后，桌面端与手机背屏将同步展示实时专注进度条，配合手机勿扰模式帮助你远离分心通知。",
                        BODY,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button(
                            "wb-pomo-15",
                            "15 分钟轻度专注",
                            Action::BackScreen(BackScreenAction::StartPomodoro(15)),
                            Btn::Secondary,
                        ),
                        self.button(
                            "wb-pomo-25",
                            "25 分钟标准番茄钟",
                            Action::BackScreen(BackScreenAction::StartPomodoro(25)),
                            Btn::Primary,
                        ),
                        self.button(
                            "wb-pomo-45",
                            "45 分钟深度心流",
                            Action::BackScreen(BackScreenAction::StartPomodoro(45)),
                            Btn::Secondary,
                        ),
                        self.button(
                            "wb-pomo-dnd",
                            "同步切换手机勿扰模式",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "toggle_dnd".into(),
                            )),
                            Btn::Ghost,
                        ),
                    ]),
                ]),
                self.panel(vec![
                    self.between(
                        self.heading("AlphaMe 个人数字分身与场景画像 (com.stepos.alphame)"),
                        self.pill_badge(
                            if wb.alphame_running {
                                "AlphaMe 服务在线"
                            } else {
                                "按需唤醒"
                            },
                            if wb.alphame_running {
                                p.success
                            } else {
                                p.text2
                            },
                            p.surface,
                        ),
                    ),
                    self.text(
                        "AlphaMe 负责维护你的端侧个人画像、家庭与公司常用地址、场景 Wi-Fi 自动感知、车载蓝牙触发策略以及 Amoo 长期个人记忆，让手机与桌面智能体无需重复询问你的日常偏好。",
                        BODY,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button(
                            "wb-open-alphame",
                            "打开 AlphaMe 个人画像中心",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_alphame".into(),
                            )),
                            Btn::Primary,
                        ),
                        self.button(
                            "wb-open-alphame-addr",
                            "常用地址管理（家 / 公司）",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_alphame_address".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "wb-open-alphame-wifi",
                            "场景 Wi-Fi 绑定",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_alphame_wifi".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "wb-open-alphame-bt",
                            "车载蓝牙绑定",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_alphame_bt".into(),
                            )),
                            Btn::Secondary,
                        ),
                        self.button(
                            "wb-open-amoo-mem",
                            "打开 Amoo 记忆中心",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_amoo_memory".into(),
                            )),
                            Btn::Ghost,
                        ),
                    ]),
                ]),
            ],
        )
    }

    fn companion_tool_page(&self) -> El {
        let p = self.p();
        let companion = &self.companion_status;
        let mut sections = vec![self.panel(vec![
            self.between(
                self.vstack(
                    S1,
                    vec![
                        self.heading("StepX伴侣"),
                        self.text(
                            if companion.installed {
                                format!("手机版本 {}", companion.version)
                            } else if companion.checked {
                                "尚未安装".into()
                            } else {
                                "等待检测手机状态".into()
                            },
                            CAPTION,
                            p.text2,
                        ),
                    ],
                ),
                self.utility_button(
                    "companion-install",
                    if companion.installed {
                        "更新／修复伴侣"
                    } else {
                        "安装 StepX伴侣"
                    },
                    Action::CompanionInstall,
                    true,
                ),
            ),
            self.text(
                "用于扫码配对、文件互传、背屏管理与后台连接。",
                BODY,
                p.text2,
            ),
            self.text(
                if companion.checked {
                    &companion.message
                } else {
                    "支持 USB 或无线检测；安装与修复需通过已授权的 USB 连接。"
                },
                CAPTION,
                p.text2,
            ),
            self.wrap(vec![
                self.button(
                    "companion-guide",
                    "首次连接引导",
                    Action::ConnectGuide(Some(if self.selected.is_some() { 2 } else { 0 })),
                    Btn::Secondary,
                ),
                if companion.installed {
                    self.utility_button(
                        "companion-open",
                        "在手机打开伴侣",
                        Action::OpenApp(stepx_companion::companion_app::PACKAGE.into()),
                        false,
                    )
                } else {
                    self.disabled_button("在手机打开伴侣")
                },
            ]),
        ])];
        if let Some((message, error)) = &self.phone_link_notice {
            sections.push(self.panel(vec![
                self.heading("手机文件连接"),
                self.text(message, CAPTION, if *error { p.warning } else { p.success }),
            ]));
        }
        if !companion.installed {
            return self.vstack(S4, sections);
        }
        sections.push(self.panel(vec![
            self.between(
                self.heading("伴侣系统服务 · Amoo 后台连接"),
                self.pill_badge(
                    if companion.background_supported == Some(false) {
                        "熄屏待支持"
                    } else {
                        match companion.background_ready {
                            Some(true) => "通道已确认",
                            Some(false) => "待连接确认",
                            None => "待检测",
                        }
                    },
                    if companion.background_ready == Some(true) {
                        p.success_text
                    } else {
                        p.text2
                    },
                    p.surface,
                ),
            ),
            self.text(&companion.background_message, BODY, p.text2),
            self.text(
                "发送任务时按需保持后台连接，停止后自动释放。",
                CAPTION,
                p.text3,
            ),
            self.wrap(vec![
                if companion.background_supported == Some(true) {
                    self.utility_button(
                        "companion-keep-start",
                        "开启后台连接",
                        Action::CompanionBackground(true),
                        companion.background_enabled != Some(true),
                    )
                } else {
                    self.disabled_button("熄屏服务待支持")
                },
                self.utility_button(
                    "companion-keep-stop",
                    "停止后台连接",
                    Action::CompanionBackground(false),
                    false,
                ),
            ]),
        ]));
        sections.push(self.panel(vec![
            self.between(
                self.heading("文件互传"),
                self.hstack(
                    2.,
                    vec![
                        self.status_icon(
                            "transfer-usb-support",
                            "支持 USB 有线传输",
                            UiIcon::Usb,
                            p.text3,
                        ),
                        self.status_icon(
                            "transfer-wifi-support",
                            "支持已配对的无线传输",
                            UiIcon::Wifi,
                            p.text3,
                        ),
                    ],
                ),
            ),
            self.text(
                "单次可发 1–10 个文件（单文件 ≤ 256 MB），手机端可在伴侣「文件」页回传。",
                BODY,
                p.text2,
            ),
            self.wrap(vec![
                self.utility_button(
                    "companion-send",
                    "发送文件到手机",
                    Action::CompanionSendFiles,
                    false,
                ),
                self.button(
                    "companion-receive",
                    "打开电脑接收文件夹",
                    Action::CompanionReceiveFolder,
                    Btn::Secondary,
                ),
            ]),
        ]));
        sections.push(self.panel(vec![
            self.between(
                self.heading("扫码连接电脑"),
                self.line(
                    if companion.pairing_ready {
                        "系统配对服务已核验"
                    } else if companion.pairing_permission_granted {
                        "权限已授权 · 服务未核验"
                    } else {
                        "权限尚未就绪"
                    },
                    CAPTION,
                    if companion.pairing_ready {
                        p.success
                    } else {
                        p.warning
                    },
                ),
            ),
            self.text(&companion.pairing_message, BODY, p.text2),
            self.text(
                "手机重启解锁后可在同一 Wi-Fi 下自动重连；首次配对或更换网络需扫码确认。",
                CAPTION,
                p.text3,
            ),
            self.button(
                "companion-pair",
                if companion.pairing_ready {
                    "打开配对二维码"
                } else {
                    "查看系统无线配对方式"
                },
                Action::ConnectGuide(Some(if companion.pairing_ready { 4 } else { 3 })),
                Btn::Secondary,
            ),
        ]));
        let power_state = |value: Option<bool>| match value {
            Some(true) => "已豁免",
            Some(false) => "未豁免",
            None => "检测失败／未知",
        };
        let mut power = vec![
            self.heading("手机界面 APK · 电源权限"),
            self.between(
                self.line("Android 电池优化（Doze）", BODY, p.text),
                self.line(
                    power_state(companion.doze_exempt),
                    CAPTION,
                    if companion.doze_exempt == Some(true) {
                        p.success
                    } else {
                        p.warning
                    },
                ),
            ),
            self.between(
                self.line("厂商后台冻结策略", BODY, p.text),
                self.line(
                    power_state(companion.oem_freezer_exempt),
                    CAPTION,
                    if companion.oem_freezer_exempt == Some(true) {
                        p.success
                    } else {
                        p.warning
                    },
                ),
            ),
            self.text(
                "仅豁免 StepX伴侣自身后台限制，不影响系统全局省电策略。",
                CAPTION,
                p.text2,
            ),
        ];
        if !companion.power_message.is_empty() {
            power.push(self.text(&companion.power_message, CAPTION, p.warning));
        }
        sections.push(self.panel(power));
        let mut permissions = vec![
            self.heading("权限与用途"),
            self.text(
                "按需授予伴侣权限；文件收发使用系统选择器，无需全盘存储权限。",
                CAPTION,
                p.text2,
            ),
        ];
        for permission in &companion.permissions {
            permissions.push(self.divider());
            permissions.push(self.between(
                self.vstack(
                    S1,
                    vec![
                        self.line(permission.label, BODY, p.text),
                        self.text(permission.purpose, CAPTION, p.text2),
                    ],
                ),
                self.line(
                    if permission.granted {
                        "已授权"
                    } else {
                        "未授权"
                    },
                    CAPTION,
                    if permission.granted {
                        p.success
                    } else {
                        p.warning
                    },
                ),
            ));
        }
        sections.push(self.panel(permissions));
        self.vstack(S4, sections)
    }

    fn google_tool_page(&self) -> El {
        let p = self.p();
        let g = &self.google_suite;
        let automatic = self.settings.value.google_auto_repair;
        let maintenance = self.google_recovery.snapshot.as_ref();
        let maintaining = maintenance.is_some_and(|state| state.in_flight);
        let mutation_blocked = self.busy || self.google_mutation_wait_reason().is_some();
        let reading_details = self.google_recovery.is_inspecting() || self.google_recovery.inspect_requested;

        let pill_badge =
            |label: &str, fg: facloud_ui::color::ColorU, bg: facloud_ui::color::ColorU| -> El {
                Container::new(self.line_w(label, MICRO, fg, Weight::Medium))
                    .with_horizontal_padding(8.)
                    .with_vertical_padding(3.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_TAG)))
                    .finish()
            };

        let perms_ok = g.boot_module_ready
            && g.granted_count >= g.total_permissions
            && g.total_permissions > 0;
        let patch_or_login_ok =
            g.compatibility_active || (!g.compatibility_supported && !g.health_needs_repair);
        let all_ready =
            g.checked && g.installed && perms_ok && !g.health_needs_repair && patch_or_login_ok;
        let needs_repair = g.installed
            && (!perms_ok
                || g.health_needs_repair
                || (g.compatibility_supported && !g.compatibility_active));

        let (overall_label, overall_fg, overall_bg) = if maintaining {
            ("自动维护中", p.blue, crate::ui::pages::home::soft_tint(p.blue, p.dark))
        } else if reading_details {
            ("正在读取状态", p.blue, crate::ui::pages::home::soft_tint(p.blue, p.dark))
        } else if !g.checked {
            ("待检测", p.text2, p.sidebar)
        } else if all_ready {
            ("服务检查通过", p.success, p.success_soft)
        } else if g.installed {
            ("已安装 · 建议修复", p.warning, p.warning_soft)
        } else {
            ("尚未安装", p.warning, p.warning_soft)
        };

        let mut top_actions = Vec::new();
        if mutation_blocked {
            top_actions.push(self.disabled_button(if maintaining { "自动维护中…" } else { "暂不可修复" }));
        } else if !g.installed {
            top_actions.push(self.button(
                "tool-google-install",
                "一键安装",
                Action::PrepareUtility(Box::new(Action::GoogleSuiteInstall)),
                Btn::Primary,
            ));
            top_actions.push(self.button(
                "google-step1-recheck-btn",
                "检测状态",
                Action::GoogleSuiteInspect,
                Btn::Secondary,
            ));
        } else {
            if needs_repair && !maintaining {
                top_actions.push(self.button(
                    "tool-google-compat-apply",
                    "一键修复",
                    Action::GoogleSuiteMaintain,
                    Btn::Primary,
                ));
            }
            top_actions.push(self.button(
                "tool-google-mirror",
                "投屏打开 Play",
                Action::OpenAppInMirror("com.android.vending".into()),
                if needs_repair {
                    Btn::Secondary
                } else {
                    Btn::Primary
                },
            ));
            top_actions.push(self.button(
                "tool-google-open",
                "手机打开",
                Action::OpenApp("com.android.vending".into()),
                Btn::Secondary,
            ));
            if !needs_repair {
                top_actions.push(self.button(
                    "tool-google-compat-apply",
                    "一键修复",
                    Action::GoogleSuiteMaintain,
                    Btn::Secondary,
                ));
            }
        }

        let header_row = self.between(
            self.vstack(
                4.,
                vec![
                    self.hstack(
                        S2,
                        vec![
                            self.line_w("Google Play", HEADING, p.text, Weight::Semibold),
                            pill_badge(overall_label, overall_fg, overall_bg),
                        ],
                    ),
                    self.line(
                        if !g.checked {
                            "连接手机后自动读取安装与权限状态"
                        } else if all_ready {
                            "三件套、系统权限与登录入口检查通过；具体应用仍需网络和认证条件支持"
                        } else if g.installed {
                            if automatic { "连接电脑后自动检查并恢复已配置的服务，进度显示在下方" }
                            else { "电脑自动维护已暂停，可手动检查或修复服务" }
                        } else {
                            "一键安装 Google 框架、Play 服务、Play 商店及 StepX 手机伴侣"
                        },
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            self.hstack(S2, top_actions),
        );

        let stat_card =
            |title: &str, val: String, val_color: facloud_ui::color::ColorU, sub: String| -> El {
                Expanded::new(
                    1.,
                    Container::new(self.vstack(
                        4.,
                        vec![
                            self.line(title, CAPTION, p.text2),
                            self.line_w(val, SUBHEAD, val_color, Weight::Semibold),
                            self.line(sub, MICRO, p.text3),
                        ],
                    ))
                    .with_uniform_padding(S3)
                    .with_background_color(p.sidebar)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_LG)))
                    .finish(),
                )
                .finish()
            };

        let suite_val = if !g.checked {
            ("待检测".into(), p.text2)
        } else if g.installed {
            ("已安装".into(), p.success)
        } else {
            ("未安装".into(), p.warning)
        };
        let suite_sub = if g.installed {
            format!("Play {} · GMS {}", g.play_short(), g.gms_short())
        } else {
            "GSF / GMS / Play 商店".into()
        };

        let perm_val = if !g.checked {
            ("待检测".into(), p.text2)
        } else if perms_ok {
            (format!("{}/{} 正常", g.granted_count, g.total_permissions), p.success)
        } else if g.installed {
            ("需恢复权限".into(), p.warning)
        } else {
            ("未就绪".into(), p.text3)
        };

        let patch_val = if !g.checked {
            ("待检测".into(), p.text2)
        } else if patch_or_login_ok {
            ("已就绪".into(), p.success)
        } else if g.installed {
            ("待修复".into(), p.warning)
        } else {
            ("未启用".into(), p.text3)
        };
        let patch_sub = if g.compatibility_active {
            "兼容补丁与登录入口已生效"
        } else if patch_or_login_ok {
            "登录入口与自动恢复已生效"
        } else {
            "解除未认证登录拦截"
        };

        let account_val = if !g.checked {
            ("待检测".into(), p.text2)
        } else if g.google_accounts > 0 {
            (format!("已登录 {} 个", g.google_accounts), p.success)
        } else {
            ("未登录".into(), p.text)
        };

        let metrics_grid = self.hstack(
            S3,
            vec![
                stat_card("基础三件套", suite_val.0, suite_val.1, suite_sub),
                stat_card(
                    "系统权限",
                    perm_val.0,
                    perm_val.1,
                    "开机与更新自动恢复".into(),
                ),
                stat_card("商店与登录", patch_val.0, patch_val.1, patch_sub.into()),
                stat_card(
                    "Google 账号",
                    account_val.0,
                    account_val.1,
                    "需开启国际网络登录".into(),
                ),
            ],
        );

        let automatic_message = if !automatic {
            if maintaining { "当前步骤完成后暂停电脑维护；手机自身的更新恢复继续运行" }
            else { "已暂停电脑自动维护；手机自身的更新恢复继续运行" }.to_string()
        } else if let Some(state) = maintenance {
            if state.message.is_empty() { "连接后自动检查 Google 服务".into() }
            else { state.message.clone() }
        } else {
            "连接后自动检查 Google 服务".into()
        };
        let automatic_color = if !automatic { p.text2 }
            else if maintenance.is_some_and(|state| matches!(state.phase,
                stepx_companion::google::recovery::RecoveryPhase::NeedsAttention |
                stepx_companion::google::recovery::RecoveryPhase::Blocked)) { p.warning }
            else if maintenance.is_some_and(|state| state.phase == stepx_companion::google::recovery::RecoveryPhase::Healthy) { p.success }
            else { p.blue };
        let mut maintenance_content = vec![
            self.between(
                self.section_heading(UiIcon::Refresh, "服务自动维护"),
                self.hstack(S2, vec![
                    if maintaining { self.disabled_button("检查中…") }
                    else { self.button("google-auto-check", "立即检查", Action::GoogleAutoRepairNow, Btn::Secondary) },
                    self.button("google-auto-toggle", if automatic { "暂停电脑维护" } else { "开启自动维护" },
                        Action::GoogleAutoRepairToggle, Btn::Ghost),
                ]),
            ),
            self.line_w(automatic_message, BODY, automatic_color, Weight::Medium),
            self.line("Google 组件更新后，手机先恢复权限与登录服务；电脑检查结果并在需要时补救。保留已有账号和应用数据。", CAPTION, p.text2),
        ];
        if !self.google_recovery.inspection_error.is_empty() {
            maintenance_content.push(self.line(&self.google_recovery.inspection_error, CAPTION, p.warning));
        }
        let maintenance_panel = self.panel(maintenance_content);
        let overview_panel = self.panel(vec![header_row, metrics_grid]);

        let quick_fix_row = |title: &str, desc: String, right_btn: El| -> El {
            self.between(
                self.vstack(
                    2.,
                    vec![
                        self.line_w(title, BODY, p.text, Weight::Medium),
                        self.line(desc, CAPTION, p.text2),
                    ],
                ),
                right_btn,
            )
        };

        let mut compat_btns = vec![if mutation_blocked { self.disabled_button("稍后修复") } else { self.button(
            "google-step3-patch-btn",
            "一键修复",
            Action::GoogleSuiteMaintain,
            Btn::Secondary,
        ) }];
        if g.compatibility_can_rollback && !mutation_blocked {
            compat_btns.push(self.button(
                "tool-google-compat-rollback",
                "撤销补丁",
                Action::PrepareUtility(Box::new(Action::GoogleSuiteRollback)),
                Btn::Ghost,
            ));
        }

        let fixes_panel = self.panel(vec![
            self.heading("常见问题处理"),
            quick_fix_row(
                "卡在「正在核对信息」或商店闪退",
                "先检查手机网络。组件与权限问题会自动尝试恢复；仍未恢复时可手动重试".into(),
                if mutation_blocked { self.disabled_button("稍后修复") } else { self.button(
                    "google-step2-repair-btn",
                    "修复权限与服务",
                    Action::GoogleSuiteMaintain,
                    Btn::Secondary,
                ) },
            ),
            self.divider(),
            quick_fix_row(
                "提示「设备未认证」无法登录或下载",
                "可检查已支持的商店兼容配置；权限修复不代表 Play Integrity 或厂商认证通过".into(),
                self.hstack(S2, compat_btns),
            ),
            self.divider(),
            quick_fix_row(
                "StepX 手机伴侣",
                if self.companion_status.installed {
                    format!(
                        "已安装 {} · 用于扫码连接、文件互传与后台同步",
                        self.companion_status.version
                    )
                } else {
                    "尚未安装 · 安装 Google Play 时会一并安装".into()
                },
                self.button(
                    "google-companion",
                    "管理伴侣",
                    Action::OpenTool(Some(ToolPage::Companion)),
                    Btn::Secondary,
                ),
            ),
        ]);

        self.vstack(S4, vec![overview_panel, maintenance_panel, fixes_panel])
    }

    pub(in crate::ui) fn weather_tool_page(&self) -> El {
        let p = self.p();
        let w = &self.weather_snapshot;

        let top_bar = self.between(
            self.pill_badge_icon(
                if self.demo_mode {
                    format!("{} · 演示", w.city)
                } else if w.from_live_api {
                    format!("{} · {}", w.city, w.source_label())
                } else {
                    "天气待更新".into()
                },
                UiIcon::Sun,
                p.blue,
                crate::ui::pages::home::soft_tint(p.blue, p.dark),
            ),
            self.hstack(
                S2,
                vec![
                    self.button_icon(
                        "weather-tool-refresh",
                        "同步手机定位天气",
                        UiIcon::Refresh,
                        Action::WeatherSelectCity("手机定位".into()),
                        Btn::Primary,
                    ),
                    self.button(
                        "weather-tool-back-home",
                        "返回首页模块",
                        Action::Page(Page::Home),
                        Btn::Secondary,
                    ),
                ],
            ),
        );

        if !w.from_live_api && !self.demo_mode {
            return self.panel(vec![top_bar, self.caption("还未收到天气数据，可刷新天气或读取手机定位。")]);
        }

        let condition_icon = if w.condition.contains('雨') || w.umbrella_needed {
            UiIcon::CloudRain
        } else {
            UiIcon::Sun
        };
        let main_weather_row = self.between(
            self.hstack(
                S4,
                vec![
                    Container::new(self.content_icon(
                        condition_icon,
                        28.,
                        if w.umbrella_needed {
                            p.warning
                        } else {
                            p.accent
                        },
                    ))
                    .with_uniform_padding(12.)
                    .with_background_color(if w.umbrella_needed {
                        p.warning_soft
                    } else {
                        p.accent_soft
                    })
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_LG)))
                    .finish(),
                    self.vstack(
                        S1,
                        vec![
                            self.hstack(
                                S2,
                                vec![
                                    self.line_w(
                                        format!("{}°C", w.temp_c),
                                        32.,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                    self.pill_badge(
                                        &w.condition,
                                        if w.umbrella_needed {
                                            p.warning
                                        } else {
                                            p.accent
                                        },
                                        if w.umbrella_needed {
                                            p.warning_soft
                                        } else {
                                            p.accent_soft
                                        },
                                    ),
                                    self.pill_badge(&w.range_label(), p.text2, p.surface),
                                ],
                            ),
                            self.line(
                                format!("{} · {}", w.region_label, w.updated_label),
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                ],
            ),
            self.pill_badge(&w.backscreen_badge, p.text2, p.sidebar),
        );

        let metric_card = |label: &str, value: String, tone_warn: bool| -> El {
            Expanded::new(
                1.,
                Container::new(self.vstack(
                    4.,
                    vec![
                        self.line(label, CAPTION, p.text2),
                        self.line_w(
                            value,
                            BODY,
                            if tone_warn { p.warning } else { p.text },
                            Weight::Semibold,
                        ),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.sidebar)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_LG)))
                .finish(),
            )
            .finish()
        };

        let metrics_row = self.hstack(
            S3,
            vec![
                metric_card("体感温度", format!("{}°C", w.feels_like_c), false),
                metric_card(
                    "湿度与降水",
                    format!(
                        "湿度 {}% · {}",
                        w.humidity_pct,
                        if w.umbrella_needed {
                            "建议带伞"
                        } else {
                            "无需带伞"
                        }
                    ),
                    w.umbrella_needed,
                ),
                metric_card(
                    "风力与空气",
                    format!("{} · 空气 {}", w.wind_label, w.aqi_label),
                    w.aqi > 100,
                ),
                metric_card("今日出行建议", w.commute_advice.clone(), w.umbrella_needed),
            ],
        );

        let tmr_rainy = w.tomorrow_umbrella_needed();
        let tomorrow_advice_card = Container::new(self.vstack(
            6.,
            vec![
                self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.pill_badge_icon("明日建议", UiIcon::Sun, p.teal, p.teal_soft),
                            self.line_w(w.tomorrow_headline(), SUBHEAD, p.text, Weight::Semibold),
                            self.pill_badge(
                                if tmr_rainy {
                                    "明日有雨 · 记得带伞"
                                } else {
                                    "明日无雨 · 适宜出行"
                                },
                                if tmr_rainy { p.warning } else { p.blue },
                                if tmr_rainy {
                                    p.warning_soft
                                } else {
                                    crate::ui::pages::home::soft_tint(p.blue, p.dark)
                                },
                            ),
                        ],
                    ),
                    self.line("Amoo 每日缓存 · 次日自动更新", CAPTION, p.text3),
                ),
                self.text(w.tomorrow_advice(), BODY, p.text),
            ],
        ))
        .with_uniform_padding(S3)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(if tmr_rainy { p.warning } else { p.teal }))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_LG)))
        .finish();

        let hero_panel = self.panel(vec![
            top_bar,
            self.divider(),
            main_weather_row,
            metrics_row,
            tomorrow_advice_card,
        ]);

        let mut hourly_cards = Vec::with_capacity(w.hourly.len());
        for pt in &w.hourly {
            let sub_meta = if pt.pop_pct > 0 {
                format!("降水 {}%", pt.pop_pct)
            } else {
                format!("{}km/h", pt.wind_kmh)
            };
            let wet = pt.pop_pct >= 40 || pt.condition.contains('雨');
            let card = Container::new(self.vstack(
                4.,
                vec![
                    self.line_w(&pt.hour_label, CAPTION, p.text2, Weight::Medium),
                    self.line_w(
                        format!("{}°C", pt.temp_c),
                        SUBHEAD,
                        p.text,
                        Weight::Semibold,
                    ),
                    self.line(&pt.condition, CAPTION, if wet { p.warning } else { p.text }),
                    self.line(sub_meta, MICRO, if wet { p.warning } else { p.text3 }),
                ],
            ))
            .with_uniform_padding(S2)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
            hourly_cards.push(ConstrainedBox::new(card).with_width(112.).finish());
        }

        let hourly_panel = self.panel(vec![
            self.heading("24 小时逐时预报"),
            self.wrap(hourly_cards),
        ]);

        let mut daily_rows = Vec::with_capacity(w.daily.len().saturating_mul(2));
        for (idx, day) in w.daily.iter().enumerate() {
            if idx > 0 {
                daily_rows.push(self.divider());
            }
            let day_rainy = day.pop_pct >= 40 || day.condition.contains('雨');
            daily_rows.push(self.between(
                self.hstack(
                    S2,
                    vec![
                        self.line_w(&day.date_label, BODY, p.text, Weight::Semibold),
                        self.mono(&day.ymd, CAPTION, p.text3),
                        self.pill_badge(
                            &day.condition,
                            if day_rainy { p.warning } else { p.text2 },
                            if day_rainy { p.warning_soft } else { p.surface },
                        ),
                        self.line_w(
                            format!("{}° ~ {}°C", day.low_c, day.high_c),
                            BODY,
                            p.text,
                            Weight::Medium,
                        ),
                        self.line(
                            format!("降水 {}%", day.pop_pct),
                            CAPTION,
                            if day_rainy { p.warning } else { p.text2 },
                        ),
                    ],
                ),
                self.line(&day.advice, CAPTION, p.text2),
            ));
        }

        let daily_panel = self.panel(
            std::iter::once(self.heading("未来多日预报"))
                .chain(daily_rows)
                .collect(),
        );

        self.vstack(S4, vec![hero_panel, hourly_panel, daily_panel])
    }
}

#[cfg(test)]
mod tests {
    use super::Companion;

    #[test]
    fn mcp_service_action_key_and_label_cover_all_24_stepos_services() {
        let cases = [
            (
                "com.android.calendar/com.stepos.calendar.mcp.CalendarAbsMCPService",
                "calendar_event",
                "› 新建手机日程",
            ),
            (
                "com.android.alarmclock/com.stepos.alarmclock.mcp.AlarmMCPService",
                "alarm_15m",
                "› 设 15 分钟闹钟",
            ),
            (
                "com.stepos.calculator/.mcp.CalculatorMCPService",
                "open_calculator",
                "› 打开科学计算器",
            ),
            (
                "com.stepos.xmscore/.mcpserver.XMSMCPService",
                "open_xms_service_numbers",
                "› 智慧信息服务号",
            ),
            (
                "com.stepos.media.gallery/com.stepos.gallery.baselib.mcp.GalleryMCPService",
                "capcut_handoff",
                "› 剪映AI一键成片",
            ),
            (
                "com.stepos.filemanager/.mcp.FileManagerMcpService",
                "open_filemanager",
                "› 跨端文件寻址",
            ),
            (
                "com.android.soundrecorder/.atomicabilitymcp.RecordManagerMcpService",
                "open_soundrecorder",
                "› 打开录音机纪要",
            ),
            (
                "com.android.packageinstaller/com.stepos.packageinstaller.mcp.McpService",
                "open_app_manager",
                "› 应用部署校验",
            ),
            (
                "com.stepos.ailauncher/.agent.ability.VoiceAbilityService",
                "open_remote_agent_hub",
                "› 远程智能体协同",
            ),
            (
                "com.android.settings/.stepos.mcp.McpService",
                "toggle_dnd",
                "› 切换勿扰模式",
            ),
            (
                "com.stepos.systemuiex/.mcp.ScreenMCPService",
                "open_screen_perception",
                "› 端侧视觉看屏",
            ),
            (
                "com.android.systemui/.stepos.mcp.mcpservice.SystemUIMCPService",
                "open_amoo_voice",
                "› 唤醒语音助手",
            ),
            (
                "com.android.dialer/com.stepos.mcp.service.AbilityService",
                "open_dialer",
                "› 通话状态协同",
            ),
            (
                "com.android.mms/com.stepos.mcp.service.AbilityService",
                "open_mms",
                "› 验证码流转",
            ),
            (
                "com.stepos.net.search/.server.SearchManagerMcpService",
                "search_files",
                "› 知识检索协同",
            ),
            (
                "com.stepos.feedback/.mcp.FeedBackMCPService",
                "open_feedback",
                "› 提交诊断反馈",
            ),
            (
                "com.stepos.common.ability.proxy/com.stepos.common.ability.biz.CommonAbilityService",
                "open_backscreen",
                "› 背屏与基础协同",
            ),
            (
                "com.stepos.ability/com.stepos.taskrobot.ability.TaskRobotMCPService",
                "wake_amoo_assistant",
                "› 自动化任务编排",
            ),
            (
                "com.stepos.agentsafetycenter/.mcp.service.AbilityService",
                "open_safety_ledger",
                "› 打开安全审计台账",
            ),
            (
                "com.stepos.safebox/com.stepos.mcp.service.AbilityService",
                "open_safebox",
                "› 私密存储保护",
            ),
            (
                "com.stepos.safe/com.stepos.mcp.service.AbilityService",
                "open_security_scan",
                "› 系统防护策略",
            ),
            (
                "com.stepos.logreport/.mcp.LogReportAbilityService",
                "open_feedback",
                "› 故障健康诊断",
            ),
            (
                "com.android.permissioncontroller/com.stepos.permissioncontroller.mcp.McpService",
                "toggle_global_camera",
                "› 权限自愈守护",
            ),
            (
                "com.stepos.secureservice/com.stepos.mcp.service.AbilityService",
                "open_permission_manager",
                "› 可信执行环境",
            ),
        ];

        assert_eq!(cases.len(), 24);
        for (service_id, expected_key, expected_label) in cases {
            let actual_key = Companion::mcp_service_action_key(service_id);
            let actual_label = Companion::mcp_service_action_label(service_id);
            assert_eq!(
                actual_key, expected_key,
                "service {service_id} should map to action key {expected_key}, got {actual_key}"
            );
            assert_ne!(
                actual_key, "open_aiengine",
                "catalog service {service_id} should not fall through to generic open_aiengine"
            );
            assert_eq!(
                actual_label, expected_label,
                "service {service_id} should map to label {expected_label}, got {actual_label}"
            );
        }

        // Verify fallback for unknown service ID
        assert_eq!(
            Companion::mcp_service_action_key("com.stepos.unknown/.Mcp"),
            "open_aiengine"
        );
        assert_eq!(
            Companion::mcp_service_action_label("com.stepos.unknown/.Mcp"),
            "› 打开 AI 引擎"
        );
    }

    #[test]
    fn weather_tool_page_metadata_and_render_smoke() {
        use super::{tool_card_blurb, ToolPage};

        assert_eq!(ToolPage::ALL.len(), 15);
        assert!(ToolPage::ALL.contains(&ToolPage::Weather));
        assert_eq!(ToolPage::Weather.title(), "实时天气与出行");
        assert_eq!(ToolPage::Weather.category(), 3);
        assert_eq!(ToolPage::from_arg("weather"), Some(ToolPage::Weather));
        assert_eq!(ToolPage::from_arg("forecast"), Some(ToolPage::Weather));
        assert_eq!(ToolPage::from_arg("commute"), Some(ToolPage::Weather));
        assert_eq!(
            tool_card_blurb(ToolPage::Weather),
            "实时天气、逐时预报与出行带入"
        );

        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        assert!(app.weather_snapshot.city.contains("上海"));
        assert_eq!(app.weather_selected_day, 0);
        app.tool_page = Some(ToolPage::Weather);
        let _ = app.weather_tool_page();
        let _ = app.tools_page();
        assert_eq!(app.tools_actions().len(), 1, "weather context needs a real snapshot");
        app.weather_snapshot.from_live_api = true;
        assert_eq!(app.tools_actions().len(), 2);
    }
}

/// Short (≤16 字) card blurb for the tool grid; full descriptions live on ToolPage.
fn tool_card_blurb(tool: ToolPage) -> &'static str {
    match tool {
        ToolPage::Weather => "实时天气、逐时预报与出行带入",
        ToolPage::Updates => "OTA 更新与安装条件",
        ToolPage::Storage => "清理缓存与重复文件",
        ToolPage::Google => "Google 服务与 Play 商店",
        ToolPage::Companion => "伴侣安装、配对与权限",
        ToolPage::Neo => "更新内置 GUI Agent",
        ToolPage::Apk => "安装电脑上的 APK",
        ToolPage::Screen => "投屏与截图",
        ToolPage::Clipboard => "电脑与手机剪贴板同步",
        ToolPage::McpHub => "系统服务状态与控制",
        ToolPage::AiSafety => "端侧模型与权限管理",
        ToolPage::Wellbeing => "亮屏时长与使用统计",
        ToolPage::Services => "停用或恢复系统服务",
        ToolPage::InputMethod => "安装并切换输入法",
        ToolPage::Device => "设备、电量与存储信息",
    }
}

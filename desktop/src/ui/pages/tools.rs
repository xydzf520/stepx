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
                "tools-settings",
                "手机存储",
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
        Container::new(self.between(
            self.vstack(S1, details),
            self.hstack(
                S4,
                vec![
                    self.mono(format_bytes(file.bytes), CAPTION, p.text2),
                    action,
                ],
            ),
        ))
        .with_uniform_padding(S3)
        .finish()
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
        if self.tool_page.is_none() || self.selected.is_none() {
            return vec![];
        }
        if self.busy {
            return vec![self.disabled_button("正在处理…")];
        }
        match self.tool_page {
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
                "G",
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
                let ready = suite.mcp_services.iter().filter(|s| s.ready).count();
                (
                    "⚡",
                    format!("{ready} / {} 项服务在线", suite.mcp_services.len()),
                )
            }
            ToolPage::AiSafety => {
                let suite = &self.back_screen.stepos_suite;
                (
                    "🛡",
                    if suite.aiengine_running {
                        format!(
                            "端侧 NPU 在线 · {} 算子",
                            suite.aiengine_ability_count.max(33)
                        )
                    } else {
                        "端侧 NPU 按需唤醒".into()
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
            ToolPage::Device => ("◈", "读取手机状态".into()),
            ToolPage::Updates => ("↻", self.updates.title().to_owned()),
        };
        self.panel(vec![
            self.between(
                self.hstack(
                    S3,
                    vec![
                        self.icon_box(symbol, 32., p.surface, p.text2, None),
                        self.heading(tool.title()),
                    ],
                ),
                self.line(
                    if self.selected.is_none() && tool != ToolPage::Updates {
                        "需要连接手机".into()
                    } else {
                        state
                    },
                    MICRO,
                    p.text3,
                ),
            ),
            self.text(tool.description(), BODY, p.text2),
            self.between(
                self.line(
                    ["", "空间整理", "应用与服务", "设备协同"][tool.category() as usize],
                    CAPTION,
                    p.text3,
                ),
                self.button(
                    format!("tool-open-{tool:?}"),
                    "› 打开工具",
                    Action::OpenTool(Some(tool)),
                    Btn::Secondary,
                ),
            ),
        ])
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
        if self.selected.is_none() && self.tool_page != Some(ToolPage::Updates) {
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
                && (self.tool_page != Some(ToolPage::Screen) || *error)
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
                Action::NeoRestore => "将停用 GUI Agent 任务路由，撤销本模块添加的无障碍和可选输入法；保留伴侣其他服务、后台策略与数据。请先结束当前任务。",
                Action::GoogleSuiteInstall => "将安装或修复 StepX伴侣，并保留已有 Google 组件。完整安装且版本已适配时，会备份并应用 Google Play 兼容补丁；首次安装后需按提示重启解锁。",
                Action::GoogleSuiteMaintain => "将核对固件与 Google 签名，备份后修复商店配置、服务权限、通用登录入口和应用链接验证，并启用 Google 组件更新后的自动恢复。必要时短暂重启 Amoo 后台服务，请先结束任务。保留账号与应用数据；此操作不改变 Google 官方认证或第三方服务器的登录判定。请保持手机已解锁并连接电脑。",
                Action::GoogleSuiteRollback => "将核对备份并恢复补丁修改的设置，重新启动 Google 服务。保留 Google 账号和已安装应用；设备未认证的拦截页可能恢复。",
                Action::ToggleSystemService(_, true) => "将重新启用此服务，它可能恢复后台运行。",
                _ => "将停用此服务，相关功能会停止；可在此处重新启用。不会卸载应用或删除数据。",
            };
            let target = match action {
                Action::NeoRestore => "伴侣内置 GUI Agent · 当前连接的手机".into(),
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
                parts.push(self.vstack(
                    S2,
                    vec![
                        self.heading("工具集合"),
                        self.text(
                            "按需要选择功能模块，检测和操作结果会实时保留在对应工具页内。",
                            BODY,
                            p.text2,
                        ),
                    ],
                ));
                parts.push(self.segmented(
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
                ));
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
                self.heading("安装本地 APK"), self.text("选择电脑上的单个 APK 文件。已有应用会尝试更新，版本或签名不兼容时会显示失败原因。", BODY, p.text2),
                self.text("拆分安装包（APKS / XAPK）不支持在此安装。Google 服务请使用 Google Play 工具。", CAPTION, p.text3),
                self.hstack(S2, vec![self.utility_button("tool-apk", "选择 APK…", Action::InstallApk, true), self.button("tool-app-list", "已安装应用", Action::Page(Page::Apps), Btn::Secondary)]),
            ])),
            Some(ToolPage::Screen) => {
                parts.push(self.screen_and_lan_cast_tool_page());
            }
            Some(ToolPage::Services) => {
                parts.push(self.panel(vec![self.text("仅列出已适配的服务。停用会影响对应功能，请逐项选择；当前输入法不允许停用。", BODY, p.text2),
                    self.utility_button("tool-ime-settings", "手机输入法设置", Action::OpenInputSettings, false)]));
                if self.system_services.is_empty() { parts.push(self.panel(vec![self.text("尚无检测结果，请点击刷新状态。", BODY, p.text2)])); }
                for (i, service) in self.system_services.iter().enumerate() {
                    let can_toggle = service.safe_to_remove && !service.status_note.contains("当前默认") && service.status_note != "未安装";
                    parts.push(self.panel(vec![self.between(self.vstack(S1, vec![self.heading(&service.name), self.line(&service.status_note, CAPTION, p.text2)]),
                        if can_toggle { self.utility_button(&format!("tool-service-{i}"), if service.enabled { "停用" } else { "启用" }, Action::PrepareUtility(Box::new(Action::ToggleSystemService(service.package.clone(), !service.enabled))), false) } else { self.disabled_button("不可停用") }),
                        self.text(&service.package, CAPTION, p.text3),
                    ]));
                }
            }
            Some(ToolPage::Device) => {
                let report = self.selected.as_ref().map(|d| format!("型号：{}\nAndroid：{}\n连接：{}\n\n{}", d.model, d.android, d.transport, self.summary)).unwrap_or_default();
                parts.push(self.panel(vec![self.heading("设备状态"), self.text(if self.summary.is_empty() { "尚无检测结果，请点击重新检测。" } else { &report }, BODY, p.text2)]));
                if !self.summary.is_empty() { parts.push(self.button("tool-copy-report", "复制检测结果", Action::CopyText(report, "已复制设备检测结果".into()), Btn::Secondary)); }
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
            let mut shown = std::collections::HashSet::new();
            for (ci, channel) in report.channels.iter().enumerate() {
                for (ri, release) in channel.releases.iter().enumerate() {
                    if !shown.insert(release.version.clone()) {
                        continue;
                    }
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
                        "任务路由：{}   增强输入法：{}",
                        if neo.selected { "Neo" } else { "未启用" },
                        if neo.ime_enabled {
                            "已启用"
                        } else {
                            "未启用"
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
        let mut cur = String::new();
        for ch in text.chars() {
            if ch.is_ascii_digit() {
                cur.push(ch);
            } else {
                if (4..=8).contains(&cur.len()) {
                    return Some(cur);
                }
                cur.clear();
            }
        }
        if (4..=8).contains(&cur.len()) {
            Some(cur)
        } else {
            None
        }
    }

    fn clipboard_tool_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let auto_sync = self.prefs.auto_sync_clipboard;
        let phone_clip = suite.phone_clipboard.trim();
        let detected_url = Self::extract_url_from_text(phone_clip);
        let detected_otp = if detected_url.is_none() {
            Self::extract_otp_from_text(phone_clip)
        } else {
            None
        };

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

    fn mcp_service_action_key(id: &str) -> &'static str {
        if id.contains("CalendarAbs") || id.contains("CalendarEvent") {
            "calendar_event"
        } else if id.contains("AlarmMCP") {
            "alarm_15m"
        } else if id.contains("GalleryMCP") {
            "open_gallery"
        } else if id.contains("FileManager") || id.contains("packageinstaller") {
            "open_filemanager"
        } else if id.contains("VoiceAbility") {
            "voice_tts"
        } else if id.contains("settings") {
            "toggle_dnd"
        } else if id.contains("ScreenMCP") {
            "open_screen_perception"
        } else if id.contains("agentsafetycenter") || id.contains("permissioncontroller") {
            "open_safety_ledger"
        } else if id.contains("dialer") {
            "open_dialer"
        } else if id.contains("mms") {
            "open_mms"
        } else if id.contains("TaskRobot") || id.contains("agentbus") || id.contains("CommonAbility") {
            "wake_amoo_assistant"
        } else {
            "open_aiengine"
        }
    }

    fn mcp_hub_tool_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let ready_count = suite.mcp_services.iter().filter(|s| s.ready).count();
        let cat_filter = self.back_screen.mcp_category_filter;
        let categories = ["全部", "日程与时间", "相册与文件", "语音与系统", "安全与自动化"];

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
                let card = Container::new(self.vstack(
                    S2,
                    vec![
                        self.between(
                            self.hstack(
                                S2,
                                vec![
                                    self.line_w(&item.title, BODY, p.text, Weight::Medium),
                                    self.pill_badge(
                                        &item.category,
                                        p.accent,
                                        p.accent_soft,
                                    ),
                                ],
                            ),
                            self.line(
                                if item.ready { "已就绪" } else { "待唤醒" },
                                MICRO,
                                if item.ready { p.success } else { p.warning },
                            ),
                        ),
                        self.mono(&item.id, MICRO, p.text3),
                        self.between(
                            self.line("com.stepos.mcp.sdk.TOOLS_SERVICE", MICRO, p.text3),
                            self.button(
                                format!("mcp-srv-act-{}", item.id),
                                format!("› {}", item.quick_action_label),
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
                                    format!("{ready_count} / {} 项已就绪", suite.mcp_services.len()),
                                    p.success,
                                    p.surface,
                                ),
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
                        "StepOS 在框架层 (/apex/com.stepos.framework) 内置标准化 MCP 工具服务协议 (com.stepos.mcp.sdk.TOOLS_SERVICE)，由系统级 AgentBus 统一调度日历、闹钟、相册、系统设置、语音与安全中心，打破应用孤岛，让桌面端可一键直控手机底层服务。",
                        BODY,
                        p.text2,
                    ),
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
                        self.heading("StepOS 20 项系统级 MCP 服务矩阵"),
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
        let perm_label = |ok: bool| {
            if ok {
                "已授权 / 就绪"
            } else {
                "待唤醒 / 授权"
            }
        };
        let perm_color = |ok: bool| if ok { p.success } else { p.warning };

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
                                        "端侧 NPU 在线"
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
                            format!("已注册 {} 项端侧算子", suite.aiengine_ability_count.max(33)),
                            CAPTION,
                            p.accent,
                        ),
                    ),
                    self.text(&suite.aiengine_model_summary, BODY, p.text),
                    self.text(
                        "基于高通 QNN HTP (NPU) 与 MNN 异构加速引擎，提供零云端延迟、数据不出端侧的本地 AI 推理能力。涵盖屏幕视觉理解 (stepos-gui-agent / stepos-perception)、相册语义检索 (mnn-clip)、实时语音识别与合成 (stepos-asr / stepos-tts)、短信智能解析 (stepos-sms-parse) 以及 29 类隐私与安全风险分类器 (stepos-risk-classify)。",
                        CAPTION,
                        p.text2,
                    ),
                    if !suite.thermal_summary.is_empty() {
                        self.line(&suite.thermal_summary, CAPTION, p.text3)
                    } else {
                        Empty::new().finish()
                    },
                    self.wrap(vec![
                        self.button(
                            "ai-open-model-mgr",
                            "在手机打开端侧模型管理",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_aiengine".into(),
                            )),
                            Btn::Primary,
                        ),
                        self.button(
                            "ai-open-engine-settings",
                            "端侧 AI 引擎设置",
                            Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                "open_aiengine_settings".into(),
                            )),
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
                        self.vstack(
                            S1,
                            vec![
                                self.heading("AI 核心感知与操控权限四维矩阵"),
                                self.text(
                                    "当手机 GUI Agent 或跨端协同动作无法执行时，可在此逐项排查或一键自动修复全部底层授权。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.utility_button(
                            "ai-heal-all-perms",
                            "一键修复全部 AI 权限",
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
                                    perm_label(suite.screen_perception_ready),
                                    CAPTION,
                                    perm_color(suite.screen_perception_ready),
                                ),
                                self.button(
                                    "ai-toggle-perception",
                                    if suite.screen_perception_ready {
                                        "关闭识屏授权"
                                    } else {
                                        "开启识屏授权"
                                    },
                                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                                        "toggle_screen_perception".into(),
                                    )),
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
                                    "2. 无障碍 GUI 自动化操控 (Accessibility · 接管操作手机)",
                                    BODY,
                                    p.text,
                                    Weight::Medium,
                                ),
                                self.text(
                                    "允许内置 GUI Agent 与 StepOS TaskRobot 执行点击、滑动与文本输入；遇到支付或锁屏验证时自动暂停由用户接管。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.line(
                                    perm_label(suite.gui_accessibility_enabled),
                                    CAPTION,
                                    perm_color(suite.gui_accessibility_enabled),
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
                            perm_label(suite.notification_listener_enabled),
                            CAPTION,
                            perm_color(suite.notification_listener_enabled),
                        ),
                    ),
                    self.divider(),
                    self.between(
                        self.vstack(
                            S1,
                            vec![
                                self.line_w(
                                    "4. AgentBus 原生 MCP 桥接鉴权 (ACCESS_MCP_SERVICES)",
                                    BODY,
                                    p.text,
                                    Weight::Medium,
                                ),
                                self.text(
                                    "授权 AgentBus 跨应用调用日历、闹钟、相册、文件管理与系统控制等 20 项原生 MCP 服务。",
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.line(
                            perm_label(suite.mcp_permission_granted),
                            CAPTION,
                            perm_color(suite.mcp_permission_granted),
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

        let metrics = [
            (
                "今日手机亮屏时长",
                stepx_companion::toolbox::format_duration_minutes(wb.screen_time_minutes),
            ),
            ("今日解锁次数", format!("{} 次", wb.unlock_count)),
            ("今日统计活跃应用", format!("{} 款应用", wb.top_apps.len())),
        ]
        .into_iter()
        .map(|(label, val)| {
            Expanded::new(
                1.,
                Container::new(self.vstack(
                    S1,
                    vec![
                        self.line(label, CAPTION, p.text2),
                        self.line_w(val, METRIC, p.text, Weight::Semibold),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
            .finish()
        })
        .collect();

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
        ];

        if wb.top_apps.is_empty() {
            app_rows.push(self.text(
                "今日暂无高频应用统计或正在读取中，请点击右上角「刷新统计」获取最新数据。",
                BODY,
                p.text3,
            ));
        } else {
            for (idx, (name, pkg, mins)) in wb.top_apps.iter().enumerate() {
                let pct = ((*mins as u64 * 100) / max_mins as u64).clamp(6, 100) as u8;
                let total_share = if wb.screen_time_minutes > 0 {
                    ((*mins as u64 * 100) / wb.screen_time_minutes as u64).min(100)
                } else {
                    0
                };
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
                                        p.accent,
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
                app_rows.push(self.progress(Some(pct), p.accent));
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
                "管理手机伴侣、文件收发与背屏，并分别核验扫码和熄屏服务能力。",
                BODY,
                p.text2,
            ),
            self.text(
                if companion.checked {
                    &companion.message
                } else {
                    "USB 或已配对无线连接均可检测；安装、更新与修复伴侣需要已授权的 USB 连接。"
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
            self.between(self.heading("伴侣系统服务 · Amoo 后台连接"), self.pill_badge(if companion.background_supported == Some(false) { "熄屏待支持" } else { match companion.background_ready { Some(true) => "通道已确认", Some(false) => "待连接确认", None => "待检测" } }, if companion.background_ready == Some(true) { p.success_text } else { p.text2 }, p.surface)),
            self.text(&companion.background_message, BODY, p.text2),
            self.text("手机界面、任务租约和真实连接分别检测。发送任务后按需请求原生短时连接；停止会结束续期，已请求的原生窗口最多约 3 分钟后到期。", CAPTION, p.text3),
            self.wrap(vec![if companion.background_supported == Some(true) { self.utility_button("companion-keep-start", "开启后台连接", Action::CompanionBackground(true), companion.background_enabled != Some(true)) } else { self.disabled_button("熄屏服务待支持") }, self.utility_button("companion-keep-stop", "停止后台连接", Action::CompanionBackground(false), false)]),
        ]));
        sections.push(self.panel(vec![
            self.between(self.heading("文件互传"), self.line("USB / 已配对的无线连接", CAPTION, p.text3)),
            self.text("电脑每次可发送 1–10 个文件，单个文件最多 256 MB。手机在伴侣「文件」页选择文件发送到电脑；传输失败会显示具体原因。", BODY, p.text2),
            self.wrap(vec![self.utility_button("companion-send", "发送文件到手机", Action::CompanionSendFiles, false), self.button("companion-receive", "打开电脑接收文件夹", Action::CompanionReceiveFolder, Btn::Secondary)]),
        ]));
        sections.push(self.panel(vec![
            self.between(self.heading("扫码连接电脑"), self.line(if companion.pairing_ready { "系统配对服务已核验" } else if companion.pairing_permission_granted { "权限已授权 · 服务未核验" } else { "权限尚未就绪" }, CAPTION, if companion.pairing_ready { p.success } else { p.warning })),
            self.text(&companion.pairing_message, BODY, p.text2),
            self.text("已适配的 StepX 172 固件会同时安装开机恢复组件。手机重启后完成系统首次解锁，已配对电脑在同一局域网自动重连，无需重复 USB 修复。首次配对或更换 Wi-Fi 仍需手机确认；无线通道验证成功后才显示已连接。", CAPTION, p.text3),
            self.button("companion-pair", if companion.pairing_ready { "打开配对二维码" } else { "查看系统无线配对方式" }, Action::ConnectGuide(Some(if companion.pairing_ready { 4 } else { 3 })), Btn::Secondary),
        ]));
        let power_state = |value: Option<bool>| match value {
            Some(true) => "已豁免",
            Some(false) => "未豁免",
            None => "检测失败／未知",
        };
        let mut power = vec![
            self.heading("手机界面 APK · 电源权限"),
            self.between(self.line("Android 电池优化（Doze）", BODY, p.text), self.line(power_state(companion.doze_exempt), CAPTION, if companion.doze_exempt == Some(true) { p.success } else { p.warning })),
            self.between(self.line("厂商后台冻结策略", BODY, p.text), self.line(power_state(companion.oem_freezer_exempt), CAPTION, if companion.oem_freezer_exempt == Some(true) { p.success } else { p.warning })),
            self.text("安装时仅配置 StepX伴侣自身的豁免，保存原状态。电源豁免与系统服务连接能力分别核验；不会修改全局省电策略。", CAPTION, p.text2),
        ];
        if !companion.power_message.is_empty() {
            power.push(self.text(&companion.power_message, CAPTION, p.warning));
        }
        sections.push(self.panel(power));
        let mut permissions = vec![
            self.heading("权限与用途"),
            self.text(
                "安装时授予这版伴侣实际使用的权限，并逐项读取系统状态核验。文件收发使用系统文件选择器，无需全盘存储权限。",
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
        let mut rows = vec![
            self.heading("Google Play 安装与检测"),
            self.text(
                if !g.checked {
                    "请先检测手机上已有的 Google 服务。"
                } else {
                    &g.verification_note
                },
                BODY,
                p.text2,
            ),
        ];
        for (title, version) in [
            ("Google 服务框架", &g.gsf_version),
            ("Google Play 服务", &g.gms_version),
            ("Google Play 商店", &g.play_version),
        ] {
            rows.push(self.between(
                self.line(title, BODY, p.text),
                self.line(
                    if !g.checked {
                        "待检测"
                    } else if version.is_empty() {
                        "未检测到"
                    } else {
                        version
                    },
                    CAPTION,
                    p.text2,
                ),
            ));
        }
        rows.push(self.divider());
        rows.push(self.text("安装 Google Play 时会同时安装 StepX伴侣，用于扫码、文件互传、背屏与 Amoo 后台连接。仅适配已验证的 StepX 固件；保留已有 Google 包。", CAPTION, p.text2));
        if !g.health_checks.is_empty() {
            rows.push(self.heading("基础服务与通用登录"));
            for check in &g.health_checks {
                let (label, color) = match check.state.as_str() {
                    "pass" => ("已就绪", p.success),
                    "fail" => ("需要处理", p.warning),
                    _ => ("待确认", p.text3),
                };
                rows.push(self.between(
                    self.line(&check.title, BODY, p.text),
                    self.line(label, CAPTION, color),
                ));
                rows.push(self.text(&check.detail, CAPTION, p.text2));
            }
            rows.push(self.text("以上检测的是共用服务。各应用的登录结果和设备认证仍由对应服务验证；修复保留现有账号。", CAPTION, p.text3));
        }
        let mut buttons = vec![];
        if g.checked
            && (g.installed
                || (g.gsf_version.is_empty()
                    && g.gms_version.is_empty()
                    && g.play_version.is_empty()))
            && self.tool_confirmation.is_none()
        {
            buttons.push(self.utility_button(
                "tool-google-install",
                if g.installed {
                    "完善 Google Play 和伴侣"
                } else {
                    "安装 Google Play 和伴侣"
                },
                Action::PrepareUtility(Box::new(Action::GoogleSuiteInstall)),
                true,
            ));
        }
        rows.push(self.between(
            self.text(
                if self.companion_status.installed {
                    format!("StepX伴侣 · {}", self.companion_status.version)
                } else {
                    "StepX伴侣 · 尚未安装或检测".into()
                },
                BODY,
                p.text,
            ),
            self.button(
                "google-companion",
                "管理伴侣",
                Action::OpenTool(Some(ToolPage::Companion)),
                Btn::Secondary,
            ),
        ));
        if g.checked && !g.play_version.is_empty() {
            buttons.push(self.utility_button(
                "tool-google-open",
                "在手机打开 Play",
                Action::OpenApp("com.android.vending".into()),
                self.tool_confirmation.is_none(),
            ));
        }
        if !buttons.is_empty() {
            rows.push(self.hstack(S2, buttons));
        }
        let (l1_label, l1_color) = if !g.checked {
            ("待检测", p.text3)
        } else if g.installed
            && !g.gsf_version.is_empty()
            && !g.gms_version.is_empty()
            && !g.play_version.is_empty()
        {
            ("已就绪", p.success)
        } else {
            ("需安装 / 修复", p.warning)
        };
        let (l2_label, l2_color) = if !g.checked {
            ("待检测", p.text3)
        } else if g.total_permissions > 0
            && g.granted_count >= g.total_permissions
            && g.boot_module_ready
        {
            (
                if g.total_permissions == 153 {
                    "153/153 已守护"
                } else {
                    "权限已就绪"
                },
                p.success,
            )
        } else if g.installed {
            ("需恢复权限", p.warning)
        } else {
            ("未安装", p.text3)
        };
        let l3_ready = g.checked
            && g.installed
            && !g.health_needs_repair
            && g.health_checks.iter().all(|c| c.state == "pass");
        let (l3_label, l3_color) = if !g.checked {
            ("待检测", p.text3)
        } else if l3_ready {
            ("通用登录正常", p.success)
        } else if g.installed {
            ("建议修复服务", p.warning)
        } else {
            ("待安装", p.text3)
        };
        let (l4_label, l4_color) = if g.play_protect.checked && g.play_protect.certified {
            ("官方认证通过", p.success)
        } else {
            ("出厂硬件根边界", p.warning)
        };
        let matrix = vec![
            self.heading("Google 服务四层能力与认证边界矩阵"),
            self.text(
                "清晰区分已打通的系统级通用能力与受出厂硬件密钥限制的强校验场景：",
                BODY,
                p.text2,
            ),
            self.divider(),
            self.between(
                self.line_w(
                    "L1 基础框架与商店组件（GSF / GMS / Play 商店）",
                    BODY,
                    p.text,
                    Weight::Medium,
                ),
                self.line(l1_label, CAPTION, l1_color),
            ),
            self.text(
                "检测系统预装与 mtg16 适配组件状态，Google Play 商店应用浏览、下载与更新均可正常使用。",
                CAPTION,
                p.text2,
            ),
            self.divider(),
            self.between(
                self.line_w(
                    "L2 运行时权限与自愈守护（153 项权限 + 重启/更新自愈）",
                    BODY,
                    p.text,
                    Weight::Medium,
                ),
                self.line(l2_label, CAPTION, l2_color),
            ),
            self.text(
                "StepX伴侣与特权桥接在手机重启解锁、GMS 或 Play 商店自动更新后自动恢复全套 153 项权限，无需重新插拔 USB。",
                CAPTION,
                p.text2,
            ),
            self.divider(),
            self.between(
                self.line_w(
                    "L3 通用账号凭据与应用链接（Credential Manager + App Links）",
                    BODY,
                    p.text,
                    Weight::Medium,
                ),
                self.line(l3_label, CAPTION, l3_color),
            ),
            self.text(
                "已补齐 GoogleIdService 凭据回退与域名验证代理，Google Play、Gmail、Chrome、Gemini 原生对话均可正常登录使用。",
                CAPTION,
                p.text2,
            ),
            self.divider(),
            self.between(
                self.line_w(
                    "L4 硬件级密钥证明边界（Key Attestation / Play Integrity 强校验）",
                    BODY,
                    p.text,
                    Weight::Medium,
                ),
                self.line(l4_label, CAPTION, l4_color),
            ),
            self.text(
                "StepX 工程机出厂硬件证书链（Stepx Root CA）非 Google 官方认证根；针对强制校验硬件证明链的接口（如 X 原生 Google 一键登录返回 214 Validation Failed），系统无法在无内核模块环境下伪造硬件根证书，建议在 X 内使用「账号密码 + 通行密钥」或官方网页版登录。",
                CAPTION,
                p.text2,
            ),
        ];
        let mut compatibility = vec![
            self.between(
                self.heading("Google Play 兼容补丁"),
                self.line(
                    if g.compatibility_active {
                        "已应用"
                    } else if g.compatibility_checked && g.compatibility_supported {
                        "可应用"
                    } else {
                        "待适配 / 检测"
                    },
                    CAPTION,
                    if g.compatibility_active {
                        p.success
                    } else {
                        p.text3
                    },
                ),
            ),
            self.text(&g.compatibility_note, BODY, p.text2),
            self.text(
                "用于已适配设备的商店浏览和手动下载安装；应用前核对组件版本并备份，可在这里撤销。检测只读取补丁配置，不代表 Google 账号登录、Gmail 或 Gemini 已验证可用。",
                CAPTION,
                p.text2,
            ),
        ];
        if g.checked && g.installed && self.tool_confirmation.is_none() {
            let mut actions = vec![];
            if g.compatibility_checked
                && g.compatibility_supported
                && (!g.compatibility_active || !g.boot_module_ready || g.health_needs_repair)
            {
                actions.push(self.utility_button(
                    "tool-google-compat-apply",
                    "修复 Google 服务",
                    Action::PrepareUtility(Box::new(Action::GoogleSuiteMaintain)),
                    true,
                ));
            }
            if g.compatibility_can_rollback {
                actions.push(self.utility_button(
                    "tool-google-compat-rollback",
                    "撤销兼容补丁",
                    Action::PrepareUtility(Box::new(Action::GoogleSuiteRollback)),
                    false,
                ));
            }
            if !actions.is_empty() {
                compatibility.push(self.hstack(S2, actions));
            }
        }
        let mut certification = vec![
            self.heading("Google 官方认证"),
            self.text(&g.certification_note, BODY, p.text2),
            self.text(
                "兼容补丁与 Google 官方认证分别检测。商店下载可用不代表 Gmail、Gemini、Google 账号登录、支付或 Play Integrity 已通过。",
                CAPTION,
                p.text3,
            ),
        ];
        if g.play_protect.checked {
            certification.push(self.between(
                self.line("认证四层校验", BODY, p.text),
                self.line(
                    g.play_protect.state_label(),
                    CAPTION,
                    if g.play_protect.certified {
                        p.success
                    } else {
                        p.danger
                    },
                ),
            ));
            for layer in &g.play_protect.layers {
                let (label, color) = match layer.state.as_str() {
                    "pass" => ("通过", p.success),
                    "fail" => ("未通过", p.danger),
                    _ => ("未取证", p.warning),
                };
                certification.push(self.between(
                    self.line(&layer.title, CAPTION, p.text),
                    self.line(label, CAPTION, color),
                ));
                for entry in layer.checks.iter().filter(|entry| entry.ok == Some(false)) {
                    certification.push(self.text(
                        format!(
                            "· {}：{}（期望 {}）",
                            entry.check, entry.value, entry.expected
                        ),
                        CAPTION,
                        p.text3,
                    ));
                }
                if layer.state == "unknown" {
                    certification.push(self.text(&layer.note, CAPTION, p.text3));
                }
            }
            for blocker in &g.play_protect.blockers {
                certification.push(self.text(
                    format!("[{}] {}", blocker.owner, blocker.required_action),
                    CAPTION,
                    p.text3,
                ));
            }
            // Highest-leverage OEM action first: the phone cannot do any of these.
            if let Some(first) = g.play_protect.leverage.first() {
                certification.push(self.between(
                    self.line("最优先的厂商动作", BODY, p.text),
                    self.line(&first.owner, CAPTION, p.text3),
                ));
                certification.push(self.text(&first.action, BODY, p.text2));
                certification.push(self.text(
                    format!("可解除：{}。{}", first.clears.join(" + "), first.reason),
                    CAPTION,
                    p.text3,
                ));
            }
            if let Some(patch) = &g.play_protect.patch_freshness {
                if let Some(age) = patch.age_days {
                    certification.push(self.between(
                        self.line("安全补丁新鲜度", BODY, p.text),
                        self.line(
                            format!("{age} 天"),
                            CAPTION,
                            if patch.stale == Some(true) {
                                p.warning
                            } else {
                                p.success
                            },
                        ),
                    ));
                    certification.push(self.text(
                        "Play Integrity 的 MEETS_DEVICE_INTEGRITY 会检查补丁新鲜度，\
                         与 Play 商店的认证不是同一项；Google 未公布阈值，这里只报告实测天数。",
                        CAPTION,
                        p.text3,
                    ));
                }
            }
            if let Some(firmware) = &g.play_protect.firmware {
                if firmware.available {
                    certification.push(self.between(
                        self.line("官方固件通道", BODY, p.text),
                        self.line(
                            if firmware.same_as_current {
                                "已在最新"
                            } else {
                                "有更新构建"
                            },
                            CAPTION,
                            p.text3,
                        ),
                    ));
                    certification.push(self.text(
                        match (&firmware.latest_version, &firmware.release_date) {
                            (Some(version), Some(date)) => format!(
                                "通道最新 {version}（{date}）；设备当前 {}。",
                                firmware.current_build.as_deref().unwrap_or("未知")
                            ),
                            (Some(version), None) => format!(
                                "通道最新 {version}；设备当前 {}。",
                                firmware.current_build.as_deref().unwrap_or("未知")
                            ),
                            _ => "固件通道状态未完整取得。".into(),
                        },
                        CAPTION,
                        p.text3,
                    ));
                }
            }
            if let Some(vendor) = &g.play_protect.vendor_services {
                if !vendor.note.is_empty() {
                    certification.push(self.text("厂商服务身份（不影响认证判定）", BODY, p.text));
                    certification.push(self.text(&vendor.note, CAPTION, p.text3));
                }
                if let Some(readable) = vendor.dsid_readable_from_host {
                    certification.push(self.text(
                        format!(
                            "固件更新查询：{}。GSF ID {}",
                            if readable {
                                "宿主可读 dsid"
                            } else {
                                "宿主读不到 dsid，请在手机设置 → 系统更新中检查"
                            },
                            if vendor.gsf_android_id.is_empty() {
                                "未取到".to_string()
                            } else {
                                vendor.gsf_android_id.clone()
                            }
                        ),
                        CAPTION,
                        p.text3,
                    ));
                }
            }
        } else if !g.play_protect.note.is_empty() {
            certification.push(self.text(&g.play_protect.note, CAPTION, p.text3));
        }
        if !g.boot_security_note.is_empty() {
            certification.push(self.text(&g.boot_security_note, CAPTION, p.text2));
        }
        if !g.certification_catalog.is_empty() {
            certification.push(self.text(&g.certification_catalog, CAPTION, p.text2));
        }
        certification.push(self.hstack(
            S2,
            vec![
                self.button(
                    "google-cert-help",
                    "官方排查说明",
                    Action::StudioOpenUrl(
                        "https://support.google.com/googleplay/answer/7165974?hl=zh-Hans".into(),
                    ),
                    Btn::Secondary,
                ),
                self.button(
                    "google-cert-register",
                    "官方设备登记",
                    Action::StudioOpenUrl("https://www.google.com/android/uncertified/".into()),
                    Btn::Secondary,
                ),
            ],
        ));
        self.vstack(
            S4,
            vec![
                self.panel(rows),
                self.panel(matrix),
                self.panel(compatibility),
                self.panel(certification),
            ],
        )
    }
}

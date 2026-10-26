use super::super::{
    backscreen_state::BackScreenAction, icons::UiIcon, studio_state::StudioAction, theme::*,
    update_state::UpdateAction, widgets::*, work_state::WorkAction, Action, Companion, Page,
    ToolOperation, ToolPage, F_OV_TEXT,
};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use stepx_companion::{storage::format_bytes, toolbox::FileInfo};

fn file_category_badge(path: &str, p: &Palette) -> (UiIcon, &'static str, ColorU, ColorU) {
    let lower = path.to_ascii_lowercase();
    if lower.contains("micromsg") || lower.contains("wechat") || lower.contains("weixin") {
        (UiIcon::Chat, "微信缓存", p.success_text, p.success_soft)
    } else if lower.ends_with(".mp4")
        || lower.ends_with(".mov")
        || lower.ends_with(".mkv")
        || lower.ends_with(".avi")
    {
        (UiIcon::Video, "视频", p.accent, p.accent_soft)
    } else if lower.ends_with(".apk")
        || lower.ends_with(".zip")
        || lower.ends_with(".7z")
        || lower.ends_with(".rar")
        || lower.ends_with(".tar")
        || lower.ends_with(".gz")
    {
        (UiIcon::Package, "安装包/压缩包", p.warning, p.warning_soft)
    } else if lower.ends_with(".pdf")
        || lower.ends_with(".doc")
        || lower.ends_with(".docx")
        || lower.ends_with(".xls")
        || lower.ends_with(".xlsx")
        || lower.ends_with(".ppt")
        || lower.ends_with(".pptx")
        || lower.ends_with(".txt")
    {
        (UiIcon::File, "文档", p.text2, p.surface)
    } else {
        (UiIcon::Folder, "文件", p.text2, p.surface)
    }
}

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
        let (cat_icon, cat_label, cat_fg, cat_bg) = file_category_badge(&file.path, &p);
        let mut details = vec![
            self.hstack(
                S2,
                vec![
                    self.pill_badge_icon(cat_label, cat_icon, cat_fg, cat_bg),
                    self.line_w(name, BODY, p.text, Weight::Medium),
                ],
            ),
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
        let row_inner = self.between(
            self.vstack(S1, details),
            self.hstack(
                S3,
                vec![
                    self.mono(format_bytes(file.bytes), CAPTION, p.text2),
                    self.button_icon(
                        format!("tool-backup-file-{index}"),
                        "先备份到电脑",
                        UiIcon::Storage,
                        Action::BackupPhoneFileToDesktop(file.path.clone()),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        format!("tool-copy-path-{index}"),
                        "复制路径",
                        UiIcon::Clipboard,
                        Action::CopyText(file.path.clone(), "已复制手机文件路径".into()),
                        Btn::Ghost,
                    ),
                    action,
                ],
            ),
        );
        let dbl_path = file.path.clone();
        let menu_path = file.path.clone();
        Hoverable::new(
            self.mouse_state(format!("tool-file-row-{index}")),
            move |mouse| {
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else if mouse.is_mouse_over_element() {
                    p.hover
                } else {
                    ColorU::transparent_black()
                };
                Container::new(row_inner)
                    .with_uniform_padding(S3)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            },
        )
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::LocateToolFile(dbl_path.clone()))
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::ToolFile {
                    path: menu_path.clone(),
                    operation: menu_op.clone(),
                },
                at,
            ))
        })
        .finish()
    }

    fn input_method_tool_page(&self) -> El {
        let p = self.p();
        let r = &self.input_method;
        self.vstack(
            S4,
            vec![
                self.panel(vec![
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::Keyboard, 18., p.text),
                            self.heading("电脑打字直输手机 & 剪贴板同步工作台"),
                        ],
                    ),
                    self.text(
                        "在电脑键盘输入长句、链接或密码，一键发送到手机当前输入框或双向同步剪贴板。",
                        CAPTION,
                        p.text2,
                    ),
                    self.input(
                        F_OV_TEXT,
                        "在电脑输入文字，一键发送到手机当前输入框或手机剪贴板…",
                        None,
                    ),
                    self.wrap(vec![
                        self.button_icon(
                            "ime-send-focus-text",
                            "发送到手机输入框",
                            UiIcon::Send,
                            Action::SendFocusText,
                            Btn::Primary,
                        ),
                        self.button_icon(
                            "ime-push-clipboard",
                            "写入手机剪贴板",
                            UiIcon::Clipboard,
                            Action::BackScreen(BackScreenAction::PushDesktopClipboard),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "ime-pull-clipboard",
                            "拉取手机剪贴板到电脑",
                            UiIcon::Download,
                            Action::BackScreen(BackScreenAction::PullPhoneClipboard),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "ime-refresh-list",
                            "刷新输入法列表",
                            UiIcon::Refresh,
                            Action::InspectInputMethod,
                            Btn::Ghost,
                        ),
                    ]),
                ]),
                self.panel(vec![
                    self.heading("小企鹅中文输入法"),
                    self.text(
                        "开源中文输入法，支持拼音、中文候选与中英文切换。",
                        BODY,
                        p.text2,
                    ),
                    self.divider(),
                    self.line(
                        if !r.checked {
                            "尚未读取手机状态"
                        } else if r.active {
                            "默认输入法：小企鹅 Fcitx5"
                        } else {
                            "默认输入法：其他输入法"
                        },
                        BODY,
                        p.text,
                    ),
                    self.text(
                        if !r.checked {
                            "读取后显示真实安装与默认状态。"
                        } else if !r.sogou_installed {
                            "搜狗：当前用户未安装"
                        } else if r.sogou_disabled {
                            "搜狗：已停用"
                        } else {
                            "搜狗：已安装，可在切换后停用"
                        },
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(if self.busy {
                        vec![self.disabled_button("正在处理，请稍候…")]
                    } else {
                        vec![
                            self.button(
                                "ime-install",
                                "一键安装并使用",
                                Action::SwitchInputMethodFcitx5(true),
                                Btn::Primary,
                            ),
                            self.button(
                                "ime-refresh",
                                "刷新状态",
                                Action::InspectInputMethod,
                                Btn::Secondary,
                            ),
                            self.button(
                                "ime-settings",
                                "手机输入法设置",
                                Action::OpenInputSettings,
                                Btn::Secondary,
                            ),
                            self.button(
                                "ime-restore",
                                "恢复搜狗…",
                                Action::SwitchInputMethodFcitx5(false),
                                Btn::Ghost,
                            ),
                        ]
                    }),
                    self.text(
                        "确认后应用到当前手机：校验安装包 → 启用小企鹅 → 核对默认输入法 → 停用搜狗。已有输入法和词库不删除，系统安全输入法保留。",
                        CAPTION,
                        p.text2,
                    ),
                    self.link_muted(
                        "ime-source",
                        "官方项目与开源说明",
                        Action::StudioOpenUrl(
                            "https://github.com/fcitx5-android/fcitx5-android".into(),
                        ),
                    ),
                ]),
            ],
        )
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
            sections.push(self.panel(vec![self.between(
                self.text(result, CAPTION, p.text),
                self.button_icon(
                    "tools-copy-last-result",
                    "复制处理结果",
                    UiIcon::Clipboard,
                    Action::CopyText(result.clone(), "已复制处理结果".into()),
                    Btn::Ghost,
                ),
            )]));
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
        let tab_counts = self.storage_report.as_ref().map(|r| {
            [
                r.documents.len(),
                r.duplicates
                    .iter()
                    .map(|g| g.len().saturating_sub(1))
                    .sum::<usize>(),
                r.largest.len(),
            ]
        });
        sections.push(
            self.hstack(
                S2,
                ["文档压缩", "重复文件", "大文件"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, label)| {
                        let active = self.tools_tab as usize == i;
                        let text = match tab_counts {
                            Some(counts) => format!("{label} ({})", counts[i]),
                            None => label.to_string(),
                        };
                        self.button(
                            format!("tools-tab-{i}"),
                            text,
                            Action::ToolTab(i as u8),
                            if active { Btn::Primary } else { Btn::Secondary },
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
        sections.push(self.between(
            self.text(scope.clone(), CAPTION, p.text2),
            self.button_icon(
                "tools-copy-storage-scope",
                "复制扫描摘要",
                UiIcon::Clipboard,
                Action::CopyText(
                    format!(
                        "{scope}\n可用空间：{} · 数据分区总量：{} · 重复副本大小：{}",
                        format_bytes(report.available_bytes),
                        format_bytes(report.total_bytes),
                        format_bytes(report.duplicate_bytes)
                    ),
                    "已复制存储扫描报告".into(),
                ),
                Btn::Ghost,
            ),
        ));
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
        let (icon, state) = match tool {
            ToolPage::InputMethod => (
                UiIcon::Keyboard,
                if self.input_method.active {
                    "小企鹅已启用"
                } else {
                    "安装中文输入法"
                }
                .into(),
            ),
            ToolPage::Storage => (
                UiIcon::File,
                self.storage_report
                    .as_ref()
                    .map(|s| format!("可用 {}", format_bytes(s.available_bytes)))
                    .unwrap_or("进入后扫描".into()),
            ),
            ToolPage::Google => (
                UiIcon::Shield,
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
                UiIcon::Smartphone,
                if self.companion_status.installed {
                    if self.companion_status.version.trim().is_empty() {
                        "已安装 · 已就绪".into()
                    } else {
                        format!("已安装 · {}", self.companion_status.version)
                    }
                } else if self.companion_status.checked {
                    "等待安装".into()
                } else {
                    "进入后检测".into()
                },
            ),
            ToolPage::Neo => (
                UiIcon::Sparkles,
                if self.neo_status.selected {
                    "手机执行已接入"
                } else if self.neo_status.installed {
                    "已安装 · 待配置"
                } else {
                    "更新伴侣即可使用"
                }
                .into(),
            ),
            ToolPage::Apk => (UiIcon::Apps, "选择本地文件".into()),
            ToolPage::Services => (
                UiIcon::Menu,
                if self.system_services.is_empty() {
                    "进入后检测".into()
                } else {
                    format!("{} 项服务", self.system_services.len())
                },
            ),
            ToolPage::Screen => (
                UiIcon::Monitor,
                if self.mirror.is_some() {
                    "正在投屏"
                } else {
                    "按需开启"
                }
                .into(),
            ),
            ToolPage::Device => (UiIcon::Home, "读取手机状态".into()),
            ToolPage::Updates => (UiIcon::Refresh, self.updates.title().to_owned()),
        };
        let content = self.vstack(
            S3,
            vec![
                self.between(
                    self.hstack(
                        S3,
                        vec![
                            self.ui_icon_box(icon, 36., p.surface, p.text2, None),
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
            ],
        );
        self.hover_panel_interactive(
            format!("tool-card-{tool:?}"),
            content,
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
                && (self.tool_page.is_none() || self.tool_page == *tool)
            {
                let label = format!(
                    "{} · {}",
                    tool.map(ToolPage::title).unwrap_or("小工具"),
                    message
                );
                parts.push(self.panel(vec![self.between(
                    self.text(
                        label.clone(),
                        CAPTION,
                        if *error { p.danger } else { p.text2 },
                    ),
                    self.button_icon(
                        "tools-copy-feedback",
                        "复制反馈信息",
                        UiIcon::Clipboard,
                        Action::CopyText(label, "已复制工具输出".into()),
                        Btn::Ghost,
                    ),
                )]));
            }
        }
        if !self.content.trim().is_empty() && self.output_page == Page::Tools {
            parts.push(self.context_target(
                "tools-run-output",
                crate::ui::context_menu::Target::CopyableText {
                    title: "诊断/执行结果".into(),
                    text: self.content.clone(),
                },
                self.panel(vec![
                    self.between(
                        self.heading("工具运行输出"),
                        self.button_icon(
                            "tools-copy-run-output",
                            "复制运行输出",
                            UiIcon::Clipboard,
                            Action::CopyText(self.content.clone(), "已复制工具运行输出".into()),
                            Btn::Secondary,
                        ),
                    ),
                    self.mono(&self.content, CAPTION, p.text2),
                ]),
            ));
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
                parts.push(self.vstack(S2, vec![self.heading("工具集合"), self.text("按需要选择工具，检测和操作结果会留在这里。", BODY, p.text2)]));
                parts.push(self.segmented("tool-categories", ["全部", "空间整理", "应用与服务", "设备协同"].into_iter().enumerate().map(|(i, title)|
                    (format!("{title} · {}",ToolPage::ALL.iter().filter(|t|i==0 || t.category()==i as u8).count()), i as u8 == self.tool_category, Action::ToolCategory(i as u8))
                ).collect()));
                let tools: Vec<_> = ToolPage::ALL.into_iter().filter(|t| self.tool_category == 0 || t.category() == self.tool_category).collect();
                for row in tools.chunks(2) {
                    let mut cards: Vec<El> = row.iter().map(|t| Expanded::new(1., self.tool_card(*t)).finish()).collect();
                    if row.len() == 1 { cards.push(Expanded::new(1., Empty::new().finish()).finish()); }
                    parts.push(self.hstack(S4, cards));
                }
            }
            Some(ToolPage::Updates) => parts.push(self.updates_tool_page()),
            Some(ToolPage::Companion) => parts.push(self.companion_tool_page()),
            Some(ToolPage::Neo) => parts.push(self.neo_tool_page()),
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
                let status_badge = if self.live_screen_stream {
                    self.tag("● 实时连拍同步中", p.success)
                } else if self.mirror.is_some() {
                    self.tag("● 独立窗投屏中", p.accent)
                } else {
                    self.tag("单击画面直接触控", p.text2)
                };

                let left_studio = self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.ui_icon(UiIcon::Smartphone, 18., p.text),
                                self.heading("手机屏幕实时触控台"),
                            ],
                        ),
                        status_badge,
                    ),
                    self.wrap(vec![
                        self.button_icon(
                            "screen-toggle-live",
                            if self.live_screen_stream {
                                "暂停内嵌连拍"
                            } else {
                                "开启内嵌实时画面"
                            },
                            UiIcon::Refresh,
                            Action::ToggleLiveScreenStream,
                            if self.live_screen_stream {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button_icon(
                            "screen-refresh-now",
                            "立即刷新画面",
                            UiIcon::Camera,
                            Action::Screenshot,
                            Btn::Secondary,
                        ),
                        if self.mirror.is_some() {
                            self.button(
                                "mirror-control-stop",
                                "结束独立投屏",
                                Action::StopMirror,
                                Btn::Danger,
                            )
                        } else {
                            self.utility_button(
                                "mirror-control-start",
                                "独立高帧率窗口 (scrcpy)",
                                Action::Mirror,
                                true,
                            )
                        },
                    ]),
                    Container::new(
                        Align::new(self.interactive_phone_bezel("tool-screen", 216., 452., None))
                            .finish(),
                    )
                    .with_vertical_padding(S2)
                    .finish(),
                    self.text(
                        "提示：单击屏幕画面直接触控手机对应坐标 · 双击画面开启 60/120帧 独立投屏 · 右侧与底部按键操作后自动刷新画面",
                        CAPTION,
                        p.text3,
                    ),
                ]);

                let panel_quality = self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.ui_icon(UiIcon::Monitor, 18., p.text),
                                self.heading("独立窗口投屏画质与模式"),
                            ],
                        ),
                        self.line("scrcpy 硬件解码", CAPTION, p.text3),
                    ),
                    self.wrap(vec![
                        self.chip(
                            "mirror-quality-0",
                            "720P 流畅 (30帧)",
                            Some(UiIcon::Monitor),
                            vec![Action::SetMirrorQuality(0)],
                            self.mirror_options.quality == 0,
                            false,
                        ),
                        self.chip(
                            "mirror-quality-1",
                            "1080P 高清 (60帧)",
                            Some(UiIcon::Monitor),
                            vec![Action::SetMirrorQuality(1)],
                            self.mirror_options.quality == 1,
                            false,
                        ),
                        self.chip(
                            "mirror-quality-2",
                            "原画超清 (120帧)",
                            Some(UiIcon::Sparkles),
                            vec![Action::SetMirrorQuality(2)],
                            self.mirror_options.quality >= 2,
                            false,
                        ),
                    ]),
                    self.wrap(vec![
                        self.chip(
                            "mirror-opt-top",
                            "窗口始终置顶",
                            Some(UiIcon::Pin),
                            vec![Action::ToggleMirrorAlwaysOnTop],
                            self.mirror_options.always_on_top,
                            false,
                        ),
                        self.chip(
                            "mirror-opt-screen-off",
                            "投屏时手机息屏省电",
                            Some(UiIcon::Lock),
                            vec![Action::ToggleMirrorTurnScreenOff],
                            self.mirror_options.turn_screen_off,
                            false,
                        ),
                        self.chip(
                            "mirror-opt-awake",
                            "保持手机屏幕常亮",
                            Some(UiIcon::Sun),
                            vec![Action::ToggleMirrorStayAwake],
                            self.mirror_options.stay_awake,
                            false,
                        ),
                        self.chip(
                            "mirror-opt-touches",
                            "显示手机触控点轨迹",
                            Some(UiIcon::Smartphone),
                            vec![Action::ToggleMirrorShowTouches],
                            self.mirror_options.show_touches,
                            false,
                        ),
                    ]),
                ]);

                let panel_gestures = self.panel(vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.ui_icon(UiIcon::Bolt, 18., p.text),
                                self.heading("屏幕手势与系统状态栏"),
                            ],
                        ),
                        self.line("执行后自动同步画面", CAPTION, p.text3),
                    ),
                    self.wrap(vec![
                        self.button_icon(
                            "screen-gesture-unlock",
                            "一键点亮并解锁",
                            UiIcon::Sun,
                            Action::ScreenGesture(7),
                            Btn::Primary,
                        ),
                        self.button_icon(
                            "screen-gesture-up",
                            "上滑翻页",
                            UiIcon::ArrowUp,
                            Action::ScreenGesture(0),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-gesture-down",
                            "下滑刷新",
                            UiIcon::ArrowDown,
                            Action::ScreenGesture(1),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-gesture-left",
                            "左滑切屏",
                            UiIcon::ArrowLeft,
                            Action::ScreenGesture(2),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-gesture-right",
                            "右滑返回",
                            UiIcon::ArrowRight,
                            Action::ScreenGesture(3),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-gesture-notif",
                            "下拉通知栏",
                            UiIcon::Bell,
                            Action::ScreenGesture(4),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-gesture-qs",
                            "展开快捷设置",
                            UiIcon::Settings,
                            Action::ScreenGesture(5),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-gesture-collapse",
                            "收起状态栏",
                            UiIcon::ChevronUp,
                            Action::ScreenGesture(6),
                            Btn::Ghost,
                        ),
                    ]),
                    self.wrap(vec![
                        self.button_icon("screen-key-wake", "唤醒亮屏", UiIcon::Sun, Action::Key(224), Btn::Secondary),
                        self.button_icon("screen-key-sleep", "锁屏", UiIcon::Lock, Action::Key(223), Btn::Secondary),
                        self.button_icon("screen-key-home", "桌面", UiIcon::Home, Action::Key(3), Btn::Secondary),
                        self.button_icon("screen-key-back", "返回", UiIcon::ArrowLeft, Action::Key(4), Btn::Secondary),
                        self.button_icon("screen-key-recents", "多任务", UiIcon::Apps, Action::Key(187), Btn::Secondary),
                        self.button_icon("screen-key-vol-up", "音量+", UiIcon::Speaker, Action::Key(24), Btn::Secondary),
                        self.button_icon("screen-key-vol-down", "音量-", UiIcon::Speaker, Action::Key(25), Btn::Secondary),
                        self.button_icon("screen-key-mute", "静音", UiIcon::Mute, Action::Key(164), Btn::Ghost),
                    ]),
                ]);

                let panel_input = self.panel(vec![
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::Keyboard, 18., p.text),
                            self.heading("向手机直接打字与剪贴板同步"),
                        ],
                    ),
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(S2)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Expanded::new(
                                1.,
                                self.input(F_OV_TEXT, "向手机当前焦点输入框直接发送文字…", None),
                            )
                            .finish(),
                        )
                        .with_child(self.button(
                            "screen-send-text",
                            "发送文字",
                            Action::SendFocusText,
                            Btn::Primary,
                        ))
                        .finish(),
                    self.wrap(vec![
                        self.button_icon(
                            "screen-key-enter",
                            "发送回车 (Enter)",
                            UiIcon::Send,
                            Action::Key(66),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-key-backspace",
                            "退格删除 (Backspace)",
                            UiIcon::Backspace,
                            Action::Key(67),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-key-media",
                            "播放/暂停媒体",
                            UiIcon::Play,
                            Action::Key(85),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-clip-push",
                            "电脑剪贴板发手机",
                            UiIcon::Upload,
                            Action::BackScreen(BackScreenAction::PushDesktopClipboard),
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "screen-clip-pull",
                            "拉取手机剪贴板",
                            UiIcon::Download,
                            Action::BackScreen(BackScreenAction::PullPhoneClipboard),
                            Btn::Secondary,
                        ),
                    ]),
                ]);

                let mut shot_actions = vec![
                    self.utility_button("tool-shot", "截取画面", Action::Screenshot, true),
                    if self.screenshot.is_some() {
                        self.utility_button("tool-save-shot", "保存截图", Action::SaveScreenshot, false)
                    } else {
                        self.disabled_button("保存截图")
                    },
                ];
                if let Some((_, path)) = &self.screenshot {
                    let path_str = path.to_string_lossy().into_owned();
                    shot_actions.push(self.button_icon(
                        "tool-zoom-shot",
                        "全屏放大看图",
                        UiIcon::Search,
                        Action::Work(WorkAction::OpenPreview(path_str.clone())),
                        Btn::Secondary,
                    ));
                    shot_actions.push(self.button_icon(
                        "tool-open-shot-sys",
                        "系统看图打开",
                        UiIcon::Image,
                        Action::Studio(StudioAction::OpenPath(path.clone())),
                        Btn::Secondary,
                    ));
                    shot_actions.push(self.button_icon(
                        "tool-reveal-shot-dir",
                        "打开所在文件夹",
                        UiIcon::Folder,
                        Action::RevealLocalPath(path.clone()),
                        Btn::Secondary,
                    ));
                    shot_actions.push(self.button_icon(
                        "tool-copy-shot-path",
                        "复制路径",
                        UiIcon::Clipboard,
                        Action::CopyText(path_str, "已复制截图本地路径".into()),
                        Btn::Ghost,
                    ));
                    shot_actions.push(self.button_icon(
                        "tool-shot-to-backscreen",
                        "转为背屏像素画",
                        UiIcon::Sparkles,
                        Action::ScreenshotToBackScreen,
                        Btn::Secondary,
                    ));
                }
                let panel_shot = self.panel(vec![
                    self.heading("画面截图与跨端流转"),
                    self.text("截取当前手机画面，预览后支持全屏放大查看、转背屏像素画或选择保存位置。", BODY, p.text2),
                    self.wrap(shot_actions),
                ]);

                let right_hub = self.vstack(
                    S4,
                    vec![panel_quality, panel_gestures, panel_input, panel_shot],
                );

                parts.push(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(S4)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(Expanded::new(5., left_studio).finish())
                        .with_child(Expanded::new(7., right_hub).finish())
                        .finish(),
                );
            }
            Some(ToolPage::Services) => {
                parts.push(self.panel(vec![self.text("仅列出已适配的服务。停用会影响对应功能，请逐项选择；当前输入法不允许停用。", BODY, p.text2),
                    self.utility_button("tool-ime-settings", "手机输入法设置", Action::OpenInputSettings, false)]));
                if self.system_services.is_empty() { parts.push(self.panel(vec![self.text("尚无检测结果，请点击刷新状态。", BODY, p.text2)])); }
                for (i, service) in self.system_services.iter().enumerate() {
                    let can_toggle = service.safe_to_remove && !service.status_note.contains("当前默认") && service.status_note != "未安装";
                    let enabled = service.enabled;
                    parts.push(self.hover_panel_interactive(
                        format!("tool-service-row-{i}"),
                        self.vstack(
                            S3,
                            vec![
                                self.between(
                                    self.vstack(S1, vec![self.heading(&service.name), self.line(&service.status_note, CAPTION, p.text2)]),
                                    if can_toggle {
                                        self.utility_button(
                                            &format!("tool-service-{i}"),
                                            if enabled { "停用" } else { "启用" },
                                            Action::PrepareUtility(Box::new(Action::ToggleSystemService(
                                                service.package.clone(),
                                                !enabled,
                                            ))),
                                            false,
                                        )
                                    } else {
                                        self.disabled_button("不可停用")
                                    },
                                ),
                                self.between(
                                    self.text(&service.package, CAPTION, p.text3),
                                    self.button_icon(
                                        format!("tool-service-copy-{i}"),
                                        "复制包名",
                                        UiIcon::Clipboard,
                                        Action::CopyText(service.package.clone(), "已复制系统服务包名".into()),
                                        Btn::Ghost,
                                    ),
                                ),
                            ],
                        ),
                        None,
                        can_toggle.then(|| {
                            Action::PrepareUtility(Box::new(Action::ToggleSystemService(
                                service.package.clone(),
                                !enabled,
                            )))
                        }),
                        Some(crate::ui::context_menu::Target::ServiceRow {
                            name: service.name.clone(),
                            pkg: service.package.clone(),
                            enabled,
                        }),
                    ));
                }
            }
            Some(ToolPage::Device) => {
                let report = self.selected.as_ref().map(|d| format!("型号：{}\nAndroid：{}\n连接：{}\n\n{}", d.model, d.android, d.transport, self.summary)).unwrap_or_default();
                parts.push(self.context_target(
                    "tools-device-report",
                    crate::ui::context_menu::Target::CopyableText {
                        title: "诊断/执行结果".into(),
                        text: report.clone(),
                    },
                    self.panel(vec![
                        self.between(
                            self.heading("设备状态"),
                            if !self.summary.is_empty() {
                                self.button_icon(
                                    "tool-copy-report",
                                    "复制设备报告",
                                    UiIcon::Clipboard,
                                    Action::CopyText(report.clone(), "已复制设备检测结果".into()),
                                    Btn::Secondary,
                                )
                            } else {
                                Empty::new().finish()
                            },
                        ),
                        self.text(if self.summary.is_empty() { "尚无检测结果，请点击重新检测。" } else { &report }, BODY, p.text2),
                    ]),
                ));
            }
        }
        self.sub_enter(self.vstack(S4, parts))
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
        let mut rows = vec![
            self.panel(vec![
                self.heading("StepX伴侣 · 手机屏幕自动操作与伴侣服务状态"),
                self.text("GUI Agent 集成在 StepX伴侣内，在手机上看屏、规划与执行，断开电脑后也可使用。Amoo 可以委派任务，伴侣提供暂停、继续和停止控制。", BODY, p.text2),
                self.text("选择内置 GUI Agent 的完整 StepX伴侣 APK。安装会核对签名与内置组件，保留伴侣数据，并启用本模块的无障碍服务；操作哪些应用由你在手机确认。", CAPTION, p.text3),
                self.hstack(S2, vec![
                    self.utility_button("neo-install", if neo.companion_installed { "更新 StepX伴侣…" } else { "安装 StepX伴侣…" }, Action::NeoInstall, true),
                    self.button("neo-ime", if self.neo_enable_ime { "增强中文输入：开启" } else { "增强中文输入：可选" }, Action::NeoIme(!self.neo_enable_ime), Btn::Secondary),
                ]),
                self.text("增强中文输入会启用 Agent 输入法；任务结束恢复原键盘。普通文字输入仍可使用无障碍。", CAPTION, p.text3),
            ]),
            self.panel(vec![
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Bolt, 18., p.text),
                        self.heading("一键下发手机操作任务"),
                    ],
                ),
                self.text(
                    "无需编写复杂指令，点击下方常用场景即可一键在手机端执行屏幕操作或转入 Amoo 操控台。",
                    CAPTION,
                    p.text2,
                ),
                self.wrap(vec![
                    self.button_icon(
                        "neo-quick-wake",
                        "点亮并解锁屏幕",
                        UiIcon::Sun,
                        Action::QuickGuiScenario {
                            app: "当前屏幕".into(),
                            goal: "点亮屏幕".into(),
                        },
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "neo-quick-shot",
                        "截取当前手机画面",
                        UiIcon::Camera,
                        Action::QuickGuiScenario {
                            app: "当前屏幕".into(),
                            goal: "截取手机画面".into(),
                        },
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "neo-quick-ocr",
                        "提取手机屏幕文字",
                        UiIcon::ScanText,
                        Action::QuickGuiScenario {
                            app: "当前屏幕".into(),
                            goal: "提取屏幕文字".into(),
                        },
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "neo-quick-wifi",
                        "打开手机 Wi-Fi 设置",
                        UiIcon::Wifi,
                        Action::QuickGuiScenario {
                            app: "设置".into(),
                            goal: "Wi-Fi 设置".into(),
                        },
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "neo-quick-bt",
                        "打开手机蓝牙设置",
                        UiIcon::Bluetooth,
                        Action::QuickGuiScenario {
                            app: "设置".into(),
                            goal: "蓝牙设置".into(),
                        },
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "neo-quick-amoo",
                        "进入 Amoo 操控台自定义任务",
                        UiIcon::Cpu,
                        Action::QuickGuiScenario {
                            app: "".into(),
                            goal: "".into(),
                        },
                        Btn::Primary,
                    ),
                ]),
            ]),
        ];
        if neo.checked {
            let state = |ready: bool| if ready { "已就绪" } else { "待配置" };
            let neo_summary = format!(
                "伴侣内置版本：{}\n状态说明：{}\n签名核验：{} · 无障碍启用：{} · 电池豁免：{}\n任务路由：{} · 增强输入法：{}",
                if neo.installed { neo.version.as_str() } else { "未安装" },
                neo.message,
                state(neo.signature_verified),
                state(neo.accessibility_enabled),
                state(neo.doze_exempt),
                if neo.selected { "Neo" } else { "未启用" },
                if neo.ime_enabled { "已启用" } else { "未启用" }
            );
            rows.push(self.panel(vec![
                self.between(
                    self.heading(if neo.installed {
                        format!("伴侣内置版本 · {}", neo.version)
                    } else {
                        "等待安装内置 GUI Agent 的伴侣版本".into()
                    }),
                    self.button_icon(
                        "neo-copy-status",
                        "复制状态报告",
                        UiIcon::Clipboard,
                        Action::CopyText(neo_summary, "已复制 GUI Agent 状态报告".into()),
                        Btn::Ghost,
                    ),
                ),
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
            rows.push(self.panel(vec![self.heading("恢复设置"),
                self.text("仅撤销本机添加的 GUI Agent 组件配置。伴侣原有无障碍、后台豁免、文件服务和数据保持原样。", CAPTION, p.text2),
                self.utility_button("neo-restore", "撤销组件配置", Action::PrepareUtility(Box::new(Action::NeoRestore)), false),
            ]));
        }
        self.vstack(S4, rows)
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
                if companion.checked {
                    self.button_icon(
                        "companion-copy-diag",
                        "复制检测结果",
                        UiIcon::Clipboard,
                        Action::CopyText(
                            format!(
                                "StepX伴侣：{}\n说明：{}\n后台连接：{}\n配对服务：{}",
                                if companion.installed {
                                    &companion.version
                                } else {
                                    "未安装"
                                },
                                companion.message,
                                companion.background_message,
                                companion.pairing_message
                            ),
                            "已复制 StepX伴侣检测输出".into(),
                        ),
                        Btn::Ghost,
                    )
                } else {
                    Empty::new().finish()
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
            self.between(
                self.heading("Google Play 安装与检测"),
                if g.checked {
                    self.button_icon(
                        "google-copy-diag",
                        "复制诊断摘要",
                        UiIcon::Clipboard,
                        Action::CopyText(
                            format!(
                                "Google 服务检测：{}\nGSF：{} · GMS：{} · Play：{}\n兼容补丁：{}\n官方认证：{}",
                                g.verification_note,
                                if g.gsf_version.is_empty() { "未检测到" } else { &g.gsf_version },
                                if g.gms_version.is_empty() { "未检测到" } else { &g.gms_version },
                                if g.play_version.is_empty() { "未检测到" } else { &g.play_version },
                                g.compatibility_note,
                                g.certification_note
                            ),
                            "已复制 Google 服务诊断结果".into(),
                        ),
                        Btn::Ghost,
                    )
                } else {
                    Empty::new().finish()
                },
            ),
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
            let open_primary = buttons.is_empty() && self.tool_confirmation.is_none();
            buttons.push(self.utility_button(
                "tool-google-open",
                "在手机打开 Play",
                Action::OpenApp("com.android.vending".into()),
                open_primary,
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

use super::super::{
    expansion_state::ExpansionAction as EA, icons::UiIcon, studio_state::StudioAction, theme::*,
    widgets::*, Action, Companion, FileModal, Page, F_FILE_NAME, F_FILE_SEARCH,
};
use facloud_ui::{
    assets::asset_cache::AssetSource, color::ColorU, elements::*, fonts::Weight, platform::Cursor,
};

const PLACES: [(UiIcon, &str, &str); 9] = [
    (UiIcon::Home, "手机主目录", "/sdcard"),
    (UiIcon::Camera, "相机相册", "/sdcard/DCIM/Camera"),
    (UiIcon::Image, "屏幕截图", "/sdcard/Pictures/Screenshots"),
    (UiIcon::Download, "下载目录", "/sdcard/Download"),
    (UiIcon::Chat, "微信接收文件", "/sdcard/Download/WeiXin"),
    (UiIcon::File, "文档目录", "/sdcard/Documents"),
    (UiIcon::Music, "音乐与录音", "/sdcard/Music"),
    (UiIcon::Video, "视频目录", "/sdcard/Movies"),
    (UiIcon::Trash, "回收站", "/sdcard/.stepx-trash"),
];

fn mix_color(a: ColorU, b: ColorU, t: f32) -> ColorU {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    if a.a == 0 {
        return ColorU::new(b.r, b.g, b.b, mix(0, b.a));
    }
    if b.a == 0 {
        return ColorU::new(a.r, a.g, a.b, mix(a.a, 0));
    }
    ColorU::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
}

#[allow(dead_code)]
struct FileMock {
    name: &'static str,
    date: &'static str,
    kind: &'static str,
    size: &'static str,
    is_dir: bool,
    selected: bool,
}

const FILE_MOCKS: [FileMock; 7] = [
    FileMock {
        name: "Camera",
        date: "今天 14:10",
        kind: "文件夹",
        size: "--",
        is_dir: true,
        selected: false,
    },
    FileMock {
        name: "ScreenRecorder",
        date: "昨天 19:42",
        kind: "文件夹",
        size: "--",
        is_dir: true,
        selected: false,
    },
    FileMock {
        name: "Screenshot_20261021_142809.png",
        date: "今天 14:28",
        kind: "PNG 图像",
        size: "2.4 MB",
        is_dir: false,
        selected: true,
    },
    FileMock {
        name: "Screenshot_20261021_110522.png",
        date: "今天 11:05",
        kind: "PNG 图像",
        size: "1.8 MB",
        is_dir: false,
        selected: false,
    },
    FileMock {
        name: "StepX_Product_Demo_1080p.mp4",
        date: "10-19 16:30",
        kind: "MP4 视频",
        size: "48.2 MB",
        is_dir: false,
        selected: false,
    },
    FileMock {
        name: "StepX_Design_Spec_v2.pdf",
        date: "10-18 09:15",
        kind: "PDF 文档",
        size: "4.1 MB",
        is_dir: false,
        selected: false,
    },
    FileMock {
        name: "device_log_20261018.txt",
        date: "10-18 08:40",
        kind: "纯文本",
        size: "128 KB",
        is_dir: false,
        selected: false,
    },
];

/// Common Android media directories keep their names and gain a semantic icon.
fn file_symbol(entry: &stepx_companion::storage::Entry) -> (UiIcon, &'static str) {
    if entry.directory {
        let name = entry.name.to_ascii_lowercase();
        return match name.as_str() {
            "music" => (UiIcon::Music, "音乐文件夹"),
            "audiobooks" => (UiIcon::Music, "有声书文件夹"),
            "podcasts" => (UiIcon::Mic, "播客文件夹"),
            "recordings" | "recording" | "soundrecorder" => (UiIcon::Mic, "录音文件夹"),
            "movies" | "screenrecorder" => (UiIcon::Video, "视频文件夹"),
            "dcim" | "camera" => (UiIcon::Camera, "相机文件夹"),
            "pictures" | "screenshots" => (UiIcon::Image, "图片文件夹"),
            "download" | "downloads" => (UiIcon::Download, "下载文件夹"),
            "documents" => (UiIcon::File, "文档文件夹"),
            "alarms" => (UiIcon::Clock, "闹钟铃声"),
            "ringtones" | "notifications" => (UiIcon::Bell, "铃声文件夹"),
            "android" => (UiIcon::Settings, "应用数据目录"),
            ".stepx-trash" => (UiIcon::Trash, "回收站"),
            ".stepaossafebox" => (UiIcon::Lock, "保险箱目录"),
            _ => (UiIcon::Folder, entry.kind_badge()),
        };
    }
    (
        match entry.kind_badge() {
            "图片" => UiIcon::Image,
            "视频" => UiIcon::Video,
            "音频" => UiIcon::Music,
            "安装包" | "压缩包" => UiIcon::Package,
            _ => UiIcon::File,
        },
        entry.kind_badge(),
    )
}

impl Companion {
    pub(in super::super) fn files_actions(&self) -> Vec<El> {
        vec![
            self.button_icon(
                "files-new-txt",
                "新建文本",
                UiIcon::File,
                Action::OpenNewTextFile,
                Btn::Secondary,
            ),
            self.button_icon(
                "files-mkdir",
                "新建文件夹",
                UiIcon::Plus,
                Action::OpenNewFolder,
                Btn::Secondary,
            ),
            self.button_icon(
                "files-open-recv-dir",
                "电脑接收目录",
                UiIcon::Folder,
                Action::CompanionReceiveFolder,
                Btn::Secondary,
            ),
            self.button(
                "files-refresh",
                "刷新",
                Action::Browse(self.directory.clone()),
                Btn::Secondary,
            ),
            self.button_icon(
                "files-upload",
                "上传文件",
                UiIcon::Upload,
                Action::Upload,
                Btn::Primary,
            ),
        ]
    }

    pub(in super::super) fn storage_mode_bar(&self) -> El {
        let p = self.p();
        let mode_seg = Container::new(self.hstack(
            2.,
            vec![
                self.button_icon(
                    "storage-mode-files",
                    "手机文件管理",
                    UiIcon::Folder,
                    Action::Page(Page::Files),
                    if self.page == Page::Files {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                ),
                self.button_icon(
                    "storage-mode-expansion",
                    "手机存储扩容",
                    UiIcon::Storage,
                    Action::Page(Page::Expansion),
                    if self.page == Page::Expansion {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                ),
            ],
        ))
        .with_uniform_padding(2.)
        .with_background_color(p.sidebar)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL + 1.)))
        .finish();
        let archived_count = self.expansion.snapshot.status["count"]
            .as_u64()
            .unwrap_or_else(|| {
                self.expansion
                    .snapshot
                    .files
                    .iter()
                    .filter(|j| j["state"] == "archived")
                    .count() as u64
            });
        let right_info = if archived_count > 0 {
            self.pill_badge(
                format!("电脑已归档 {archived_count} 项"),
                p.success_text,
                p.success_soft,
            )
        } else {
            self.pill_badge("电脑本地归档 · 释放手机空间", p.text2, p.tag_bg)
        };
        Container::new(self.between(mode_seg, right_info))
            .with_horizontal_padding(S3)
            .with_vertical_padding(6.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    fn file_grid_row(
        &self,
        col_name: El,
        col_date: El,
        col_type: El,
        col_size: El,
        col_actions: El,
    ) -> El {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(Expanded::new(5., col_name).finish())
            .with_child(Expanded::new(2., col_date).finish())
            .with_child(Expanded::new(2., col_type).finish())
            .with_child(
                Expanded::new(
                    1.,
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .with_child(col_size)
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                ConstrainedBox::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .with_child(col_actions)
                        .finish(),
                )
                .with_width(200.)
                .finish(),
            )
            .finish()
    }

    fn received_transfers(&self) -> Vec<(String, std::path::PathBuf, u64)> {
        type ReceivedTransferItem = (String, std::path::PathBuf, u64);
        type ReceivedCacheKey = (usize, Option<String>, bool, String);
        static RECEIVED_TRANSFERS_CACHE: std::sync::OnceLock<
            std::sync::Mutex<
                Option<(
                    std::time::Instant,
                    ReceivedCacheKey,
                    Vec<ReceivedTransferItem>,
                )>,
            >,
        > = std::sync::OnceLock::new();

        let cache_key = (
            self.files.len(),
            self.expansion.files_preview_path.clone(),
            self.demo_mode,
            self.status.clone(),
        );
        let cache = RECEIVED_TRANSFERS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
        if let Ok(guard) = cache.lock() {
            if let Some((at, key, cached)) = guard.as_ref() {
                if at.elapsed() < std::time::Duration::from_millis(1500) && *key == cache_key {
                    return cached.clone();
                }
            }
        }

        let mut items = Vec::new();
        let mut scan_dir = |root: &std::path::Path| {
            let Ok(rd) = std::fs::read_dir(root) else {
                return;
            };
            for entry in rd.take(64).flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if !name.starts_with('.')
                            && name != "manifest.json"
                            && name != "receipt.json"
                        {
                            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            items.push((name.to_string(), path, bytes));
                        }
                    }
                } else if path.is_dir() {
                    if let Ok(sub) = std::fs::read_dir(&path) {
                        for sub_entry in sub.take(8).flatten() {
                            let sp = sub_entry.path();
                            if sp.is_file() {
                                if let Some(name) = sp.file_name().and_then(|n| n.to_str()) {
                                    if !name.starts_with('.')
                                        && name != "manifest.json"
                                        && name != "receipt.json"
                                    {
                                        let bytes =
                                            sub_entry.metadata().map(|m| m.len()).unwrap_or(0);
                                        items.push((name.to_string(), sp, bytes));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        if let Ok(dl) = stepx_companion::companion_link::receive_directory() {
            scan_dir(&dl);
        }
        let previews_dir = stepx_companion::resources::user_state().join("file-previews");
        scan_dir(&previews_dir);

        if items.is_empty() && self.demo_mode {
            for (demo_name, rel_path, fallback_bytes) in [
                (
                    "Screenshot_20261021_142809.png",
                    "desktop/assets/stepx_home_screen.png",
                    2_516_582u64,
                ),
                (
                    "Screenshot_20261021_110522.png",
                    "desktop/assets/neo_front_frame.png",
                    1_887_436u64,
                ),
            ] {
                let p1 = std::path::PathBuf::from(rel_path);
                let resolved = if cached_is_file(&p1) {
                    p1
                } else if let Some(stripped) = rel_path.strip_prefix("desktop/") {
                    std::path::PathBuf::from(stripped)
                } else {
                    p1
                };
                let size = cached_file_len(&resolved).unwrap_or(fallback_bytes);
                items.push((demo_name.to_string(), resolved, size));
            }
        }
        items.truncate(8);
        if let Ok(mut guard) = cache.lock() {
            *guard = Some((std::time::Instant::now(), cache_key, items.clone()));
        }
        items
    }

    pub(in super::super) fn files_page(&self) -> El {
        let p = self.p();

        // 1. Top Bar: Quick Places + Breadcrumbs + Category Filter + Sort + Direction
        let place_btns: Vec<El> = PLACES
            .iter()
            .enumerate()
            .map(|(i, (icon, label, path))| {
                let active = self.directory == *path;
                let state = self.mouse_state(format!("files-place:{i}"));
                let act = Action::Browse((*path).to_string());
                Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let t = if self.settings.value.reduced_motion {
                        if hovered {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        mouse.hover_progress().clamp(0.0, 1.0)
                    };
                    let active_bg = if p.dark {
                        rgb(0x27, 0x27, 0x2A)
                    } else {
                        rgb(0x18, 0x18, 0x1B)
                    };
                    let bg = if active {
                        if mouse.is_clicked() {
                            p.accent_hover
                        } else {
                            active_bg
                        }
                    } else if mouse.is_clicked() {
                        p.accent_soft
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    let fg = if active {
                        ColorU::white()
                    } else if mouse.is_clicked() || hovered {
                        p.text
                    } else {
                        mix_color(p.text2, p.text, t)
                    };
                    Container::new(self.hstack(
                        8.,
                        vec![
                            self.ui_icon(*icon, 15., fg),
                            self.line_w(*label, CAPTION, fg, Weight::Medium),
                        ],
                    ))
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(5.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(act.clone()))
                .finish()
            })
            .collect();

        let places_bar = self.between(
            Container::new(self.wrap(place_btns))
                .with_uniform_padding(4.)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            self.button_icon(
                "files-copy-cur-dir",
                "复制当前目录路径",
                UiIcon::Clipboard,
                Action::CopyText(self.directory.clone(), "已复制当前目录路径".into()),
                Btn::Ghost,
            ),
        );

        let category_btns: Vec<El> = [
            ("all", None, "全部"),
            ("image", Some(UiIcon::Image), "图片"),
            ("video", Some(UiIcon::Video), "视频"),
            ("docs", Some(UiIcon::File), "文档"),
            ("audio", Some(UiIcon::Music), "音频"),
            ("apk", Some(UiIcon::Package), "安装包 APK"),
            ("archive", Some(UiIcon::Archive), "压缩包"),
            ("dirs", Some(UiIcon::Folder), "文件夹"),
        ]
        .into_iter()
        .map(|(k, icon, label)| {
            let id = format!("files-cat-filter-{k}");
            let act = Action::Expansion(EA::FilesFilter(k.into()));
            let kind = if self.expansion.files_filter == k {
                Btn::Primary
            } else {
                Btn::Secondary
            };
            match icon {
                Some(ic) => self.button_icon(id, label, ic, act, kind),
                None => self.button(id, label, act, kind),
            }
        })
        .collect();

        let mut sort_btns: Vec<El> = [
            ("name", "按名称"),
            ("time", "按时间"),
            ("size", "按大小"),
            ("kind", "按类型"),
        ]
        .into_iter()
        .map(|(k, label)| {
            self.button(
                format!("files-sort-btn-{k}"),
                label,
                Action::Expansion(EA::FilesSort(k.into())),
                if self.expansion.files_sort == k {
                    Btn::Secondary
                } else {
                    Btn::Ghost
                },
            )
        })
        .collect();
        sort_btns.push(self.button_icon(
            "files-sort-dir-toggle",
            if self.expansion.files_desc {
                "降序"
            } else {
                "升序"
            },
            if self.expansion.files_desc {
                UiIcon::ArrowDown
            } else {
                UiIcon::ArrowUp
            },
            Action::Expansion(EA::FilesToggleDesc),
            Btn::Ghost,
        ));

        let breadcrumb = self.wrap(
            breadcrumbs(&self.directory)
                .into_iter()
                .enumerate()
                .flat_map(|(i, (label, path))| {
                    let mut nodes = Vec::new();
                    if i > 0 {
                        nodes.push(self.ui_icon(UiIcon::ChevronRight, 12., p.text3));
                    }
                    nodes.push(self.button(
                        format!("files-crumb-{i}"),
                        &label,
                        Action::Browse(path.clone()),
                        if path == self.directory {
                            Btn::Secondary
                        } else {
                            Btn::Ghost
                        },
                    ));
                    nodes
                })
                .collect(),
        );
        let navigation = Hoverable::new(self.mouse_state("files-breadcrumb-ctx"), move |_| {
            self.hstack(
                S2,
                vec![
                    if self.directory == "/sdcard" {
                        self.disabled_button("上一级")
                    } else {
                        self.button_icon(
                            "files-up-inline",
                            "上一级",
                            UiIcon::ArrowUp,
                            Action::ParentDirectory,
                            Btn::Secondary,
                        )
                    },
                    breadcrumb,
                ],
            )
        })
        .with_defer_events_to_children()
        .on_right_click(|ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::FilesDirectory,
                at,
            ))
        })
        .finish();
        let top_bar = self.vstack(
            S2,
            vec![
                places_bar,
                self.between(
                    navigation,
                    self.hstack(
                        S2,
                        vec![
                            self.input(F_FILE_SEARCH, "筛选当前目录…", Some(180.)),
                            if !self.fields[F_FILE_SEARCH].text.trim().is_empty() {
                                self.button(
                                    "files-clear-search",
                                    "清空",
                                    Action::ClearField(F_FILE_SEARCH),
                                    Btn::Ghost,
                                )
                            } else {
                                Empty::new().finish()
                            },
                        ],
                    ),
                ),
                self.between(self.wrap(category_btns), self.wrap(sort_btns)),
            ],
        );

        let mut modal_banners: Vec<El> = Vec::new();
        if let Some(modal) = &self.file_modal {
            let title = match modal {
                FileModal::NewFolder => "新建文件夹：",
                FileModal::NewTextFile => "新建文本文档名称（默认 .txt）：",
                FileModal::Rename(_) => "重命名所选项目：",
            };
            modal_banners.push(
                Container::new(self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.line_w(title, CAPTION, p.text, Weight::Semibold),
                            self.input(F_FILE_NAME, "输入新名称…", Some(260.)),
                        ],
                    ),
                    self.hstack(
                        8.,
                        vec![
                            self.button(
                                "file-modal-cancel",
                                "取消",
                                Action::CancelFileModal,
                                Btn::Secondary,
                            ),
                            self.button(
                                "file-modal-ok",
                                "确定",
                                Action::SubmitFileModal,
                                Btn::Primary,
                            ),
                        ],
                    ),
                ))
                .with_horizontal_padding(14.)
                .with_vertical_padding(8.)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.accent))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        }

        if self.trash_confirm {
            modal_banners.push(
                Container::new(self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.dot(p.danger),
                            self.line_w(
                                if self.permanent_delete {
                                    "确认永久删除所选文件？此操作不可撤销。"
                                } else {
                                    "确认将所选文件移至 StepX 安全回收站 (/sdcard/.stepx-trash)？"
                                },
                                CAPTION,
                                p.text,
                                Weight::Medium,
                            ),
                        ],
                    ),
                    self.hstack(
                        8.,
                        vec![
                            self.button(
                                "trash-cancel",
                                "取消",
                                Action::CancelTrash,
                                Btn::Secondary,
                            ),
                            self.button(
                                "trash-confirm",
                                "确认执行",
                                Action::ConfirmTrash,
                                Btn::DangerSolid,
                            ),
                        ],
                    ),
                ))
                .with_horizontal_padding(14.)
                .with_vertical_padding(8.)
                .with_background_color(p.warning_soft)
                .with_border(Border::all(1.).with_border_color(p.warning_border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        } else if let Some(record) = &self.trashed {
            modal_banners.push(
                Container::new(self.between(
                    self.line(
                        format!("已将「{}」移至安全回收站", record.original),
                        CAPTION,
                        p.text,
                    ),
                    self.button_icon(
                        "trash-restore",
                        "撤销并恢复",
                        UiIcon::Refresh,
                        Action::Restore,
                        Btn::Secondary,
                    ),
                ))
                .with_horizontal_padding(14.)
                .with_vertical_padding(8.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        }

        // 2. File Table Container
        let chk_box = |checked: bool| -> El {
            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(if checked {
                            self.ui_icon(UiIcon::Check, 11., p.accent_fg)
                        } else {
                            Empty::new().finish()
                        })
                        .finish(),
                )
                .with_background_color(if checked { p.accent } else { p.bg })
                .with_border(Border::all(1.).with_border_color(if checked {
                    p.accent
                } else {
                    p.border_strong
                }))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                .finish(),
            )
            .with_width(15.)
            .with_height(15.)
            .finish()
        };

        let name_hdr = "名称";
        let date_hdr = "日期 / 目录";
        let kind_hdr = "类型";
        let size_hdr = "大小";

        let table_header = Hoverable::new(self.mouse_state("files-table-hdr-ctx"), move |_| {
            ConstrainedBox::new(
                Container::new(self.file_grid_row(
                    self.line_w(name_hdr, CAPTION, p.text2, Weight::Medium),
                    self.line_w(date_hdr, CAPTION, p.text2, Weight::Medium),
                    self.line_w(kind_hdr, CAPTION, p.text2, Weight::Medium),
                    self.line_w(size_hdr, CAPTION, p.text2, Weight::Medium),
                    self.line_w("操作", CAPTION, p.text2, Weight::Medium),
                ))
                .with_horizontal_padding(S4)
                .with_vertical_padding(8.)
                .with_background_color(if p.dark {
                    rgb(0x20, 0x20, 0x26)
                } else {
                    p.surface
                })
                .with_border(Border::bottom(1.).with_border_color(p.border))
                .finish(),
            )
            .with_height(36.)
            .finish()
        })
        .on_right_click(|ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::FilesDirectory,
                at,
            ))
        })
        .finish();

        let matches_cat = |is_dir: bool, badge: &str, name: &str| -> bool {
            self.matches_file_category(is_dir, badge, name)
        };

        let real_files = self.visible_real_files();

        let received_transfers = self.received_transfers();

        let active_real_idx = self
            .selected_file
            .filter(|&idx| real_files.iter().any(|&(orig_idx, _)| orig_idx == idx))
            .or_else(|| real_files.first().map(|&(orig_idx, _)| orig_idx));

        let mut table_rows: Vec<El> = Vec::new();
        let mut visible_mock_count = 0usize;
        if !self.files.is_empty() {
            for (row_idx, &(orig_idx, entry)) in real_files.iter().enumerate() {
                if row_idx > 0 {
                    table_rows.push(self.divider());
                }
                let selected = active_real_idx == Some(orig_idx);
                let (symbol, kind_label) = file_symbol(entry);
                let icon =
                    self.ui_icon(symbol, 18., if entry.directory { p.blue } else { p.text2 });
                let name_col = self.hstack(
                    12.,
                    vec![
                        chk_box(selected),
                        icon,
                        self.line_w(
                            &entry.name,
                            CAPTION,
                            p.text,
                            if selected {
                                Weight::Semibold
                            } else {
                                Weight::Medium
                            },
                        ),
                    ],
                );
                let mut act_links = vec![self.link(
                    format!("f-real-open-{orig_idx}"),
                    if entry.directory { "进入" } else { "打开" },
                    Action::ActivateFile(orig_idx),
                )];
                if !entry.directory {
                    act_links.push(self.link(
                        format!("f-real-save-{orig_idx}"),
                        "下载",
                        Action::DownloadFile(orig_idx),
                    ));
                }
                act_links.push(self.link_muted(
                    format!("f-real-ren-{orig_idx}"),
                    "重命名",
                    Action::RenameFile(orig_idx),
                ));
                act_links.push(self.link(
                    format!("f-real-del-{orig_idx}"),
                    "删除",
                    Action::TrashFile(orig_idx),
                ));
                let actions_col = self.hstack(8., act_links);
                let size_str = if entry.directory {
                    "--".to_string()
                } else if entry.bytes >= 1_048_576 {
                    format!("{:.1} MB", entry.bytes as f64 / 1_048_576.0)
                } else if entry.bytes >= 1024 {
                    format!("{} KB", entry.bytes / 1024)
                } else {
                    format!("{} B", entry.bytes)
                };
                let date_col = if let Some(date) = entry.date_label() {
                    self.mono(date, CAPTION, p.text2)
                } else {
                    self.mono(&self.directory, CAPTION, p.text2)
                };
                let row_inner = self.file_grid_row(
                    name_col,
                    date_col,
                    self.line(kind_label, CAPTION, p.text2),
                    self.mono(size_str, CAPTION, p.text2),
                    actions_col,
                );
                let menu_path = entry.path.clone();
                let state = self.mouse_state(format!("file-real-row:{orig_idx}"));
                let row_el = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let t = if self.settings.value.reduced_motion {
                        if hovered {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        mouse.hover_progress().clamp(0.0, 1.0)
                    };
                    let selected_bg = if p.dark {
                        rgb(0x25, 0x25, 0x2D)
                    } else {
                        rgb(0xF4, 0xF4, 0xF8)
                    };
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else if selected {
                        mix_color(selected_bg, p.hover, t * 0.5)
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    ConstrainedBox::new(
                        Container::new(row_inner)
                            .with_horizontal_padding(S4)
                            .with_vertical_padding(9.)
                            .with_background_color(bg)
                            .finish(),
                    )
                    .with_height(42.)
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(Action::SelectFile(orig_idx)))
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::ActivateFile(orig_idx))
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::File(menu_path.clone()),
                        at,
                    ))
                })
                .finish();
                table_rows.push(row_el);
            }
        } else if self.demo_mode {
            let query = self.fields[F_FILE_SEARCH].text.trim().to_lowercase();
            let mut mock_items: Vec<(usize, &FileMock)> = FILE_MOCKS
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    let q_ok = query.is_empty()
                        || r.name.to_lowercase().contains(&query)
                        || r.kind.to_lowercase().contains(&query);
                    q_ok && matches_cat(r.is_dir, r.kind, r.name)
                })
                .collect();
            mock_items.sort_by(|(_, a), (_, b)| {
                let dir_cmp = b.is_dir.cmp(&a.is_dir);
                let inner = match self.expansion.files_sort.as_str() {
                    "size" => a.size.cmp(b.size).then_with(|| a.name.cmp(b.name)),
                    "kind" => a.kind.cmp(b.kind).then_with(|| a.name.cmp(b.name)),
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                };
                let inner = if self.expansion.files_desc {
                    inner.reverse()
                } else {
                    inner
                };
                dir_cmp.then(inner)
            });
            visible_mock_count = mock_items.len();
            for (row_idx, (i, r)) in mock_items.into_iter().enumerate() {
                if row_idx > 0 {
                    table_rows.push(self.divider());
                }
                let selected = match self.selected_file {
                    Some(sel) => sel == i,
                    None => r.selected,
                };
                let icon = self.ui_icon(
                    if r.is_dir {
                        UiIcon::Folder
                    } else {
                        UiIcon::File
                    },
                    18.,
                    if r.is_dir { p.blue } else { p.text2 },
                );
                let name_col = self.hstack(
                    12.,
                    vec![
                        chk_box(selected),
                        icon,
                        self.line_w(
                            r.name,
                            CAPTION,
                            p.text,
                            if selected {
                                Weight::Semibold
                            } else {
                                Weight::Medium
                            },
                        ),
                    ],
                );
                let actions_col = self.hstack(
                    8.,
                    vec![
                        self.link(
                            format!("f-act-open-{i}"),
                            if r.is_dir { "进入" } else { "打开" },
                            Action::ActivateFile(i),
                        ),
                        self.link_muted(format!("f-act-ren-{i}"), "重命名", Action::OpenRename),
                        self.link(format!("f-act-del-{i}"), "删除", Action::Trash),
                    ],
                );
                let row_inner = self.file_grid_row(
                    name_col,
                    self.mono(&self.directory, CAPTION, p.text2),
                    self.line(r.kind, CAPTION, p.text2),
                    self.mono(r.size, CAPTION, p.text2),
                    actions_col,
                );
                let mock_path = format!("{}/{}", self.directory.trim_end_matches('/'), r.name);
                let dbl_action = if r.is_dir {
                    Action::Browse(mock_path.clone())
                } else {
                    Action::ActivateFile(i)
                };
                let state = self.mouse_state(format!("file-row:{i}"));
                let row_el = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let t = if self.settings.value.reduced_motion {
                        if hovered {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        mouse.hover_progress().clamp(0.0, 1.0)
                    };
                    let selected_bg = if p.dark {
                        rgb(0x25, 0x25, 0x2D)
                    } else {
                        rgb(0xF4, 0xF4, 0xF8)
                    };
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else if selected {
                        mix_color(selected_bg, p.hover, t * 0.5)
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    ConstrainedBox::new(
                        Container::new(row_inner)
                            .with_horizontal_padding(S4)
                            .with_vertical_padding(10.)
                            .with_background_color(bg)
                            .finish(),
                    )
                    .with_height(44.)
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(Action::SelectFile(i)))
                .on_double_click(move |ctx, _, _| ctx.dispatch_typed_action(dbl_action.clone()))
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::File(mock_path.clone()),
                        at,
                    ))
                })
                .finish();
                table_rows.push(row_el);
            }
        }

        if table_rows.is_empty() {
            table_rows.push(
                Hoverable::new(self.mouse_state("files-empty-dir-area"), move |_| {
                    Container::new(self.vstack(
                        S1,
                        vec![
                            self.text("当前目录没有符合筛选条件的文件", BODY, p.text2),
                            self.line(
                                "提示：可在此处点击鼠标右键，快速新建文件夹、新建文本文档或上传电脑文件",
                                MICRO,
                                p.text3,
                            ),
                        ],
                    ))
                    .with_uniform_padding(S4)
                    .finish()
                })
                .on_right_click(|ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::FilesDirectory,
                        at,
                    ))
                })
                .finish(),
            );
        }
        let total_items = if self.files.is_empty() && self.demo_mode {
            visible_mock_count
        } else {
            real_files.len()
        };

        let selected_entry: Option<stepx_companion::storage::Entry> = self
            .selected_file
            .filter(|&idx| real_files.iter().any(|&(orig_idx, _)| orig_idx == idx))
            .and_then(|idx| self.files.get(idx).cloned())
            .or_else(|| {
                if !real_files.is_empty() {
                    real_files.first().map(|&(_, e)| e.clone())
                } else if self.demo_mode {
                    let idx = self
                        .selected_file
                        .unwrap_or(2)
                        .min(FILE_MOCKS.len().saturating_sub(1));
                    let m = &FILE_MOCKS[idx];
                    Some(stepx_companion::storage::Entry {
                        name: m.name.to_string(),
                        path: format!("{}/{}", self.directory.trim_end_matches('/'), m.name),
                        bytes: 2_516_582,
                        directory: m.is_dir,
                        symlink: false,
                    })
                } else {
                    None
                }
            });

        let has_sel = selected_entry.is_some();
        let sel_is_file = selected_entry.as_ref().is_some_and(|e| !e.directory);
        let mut footer_left = vec![self.line(
            format!("{total_items} 项 · 单击选择，双击打开"),
            CAPTION,
            p.text2,
        )];
        if let Some(sel) = selected_entry.as_ref() {
            footer_left.push(self.line("·", CAPTION, p.text2));
            footer_left.push(self.line_w(
                format!("已选：{}", sel.name),
                CAPTION,
                p.text,
                Weight::Medium,
            ));
        }

        let table_footer = Hoverable::new(self.mouse_state("files-table-ftr-ctx"), move |_| {
            Container::new(self.between(
                self.hstack(10., footer_left),
                self.hstack(
                    8.,
                    vec![
                        if has_sel {
                            self.button(
                                "files-rename-sel",
                                "重命名",
                                Action::OpenRename,
                                Btn::Secondary,
                            )
                        } else {
                            self.disabled_button("重命名")
                        },
                        if sel_is_file {
                            self.button(
                                "files-dl-sel",
                                "下载所选",
                                Action::Download,
                                Btn::Secondary,
                            )
                        } else {
                            self.disabled_button("下载所选")
                        },
                        if has_sel {
                            self.button("files-trash-sel", "移至回收站", Action::Trash, Btn::Danger)
                        } else {
                            self.disabled_button("移至回收站")
                        },
                    ],
                ),
            ))
            .with_horizontal_padding(S4)
            .with_vertical_padding(8.)
            .with_background_color(if p.dark {
                rgb(0x1C, 0x1C, 0x21)
            } else {
                p.workspace
            })
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish()
        })
        .with_defer_events_to_children()
        .on_right_click(|ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::FilesDirectory,
                at,
            ))
        })
        .finish();

        let rows_scroll = self.pane_scroll(
            self.scroll.clone(),
            Container::new(self.vstack(0., table_rows))
                .with_vertical_padding(S1)
                .finish(),
        );

        let table_card = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(table_header)
                .with_child(Expanded::new(1., rows_scroll).finish())
                .with_child(table_footer)
                .finish(),
        )
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut received_cards: Vec<El> = Vec::new();
        for (idx, (r_name, r_path, r_bytes)) in received_transfers.iter().enumerate() {
            let lower = r_name.to_ascii_lowercase();
            let is_img = lower.ends_with(".png")
                || lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".webp")
                || lower.ends_with(".gif");
            let path_str = r_path.to_string_lossy().into_owned();
            let r_is_file = cached_is_file(r_path);
            let mut card_rows = vec![self.between(
                self.line_w(r_name, CAPTION, p.text, Weight::Semibold),
                self.mono(
                    stepx_companion::storage::format_bytes(*r_bytes),
                    MICRO,
                    p.text2,
                ),
            )];
            if is_img {
                let thumb = if r_is_file {
                    ConstrainedBox::new(
                        Image::new(
                            AssetSource::LocalFile {
                                path: path_str.clone(),
                            },
                            CacheOption::BySize,
                        )
                        .contain()
                        .finish(),
                    )
                    .with_max_width(180.)
                    .with_height(86.)
                    .finish()
                } else {
                    let bundled = if r_name.contains("110522") {
                        "neo_front_frame.png"
                    } else {
                        "stepx_home_screen.png"
                    };
                    ConstrainedBox::new(
                        Image::new(AssetSource::Bundled { path: bundled }, CacheOption::BySize)
                            .contain()
                            .finish(),
                    )
                    .with_max_width(180.)
                    .with_height(86.)
                    .finish()
                };
                card_rows.push(
                    Container::new(thumb)
                        .with_uniform_padding(4.)
                        .with_background_color(p.bg)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                );
            }
            let btns = vec![
                self.button(
                    format!("rx-open-{idx}"),
                    "打开",
                    Action::Studio(StudioAction::OpenPath(r_path.clone())),
                    Btn::Secondary,
                ),
                self.button(
                    format!("rx-reveal-{idx}"),
                    "在文件夹中显示",
                    Action::RevealLocalPath(r_path.clone()),
                    Btn::Secondary,
                ),
                self.button(
                    format!("rx-copy-{idx}"),
                    "复制路径",
                    Action::CopyText(path_str.clone(), "已复制电脑接收文件路径".into()),
                    Btn::Ghost,
                ),
            ];
            card_rows.push(self.wrap(btns));
            let dbl_path = r_path.clone();
            let menu_path = r_path.clone();
            let state = self.mouse_state(format!("rx-card:{idx}"));
            let card_inner = self.vstack(6., card_rows);
            received_cards.push(
                Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let border_col = if mouse.is_clicked() || hovered {
                        p.accent
                    } else {
                        p.border
                    };
                    Container::new(card_inner)
                        .with_uniform_padding(S2)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(border_col))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_defer_events_to_children()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Studio(StudioAction::OpenPath(
                        dbl_path.clone(),
                    )))
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::LocalFile(menu_path.clone()),
                        at,
                    ))
                })
                .finish(),
            );
        }

        let detail_panel = if let Some(entry) = selected_entry.as_ref() {
            let (symbol, kind_label) = file_symbol(entry);
            let size_label = if entry.directory {
                "文件夹目录".to_string()
            } else {
                stepx_companion::storage::format_bytes(entry.bytes)
            };
            let lower_sel = entry.name.to_ascii_lowercase();
            let sel_is_image = !entry.directory
                && (lower_sel.ends_with(".png")
                    || lower_sel.ends_with(".jpg")
                    || lower_sel.ends_with(".jpeg")
                    || lower_sel.ends_with(".webp")
                    || lower_sel.ends_with(".gif"));
            let matched_local_rx = received_transfers
                .iter()
                .find(|(n, _, _)| n == &entry.name)
                .map(|(_, p, _)| p.to_string_lossy().into_owned())
                .or_else(|| {
                    if sel_is_image && self.demo_mode {
                        Some("desktop/assets/stepx_home_screen.png".into())
                    } else {
                        None
                    }
                });

            let mut cross_actions: Vec<El> = Vec::new();
            if !entry.directory {
                cross_actions.push(self.button(
                    "file-detail-open-sys",
                    "用系统程序打开",
                    Action::ActivateSelectedFile,
                    Btn::Primary,
                ));
                cross_actions.push(self.button_icon(
                    "file-detail-download",
                    "下载到电脑",
                    UiIcon::Download,
                    Action::Download,
                    Btn::Secondary,
                ));
                if let Some(preview_target) = &matched_local_rx {
                    cross_actions.push(self.button(
                        "file-detail-reveal-local",
                        "在文件夹中显示",
                        Action::RevealLocalPath(std::path::PathBuf::from(preview_target)),
                        Btn::Secondary,
                    ));
                }
                if matches!(entry.kind_badge(), "图片" | "视频") {
                    cross_actions.push(self.button(
                        "file-detail-push-bs",
                        "推送到背屏创作",
                        Action::FilePushToBackScreen(entry.path.clone()),
                        Btn::Secondary,
                    ));
                }
                cross_actions.push(self.button(
                    "file-detail-to-amoo",
                    "去 Amoo 分析",
                    Action::Expansion(EA::AnalyzeWithAmoo(entry.path.clone())),
                    Btn::Secondary,
                ));
            } else {
                cross_actions.push(self.button(
                    "file-detail-open-dir",
                    "进入此文件夹",
                    Action::Browse(entry.path.clone()),
                    Btn::Primary,
                ));
            }

            let mut panel_items = vec![
                self.between(
                    self.heading("所选文件详情"),
                    self.pill_badge(kind_label, p.text2, p.tag_bg),
                ),
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(symbol, 22., if entry.directory { p.blue } else { p.text }),
                        Expanded::new(
                            1.,
                            self.vstack(
                                2.,
                                vec![
                                    self.line_w(&entry.name, BODY, p.text, Weight::Semibold),
                                    self.mono(size_label, MICRO, p.text2),
                                ],
                            ),
                        )
                        .finish(),
                    ],
                ),
                self.divider(),
                self.vstack(
                    4.,
                    vec![
                        self.between(
                            self.line("手机存储路径", MICRO, p.text3),
                            self.link_icon(
                                "file-detail-inline-copy",
                                "复制路径",
                                UiIcon::Clipboard,
                                Action::CopyText(entry.path.clone(), "已复制手机文件路径".into()),
                            ),
                        ),
                        self.mono(&entry.path, CAPTION, p.text2),
                    ],
                ),
                self.divider(),
                self.line_w("快捷操作与跨页流转", CAPTION, p.text, Weight::Medium),
                self.wrap(cross_actions),
            ];

            if !received_cards.is_empty() {
                panel_items.push(self.divider());
                panel_items.push(self.between(
                    self.line_w("电脑已接收 / 下载文件", CAPTION, p.text, Weight::Semibold),
                    self.pill_badge(format!("{} 项", received_cards.len()), p.text2, p.hover),
                ));
                panel_items.extend(received_cards);
            }

            self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(self.panel(panel_items))
                    .with_padding_bottom(S3)
                    .finish(),
            )
        } else {
            let mut empty_items = vec![
                self.heading("所选文件详情"),
                self.text(
                    "单击左侧列表中的任意文件，即可用系统程序打开、复制手机路径、推送到背屏点阵创作，或交由 Amoo 分析。",
                    CAPTION,
                    p.text2,
                ),
            ];
            if !received_cards.is_empty() {
                empty_items.push(self.divider());
                empty_items.push(self.line_w(
                    "电脑已接收 / 下载文件",
                    CAPTION,
                    p.text,
                    Weight::Semibold,
                ));
                empty_items.extend(received_cards);
            }
            self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(self.panel(empty_items))
                    .with_padding_bottom(S3)
                    .finish(),
            )
        };

        let main_split = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S3)
            .with_child(Expanded::new(1., table_card).finish())
            .with_child(self.fixed_w(288., detail_panel))
            .finish();

        let mut page_children = vec![self.storage_mode_bar(), top_bar];
        page_children.extend(modal_banners);
        page_children.push(Expanded::new(1., self.sub_enter(main_split)).finish());

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S3)
            .with_children(page_children)
            .finish()
    }

    pub(in super::super) fn matches_file_category(
        &self,
        is_dir: bool,
        badge: &str,
        name: &str,
    ) -> bool {
        let has_ext = |lower: &str, exts: &[&str]| -> bool {
            exts.iter().any(|ext| lower.ends_with(&format!(".{ext}")))
        };
        let lower = name.to_ascii_lowercase();
        match self.expansion.files_filter.as_str() {
            "dirs" => is_dir,
            "image" => {
                !is_dir
                    && (badge == "图片"
                        || badge.contains("图像")
                        || has_ext(
                            &lower,
                            &["jpg", "jpeg", "png", "webp", "gif", "heic", "bmp", "dng"],
                        ))
            }
            "video" => {
                !is_dir
                    && (badge == "视频"
                        || badge.contains("视频")
                        || has_ext(&lower, &["mp4", "mkv", "mov", "avi", "webm", "3gp", "flv"]))
            }
            "media" => {
                !is_dir
                    && (matches!(badge, "图片" | "视频" | "音频")
                        || badge.contains("图像")
                        || badge.contains("视频")
                        || has_ext(
                            &lower,
                            &[
                                "png", "jpg", "jpeg", "webp", "gif", "heic", "mp4", "mkv", "mov",
                            ],
                        ))
            }
            "docs" => {
                !is_dir
                    && (badge == "文档"
                        || badge.contains("文档")
                        || badge.contains("文本")
                        || has_ext(
                            &lower,
                            &[
                                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md",
                                "csv", "json", "xml", "html",
                            ],
                        ))
            }
            "audio" => {
                !is_dir
                    && (badge == "音频"
                        || badge.contains("音频")
                        || has_ext(
                            &lower,
                            &["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "amr"],
                        ))
            }
            "apk" => !is_dir && (badge == "安装包" || has_ext(&lower, &["apk", "xapk", "apks"])),
            "archive" => {
                !is_dir
                    && (badge == "压缩包"
                        || has_ext(&lower, &["zip", "rar", "7z", "tar", "gz", "bz2", "xz"]))
            }
            _ => true,
        }
    }

    pub(in super::super) fn visible_real_files(
        &self,
    ) -> Vec<(usize, &stepx_companion::storage::Entry)> {
        let in_trash =
            self.directory == "/sdcard/.stepx-trash" || self.directory == "/sdcard/.StepXTrash";
        let mut real_files: Vec<(usize, &stepx_companion::storage::Entry)> = self
            .filtered_files()
            .into_iter()
            .filter(|(_, e)| {
                !is_hidden_phone_file_entry(&e.name, in_trash)
                    && self.matches_file_category(e.directory, e.kind_badge(), &e.name)
            })
            .collect();
        real_files.sort_by(|(_, a), (_, b)| {
            let dir_cmp = b.directory.cmp(&a.directory);
            let folder_rank = |entry: &stepx_companion::storage::Entry| -> u8 {
                if !entry.directory {
                    return 99;
                }
                match entry.name.as_str() {
                    "DCIM" => 0,
                    "Pictures" => 1,
                    "Download" => 2,
                    "Documents" => 3,
                    "Movies" => 4,
                    "Music" => 5,
                    "Recordings" => 6,
                    _ => 50,
                }
            };
            let inner = match self.expansion.files_sort.as_str() {
                "time" => a
                    .sort_time_key()
                    .cmp(&b.sort_time_key())
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                "size" => a
                    .bytes
                    .cmp(&b.bytes)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                "kind" => a
                    .kind_badge()
                    .cmp(b.kind_badge())
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                _ => folder_rank(a)
                    .cmp(&folder_rank(b))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            };
            let inner = if self.expansion.files_desc {
                inner.reverse()
            } else {
                inner
            };
            dir_cmp.then(inner)
        });
        real_files
    }
}

pub(in super::super) fn is_hidden_phone_file_entry(name: &str, in_trash: bool) -> bool {
    if in_trash {
        return false;
    }
    name.starts_with('.')
        || name.ends_with(".stepxlink")
        || name == "StepX Companion"
        || stepx_companion::toolbox::is_expansion_stub_filename(name)
        || (name.starts_with("SC_") && name.len() >= 36)
}

/// Stable cumulative paths, with a human-readable root. No substring replacement.
fn breadcrumbs(path: &str) -> Vec<(String, String)> {
    let mut result = vec![("手机存储".into(), "/sdcard".into())];
    let mut current = String::from("/sdcard");
    for part in path
        .strip_prefix("/sdcard/")
        .unwrap_or("")
        .split('/')
        .filter(|p| !p.is_empty())
    {
        current.push('/');
        current.push_str(part);
        result.push((part.to_owned(), current.clone()));
    }
    result
}
#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[test]
    fn breadcrumbs_keep_unicode_and_repeated_names() {
        assert_eq!(breadcrumbs("/sdcard").len(), 1);
        assert_eq!(
            breadcrumbs("/sdcard/DCIM/旅行/DCIM").last().unwrap(),
            &("DCIM".into(), "/sdcard/DCIM/旅行/DCIM".into())
        );
    }
}

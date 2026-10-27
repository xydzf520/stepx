//! Visual Phone Gallery & Screenshot Hub (`Page::Gallery`).
//! Reads real photos and screenshots from `/sdcard/DCIM/Camera`,
//! `/sdcard/Pictures/Screenshots`, `/sdcard/DCIM/Screenshots`, and `/sdcard/Pictures`,
//! pulls local thumbnail previews, and supports one-click desktop save, preview,
//! Amoo vision analysis, phone screenshot capture, desktop-to-phone import, and deletion.

use super::super::{
    backscreen_state::BackScreenAction, icons::UiIcon, theme::*, widgets::*, Action, Companion,
    F_GALLERY_SEARCH,
};
use facloud_ui::{
    assets::asset_cache::AssetSource, color::ColorU, elements::*, fonts::Weight, platform::Cursor,
};
use std::path::Path;
use stepx_companion::toolbox::PhoneGalleryItem;

fn gallery_image_dimensions(item: &PhoneGalleryItem) -> Option<(u32, u32)> {
    let path = item.local_thumb_path.as_deref()?;
    cached_image_dimensions(Path::new(path))
}

impl Companion {
    pub(super) fn gallery_actions(&self) -> Vec<El> {
        vec![
            self.button_icon(
                "gallery-hdr-shot",
                "截取手机屏幕入库",
                UiIcon::Camera,
                Action::BackScreen(BackScreenAction::GalleryCapturePhoneScreenshot),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-hdr-import",
                "导入电脑图片到手机",
                UiIcon::Upload,
                Action::BackScreen(BackScreenAction::GalleryImportFromDesktop),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-hdr-export-filtered",
                "导出当前筛选",
                UiIcon::Package,
                Action::BackScreen(BackScreenAction::GalleryExportFiltered),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-hdr-open-export-dir",
                "打开电脑相册目录",
                UiIcon::Folder,
                Action::BackScreen(BackScreenAction::GalleryOpenExportFolder),
                Btn::Secondary,
            ),
            if self.back_screen.gallery_loading {
                self.disabled_button("正在读取相册…")
            } else {
                self.button_icon(
                    "gallery-hdr-refresh",
                    "刷新相册",
                    UiIcon::Refresh,
                    Action::BackScreen(BackScreenAction::GalleryRefresh),
                    Btn::Primary,
                )
            },
        ]
    }

    fn gallery_thumb_box(&self, item: &PhoneGalleryItem, height: f32) -> El {
        let p = self.p();
        let has_thumb = item
            .local_thumb_path
            .as_deref()
            .is_some_and(|path| cached_is_file(Path::new(path)));
        let inner = if let Some(path) = item.local_thumb_path.as_deref().filter(|_| has_thumb) {
            Image::new(
                AssetSource::LocalFile {
                    path: path.to_string(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish()
        } else {
            Align::new(self.vstack(
                S1,
                vec![
                    Align::new(self.ui_icon(UiIcon::Image, 24., p.text3)).finish(),
                    Align::new(self.line("点击加载预览", MICRO, p.text3)).finish(),
                ],
            ))
            .finish()
        };

        ConstrainedBox::new(
            Container::new(inner)
                .with_uniform_padding(S2)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        )
        .with_max_width(height * 1.6)
        .with_height(height)
        .finish()
    }

    fn gallery_interactive_preview_box(&self, id_prefix: &str, item: &PhoneGalleryItem) -> El {
        let p = self.p();
        let rot_deg = self
            .back_screen
            .gallery_rotations
            .get(&item.remote_path)
            .copied()
            .unwrap_or(0);
        let dims = gallery_image_dimensions(item);
        let has_thumb = item
            .local_thumb_path
            .as_deref()
            .is_some_and(|path| cached_is_file(Path::new(path)));
        let image_el = if let Some(path) = item.local_thumb_path.as_deref().filter(|_| has_thumb) {
            Image::new(
                AssetSource::LocalFile {
                    path: path.to_string(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish()
        } else {
            Align::new(
                self.vstack(
                    S2,
                    vec![
                        Align::new(self.ui_icon(UiIcon::Image, 28., p.text3)).finish(),
                        Align::new(self.line(
                            "正在从手机拉取高清预览图，或点击画面重试…",
                            BODY,
                            p.text2,
                        ))
                        .finish(),
                    ],
                ),
            )
            .finish()
        };

        let mut badges = Vec::new();
        if let Some((w, h)) = dims {
            badges.push(self.mono(format!("分辨率: {w} × {h} px"), MICRO, p.accent));
        } else {
            badges.push(self.line("分辨率: 待加载预览", MICRO, p.text3));
        }
        if rot_deg > 0 {
            badges.push(self.pill_badge(format!("↻ {rot_deg}°"), p.accent, p.accent_soft));
        }

        let inner_box = self.vstack(
            S2,
            vec![
                Container::new(ConstrainedBox::new(image_el).with_height(220.0).finish())
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                self.between(
                    self.hstack(
                        6.,
                        vec![
                            self.ui_icon(UiIcon::Lightbulb, 13., p.text3),
                            self.line_w(
                                "双击大图全屏沉浸看图 · 右键支持旋转 / 设为背屏 / 发给 Amoo 分析",
                                MICRO,
                                p.text3,
                                Weight::Normal,
                            ),
                        ],
                    ),
                    self.hstack(S2, badges),
                ),
            ],
        );

        let sel_path = item.remote_path.clone();
        let sel_idx = self.back_screen.gallery_selected;

        Hoverable::new(
            self.mouse_state(format!("{id_prefix}-preview-click")),
            move |mouse| {
                let hovered = mouse.is_mouse_over_element();
                let border_color = if mouse.is_clicked() || hovered {
                    p.accent
                } else {
                    p.border
                };
                Container::new(inner_box)
                    .with_uniform_padding(S2)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            if !has_thumb {
                ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::GallerySelect(
                    sel_idx,
                )));
            }
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::GalleryToggleLightbox))
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::GalleryItem {
                    idx: sel_idx,
                    remote_path: sel_path.clone(),
                },
                at,
            ))
        })
        .finish()
    }

    fn gallery_lightbox_overlay(
        &self,
        sel_idx: usize,
        item: &PhoneGalleryItem,
        pos_label: &str,
    ) -> El {
        let p = self.p();
        let zoom_pct = self.back_screen.gallery_zoom_pct.clamp(50, 400);
        let rot_deg = self
            .back_screen
            .gallery_rotations
            .get(&item.remote_path)
            .copied()
            .unwrap_or(0);
        let dims = gallery_image_dimensions(item);
        let has_thumb = item
            .local_thumb_path
            .as_deref()
            .is_some_and(|path| cached_is_file(Path::new(path)));
        let canvas_height = (360.0 * (zoom_pct as f32 / 100.0)).clamp(220.0, 560.0);

        let image_el = if let Some(path) = item.local_thumb_path.as_deref().filter(|_| has_thumb) {
            Image::new(
                AssetSource::LocalFile {
                    path: path.to_string(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish()
        } else {
            Align::new(self.vstack(
                S2,
                vec![
                    Align::new(self.ui_icon(UiIcon::Image, 32., p.text3)).finish(),
                    Align::new(self.line("正在从手机拉取高清大图预览…", BODY, p.text2)).finish(),
                ],
            ))
            .finish()
        };

        let sel_path = item.remote_path.clone();
        let interactive_photo = Hoverable::new(
            self.mouse_state(format!("gallery-lightbox-photo-{sel_idx}")),
            move |mouse| {
                let hovered = mouse.is_mouse_over_element();
                let border_color = if mouse.is_clicked() || hovered {
                    p.accent
                } else {
                    p.border
                };
                Container::new(
                    ConstrainedBox::new(image_el)
                        .with_height(canvas_height)
                        .finish(),
                )
                .with_uniform_padding(S3)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::GalleryToggleLightbox))
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::GalleryItem {
                    idx: sel_idx,
                    remote_path: sel_path.clone(),
                },
                at,
            ))
        })
        .finish();

        let mut meta_badges = vec![
            self.pill_badge(
                if item.category == "expansion" {
                    "电脑扩容 · 原图直读"
                } else {
                    item.category_label()
                },
                p.accent,
                p.accent_soft,
            ),
            self.pill_badge(format!("缩放 {zoom_pct}%"), p.text2, p.surface),
        ];
        if let Some((w, h)) = dims {
            meta_badges.push(self.mono(format!("{w} × {h} px"), MICRO, p.text2));
        }
        if rot_deg > 0 {
            meta_badges.push(self.pill_badge(format!("↻ {rot_deg}°"), p.accent, p.accent_soft));
        }

        let controls = self.wrap(vec![
            self.button_icon(
                "gallery-lb-prev",
                "上一张",
                UiIcon::ArrowLeft,
                Action::BackScreen(BackScreenAction::GalleryPrevPhoto),
                Btn::Secondary,
            ),
            self.pill_badge(pos_label, p.text2, p.surface),
            self.button_icon(
                "gallery-lb-next",
                "下一张",
                UiIcon::ArrowRight,
                Action::BackScreen(BackScreenAction::GalleryNextPhoto),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-lb-zoom-out",
                "缩小",
                UiIcon::Minus,
                Action::BackScreen(BackScreenAction::GalleryZoomOut),
                Btn::Secondary,
            ),
            self.button(
                "gallery-lb-zoom-100",
                "100%",
                Action::BackScreen(BackScreenAction::GalleryZoomSet(100)),
                if zoom_pct == 100 {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.button(
                "gallery-lb-zoom-200",
                "200%",
                Action::BackScreen(BackScreenAction::GalleryZoomSet(200)),
                if zoom_pct == 200 {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.button_icon(
                "gallery-lb-zoom-in",
                "放大",
                UiIcon::Plus,
                Action::BackScreen(BackScreenAction::GalleryZoomIn),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-lb-rot",
                "旋转 90°",
                UiIcon::Refresh,
                Action::BackScreen(BackScreenAction::GalleryRotateRight(
                    item.remote_path.clone(),
                )),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-lb-open-os",
                "系统看图打开",
                UiIcon::ExternalLink,
                Action::BackScreen(BackScreenAction::GalleryOpenOnDesktop(
                    item.remote_path.clone(),
                )),
                Btn::Secondary,
            ),
            self.button_icon(
                "gallery-lb-amoo",
                "发给 Amoo 分析",
                UiIcon::Star,
                Action::BackScreen(BackScreenAction::GallerySendToAmoo(
                    item.remote_path.clone(),
                )),
                Btn::Secondary,
            ),
            self.button(
                "gallery-lb-backscreen",
                "设为背屏像素",
                Action::FilePushToBackScreen(item.remote_path.clone()),
                Btn::Secondary,
            ),
            self.button(
                "gallery-lb-close",
                "收起沉浸大图",
                Action::BackScreen(BackScreenAction::GalleryToggleLightbox),
                Btn::Primary,
            ),
        ]);

        self.panel(vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.heading(format!("沉浸大图检视 · {}", item.name)),
                        self.hstack(S1, meta_badges),
                    ],
                ),
                self.line(
                    "双击画面退出沉浸模式 · 右键画面打开照片快捷菜单",
                    CAPTION,
                    p.text3,
                ),
            ),
            controls,
            interactive_photo,
        ])
    }

    fn gallery_zoom_nav_bar(
        &self,
        id_prefix: &str,
        item: &PhoneGalleryItem,
        pos_label: &str,
    ) -> El {
        let p = self.p();
        let rot_deg = self
            .back_screen
            .gallery_rotations
            .get(&item.remote_path)
            .copied()
            .unwrap_or(0);

        let mut nav_controls = vec![
            self.button_icon(
                format!("{id_prefix}-prev"),
                "上一张",
                UiIcon::ArrowLeft,
                Action::BackScreen(BackScreenAction::GalleryPrevPhoto),
                Btn::Secondary,
            ),
            self.pill_badge(pos_label, p.text2, p.surface),
            self.button_icon(
                format!("{id_prefix}-next"),
                "下一张",
                UiIcon::ArrowRight,
                Action::BackScreen(BackScreenAction::GalleryNextPhoto),
                Btn::Secondary,
            ),
            self.button(
                format!("{id_prefix}-lightbox"),
                if self.back_screen.gallery_lightbox {
                    "收起沉浸大图"
                } else {
                    "沉浸大图"
                },
                Action::BackScreen(BackScreenAction::GalleryToggleLightbox),
                if self.back_screen.gallery_lightbox {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.button_icon(
                format!("{id_prefix}-rot"),
                if rot_deg > 0 {
                    format!("旋转 90° ({rot_deg}°)")
                } else {
                    "旋转 90°".into()
                },
                UiIcon::Refresh,
                Action::BackScreen(BackScreenAction::GalleryRotateRight(
                    item.remote_path.clone(),
                )),
                Btn::Secondary,
            ),
        ];
        if rot_deg > 0 {
            nav_controls.push(self.button_icon(
                format!("{id_prefix}-rot-save"),
                "保存旋转后图片到电脑",
                UiIcon::Download,
                Action::BackScreen(BackScreenAction::GallerySaveToDesktop(
                    item.remote_path.clone(),
                )),
                Btn::Primary,
            ));
        }

        Container::new(self.wrap(nav_controls))
            .with_uniform_padding(S2)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
    }

    fn gallery_grid_card(&self, orig_idx: usize, item: &PhoneGalleryItem) -> El {
        let p = self.p();
        let selected = orig_idx == self.back_screen.gallery_selected;
        let cols = self.back_screen.gallery_grid_cols.clamp(2, 4);
        let thumb_height = match cols {
            2 => 192.,
            4 => 104.,
            _ => 136.,
        };
        let max_title_chars = match cols {
            2 => 26,
            4 => 14,
            _ => 18,
        };
        let thumb = self.gallery_thumb_box(item, thumb_height);
        let title = Self::truncate_chars(&item.name, max_title_chars);
        let (cat_fg, cat_bg) = match item.category.as_str() {
            "camera" => (p.accent, p.bg),
            "screenshot" | "screenshots" => (p.success, p.bg),
            "expansion" => (p.accent, p.accent_soft),
            _ => (p.text2, p.bg),
        };
        let rot_deg = self
            .back_screen
            .gallery_rotations
            .get(&item.remote_path)
            .copied()
            .unwrap_or(0);
        let meta_row = self.between(
            self.hstack(
                S1,
                vec![
                    self.pill_badge(item.category_label(), cat_fg, cat_bg),
                    if rot_deg > 0 {
                        self.pill_badge(format!("↻ {rot_deg}°"), p.accent, p.accent_soft)
                    } else {
                        Empty::new().finish()
                    },
                ],
            ),
            self.mono(item.size_label(), MICRO, p.text3),
        );
        let res_label = if let Some((w, h)) = gallery_image_dimensions(item) {
            if cols >= 4 {
                format!("{w}×{h} · {}", item.time_label())
            } else {
                format!("{w} × {h} px · {}", item.full_datetime_label())
            }
        } else if cols >= 4 {
            item.time_label()
        } else {
            item.full_datetime_label()
        };
        let body = self.vstack(
            S2,
            vec![
                thumb,
                self.line_w(title, BODY, p.text, Weight::Medium),
                meta_row,
                self.line(res_label, MICRO, p.text3),
            ],
        );

        let dbl_path = item.remote_path.clone();
        let menu_path = item.remote_path.clone();

        Hoverable::new(
            self.mouse_state(format!("gallery-card-{orig_idx}")),
            move |mouse| {
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
                let blend = |c1: ColorU, c2: ColorU| {
                    ColorU::new(
                        mix(c1.r, c2.r),
                        mix(c1.g, c2.g),
                        mix(c1.b, c2.b),
                        mix(c1.a, c2.a),
                    )
                };
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else if selected {
                    p.selected
                } else {
                    blend(p.bg, p.hover)
                };
                let border_color = if mouse.is_clicked() || selected {
                    p.accent
                } else {
                    blend(p.border, p.border_strong)
                };
                Container::new(body)
                    .with_uniform_padding(S3)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::GallerySelect(
                orig_idx,
            )))
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::GalleryOpenOnDesktop(
                dbl_path.clone(),
            )))
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::GalleryItem {
                    idx: orig_idx,
                    remote_path: menu_path.clone(),
                },
                at,
            ))
        })
        .finish()
    }

    fn gallery_inspector_kv(&self, label: &str, value: &str, mono_value: bool) -> El {
        let p = self.p();
        Container::new(self.vstack(
            2.,
            vec![
                self.line(label, MICRO, p.text3),
                if mono_value {
                    self.mono(value, CAPTION, p.text2)
                } else {
                    self.text_w(value, BODY, p.text, Weight::Medium)
                },
            ],
        ))
        .with_uniform_padding(S2)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish()
    }

    pub(in super::super) fn gallery_page(&self) -> El {
        let p = self.p();
        let items = &self.back_screen.gallery_items;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let is_recent = |idx: usize, item: &PhoneGalleryItem| -> bool {
            (item.modified_ms > 0 && now_ms - item.modified_ms <= 7 * 86_400_000) || idx < 8
        };
        let visible_items = items.iter().filter(|i| !i.name.starts_with('.'));
        let total_count = visible_items.clone().count();
        let camera_count = visible_items
            .clone()
            .filter(|i| i.category == "camera")
            .count();
        let screenshot_count = visible_items
            .clone()
            .filter(|i| i.category == "screenshot" || i.category == "screenshots")
            .count();
        let weixin_count = visible_items
            .clone()
            .filter(|i| i.category == "pictures" || i.category == "weixin")
            .count();
        let recent_count = items
            .iter()
            .enumerate()
            .filter(|(idx, i)| !i.name.starts_with('.') && is_recent(*idx, i))
            .count();
        let expansion_count = visible_items
            .filter(|i| i.category == "expansion")
            .count();
        let active_cat = self.back_screen.gallery_category;
        let grid_cols = self.back_screen.gallery_grid_cols.clamp(2, 4);
        let grid_cols_usize = grid_cols as usize;

        let query = self.fields[F_GALLERY_SEARCH].text.trim().to_lowercase();
        let filtered: Vec<(usize, &PhoneGalleryItem)> = items
            .iter()
            .enumerate()
            .filter(|(idx, item)| {
                if item.name.starts_with('.') {
                    return false;
                }
                let cat_ok = match active_cat {
                    1 => item.category == "camera",
                    2 => item.category == "screenshot" || item.category == "screenshots",
                    3 => item.category == "pictures" || item.category == "weixin",
                    4 => is_recent(*idx, item),
                    5 => item.category == "expansion",
                    _ => true,
                };
                let search_ok = query.is_empty()
                    || item.name.to_lowercase().contains(&query)
                    || item.remote_path.to_lowercase().contains(&query)
                    || item.category_label().to_lowercase().contains(&query)
                    || item.time_label().to_lowercase().contains(&query)
                    || item.full_datetime_label().to_lowercase().contains(&query);
                cat_ok && search_ok
            })
            .collect();

        let top_bar = self.panel(vec![
            self.between(
                self.wrap(vec![
                    self.button(
                        "gallery-cat-0",
                        format!("全部照片 ({total_count})"),
                        Action::BackScreen(BackScreenAction::GalleryCategory(0)),
                        if active_cat == 0 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    self.button_icon(
                        "gallery-cat-1",
                        format!("相机拍摄 ({camera_count})"),
                        UiIcon::Camera,
                        Action::BackScreen(BackScreenAction::GalleryCategory(1)),
                        if active_cat == 1 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    self.button_icon(
                        "gallery-cat-2",
                        format!("手机截图 ({screenshot_count})"),
                        UiIcon::Image,
                        Action::BackScreen(BackScreenAction::GalleryCategory(2)),
                        if active_cat == 2 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    self.button_icon(
                        "gallery-cat-3",
                        format!("微信与保存 ({weixin_count})"),
                        UiIcon::Chat,
                        Action::BackScreen(BackScreenAction::GalleryCategory(3)),
                        if active_cat == 3 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    self.button_icon(
                        "gallery-cat-4",
                        format!("近期照片 ({recent_count})"),
                        UiIcon::Calendar,
                        Action::BackScreen(BackScreenAction::GalleryCategory(4)),
                        if active_cat == 4 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    self.button_icon(
                        "gallery-cat-5",
                        format!("电脑扩容 ({expansion_count})"),
                        UiIcon::Storage,
                        Action::BackScreen(BackScreenAction::GalleryCategory(5)),
                        if active_cat == 5 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                ]),
                self.hstack(
                    S1,
                    vec![
                        self.pill_badge(
                            format!("当前显示 {} 张", filtered.len()),
                            p.accent,
                            p.accent_soft,
                        ),
                        self.button(
                            "gallery-cols-2",
                            "2列大图",
                            Action::BackScreen(BackScreenAction::GallerySetGridCols(2)),
                            if grid_cols == 2 {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            "gallery-cols-3",
                            "3列标准",
                            Action::BackScreen(BackScreenAction::GallerySetGridCols(3)),
                            if grid_cols == 3 {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            "gallery-cols-4",
                            "4列紧凑",
                            Action::BackScreen(BackScreenAction::GallerySetGridCols(4)),
                            if grid_cols == 4 {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    ],
                ),
            ),
            self.input(
                F_GALLERY_SEARCH,
                "输入照片名称、日期（如 20261025）筛选手机图片…",
                None,
            ),
            self.between(
                self.hstack(
                    6.,
                    vec![
                        self.ui_icon(UiIcon::Lightbulb, 13., p.accent),
                        self.line_w(
                            "左键选图预览 · 双击大图全屏沉浸看图 · 右键支持旋转 / 设为背屏 / 发给 Amoo 分析",
                            CAPTION,
                            p.text2,
                            Weight::Medium,
                        ),
                    ],
                ),
                self.button(
                    "gallery-top-toggle-lightbox",
                    if self.back_screen.gallery_lightbox {
                        "退出沉浸看图"
                    } else {
                        "沉浸大图看图"
                    },
                    Action::BackScreen(BackScreenAction::GalleryToggleLightbox),
                    if self.back_screen.gallery_lightbox {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
            ),
        ]);

        // Left Column: Visual Photo Thumbnail Grid (configurable 2 / 3 / 4 columns)
        let mut grid_rows = vec![self.between(
            self.heading("照片缩略图网格"),
            self.line(
                if self.back_screen.gallery_loading {
                    "正在同步手机相册与缩略图…"
                } else if self.back_screen.gallery_checked {
                    "点击卡片在右侧检视与操作"
                } else {
                    "准备读取手机相册"
                },
                CAPTION,
                p.text3,
            ),
        )];

        if filtered.is_empty() {
            grid_rows.push(
                Container::new(self.vstack(
                    S2,
                    vec![
                        self.line_w(
                            if self.back_screen.gallery_loading {
                                "正在从手机读取相册照片与缩略图…"
                            } else {
                                "当前分类下暂无照片，可点击右上角「截取手机屏幕入库」或「导入电脑图片到手机」。"
                            },
                            BODY,
                            p.text2,
                            Weight::Medium,
                        ),
                        self.wrap(vec![
                            self.button_icon(
                                "gallery-empty-shot",
                                "截取手机屏幕入库",
                                UiIcon::Camera,
                                Action::BackScreen(
                                    BackScreenAction::GalleryCapturePhoneScreenshot,
                                ),
                                Btn::Primary,
                            ),
                            self.button_icon(
                                "gallery-empty-import",
                                "导入电脑图片到手机",
                                UiIcon::Upload,
                                Action::BackScreen(BackScreenAction::GalleryImportFromDesktop),
                                Btn::Secondary,
                            ),
                        ]),
                    ],
                ))
                .with_uniform_padding(S4)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        } else {
            for chunk in filtered.chunks(grid_cols_usize) {
                let mut row_cards: Vec<El> = chunk
                    .iter()
                    .map(|&(orig_idx, item)| {
                        Expanded::new(1., self.gallery_grid_card(orig_idx, item)).finish()
                    })
                    .collect();
                while row_cards.len() < grid_cols_usize {
                    row_cards.push(Expanded::new(1., Empty::new().finish()).finish());
                }
                grid_rows.push(self.hstack(S3, row_cards));
            }
        }
        let left_panel = self.panel(grid_rows);

        // Selected Photo Preview & Action Inspector
        let selected_pair = items
            .get(self.back_screen.gallery_selected)
            .map(|item| (self.back_screen.gallery_selected, item))
            .or_else(|| filtered.first().copied());
        let selected_item = selected_pair.map(|(_, item)| item);
        let short_pos_label = if let Some((sel_idx, _)) = selected_pair {
            let cur_pos = filtered
                .iter()
                .position(|&(i, _)| i == sel_idx)
                .map(|p| p + 1)
                .unwrap_or(1);
            let total = filtered.len().max(1);
            format!("{cur_pos} / {total}")
        } else {
            "0 / 0".to_string()
        };

        let right_panel = if let Some(item) = selected_item {
            let mut inspector_parts = vec![self.between(
                self.heading("照片预览与操作"),
                self.pill_badge(
                    if item.category == "expansion" {
                        "电脑扩容 · 原图直读"
                    } else {
                        item.category_label()
                    },
                    p.accent,
                    p.accent_soft,
                ),
            )];

            inspector_parts.push(self.gallery_zoom_nav_bar("gallery-insp", item, &short_pos_label));
            inspector_parts.push(self.gallery_interactive_preview_box("gallery-insp", item));

            // Core action buttons placed directly below the preview image so they are always visible above the fold
            inspector_parts.push(self.wrap(vec![
                self.button_icon(
                    "gallery-act-open",
                    "用系统看图打开",
                    UiIcon::ExternalLink,
                    Action::BackScreen(BackScreenAction::GalleryOpenOnDesktop(
                        item.remote_path.clone(),
                    )),
                    Btn::Primary,
                ),
                self.button_icon(
                    "gallery-act-save",
                    if self
                        .back_screen
                        .gallery_rotations
                        .get(&item.remote_path)
                        .copied()
                        .unwrap_or(0)
                        > 0
                    {
                        "保存旋转后图片到电脑"
                    } else {
                        "保存到电脑"
                    },
                    UiIcon::Download,
                    Action::BackScreen(BackScreenAction::GallerySaveToDesktop(
                        item.remote_path.clone(),
                    )),
                    Btn::Secondary,
                ),
                self.button_icon(
                    "gallery-act-amoo",
                    "发给 Amoo 分析",
                    UiIcon::Star,
                    Action::BackScreen(BackScreenAction::GallerySendToAmoo(
                        item.remote_path.clone(),
                    )),
                    Btn::Secondary,
                ),
                self.button(
                    "gallery-act-backscreen",
                    "设为背屏像素",
                    Action::FilePushToBackScreen(item.remote_path.clone()),
                    Btn::Secondary,
                ),
                self.button(
                    "gallery-act-copypath-btn",
                    "复制路径",
                    Action::BackScreen(BackScreenAction::GalleryCopyPath(
                        item.remote_path.clone(),
                    )),
                    Btn::Secondary,
                ),
                self.button_icon(
                    "gallery-act-delete",
                    if item.category == "expansion" {
                        "在扩容库管理"
                    } else {
                        "删除"
                    },
                    if item.category == "expansion" {
                        UiIcon::Storage
                    } else {
                        UiIcon::Trash
                    },
                    Action::BackScreen(BackScreenAction::GalleryDelete(
                        item.remote_path.clone(),
                    )),
                    Btn::Ghost,
                ),
            ]));

            // Single compact metadata card (replaces the 4 tall stacked metadata cards)
            inspector_parts.push(
                Container::new(self.vstack(
                    S1,
                    vec![
                        self.between(
                            self.line_w(&item.name, CAPTION, p.text, Weight::Semibold),
                            self.mono(
                                format!(
                                    "{} · {}",
                                    item.size_label(),
                                    item.full_datetime_label(),
                                ),
                                MICRO,
                                p.text2,
                            ),
                        ),
                        self.between(
                            self.mono(&item.remote_path, MICRO, p.text3),
                            self.link_icon(
                                "gallery-act-copypath",
                                "复制路径",
                                UiIcon::Clipboard,
                                Action::BackScreen(BackScreenAction::GalleryCopyPath(
                                    item.remote_path.clone(),
                                )),
                            ),
                        ),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
            self.panel(inspector_parts)
        } else {
            self.panel(vec![
                self.heading("照片预览与操作"),
                Container::new(
                    ConstrainedBox::new(
                        Align::new(self.vstack(
                            S2,
                            vec![
                                Align::new(self.ui_icon(UiIcon::Image, 32., p.text3)).finish(),
                                Align::new(self.line(
                                    "从左侧选择一张手机照片查看高清预览",
                                    BODY,
                                    p.text2,
                                ))
                                .finish(),
                            ],
                        ))
                        .finish(),
                    )
                    .with_height(260.)
                    .finish(),
                )
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
                self.text(
                    "双击任意照片直接调用系统看图软件放大打开，支持右键常用操作、90° 旋转、一键保存到电脑及发给 Amoo 分析。",
                    CAPTION,
                    p.text3,
                ),
            ])
        };

        let right_col = super::super::motion::enter(
            right_panel,
            self.motion_started,
            !self.settings.value.reduced_motion && !self.settings.hidden,
        );

        let main_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S4)
            .with_child(Expanded::new(1.5, left_panel).finish())
            .with_child(Expanded::new(1.0, right_col).finish())
            .finish();

        let mut page_sections = vec![top_bar];
        if self.back_screen.gallery_lightbox {
            if let Some((sel_idx, item)) = selected_pair {
                page_sections.push(self.gallery_lightbox_overlay(sel_idx, item, &short_pos_label));
            }
        }
        page_sections.push(main_row);

        Container::new(self.vstack(S4, page_sections))
            .with_padding_bottom(S2)
            .finish()
    }
}

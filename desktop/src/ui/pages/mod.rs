//! One module per page. Each page exposes `<name>_page()` and `<name>_actions()`.

mod about;
mod amoo;
mod apps;
mod backscreen;
mod connect;
mod expansion;
pub(super) mod files;
mod gallery;
mod notifications;
mod overview;
mod phone;
mod schedule;
mod settings;
pub(super) mod sms;
mod studio;
mod studio_lab;
mod tools;
mod work;
mod work_rich_text;
mod workspace;
pub(super) use work_rich_text::markdown_images;

use super::{theme::*, widgets::*, Companion, Page};
use facloud_ui::elements::*;

impl Companion {
    /// Phone-uploaded documents may retain a shared-storage path rather than a
    /// remote URL. Keep that provenance and save through the verified ADB path.
    pub(in super::super) fn amoo_phone_attachment_card(
        &self,
        name: &str,
        kind: &str,
        mime: &str,
        url: &str,
        path: &str,
        size: Option<u64>,
    ) -> El {
        if !url.is_empty()
            || !path.starts_with("/sdcard/")
            || stepx_companion::storage::validate_path(path).is_err()
        {
            return self.amoo_attachment_card(name, kind, mime, url, size);
        }
        let p = self.p();
        let icon = if kind == "image" || mime.starts_with("image/") {
            super::icons::UiIcon::Image
        } else {
            super::icons::UiIcon::File
        };
        let meta = [
            "手机附件".to_string(),
            mime.to_string(),
            size.map(stepx_companion::storage::format_bytes)
                .unwrap_or_default(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        self.panel(vec![
            self.hstack(
                S2,
                vec![
                    self.ui_icon(icon, 20., p.text2),
                    Expanded::new(1., self.heading(name)).finish(),
                ],
            ),
            self.text(meta, CAPTION, p.text3),
            self.text("来自手机共享存储，可以保存到电脑查看。", CAPTION, p.text2),
            self.wrap(vec![self.button(
                format!("amoo-phone-attachment-save-{path}"),
                "另存为",
                super::Action::AmooUi(super::AmooUiAction::DownloadPhoneAttachment {
                    path: path.into(),
                    name: name.into(),
                }),
                Btn::Secondary,
            )]),
        ])
    }

    pub(in super::super) fn amoo_attachment_card(
        &self,
        name: &str,
        kind: &str,
        mime: &str,
        url: &str,
        size: Option<u64>,
    ) -> El {
        let p = self.p();
        let is_image = kind == "image" || mime.starts_with("image/");
        let icon = if is_image {
            super::icons::UiIcon::Image
        } else {
            super::icons::UiIcon::File
        };
        let meta = [
            mime.to_string(),
            size.map(stepx_companion::storage::format_bytes)
                .unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let mut body = vec![
            self.hstack(
                S2,
                vec![
                    self.ui_icon(icon, 20., p.text2),
                    Expanded::new(1., self.heading(name)).finish(),
                ],
            ),
            self.line(meta, CAPTION, p.text3),
        ];
        if url.starts_with("https://") || url.starts_with("http://") {
            if is_image {
                if let Some(path) = self
                    .work
                    .image_previews
                    .get(url)
                    .filter(|path| cached_is_file(path))
                {
                    let path = path.to_string_lossy().into_owned();
                    let image_url = url.to_owned();
                    body.push(
                        Hoverable::new(
                            self.mouse_state(format!("attachment-image-{url}")),
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
                                let mix = |a: u8, b: u8| {
                                    (a as f32 + (b as f32 - a as f32) * t) as u8
                                };
                                let bg = if mouse.is_clicked() {
                                    p.accent_soft
                                } else {
                                    facloud_ui::color::ColorU::new(
                                        mix(p.bg.r, p.hover.r),
                                        mix(p.bg.g, p.hover.g),
                                        mix(p.bg.b, p.hover.b),
                                        mix(p.bg.a, p.hover.a),
                                    )
                                };
                                let border_color = if mouse.is_clicked() {
                                    p.accent
                                } else {
                                    facloud_ui::color::ColorU::new(
                                        mix(p.border.r, p.border_strong.r),
                                        mix(p.border.g, p.border_strong.g),
                                        mix(p.border.b, p.border_strong.b),
                                        mix(p.border.a, p.border_strong.a),
                                    )
                                };
                                Container::new(
                                    ConstrainedBox::new(
                                        Image::new(
                                            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                                                path: path.clone(),
                                            },
                                            CacheOption::BySize,
                                        )
                                        .contain()
                                        .finish(),
                                    )
                                    .with_max_width(440.)
                                    .with_height(240.)
                                    .finish(),
                                )
                                .with_uniform_padding(S1)
                                .with_background_color(bg)
                                .with_border(Border::all(1.).with_border_color(border_color))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish()
                            },
                        )
                        .with_cursor(facloud_ui::platform::Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(super::Action::Work(
                                super::WorkAction::OpenPreview(image_url.clone()),
                            ))
                        })
                        .finish(),
                    );
                } else if let Some(error) = self.work.preview_errors.get(url) {
                    body.push(self.text("图片暂时无法载入", BODY, p.text2));
                    body.push(self.text(error, CAPTION, p.text3));
                    body.push(self.button(
                        format!("attachment-preview-{url}"),
                        "重试",
                        super::Action::Work(super::WorkAction::PreviewAttachment {
                            name: name.into(),
                            kind: kind.into(),
                            mime: mime.into(),
                            url: url.into(),
                        }),
                        Btn::Secondary,
                    ));
                } else {
                    let label = if self.selected.is_none() {
                        "连接手机后自动载入图片"
                    } else if self.work.preview_pending.contains(url) {
                        "正在载入图片…"
                    } else {
                        "图片等待载入…"
                    };
                    body.push(
                        Container::new(
                            ConstrainedBox::new(
                                Align::new(self.hstack(
                                    S2,
                                    vec![
                                        self.ui_icon(super::icons::UiIcon::Image, 22., p.text3),
                                        self.text(label, CAPTION, p.text2),
                                    ],
                                ))
                                .finish(),
                            )
                            .with_height(240.)
                            .finish(),
                        )
                        .with_background_color(p.bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                    );
                }
            }
            let mut actions = vec![self.button(
                format!("attachment-save-{url}"),
                "另存为",
                super::Action::Work(super::WorkAction::SaveAttachment {
                    name: name.into(),
                    kind: kind.into(),
                    mime: mime.into(),
                    url: url.into(),
                }),
                Btn::Secondary,
            )];
            if self.work.image_previews.contains_key(url) {
                actions.push(self.button(
                    format!("attachment-view-{url}"),
                    "放大查看",
                    super::Action::Work(super::WorkAction::OpenPreview(url.into())),
                    Btn::Ghost,
                ));
            } else if !is_image && !url.starts_with("http://127.0.0.1:") {
                actions.push(self.button(
                    format!("attachment-open-{url}"),
                    "打开附件",
                    super::Action::Work(super::WorkAction::OpenRemote(url.into())),
                    Btn::Ghost,
                ));
            }
            actions.push(self.button(
                format!("attachment-copy-{url}"),
                "复制链接",
                super::Action::CopyText(url.into(), "已复制手机附件链接".into()),
                Btn::Ghost,
            ));
            body.push(self.wrap(actions));
        } else {
            body.push(self.text(
                "该附件尚未返回可下载地址，可在原会话中继续处理。",
                CAPTION,
                p.text3,
            ));
        }
        self.panel(body)
    }

    /// A viewer inside the current window: opening and closing never changes shell mode.
    pub(super) fn amoo_image_viewer(&self) -> Option<El> {
        let url = self.work.image_viewer.as_ref()?;
        let base_path = self.work.image_previews.get(url).cloned().or_else(|| {
            let p = std::path::PathBuf::from(url);
            cached_is_file(&p).then_some(p)
        })?;
        let display_path = self
            .work
            .image_viewer_rotated_path
            .as_ref()
            .filter(|p| cached_is_file(p))
            .unwrap_or(&base_path);
        let p = self.p();
        let zoom = self.work.image_viewer_zoom.clamp(50, 400);
        let dims = cached_image_dimensions(display_path).map(|(w, h)| format!("{w} × {h} px"));
        let file_size = cached_file_len(&base_path).map(stepx_companion::storage::format_bytes);
        let file_name = base_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("图片预览");
        let meta_parts: Vec<String> = [dims, file_size].into_iter().flatten().collect();

        let mut header_left = vec![
            self.heading("图片预览"),
            self.pill_badge(format!("{zoom}%"), p.accent, p.accent_soft),
        ];
        if self.work.image_viewer_rotation > 0 {
            header_left.push(self.pill_badge(
                format!("旋转 {}°", self.work.image_viewer_rotation),
                p.blue,
                p.accent_soft,
            ));
        }
        if !meta_parts.is_empty() {
            header_left.push(self.pill_badge(meta_parts.join(" · "), p.text2, p.tag_bg));
        }
        header_left.push(self.line(file_name, CAPTION, p.text3));

        let header = self.between(
            self.wrap(header_left),
            self.button(
                "image-viewer-close",
                "返回 (Esc)",
                super::Action::Work(super::WorkAction::CloseImagePreview),
                Btn::Secondary,
            ),
        );

        let mut toolbar = vec![self.button(
            "image-viewer-zoom-out",
            "− 缩小",
            super::Action::Work(super::WorkAction::ZoomOutImagePreview),
            Btn::Ghost,
        )];
        for preset in [100u16, 150, 200, 300, 400] {
            toolbar.push(self.button(
                format!("image-viewer-zoom-{preset}"),
                format!("{preset}%"),
                super::Action::Work(super::WorkAction::ZoomImagePreview(preset)),
                if zoom == preset {
                    Btn::Primary
                } else {
                    Btn::Ghost
                },
            ));
        }
        toolbar.extend([
            self.button(
                "image-viewer-zoom-in",
                "+ 放大",
                super::Action::Work(super::WorkAction::ZoomInImagePreview),
                Btn::Ghost,
            ),
            self.button_icon(
                "image-viewer-rotate",
                "旋转 90°",
                super::icons::UiIcon::Refresh,
                super::Action::Work(super::WorkAction::RotateImagePreview),
                Btn::Secondary,
            ),
            self.button_icon(
                "image-viewer-copy-path",
                "复制路径",
                super::icons::UiIcon::Clipboard,
                super::Action::CopyText(base_path.display().to_string(), "已复制图片路径".into()),
                Btn::Ghost,
            ),
            self.button_icon(
                "image-viewer-open-os",
                "系统打开",
                super::icons::UiIcon::Folder,
                super::Action::Work(super::WorkAction::OpenLocalPreview),
                Btn::Ghost,
            ),
        ]);

        let image = Image::new(
            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                path: display_path.to_string_lossy().into_owned(),
            },
            CacheOption::BySize,
        )
        .contain()
        .finish();

        let viewport = if zoom > 100 {
            let scaled_h = 480.0 * (zoom as f32 / 100.0);
            Expanded::new(
                1.,
                self.pane_scroll(
                    self.work.preview_scroll.clone(),
                    ConstrainedBox::new(Container::new(image).with_vertical_padding(S2).finish())
                        .with_height(scaled_h)
                        .finish(),
                ),
            )
            .finish()
        } else {
            Expanded::new(1., Container::new(image).with_vertical_padding(S3).finish()).finish()
        };

        Some(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_spacing(S2)
                    .with_child(header)
                    .with_child(self.wrap(toolbar))
                    .with_child(viewport)
                    .with_child(self.line(
                        format!("{} · Esc 返回 · 会话、附件和草稿保留", base_path.display()),
                        CAPTION,
                        p.text3,
                    ))
                    .finish(),
            )
            .with_horizontal_padding(S5)
            .with_padding_top(S5)
            .with_padding_bottom(S6)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish(),
        )
    }

    /// Reusable smooth-hover sub-navigation bar for page sub-sections.
    #[allow(dead_code)]
    pub(super) fn sub_nav_bar(&self, items: &[(&'static str, bool, super::Action)]) -> El {
        let p = self.p();
        let font = self.font;
        let tabs: Vec<El> = items
            .iter()
            .enumerate()
            .map(|(idx, (label, active, action))| {
                let active = *active;
                let label = *label;
                let action = action.clone();
                Hoverable::new(
                    self.mouse_state(format!("sub-nav-{idx}-{label}")),
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
                        let bg = if mouse.is_clicked() || active {
                            p.accent_soft
                        } else {
                            facloud_ui::color::ColorU::new(
                                mix(p.surface.r, p.hover.r),
                                mix(p.surface.g, p.hover.g),
                                mix(p.surface.b, p.hover.b),
                                mix(p.surface.a, p.hover.a),
                            )
                        };
                        let border_color = if mouse.is_clicked() || active {
                            p.accent
                        } else {
                            facloud_ui::color::ColorU::new(
                                mix(p.border.r, p.border_strong.r),
                                mix(p.border.g, p.border_strong.g),
                                mix(p.border.b, p.border_strong.b),
                                mix(p.border.a, p.border_strong.a),
                            )
                        };
                        Container::new(
                            Text::new(label.to_string(), font, BODY)
                                .with_color(if active {
                                    p.accent
                                } else if hovered {
                                    p.text
                                } else {
                                    p.text2
                                })
                                .with_style(facloud_ui::fonts::Properties::default().weight(
                                    if active {
                                        facloud_ui::fonts::Weight::Semibold
                                    } else {
                                        facloud_ui::fonts::Weight::Medium
                                    },
                                ))
                                .soft_wrap(false)
                                .finish(),
                        )
                        .with_horizontal_padding(S3)
                        .with_vertical_padding(6.)
                        .with_background_color(bg)
                        .with_border(Border::all(1.).with_border_color(border_color))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                    },
                )
                .with_cursor(facloud_ui::platform::Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish()
            })
            .collect();
        self.wrap(tabs)
    }

    pub(super) fn page_content(&self) -> El {
        match self.page {
            Page::Overview => self.overview_page(),
            Page::Expansion => self.expansion_page(),
            Page::BackScreen => self.backscreen_page(),
            Page::Amoo => self.workspace_page(false),
            Page::Calendar | Page::Schedule => self.calendar_page(),
            Page::Alarms => self.alarms_page(),
            Page::Gallery => self.gallery_page(),
            Page::Account => self.account_usage_page(),
            Page::About | Page::Settings => self.settings_page(),
            Page::Work => self.work_page(),
            Page::Studio => self.studio_page(),
            Page::Sms => self.sms_page(),
            Page::Notifications => self.notifications_page(),
            Page::Phone => self.phone_page(),
            Page::Files => self.files_page(),
            Page::Apps => self.apps_page(),
            Page::Tools => self.tools_page(),
            Page::Connect => self.connect_page(),
        }
    }

    /// Buttons on the right of the page header. At most one `Primary`.
    pub(super) fn page_actions(&self) -> Vec<El> {
        match self.page {
            Page::About | Page::Settings => Vec::new(),
            Page::Overview => self.overview_actions(),
            Page::Expansion => self.expansion_actions(),
            Page::BackScreen => self.backscreen_actions(),
            Page::Amoo => self.workspace_actions(),
            Page::Calendar | Page::Schedule => self.calendar_actions(),
            Page::Alarms => self.alarms_actions(),
            Page::Gallery => self.gallery_actions(),
            Page::Account => vec![self.button(
                "account-refresh",
                "刷新用量",
                super::Action::AmooUi(super::AmooUiAction::RefreshUsage),
                Btn::Secondary,
            )],
            Page::Work => self.work_actions(),
            Page::Studio => self.studio_actions(),
            Page::Sms => self.sms_actions(),
            Page::Notifications => self.notifications_actions(),
            Page::Phone => self.phone_actions(),
            Page::Files => self.files_actions(),
            Page::Apps => self.apps_actions(),
            Page::Tools => self.tools_actions(),
            Page::Connect => self.connect_actions(),
        }
    }

    /// Chat bubble shared by Amoo and SMS. `mine` aligns right with the accent tint.
    #[allow(dead_code)]
    pub(super) fn bubble(&self, mine: bool, meta: String, body: El, footer: Option<El>) -> El {
        let p = self.p();
        let mut inner = vec![body];
        if let Some(f) = footer {
            inner.push(f);
        }
        let bubble = Container::new(
            Flex::column()
                .with_spacing(S1 + 2.)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_children(inner)
                .finish(),
        )
        .with_horizontal_padding(S3)
        .with_vertical_padding(S2 + 1.)
        .with_background_color(if mine { p.accent_soft } else { p.surface })
        .with_border(Border::all(1.).with_border_color(if mine { p.accent_soft } else { p.border }))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL + 2.)))
        .finish();
        let column = Flex::column()
            .with_spacing(S1)
            .with_cross_axis_alignment(if mine {
                CrossAxisAlignment::End
            } else {
                CrossAxisAlignment::Start
            })
            .with_child(self.text(meta, MICRO, p.text3))
            .with_child(ConstrainedBox::new(bubble).with_max_width(560.).finish())
            .finish();
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(if mine {
                MainAxisAlignment::End
            } else {
                MainAxisAlignment::Start
            })
            .with_child(column)
            .finish()
    }
}

/// "14:05" for today, "昨天 14:05", otherwise "10-18 14:05".
pub(super) fn format_time(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    let day = t.date_naive();
    if day == today {
        t.format("%H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        t.format("昨天 %H:%M").to_string()
    } else {
        t.format("%m-%d %H:%M").to_string()
    }
}

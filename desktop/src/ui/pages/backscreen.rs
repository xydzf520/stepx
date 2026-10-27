//! A compact LED studio: edit locally, inspect the result, then play on the phone.
use super::super::{
    backscreen_state::BackScreenAction as BsAction, icons::UiIcon, studio_state::StudioAction,
    theme::*, widgets::*, Action, Companion, Page, F_BACK_MARQUEE, F_BACK_PROMPT,
};
use facloud_ui::{elements::*, fonts::Weight, platform::Cursor};
use std::time::Duration;
use stepx_companion::backscreen as bs;

static GALLERY_MTIME_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, (std::time::Instant, String)>>,
> = std::sync::OnceLock::new();

fn cached_gallery_mtime_label(path: &std::path::Path) -> String {
    const TTL: Duration = Duration::from_secs(2);
    let cache =
        GALLERY_MTIME_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(map) = cache.lock() {
        if let Some((at, label)) = map.get(path) {
            if at.elapsed() < TTL {
                return label.clone();
            }
        }
    }
    let date = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .map(|time| {
            chrono::DateTime::<chrono::Local>::from(time)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    if let Ok(mut map) = cache.lock() {
        if map.len() >= 256 {
            map.retain(|_, (at, _)| at.elapsed() < TTL);
            if map.len() >= 256 {
                map.clear();
            }
        }
        map.insert(
            path.to_path_buf(),
            (std::time::Instant::now(), date.clone()),
        );
    }
    date
}

impl Companion {
    fn bs_button(&self, id: &str, label: &str, action: BsAction, style: Btn) -> El {
        self.button(format!("bs-{id}"), label, Action::BackScreen(action), style)
    }
    fn bs_button_icon(
        &self,
        id: &str,
        label: impl Into<String>,
        icon: UiIcon,
        a: BsAction,
        kind: Btn,
    ) -> El {
        self.button_icon(format!("bs-{id}"), label, icon, Action::BackScreen(a), kind)
    }
    fn bs_title(&self, icon: UiIcon, title: &str) -> El {
        self.hstack(
            S2,
            vec![self.ui_icon(icon, 16., self.p().text2), self.heading(title)],
        )
    }
    pub(in super::super) fn backscreen_actions(&self) -> Vec<El> {
        let custom_active = self.back_screen.playing
            || (self.back_screen.applied_to_phone && self.back_screen.companion_mode != "native")
            || self.back_screen.pomodoro_active;
        let restore_action = if self.back_screen.playing {
            BsAction::Stop
        } else if self.back_screen.pomodoro_active {
            BsAction::StopPomodoro
        } else {
            BsAction::CompanionStop
        };
        let mut actions = Vec::new();
        if custom_active || self.selected.is_some() {
            actions.push(self.bs_button(
                "restore-default-top",
                "恢复手机默认背屏",
                restore_action,
                if custom_active {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ));
        }
        actions.push(self.bs_button(
            "inspect-top",
            "刷新手机状态",
            BsAction::CompanionInspect,
            Btn::Secondary,
        ));
        actions.push(self.bs_button(
            "advanced-top",
            if self.back_screen.advanced {
                "收起设置"
            } else {
                "背屏设置"
            },
            BsAction::Advanced,
            Btn::Secondary,
        ));
        actions
    }
    fn bs_preview(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        let game = state.tab == 4;
        let game_frame = game_preview_frame(state.elapsed_ms);
        let preview_frame = if game { &game_frame } else { &state.frame };
        let editable_canvas = !game && state.tab != 3 && state.native_feature.is_none();
        let mut parts = vec![
            self.between(
                self.bs_title(UiIcon::Matrix, if game { "玩法预览" } else { "点阵预览" }),
                self.pill_badge(
                    if game || state.native_feature.is_some() {
                        "功能示意"
                    } else if state.has_unapplied_changes() {
                        "待应用"
                    } else {
                        "已应用"
                    },
                    if !game && !state.has_unapplied_changes() {
                        p.success
                    } else {
                        p.text2
                    },
                    p.surface,
                ),
            ),
            self.line_w(
                if game { "重力对齐" } else { &state.title },
                TITLE,
                p.text,
                Weight::Semibold,
            ),
            Align::new(self.hstack(
                S4,
                vec![
                    self.phone_rear_housing(100., 210., preview_frame, 3.6, 0.95),
                    ConstrainedBox::new(self.vstack(
                        S2,
                        vec![
                            self.line(
                                if editable_canvas {
                                    "手绘点阵"
                                } else {
                                    "实时点阵"
                                },
                                MICRO,
                                p.text3,
                            ),
                            Container::new(
                                super::super::led_grid::LedGrid::new(
                                    preview_frame,
                                    3.6,
                                    0.95,
                                )
                                .on_pixel_click(|x, y| {
                                    Action::BackScreen(BsAction::TogglePixel(x, y))
                                })
                                .finish(),
                            )
                            .with_uniform_padding(S2)
                            .with_background_color(rgb(24, 24, 30))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                R_CONTROL,
                            )))
                            .finish(),
                        ],
                    ))
                    .with_width(104.)
                    .finish(),
                ],
            ))
            .finish(),
            self.mono("17 × 23 · 379 灯珠", MICRO, p.text2),
        ];
        if editable_canvas {
            parts.push(self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Lightbulb, 13., p.text3),
                    self.line("点击上方灯阵格可直接手绘/擦除像素", MICRO, p.text3),
                ],
            ));
            parts.push(self.wrap(vec![
                self.bs_button_icon(
                    "diy-invert",
                    "反相",
                    UiIcon::Contrast,
                    BsAction::InvertFrame,
                    Btn::Secondary,
                ),
                self.bs_button_icon(
                    "diy-left",
                    "左移",
                    UiIcon::ArrowLeft,
                    BsAction::ShiftFrame(-1, 0),
                    Btn::Secondary,
                ),
                self.bs_button_icon(
                    "diy-up",
                    "上移",
                    UiIcon::ArrowUp,
                    BsAction::ShiftFrame(0, -1),
                    Btn::Secondary,
                ),
                self.bs_button_icon(
                    "diy-down",
                    "下移",
                    UiIcon::ArrowDown,
                    BsAction::ShiftFrame(0, 1),
                    Btn::Secondary,
                ),
                self.bs_button_icon(
                    "diy-right",
                    "右移",
                    UiIcon::ArrowRight,
                    BsAction::ShiftFrame(1, 0),
                    Btn::Secondary,
                ),
                self.bs_button_icon(
                    "diy-clear",
                    "清空画布",
                    UiIcon::Trash,
                    BsAction::ClearDiyFrame,
                    Btn::Ghost,
                ),
            ]));
        }
        parts.push(self.bs_button(
            "preview-toggle",
            if state.preview_running {
                "暂停预览"
            } else {
                "播放预览"
            },
            BsAction::TogglePreview,
            Btn::Secondary,
        ));
        parts.push(self.text(
            if game {
                "面对背屏，左倾向左、右倾向右。翻到正面结束本局。"
            } else {
                &state.status
            },
            CAPTION,
            if state.error.is_some() && !game {
                p.danger
            } else {
                p.text2
            },
        ));
        if !state.applied_title.is_empty() {
            parts.push(self.divider());
            parts.push(self.line("手机当前", MICRO, p.text3));
            parts.push(self.line_w(&state.applied_title, BODY, p.text, Weight::Medium));
            parts.push(self.text(&state.applied_status, MICRO, p.text2));
        }
        if state
            .error
            .as_ref()
            .is_some_and(|error| error.contains("自定义背屏"))
        {
            parts.push(self.bs_button(
                "resolve-occupied",
                "处理旧背屏占用",
                BsAction::Advanced,
                Btn::Secondary,
            ));
        }
        let primary = if self.selected.is_none() {
            self.disabled_button("请先连接手机")
        } else if state.converting_image {
            self.disabled_button("正在转换图片")
        } else if state.applying() {
            self.disabled_button("正在同步手机…")
        } else if state.generating {
            self.disabled_button("等待 Amoo 生成")
        } else if game {
            self.bs_button("arm-game", "启用三敲启动", BsAction::GameArm, Btn::Primary)
        } else {
            self.bs_button(
                "apply",
                if state.temporary_app() {
                    "应用临时联动"
                } else {
                    "应用到手机背屏"
                },
                BsAction::Play,
                Btn::Primary,
            )
        };
        parts.push(self.divider());
        parts.push(primary);
        if state.playing {
            parts.push(self.bs_button("stop", "停止并恢复", BsAction::Stop, Btn::Secondary));
        } else if state.applied_to_phone && state.companion_mode != "native" {
            parts.push(self.bs_button(
                "stop-companion",
                "停止并恢复",
                BsAction::CompanionStop,
                Btn::Secondary,
            ));
        }
        parts.push(self.text(
            if game {
                "背屏朝向自己，轻敲三下开始；翻到正面结束并恢复原显示。"
            } else if state.native_feature.is_some() {
                "应用后，由手机系统管理显示。"
            } else if state.temporary_app() {
                "需保持桌面连接，计时结束后恢复原显示。"
            } else if matches!(state.source, bs::Source::Preset(bs::Preset::Countdown)) {
                "倒计时在手机运行，到时恢复原显示。"
            } else {
                "应用后由手机持续显示，选择其他图案不会立即更换。"
            },
            MICRO,
            p.text3,
        ));
        let preview = Container::new(self.vstack(S3, parts))
            .with_uniform_padding(S4)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();
        self.pane_scroll(self.scroll_aux.clone(), preview)
    }
    fn bs_preset_card(&self, preset: bs::Preset) -> El {
        let p = self.p();
        let selected = self.back_screen.selection() == Some(preset);
        // Fixed gallery thumbnails are generated once on first opening the gallery.
        static THUMBNAILS: std::sync::OnceLock<Vec<(bs::Preset, Vec<u8>)>> =
            std::sync::OnceLock::new();
        let thumbnails = THUMBNAILS.get_or_init(|| {
            bs::Preset::ALL
                .into_iter()
                .map(|preset| {
                    (
                        preset,
                        bs::frame(
                            &bs::Source::Preset(preset),
                            &bs::PlaybackConfig::default(),
                            Duration::from_millis(700),
                            &bs::LiveData::default(),
                        ),
                    )
                })
                .collect()
        });
        let frame = &thumbnails.iter().find(|(key, _)| *key == preset).unwrap().1;
        let thumbnail = ConstrainedBox::new(
            Container::new(Align::new(self.matrix_grid(&frame, 1.1, 0.35, false)).finish())
                .with_background_color(rgb(0x16, 0x16, 0x1B))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        )
        .with_width(40.)
        .with_height(46.)
        .finish();
        let contents = self.hstack(
            S3,
            vec![
                thumbnail,
                Expanded::new(
                    1.,
                    self.vstack(
                        S1,
                        vec![
                            self.line_w(preset.label(), BODY, p.text, Weight::Semibold),
                            self.line(preset.description(), MICRO, p.text3),
                        ],
                    ),
                )
                .finish(),
                if selected {
                    self.ui_icon(UiIcon::Check, 16., p.text)
                } else {
                    self.ui_icon(UiIcon::ChevronRight, 14., p.text3)
                },
            ],
        );
        Hoverable::new(
            self.mouse_state(format!("bs-preset-{}", preset.key())),
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
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else if selected {
                    p.selected
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
                } else if selected {
                    p.border_strong
                } else {
                    facloud_ui::color::ColorU::new(
                        mix(p.border.r, p.border_strong.r),
                        mix(p.border.g, p.border_strong.g),
                        mix(p.border.b, p.border_strong.b),
                        mix(p.border.a, p.border_strong.a),
                    )
                };
                Container::new(contents)
                    .with_uniform_padding(S3)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BsAction::SelectPreset(preset)))
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BsAction::ApplyPreset(preset)))
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::BackScreenPreset(preset),
                at,
            ))
        })
        .finish()
    }
    fn bs_gallery(&self) -> El {
        let p = self.p();
        let active_filter = self.back_screen.preset_filter;
        let is_utility = |preset: bs::Preset| {
            matches!(
                preset,
                bs::Preset::Clock | bs::Preset::Countdown | bs::Preset::Battery
            )
        };
        let filtered_presets: Vec<bs::Preset> = bs::Preset::ALL
            .into_iter()
            .filter(|&preset| match active_filter {
                1 => !is_utility(preset),
                2 => is_utility(preset),
                _ => true,
            })
            .collect();
        let total_count = bs::Preset::ALL.len();
        let util_count = bs::Preset::ALL
            .into_iter()
            .filter(|&p| is_utility(p))
            .count();
        let anim_count = total_count.saturating_sub(util_count);

        let mut parts = vec![
            self.between(
                self.bs_title(UiIcon::Smile, "有趣，也实用"),
                self.line(
                    format!(
                        "当前显示 {} / {} 种效果",
                        filtered_presets.len(),
                        total_count
                    ),
                    CAPTION,
                    p.text3,
                ),
            ),
            self.text(
                "点击图案预览，满意后点击「应用到背屏」。时钟和电量由手机实时更新。",
                CAPTION,
                p.text2,
            ),
            self.wrap(vec![
                self.bs_button(
                    "preset-filter-0",
                    &format!("全部预设 ({total_count})"),
                    BsAction::PresetFilter(0),
                    if active_filter == 0 {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
                self.bs_button(
                    "preset-filter-1",
                    &format!("动态像素动画 ({anim_count})"),
                    BsAction::PresetFilter(1),
                    if active_filter == 1 {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
                self.bs_button(
                    "preset-filter-2",
                    &format!("实用时钟与状态 ({util_count})"),
                    BsAction::PresetFilter(2),
                    if active_filter == 2 {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
            ]),
        ];
        for row in filtered_presets.chunks(2) {
            let mut cards: Vec<El> = row
                .iter()
                .map(|preset| Expanded::new(1., self.bs_preset_card(*preset)).finish())
                .collect();
            if cards.len() == 1 {
                cards.push(Expanded::new(1., Empty::new().finish()).finish());
            }
            parts.push(self.hstack(S2, cards));
        }
        self.vstack(S2, parts)
    }
    fn bs_recent_gallery(&self, limit: usize) -> El {
        let p = self.p();
        let mut parts = vec![self.between(
            self.bs_title(UiIcon::Image, "Amoo 作品图库"),
            self.bs_button(
                "refresh-gallery",
                "刷新",
                BsAction::RefreshGallery,
                Btn::Ghost,
            ),
        )];
        if self.back_screen.recent_images.is_empty() {
            parts.push(self.text(
                "生成的图片会保存在这里，重新打开桌面后也能继续使用。",
                CAPTION,
                p.text3,
            ));
        } else {
            let images: Vec<_> = self
                .back_screen
                .recent_images
                .iter()
                .take(limit)
                .enumerate()
                .collect();
            for chunk in images.chunks(3) {
                let mut cards = Vec::new();
                for &(index, path) in chunk {
                    let image = ConstrainedBox::new(
                        Image::new(
                            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                                path: path.to_string_lossy().into_owned(),
                            },
                            CacheOption::BySize,
                        )
                        .contain()
                        .finish(),
                    )
                    .with_max_width(180.)
                    .with_height(92.)
                    .finish();
                    let date = cached_gallery_mtime_label(path);
                    let card_body = self.panel(vec![
                        image,
                        self.line(date, MICRO, p.text3),
                        self.bs_button(
                            &format!("recent-{index}"),
                            "转为像素",
                            BsAction::RecentImage(index),
                            Btn::Secondary,
                        ),
                    ]);
                    let open_path = path.clone();
                    let menu_path = path.display().to_string();
                    let card = Hoverable::new(
                        self.mouse_state(format!("bs-recent-card-{index}")),
                        move |_| card_body,
                    )
                    .with_defer_events_to_children()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(BsAction::RecentImage(index)))
                    })
                    .on_double_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::Studio(StudioAction::OpenPath(
                            open_path.clone(),
                        )))
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::BackScreenRecentImage {
                                index,
                                path: menu_path.clone(),
                            },
                            at,
                        ))
                    })
                    .finish();
                    cards.push(Expanded::new(1., card).finish());
                }
                while cards.len() < 3 {
                    cards.push(Expanded::new(1., Empty::new().finish()).finish());
                }
                parts.push(self.hstack(S2, cards));
            }
        }
        self.vstack(S2, parts)
    }
    fn bs_prompt_input(&self) -> El {
        let p = self.p();
        let field = &self.fields[F_BACK_PROMPT];
        let focused = self.focus == Some(F_BACK_PROMPT);
        let mut value = field.text.clone();
        let mut caret = 0;
        if focused {
            let cursor = field.cursor.min(value.len());
            caret = value[..cursor].chars().count() + field.preedit.chars().count();
            value.insert_str(cursor, &field.preedit);
        }
        let empty = field.text.is_empty() && field.preedit.is_empty();
        if value.is_empty() {
            value = "输入你想生成的背屏内容，例如：像素小狗眨眼、赛博霓虹雨夜".into();
        }
        Hoverable::new(self.mouse_state("bs-amoo-prompt"), move |mouse| {
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
            let bg = if mouse.is_clicked() {
                p.accent_soft
            } else {
                facloud_ui::color::ColorU::new(
                    mix(p.surface.r, p.hover.r),
                    mix(p.surface.g, p.hover.g),
                    mix(p.surface.b, p.hover.b),
                    mix(p.surface.a, p.hover.a),
                )
            };
            let border_color = if focused || mouse.is_clicked() {
                p.accent
            } else {
                facloud_ui::color::ColorU::new(
                    mix(p.border.r, p.border_strong.r),
                    mix(p.border.g, p.border_strong.g),
                    mix(p.border.b, p.border_strong.b),
                    mix(p.border.a, p.border_strong.a),
                )
            };
            ConstrainedBox::new(
                Container::new(self.multiline_field_text(
                    &value,
                    caret,
                    F_BACK_PROMPT,
                    if empty { p.text3 } else { p.text },
                ))
                .with_uniform_padding(S3)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
            .with_min_height(100.)
            .finish()
        })
        .with_cursor(Cursor::IBeam)
        .with_defer_events_to_children()
        .on_mouse_down(|ctx, _, _| ctx.dispatch_typed_action(Action::Focus(F_BACK_PROMPT)))
        .on_right_click(|ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::Input(F_BACK_PROMPT),
                at,
            ))
        })
        .finish()
    }
    fn bs_amoo(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        let modes = self.segmented(
            "bs-amoo-mode",
            vec![
                (
                    "智能编排".into(),
                    !state.amoo_image,
                    Action::BackScreen(BsAction::AmooMode(false)),
                ),
                (
                    "生成图片".into(),
                    state.amoo_image,
                    Action::BackScreen(BsAction::AmooMode(true)),
                ),
            ],
        );
        let mut parts = vec![
            self.bs_title(UiIcon::Star, "让 Amoo 理解你的想法"),
            self.text(if state.amoo_image {"调用手机 Amoo 实际生成图片，再转成适合背屏的单色像素。"} else {"描述图案或用途，Amoo 会选择动画、亮度、速度与时长，也能创作新图片。"}, CAPTION, p.text2),
            modes, self.bs_prompt_input(),
            self.wrap(vec![
                self.bs_button("prompt-cat", "猫咪陪伴", BsAction::Prompt("做一只会眨眼的猫咪，低亮度，播放 20 秒".into()), Btn::Secondary),
                self.bs_button("prompt-focus", "专注倒计时", BsAction::Prompt("我要专注 25 分钟，背屏显示倒计时，亮度柔和".into()), Btn::Secondary),
                self.bs_button("prompt-image", "宇航员像素画", BsAction::Prompt("生成一张宇航员抱着月亮的图片，单色，纯黑背景，白色轮廓，主体居中，适合 17 列 23 行的背屏".into()), Btn::Secondary),
                self.bs_button("prompt-meteor", "赛博流星雨", BsAction::Prompt("设计一组动感斜向划过的赛博流星雨，呼吸感尾迹，中高亮度循环播放".into()), Btn::Secondary),
                self.bs_button("prompt-breath", "极简呼吸灯", BsAction::Prompt("极简中心光晕缓慢呼吸明暗交替，低亮度适合桌面静置陪伴".into()), Btn::Secondary),
                self.bs_button("prompt-shiba", "像素萌宠柴犬", BsAction::Prompt("生成一张高对比度单色像素柴犬笑脸，纯黑背景，白色清晰轮廓，适合背屏灯珠显示".into()), Btn::Secondary),
            ]),
            self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Bolt, 14., p.accent),
                    self.line("常用状态灯牌一键生成 (免手机直出 23×17 点阵)", CAPTION, p.text2),
                ],
            ),
            self.wrap(vec![
                self.chip(
                    "bs-badge-dnd",
                    "勿扰",
                    Some(UiIcon::Ban),
                    vec![
                        Action::BackScreen(BsAction::Prompt("勿扰 DND".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-meeting",
                    "会议中",
                    Some(UiIcon::Briefcase),
                    vec![
                        Action::BackScreen(BsAction::Prompt("会议中 MEET".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-coffee",
                    "休息中",
                    Some(UiIcon::Coffee),
                    vec![
                        Action::BackScreen(BsAction::Prompt("休息中 COFFEE".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-pickup",
                    "取件码",
                    Some(UiIcon::Package),
                    vec![
                        Action::BackScreen(BsAction::Prompt("取件码 8-2-104".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-heart",
                    "心动",
                    Some(UiIcon::Heart),
                    vec![
                        Action::BackScreen(BsAction::Prompt("爱心 LOVE".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-cat",
                    "像素猫",
                    Some(UiIcon::Cat),
                    vec![
                        Action::BackScreen(BsAction::Prompt("像素猫咪 CAT".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-bolt",
                    "充能中",
                    Some(UiIcon::Bolt),
                    vec![
                        Action::BackScreen(BsAction::Prompt("闪电充能 POWER".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
                self.chip(
                    "bs-badge-sos",
                    "SOS",
                    Some(UiIcon::AlertTriangle),
                    vec![
                        Action::BackScreen(BsAction::Prompt("SOS".into())),
                        Action::BackScreen(BsAction::LocalSynthesize),
                    ],
                    false,
                    false,
                ),
            ]),
            if state.generating {
                self.bs_button(
                    "cancel-generation",
                    "取消生成",
                    BsAction::CancelGenerate,
                    Btn::Secondary,
                )
            } else {
                self.wrap(vec![
                    self.bs_button("amoo-generate", "生成背屏方案", BsAction::Generate, Btn::Primary),
                    self.bs_button_icon(
                        "amoo-local-synth",
                        "本地极速生成点阵",
                        UiIcon::Bolt,
                        BsAction::LocalSynthesize,
                        Btn::Secondary,
                    ),
                ])
            },
        ];
        if !state.generator_status.is_empty() {
            parts.push(self.panel(vec![
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(
                            if state.generating {
                                UiIcon::Clock
                            } else {
                                UiIcon::Check
                            },
                            16.,
                            if state.error.is_some() {
                                p.danger
                            } else {
                                p.text2
                            },
                        ),
                        self.heading(if state.generating {
                            "Amoo 正在创作"
                        } else {
                            "创作结果"
                        }),
                    ],
                ),
                self.text(&state.generator_status, BODY, p.text2),
            ]));
        }
        if state.image_path.is_some() {
            parts.push(self.bs_source_image());
        }
        parts.push(self.bs_recent_gallery(6));
        self.vstack(S3, parts)
    }
    fn bs_source_image(&self) -> El {
        let p = self.p();
        let Some(path) = &self.back_screen.image_path else {
            return Empty::new().finish();
        };
        let image = ConstrainedBox::new(
            Image::new(
                facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish(),
        )
        .with_max_width(240.)
        .with_height(148.)
        .finish();
        self.hover_panel_interactive(
            "bs-source-image",
            self.vstack(
                S3,
                vec![
                    self.between(
                        self.bs_title(UiIcon::Image, "原图"),
                        self.line(&self.back_screen.origin, MICRO, p.text3),
                    ),
                    image,
                    self.text("左侧为实际灯珠布局下的灰度转换结果。", MICRO, p.text3),
                ],
            ),
            None,
            Some(Action::Studio(StudioAction::OpenPath(path.clone()))),
            Some(crate::ui::context_menu::Target::LocalFile(path.clone())),
        )
    }
    fn bs_images(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        let mut parts = vec![
            self.bs_title(UiIcon::Image, "把喜欢的图片装进背屏"),
            self.text(
                "支持 PNG、JPEG、WebP、GIF 首帧。简单轮廓和高对比度主体更适合 379 颗灯珠。",
                CAPTION,
                p.text2,
            ),
            self.wrap(vec![
                self.bs_button(
                    "choose-image",
                    "选择电脑图片…",
                    BsAction::PickImage,
                    Btn::Primary,
                ),
                self.button(
                    "bs-image-goto-gallery",
                    "浏览手机图库",
                    Action::Page(Page::Gallery),
                    Btn::Secondary,
                ),
                self.button(
                    "bs-image-goto-files",
                    "从手机文件推送照片/GIF/视频",
                    Action::Page(Page::Files),
                    Btn::Ghost,
                ),
            ]),
        ];
        if state.image_path.is_some() {
            parts.push(self.bs_source_image());
        } else {
            parts.push(self.panel(vec![self.bs_title(UiIcon::Lightbulb, "试试这些画面"), self.text("简笔头像、宠物轮廓、像素角色、单色标志。可以先让 Amoo 生成图片，再在这里调整转换效果。", BODY, p.text2), self.bs_button("image-to-amoo", "用 Amoo 创作", BsAction::Tab(1), Btn::Secondary)]));
        }
        parts.push(self.wrap(vec![
            self.bs_button(
                "image-fit",
                match state.image_options.fit {
                    bs::ImageFit::Contain => "完整显示",
                    bs::ImageFit::Cover => "居中裁切",
                },
                BsAction::Fit,
                Btn::Secondary,
            ),
            self.bs_button(
                "image-dither",
                if state.image_options.dither {
                    "抖动：开"
                } else {
                    "抖动：关"
                },
                BsAction::Dither,
                Btn::Secondary,
            ),
            self.bs_button(
                "image-threshold",
                if state.image_options.threshold.is_some() {
                    "黑白模式"
                } else {
                    "灰度模式"
                },
                BsAction::Threshold,
                Btn::Secondary,
            ),
            self.bs_button(
                "image-invert",
                if state.config.invert {
                    "反色：开"
                } else {
                    "反色：关"
                },
                BsAction::Invert,
                Btn::Secondary,
            ),
            self.bs_button_icon(
                "image-rot90",
                "旋转 90°",
                UiIcon::Refresh,
                BsAction::RotateImage90,
                Btn::Secondary,
            ),
            self.bs_button_icon(
                "image-fliph",
                "水平翻转",
                UiIcon::Flip,
                BsAction::FlipImageH,
                Btn::Secondary,
            ),
        ]));
        parts.push(self.wrap(vec![
            self.bs_button(
                "thresh-auto",
                "自动阈值",
                BsAction::SetThresholdValue(None),
                if state.image_options.threshold.is_none() {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.bs_button(
                "thresh-96",
                "柔和 96",
                BsAction::SetThresholdValue(Some(96)),
                if state.image_options.threshold == Some(96) {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.bs_button(
                "thresh-128",
                "标准 128",
                BsAction::SetThresholdValue(Some(128)),
                if state.image_options.threshold == Some(128) {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.bs_button(
                "thresh-160",
                "高对比 160",
                BsAction::SetThresholdValue(Some(160)),
                if state.image_options.threshold == Some(160) {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.bs_button(
                "thresh-192",
                "极简 192",
                BsAction::SetThresholdValue(Some(192)),
                if state.image_options.threshold == Some(192) {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
        ]));
        parts.push(self.text(
            "抖动保留明暗层次；多档二值化阈值可强化不同明暗轮廓，支持 90° 旋转与水平翻转。",
            CAPTION,
            p.text3,
        ));
        parts.push(self.bs_recent_gallery(24));
        self.vstack(S3, parts)
    }
    fn bs_android(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        let mut parts = vec![
            self.bs_title(UiIcon::Smartphone, "让背屏跟着手机做事"),
            self.text(
                "系统功能由 Android 管理；应用动画由桌面监听当前手机前台应用。",
                CAPTION,
                p.text2,
            ),
        ];
        for feature in [
            bs::NativeFeature::Notifications,
            bs::NativeFeature::Clock,
            bs::NativeFeature::Camera,
        ] {
            let selected = state.native_feature == Some(feature);
            let icon = match feature {
                bs::NativeFeature::Notifications => UiIcon::Bell,
                bs::NativeFeature::Clock => UiIcon::Clock,
                bs::NativeFeature::Camera => UiIcon::Camera,
            };
            parts.push(self.panel(vec![
                self.between(
                    self.bs_title(icon, feature.label()),
                    self.bs_button(
                        &format!("native-{}", feature.key()),
                        if selected { "已选择" } else { "选择" },
                        BsAction::SelectNative(feature),
                        Btn::Secondary,
                    ),
                ),
                self.text(
                    super::super::backscreen_state::native_description(feature),
                    CAPTION,
                    p.text2,
                ),
            ]));
            if feature == bs::NativeFeature::Camera {
                parts.push(self.bs_button(
                    "camera-off",
                    "关闭相机背屏联动",
                    BsAction::CameraOff,
                    Btn::Secondary,
                ));
            }
        }
        parts.push(self.divider());
        parts.push(self.between(
            self.bs_title(UiIcon::Apps, "应用专属动画"),
            self.button("bs-refresh-apps", "读取应用", Action::Apps, Btn::Secondary),
        ));
        parts.push(self.text(format!("当前动画：{}。先在图库挑选图案，再选择应用；播放期间，进入该应用显示动画，离开后恢复原显示。", state.selection().unwrap_or(bs::Preset::Meteor).label()), CAPTION, p.text2));
        let apps: Vec<_> = self
            .apps
            .iter()
            .filter(|app| app.launchable && app.enabled)
            .collect();
        if apps.is_empty() {
            parts.push(self.text(
                "点击读取应用，获取手机中可启动的应用列表。",
                CAPTION,
                p.text3,
            ));
        } else {
            for app in apps {
                let selected = matches!(&state.source, bs::Source::App {package, ..} if package == &app.package);
                let icon = if let Some(path) = &app.icon_path {
                    ConstrainedBox::new(
                        Image::new(
                            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                                path: path.to_string_lossy().into_owned(),
                            },
                            CacheOption::BySize,
                        )
                        .contain()
                        .finish(),
                    )
                    .with_width(28.)
                    .with_height(28.)
                    .finish()
                } else {
                    self.ui_icon(UiIcon::Package, 28., p.text3)
                };
                let pkg_click = app.package.clone();
                let lbl_click = app.label.clone();
                let pkg_dbl = app.package.clone();
                let lbl_dbl = app.label.clone();
                let pkg_menu = app.package.clone();
                let lbl_menu = app.label.clone();
                let row_panel = self.panel(vec![self.between(
                    self.hstack(
                        S2,
                        vec![
                            icon,
                            Expanded::new(
                                1.,
                                self.vstack(
                                    S1,
                                    vec![
                                        self.line_w(&app.label, BODY, p.text, Weight::Medium),
                                        self.line(&app.package, MICRO, p.text3),
                                    ],
                                ),
                            )
                            .finish(),
                        ],
                    ),
                    self.bs_button(
                        &format!("app-{}", app.package),
                        if selected { "已选择" } else { "关联" },
                        BsAction::SelectApp(app.package.clone(), app.label.clone()),
                        Btn::Secondary,
                    ),
                )]);
                parts.push(
                    Hoverable::new(
                        self.mouse_state(format!("bs-app-row-{}", app.package)),
                        move |_| row_panel,
                    )
                    .with_defer_events_to_children()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(BsAction::SelectApp(
                            pkg_click.clone(),
                            lbl_click.clone(),
                        )))
                    })
                    .on_double_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(BsAction::SelectApp(
                            pkg_dbl.clone(),
                            lbl_dbl.clone(),
                        )))
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::BackScreenApp {
                                label: lbl_menu.clone(),
                                package: pkg_menu.clone(),
                            },
                            at,
                        ))
                    })
                    .finish(),
                );
            }
        }
        self.vstack(S3, parts)
    }
    fn bs_settings(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        if state.native_feature.is_some() {
            return self.panel(vec![self.bs_title(UiIcon::Settings, "由手机系统管理"), self.text("系统联动采用手机现有的亮屏、亮度与勿扰设置。这里的动画速度和播放时长不影响手机通知、相机或原生时钟。", CAPTION, p.text2)]);
        }
        let brightness = self.segmented(
            "bs-brightness",
            (1..=8)
                .map(|level| {
                    (
                        level.to_string(),
                        level == state.config.brightness,
                        Action::BackScreen(BsAction::Brightness(level)),
                    )
                })
                .collect(),
        );
        let speed = self.segmented(
            "bs-speed",
            [1, 3, 6, 10]
                .into_iter()
                .map(|fps| {
                    (
                        format!("{}", fps),
                        fps == state.config.fps,
                        Action::BackScreen(BsAction::Fps(fps)),
                    )
                })
                .collect(),
        );
        let length = self.segmented(
            "bs-length",
            [5, 15, 30, 60, 300, 1500]
                .into_iter()
                .map(|s| {
                    (
                        if s < 60 {
                            format!("{s} 秒")
                        } else {
                            format!("{} 分", s / 60)
                        },
                        s == state.config.duration_secs,
                        Action::BackScreen(BsAction::Duration(s)),
                    )
                })
                .collect(),
        );
        let dynamic = matches!(
            state.source,
            bs::Source::Preset(bs::Preset::Clock | bs::Preset::Battery | bs::Preset::Countdown)
        );
        let mut settings = vec![self.between(self.bs_title(UiIcon::Sun, "亮度"), brightness)];
        if !dynamic {
            settings.push(self.between(
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Wave, 16., p.text2),
                        self.line(
                            format!("帧率 · {} 帧/秒", state.config.fps),
                            CAPTION,
                            p.text2,
                        ),
                    ],
                ),
                speed,
            ));
        }
        if state.temporary_app()
            || matches!(state.source, bs::Source::Preset(bs::Preset::Countdown))
        {
            settings.push(self.between(
                self.bs_title(
                    UiIcon::Clock,
                    if state.temporary_app() {
                        "临时联动时长"
                    } else {
                        "倒计时时长"
                    },
                ),
                length,
            ));
        } else {
            settings.push(self.text("应用后持续显示；停止或应用新图案时更换。", MICRO, p.text3));
        }
        self.panel(settings)
    }
    fn bs_games(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        let mut rounds: Vec<_> = (1..=5)
            .rev()
            .map(|width| self.pill_badge(format!("{width} 格"), p.text, p.surface))
            .collect();
        rounds.push(self.pill_badge("回到 5 格", p.text2, p.surface));
        let steps = self.wrap(rounds);
        let mut game = vec![
            self.between(
                self.bs_title(UiIcon::Play, "重力对齐"),
                self.pill_badge("体感小游戏", p.success, p.surface),
            ),
            self.text(
                "轻敲三下开始。面对背屏，向左倾斜时长条向左，向右倾斜时长条向右，让它接入底部槽位。",
                BODY,
                p.text2,
            ),
            self.divider(),
            self.line_w("循环挑战，累计积分", BODY, p.text, Weight::Semibold),
            steps,
            self.text(
                "成功对齐获得积分，宽度按 5 → 4 → 3 → 2 → 1 循环，完成 1 格后继续下一轮。",
                CAPTION,
                p.text2,
            ),
            self.text(
                "想结束时，将手机翻到正面，自动退出游戏并恢复原背屏。深度休眠时先拿起或唤醒手机。",
                CAPTION,
                p.text2,
            ),
        ];
        if self.selected.is_some() && !state.companion_busy {
            game.push(self.wrap(vec![
                self.bs_button("game-now", "立即开始一局", BsAction::GameStart, Btn::Primary),
                self.bs_button(
                    "game-arm-right",
                    "启用三敲启动",
                    BsAction::GameArm,
                    Btn::Secondary,
                ),
            ]));
        }
        let mut companion = vec![
            self.between(
                self.bs_title(UiIcon::Phone, "StepX伴侣"),
                self.pill_badge(
                    if state.companion_installed == Some(true) {
                        "已安装"
                    } else if state.companion_installed == Some(false) {
                        "待安装"
                    } else {
                        "待检测"
                    },
                    if state.companion_installed == Some(true) {
                        p.success
                    } else {
                        p.text3
                    },
                    p.surface,
                ),
            ),
            self.wrap(vec![
                self.pill_badge(
                    if state.companion_installed == Some(true) {
                        "伴侣应用：已安装"
                    } else {
                        "伴侣应用：待检测"
                    },
                    if state.companion_installed == Some(true) {
                        p.success_text
                    } else {
                        p.text2
                    },
                    if state.companion_installed == Some(true) {
                        p.success_soft
                    } else {
                        p.surface
                    },
                ),
                self.pill_badge(
                    if state.companion_permission {
                        "背屏驱动：已就绪"
                    } else {
                        "背屏驱动：点击应用自动激活"
                    },
                    if state.companion_permission {
                        p.success_text
                    } else {
                        p.accent
                    },
                    if state.companion_permission {
                        p.success_soft
                    } else {
                        p.accent_soft
                    },
                ),
                self.pill_badge("姿态传感：三敲与重力感应", p.text2, p.surface),
            ]),
            self.text(
                "游戏在手机本地运行，传感器控制与动画无需电脑往返。点击上方开始后将自动激活背屏驱动。",
                CAPTION,
                p.text3,
            ),
        ];
        if !state.companion_busy {
            companion.push(self.wrap(vec![
                self.bs_button(
                    "companion-refresh",
                    "检测伴侣",
                    BsAction::CompanionInspect,
                    Btn::Secondary,
                ),
                self.bs_button(
                    "companion-install",
                    if state.companion_installed == Some(true) {
                        "更新或修复伴侣"
                    } else {
                        "安装 StepX伴侣"
                    },
                    BsAction::CompanionInstall,
                    Btn::Secondary,
                ),
            ]));
        }
        if let Some(error) = &state.error {
            companion.push(self.text(error, CAPTION, p.danger));
        }
        self.vstack(S4, vec![self.panel(game), self.panel(companion)])
    }
    fn bs_advanced(&self) -> El {
        let p = self.p();
        let mut settings = vec![
            self.between(
                self.bs_title(UiIcon::Settings, "手机默认设置"),
                self.bs_button("advanced-close", "收起设置", BsAction::Advanced, Btn::Ghost),
            ),
            self.text(
                "以下按钮直接保存到手机，即刻生效。要修改图案，请返回预览后点击「应用到背屏」。",
                CAPTION,
                p.text2,
            ),
            self.wrap(vec![
                self.bs_button(
                    "native-auto",
                    "自动亮屏",
                    BsAction::DisplayMode("auto".into()),
                    Btn::Secondary,
                ),
                self.bs_button(
                    "native-always",
                    "常亮",
                    BsAction::DisplayMode("always".into()),
                    Btn::Secondary,
                ),
                self.bs_button(
                    "native-schedule",
                    "按手机设置的时段",
                    BsAction::DisplayMode("custom".into()),
                    Btn::Secondary,
                ),
                self.bs_button(
                    "native-brightness",
                    &format!("保存 {} 级亮度", self.back_screen.config.brightness),
                    BsAction::SaveBrightness,
                    Btn::Secondary,
                ),
            ]),
            self.input(F_BACK_MARQUEE, "输入英文或数字个性文字", None),
            self.wrap(vec![
                self.bs_button(
                    "bs-marquee-preset-stepx",
                    "预设: STEPX AI",
                    BsAction::SetMarqueePreset("STEPX AI".into()),
                    Btn::Ghost,
                ),
                self.bs_button(
                    "bs-marquee-preset-focus",
                    "预设: FOCUS ON",
                    BsAction::SetMarqueePreset("FOCUS ON".into()),
                    Btn::Ghost,
                ),
                self.bs_button(
                    "bs-marquee-preset-coffee",
                    "预设: COFFEE TIME",
                    BsAction::SetMarqueePreset("COFFEE TIME".into()),
                    Btn::Ghost,
                ),
            ]),
            self.wrap(vec![
                self.bs_button(
                    "native-scroll",
                    if self.matrix_marquee_on {
                        "滚动文字：开"
                    } else {
                        "滚动文字：关"
                    },
                    BsAction::ToggleMarquee,
                    Btn::Secondary,
                ),
                self.bs_button(
                    "native-save-text",
                    "保存到手机背屏",
                    BsAction::SaveMarquee,
                    Btn::Secondary,
                ),
            ]),
            self.text(
                "手机原生字库支持英文、数字和简单标点；中文图案可通过 Amoo 图片创作。",
                MICRO,
                p.text3,
            ),
            self.divider(),
            self.bs_title(UiIcon::Matrix, "背屏控制说明"),
            self.text(
                "预览是桌面模拟。应用后由 StepX伴侣在手机上播放；应用专属临时联动需要保持桌面连接。",
                CAPTION,
                p.text2,
            ),
            self.text(
                "单色 8-bit 灰度 · 17 列 × 23 行 · 391 个地址 / 379 颗灯珠；四角缺口自动留空。",
                CAPTION,
                p.text3,
            ),
            self.text(
                "时钟、电量应用后在手机本地读取；预览时钟使用电脑时间。律动动画为内置视觉效果。",
                CAPTION,
                p.text3,
            ),
        ];
        settings.push(self.divider());
        if self.back_screen.clear_confirmation {
            settings.push(self.text(
                "将清除手机当前自定义背屏图案，恢复系统显示。固件无法导出原图，旧图案不能还原。",
                CAPTION,
                p.warning,
            ));
            settings.push(self.wrap(vec![
                self.bs_button(
                    "clear-cancel",
                    "取消",
                    BsAction::CancelClear,
                    Btn::Secondary,
                ),
                self.bs_button(
                    "clear-confirm",
                    "清除旧图案",
                    BsAction::ClearFrame,
                    Btn::Danger,
                ),
            ]));
        } else {
            settings.push(self.bs_button(
                "clear-prepare",
                "清除旧自定义图案…",
                BsAction::PrepareClear,
                Btn::Secondary,
            ));
        }
        self.panel(settings)
    }
    fn bs_smart_linkage_and_pomodoro_panel(&self) -> El {
        let p = self.p();
        let state = &self.back_screen;
        let rem = state.pomodoro_remaining_secs();
        let total = state.pomodoro_duration_secs.max(1);
        let progress_pct = if state.pomodoro_active {
            ((total.saturating_sub(rem)).saturating_mul(100) / total).min(100) as u8
        } else {
            0
        };
        let time_str = format!("{:02}:{:02}", rem / 60, rem % 60);

        let linkage_row = self.vstack(
            S2,
            vec![
                self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.line_w(
                                "Amoo 思考时自动开启呼吸光环",
                                BODY,
                                p.text,
                                Weight::Medium,
                            ),
                            self.line(
                                "桌面或手机端 Amoo 推理期间，背屏自动切入柔和呼吸光环，任务完成自动恢复。",
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                    self.bs_button(
                        "toggle-ai-breath",
                        if state.auto_ai_breath {
                            "已开启 · 呼吸联动"
                        } else {
                            "已关闭 · 点击开启"
                        },
                        BsAction::ToggleAutoAiBreath,
                        if state.auto_ai_breath {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                ),
                self.divider(),
                self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.line_w(
                                "收到验证码/重要消息时背屏闪烁",
                                BODY,
                                p.text,
                                Weight::Medium,
                            ),
                            self.line(
                                "检测到短信验证码、快递取件码或高优通知时，背屏点阵脉冲闪烁提醒。",
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                    self.bs_button(
                        "toggle-notify-flash",
                        if state.auto_notify_flash {
                            "已开启 · 闪烁提醒"
                        } else {
                            "已关闭 · 点击开启"
                        },
                        BsAction::ToggleAutoNotifyFlash,
                        if state.auto_notify_flash {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                ),
            ],
        );

        let pomodoro_card = Container::new(self.vstack(
            S2,
            vec![
                self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::Clock, 16., p.accent),
                            self.line_w(
                                "25 分钟背屏专注番茄钟",
                                BODY,
                                p.text,
                                Weight::Semibold,
                            ),
                            self.pill_badge(
                                if state.pomodoro_active {
                                    "专注倒计时中"
                                } else {
                                    "准备就绪"
                                },
                                if state.pomodoro_active {
                                    p.success
                                } else {
                                    p.text2
                                },
                                p.bg,
                            ),
                        ],
                    ),
                    self.mono(&time_str, TITLE, if state.pomodoro_active { p.accent } else { p.text }),
                ),
                self.progress(Some(progress_pct), p.accent),
                self.text(
                    if state.pomodoro_active {
                        format!(
                            "剩余 {} · 手机物理背屏正在同步显示 Countdown 像素倒计时，计时结束自动恢复原状。",
                            time_str
                        )
                    } else {
                        "一键在桌面与手机物理背屏同步开启 25 分钟专注番茄钟或 5 分钟小憩倒计时。"
                            .into()
                    },
                    CAPTION,
                    p.text2,
                ),
                self.wrap(vec![
                    self.bs_button(
                        "pomodoro-start-25",
                        "开启 25 分钟专注 (背屏同步倒计时)",
                        BsAction::StartPomodoro(1500),
                        Btn::Primary,
                    ),
                    self.bs_button(
                        "pomodoro-start-5",
                        "5 分钟小憩",
                        BsAction::StartPomodoro(300),
                        Btn::Secondary,
                    ),
                    self.bs_button(
                        "pomodoro-stop",
                        "结束专注",
                        BsAction::StopPomodoro,
                        if state.pomodoro_active {
                            Btn::Danger
                        } else {
                            Btn::Ghost
                        },
                    ),
                ]),
            ],
        ))
        .with_uniform_padding(S3)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();

        self.panel(vec![
            self.between(
                self.bs_title(UiIcon::Star, "背屏场景化自动律动 & 25 分钟专注番茄钟"),
                self.pill_badge(
                    if state.pomodoro_active {
                        "番茄钟运行中"
                    } else if state.auto_ai_breath || state.auto_notify_flash {
                        "智能律动已启用"
                    } else {
                        "手动模式"
                    },
                    p.accent,
                    p.accent_soft,
                ),
            ),
            linkage_row,
            pomodoro_card,
        ])
    }
    pub(in super::super) fn backscreen_page(&self) -> El {
        let state = &self.back_screen;
        let tabs = self.segmented(
            "bs-tabs",
            vec![
                (
                    "灵感图库".into(),
                    state.tab == 0,
                    Action::BackScreen(BsAction::Tab(0)),
                ),
                (
                    "Amoo 创作".into(),
                    state.tab == 1,
                    Action::BackScreen(BsAction::Tab(1)),
                ),
                (
                    "图片转像素".into(),
                    state.tab == 2,
                    Action::BackScreen(BsAction::Tab(2)),
                ),
                (
                    "手机联动".into(),
                    state.tab == 3,
                    Action::BackScreen(BsAction::Tab(3)),
                ),
                (
                    "小游戏".into(),
                    state.tab == 4,
                    Action::BackScreen(BsAction::Tab(4)),
                ),
            ],
        );
        let content = match state.tab {
            1 => self.bs_amoo(),
            2 => self.bs_images(),
            3 => self.bs_android(),
            4 => self.bs_games(),
            _ => self.bs_gallery(),
        };
        let mut creative = Vec::new();
        if state.advanced {
            creative.push(self.bs_advanced());
        }
        if state.tab == 3 {
            creative.push(self.bs_smart_linkage_and_pomodoro_panel());
        }
        creative.push(self.sub_enter(content));
        let right = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S3)
            .with_child(tabs)
            .with_child(
                Expanded::new(
                    1.,
                    self.pane_scroll(
                        state.scroll.clone(),
                        Container::new(self.vstack(S4, creative))
                            .with_padding_right(S2)
                            .with_padding_bottom(S4)
                            .finish(),
                    ),
                )
                .finish(),
            )
            .with_child(if state.tab == 4 {
                Empty::new().finish()
            } else {
                self.bs_settings()
            })
            .finish();
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S4)
            .with_child(self.fixed_w(268., self.bs_preview()))
            .with_child(Expanded::new(1., right).finish())
            .finish()
    }
}

fn game_preview_frame(elapsed_ms: u64) -> Vec<u8> {
    let mut frame = vec![0; bs::SLOTS];
    let mut remaining = elapsed_ms % 50_000;
    let mut width = 5;
    let mut points = (elapsed_ms / 50_000) * 150;
    for (index, duration) in [12_000, 11_000, 10_000, 9_000, 8_000]
        .into_iter()
        .enumerate()
    {
        width = 5 - index;
        if remaining < duration {
            break;
        }
        remaining -= duration;
        points += width as u64 * 10;
    }
    let phase = remaining as f32 / ((7 + width) * 1000) as f32;
    const DIGITS: [&str; 10] = [
        "111101101101111",
        "010110010010111",
        "111001111100111",
        "111001111001111",
        "101101111001001",
        "111100111001111",
        "111100111101111",
        "111001001001001",
        "111101111101111",
        "111101111001111",
    ];
    for (column, digit) in format!("{:04}", points % 10_000).bytes().enumerate() {
        for (pixel, bit) in DIGITS[(digit - b'0') as usize].bytes().enumerate() {
            if bit == b'1' {
                frame[(1 + pixel / 3) * bs::WIDTH + 1 + column * 4 + pixel % 3] = 160;
            }
        }
    }
    let target_x = (bs::WIDTH - width) / 2;
    let bar_x = (target_x as f32 + (phase * std::f32::consts::TAU).sin() * 4.).round() as usize;
    let bar_y = 7 + (phase * 13.) as usize;
    for x in 1..16 {
        frame[21 * bs::WIDTH + x] = 50;
    }
    for x in target_x..target_x + width {
        frame[20 * bs::WIDTH + x] = 90;
        frame[21 * bs::WIDTH + x] = 180;
    }
    for x in bar_x..bar_x + width {
        frame[bar_y * bs::WIDTH + x] = 255;
    }
    frame
}

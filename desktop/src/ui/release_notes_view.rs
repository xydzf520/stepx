//! Value-led release catalogue. Persistent directory; one scrollable body and
//! an inline, bounded native player. Feature CTAs only navigate.
use super::{
    desktop_update_state::DesktopUpdateAction as Update, icons::UiIcon, theme::*, widgets::*,
    Action, Companion,
};
use facloud_ui::{assets::asset_cache::AssetSource, elements::*, fonts::Weight};
use stepx_companion::release_notes::{self, Category, Destination, Feature, ReleaseNotes};

fn destination_icon(d: Destination) -> UiIcon {
    match d {
        Destination::Home | Destination::Calendar => UiIcon::Calendar,
        Destination::Search => UiIcon::Search,
        Destination::Messages | Destination::Notifications => UiIcon::Chat,
        Destination::Files | Destination::Storage => UiIcon::Folder,
        Destination::Agent | Destination::Amoo => UiIcon::Sparkles,
        Destination::Phone => UiIcon::Phone,
        Destination::About => UiIcon::Users,
        Destination::History => UiIcon::File,
        Destination::Studio | Destination::AppStore => UiIcon::Apps,
        Destination::Alarms => UiIcon::Clock,
        Destination::Gallery => UiIcon::Image,
        Destination::BackScreen => UiIcon::Matrix,
        Destination::Tools => UiIcon::Tools,
        Destination::Connect => UiIcon::Wifi,
        Destination::Account => UiIcon::User,
        Destination::Settings => UiIcon::Settings,
    }
}
fn category_icon(id: &str) -> UiIcon {
    match id {
        "alerts" => UiIcon::Bell,
        "capsule" => UiIcon::Bolt,
        "workbench" => UiIcon::Home,
        "performance" => UiIcon::Cpu,
        "search" => UiIcon::Search,
        "communication" => UiIcon::MessageCircle,
        "files" => UiIcon::Folder,
        "ai" => UiIcon::Sparkles,
        "planning" => UiIcon::Calendar,
        "phone-tools" => UiIcon::Smartphone,
        _ => UiIcon::Shield,
    }
}
fn category_label(id: &str) -> &'static str {
    match id {
        "alerts" => "弹窗",
        "capsule" => "胶囊",
        "workbench" => "首页工作台",
        "files" => "最近文件",
        "search" => "全局搜索",
        "communication" => "消息电话",
        "ai" => "AI 成果",
        "planning" => "安排",
        "phone-tools" => "手机工具",
        "performance" => "性能",
        _ => "连接与数据",
    }
}
fn bundled_image(path: &str) -> &'static str {
    match path {
        "release-notes/home.png" => "release-notes/home.png",
        "release-notes/search.png" => "release-notes/search.png",
        "release-notes/messages.png" => "release-notes/messages.png",
        "release-notes/files.png" => "release-notes/files.png",
        "release-notes/agent.png" => "release-notes/agent.png",
        "release-notes/capsule.png" => "release-notes/capsule.png",
        "release-notes/connection.png" => "release-notes/connection.png",
        "release-notes/tools.png" => "release-notes/tools.png",
        _ => "release-notes/phone.png",
    }
}
impl Companion {
    fn release_explore(&self, id: &str, label: &str, destination: Destination, style: Btn) -> El {
        self.button_icon(
            id,
            label,
            destination_icon(destination),
            Action::DesktopUpdate(Update::Explore(destination)),
            style,
        )
    }
    fn release_picture(&self, path: &str, height: f32) -> El {
        let p = self.p();
        Container::new(
            ConstrainedBox::new(
                Image::new(
                    AssetSource::Bundled {
                        path: bundled_image(path),
                    },
                    CacheOption::BySize,
                )
                .contain()
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
            .with_height(height)
            .finish(),
        )
        .with_uniform_padding(S2)
        .with_background_color(p.surface)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish()
    }
    fn release_author(&self) -> El {
        // Job seeking is the author's purpose for the project. Preserve the
        // complete original introduction, contact actions and both QR cards.
        self.personal_and_group_qr_cards("release-intro")
    }
    fn release_home(&self) -> El {
        self.vstack(
            S5,
            vec![
                self.release_author(),
                self.divider(),
                self.section_heading(UiIcon::Sparkles, "继续了解这次更新"),
                self.wrap(vec![
                    self.button_icon(
                        "release-home-overview",
                        "查看重点更新",
                        UiIcon::ArrowRight,
                        Action::DesktopUpdate(Update::Category(String::new())),
                        Btn::Primary,
                    ),
                    self.button_icon(
                        "release-home-alerts",
                        "桌面弹窗",
                        UiIcon::Bell,
                        Action::DesktopUpdate(Update::Category("alerts".into())),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "release-home-files",
                        "最近文件",
                        UiIcon::Folder,
                        Action::DesktopUpdate(Update::Category("files".into())),
                        Btn::Secondary,
                    ),
                ]),
            ],
        )
    }
    fn release_video_card(&self, notes: &ReleaseNotes) -> El {
        let p = self.p();
        let state = &self.desktop_update;
        let Some(video) = &notes.video else {
            return Empty::new().finish();
        };
        let source = if state.video_ready {
            AssetSource::Raw {
                id: stepx_companion::release_video::FRAME_ASSET.into(),
            }
        } else {
            AssetSource::Bundled {
                path: bundled_image(&video.poster),
            }
        };
        let image = ConstrainedBox::new(
            Image::new(source, CacheOption::BySize)
                .contain()
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        )
        .with_height(if state.video_expanded { 370. } else { 220. })
        .finish();
        let seconds = state.video_frame / stepx_companion::release_video::FPS;
        let controls = self.wrap(vec![
            self.button_icon(
                "release-play-video",
                if state.video_error.is_some() {
                    "重试播放"
                } else if state.video_playing {
                    "暂停"
                } else {
                    "播放"
                },
                if state.video_playing {
                    UiIcon::Pause
                } else {
                    UiIcon::Play
                },
                Action::DesktopUpdate(Update::PlayVideo),
                Btn::Primary,
            ),
            self.button(
                "release-video-back",
                "−5 秒",
                Action::DesktopUpdate(Update::SeekVideo(-5)),
                Btn::Ghost,
            ),
            self.button(
                "release-video-forward",
                "+5 秒",
                Action::DesktopUpdate(Update::SeekVideo(5)),
                Btn::Ghost,
            ),
            self.line(
                format!(
                    "{}:{:02} / {}:{:02}",
                    seconds / 60,
                    seconds % 60,
                    video.duration_seconds / 60,
                    video.duration_seconds % 60
                ),
                CAPTION,
                p.text2,
            ),
            self.button_icon(
                "release-video-expand",
                if state.video_expanded {
                    "返回更新"
                } else {
                    "放大"
                },
                if state.video_expanded {
                    UiIcon::Restore
                } else {
                    UiIcon::Maximize
                },
                Action::DesktopUpdate(Update::ExpandVideo),
                Btn::Ghost,
            ),
        ]);
        let mut rows = vec![
            self.section_heading(UiIcon::Video, &video.title),
            image,
            self.wrap(
                video
                    .chapters
                    .iter()
                    .map(|chapter| {
                        self.button(
                            &format!("release-video-chapter-{}", chapter.second),
                            format!("{} · {}s", chapter.title, chapter.second),
                            Action::DesktopUpdate(Update::SeekVideoTo(chapter.second)),
                            Btn::Ghost,
                        )
                    })
                    .collect(),
            ),
            self.progress(
                Some(
                    ((state.video_frame * 100) / stepx_companion::release_video::FRAME_COUNT)
                        .min(100) as u8,
                ),
                p.blue,
            ),
            controls,
            self.text(
                "真实客户端操作录屏 · 演示数据 · 无声音 · 当前窗口内播放",
                MICRO,
                p.text3,
            ),
        ];
        if let Some(error) = &state.video_error {
            rows.push(self.text(format!("暂时无法播放：{error}"), CAPTION, p.danger));
        }
        let card = self.panel(rows);
        #[cfg(feature = "ui-capture")]
        let card = super::scenario_check::control("release-video-card".into(), card);
        card
    }
    fn release_feature(&self, item: &Feature) -> El {
        let p = self.p();
        let mut rows = vec![
            self.hstack(
                S2,
                vec![
                    self.section_icon_badge(destination_icon(item.destination), 18., 36.),
                    self.pill_badge(&item.kind, p.blue, p.accent_soft),
                ],
            ),
            self.text_w(&item.title, 18., p.text, Weight::Semibold),
            self.text(&item.benefit, BODY, p.text2),
        ];
        for line in &item.details {
            rows.push(self.text(format!("· {line}"), CAPTION, p.text2));
        }
        if let Some(condition) = &item.condition {
            rows.push(
                Container::new(self.text(condition, MICRO, p.text3))
                    .with_uniform_padding(S2)
                    .with_background_color(p.surface)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            );
        }
        rows.push(self.release_explore(
            &format!("release-explore-{}", item.id),
            "前往使用",
            item.destination,
            Btn::Ghost,
        ));
        let card = Container::new(self.vstack(S2, rows))
            .with_uniform_padding(S4)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();
        #[cfg(feature = "ui-capture")]
        let card = super::scenario_check::control(format!("release-feature-{}", item.id), card);
        card
    }
    fn release_category_content(&self, c: &Category) -> El {
        let p = self.p();
        let mut rows = vec![self.responsive_columns(
            720.,
            S4,
            S3,
            CrossAxisAlignment::Center,
            vec![
                (
                    1.,
                    self.vstack(
                        S3,
                        vec![
                            self.section_icon_badge(category_icon(&c.id), 28., 52.),
                            self.text_w(&c.title, 30., p.text, Weight::Semibold),
                            self.text(&c.outcome, BODY, p.text2),
                            self.line(
                                format!("{} 项更新 · 按日常使用顺序", c.features.len()),
                                CAPTION,
                                p.text3,
                            ),
                        ],
                    ),
                ),
                (
                    1.15,
                    self.vstack(
                        S1,
                        vec![
                            self.release_picture(&c.image, 185.),
                            self.text("实际客户端界面 · 演示数据", MICRO, p.text3),
                        ],
                    ),
                ),
            ],
        )];
        if c.id == "performance" {
            let metric = |label: &str, before: &str, after: &str, note: &str| {
                self.panel(vec![
                    self.text_w(label, SUBHEAD, p.text, Weight::Semibold),
                    self.hstack(
                        S3,
                        vec![
                            self.text(before, 22., p.text3),
                            self.content_icon(UiIcon::ArrowRight, 18., p.text3),
                            self.text_w(after, 25., p.blue, Weight::Semibold),
                        ],
                    ),
                    self.text(note, CAPTION, p.text2),
                ])
            };
            rows.push(self.section_heading(UiIcon::Chart, "有同数据实测依据的改善"));
            rows.push(self.responsive_columns(
                700.,
                S3,
                S3,
                CrossAxisAlignment::Start,
                vec![
                    (
                        1.,
                        metric(
                            "批量热快照写入",
                            "44.054 ms",
                            "8.545 ms",
                            "20,000 条中改 1,000 条；优化前 → 优化后。",
                        ),
                    ),
                    (
                        1.,
                        metric(
                            "单条变更的保存成本",
                            "3.229 ms",
                            "0.059 ms",
                            "同轮完整 JSON → 增量含构造与读头；20,000 条中改 1 条。",
                        ),
                    ),
                ],
            ));
            rows.push(self.text("0.2.154 · 同机 Linux release · 合成数据 · 30 次测量中位数。这是对应存储操作的结果，不代表整机或页面帧率；完整批量快照仍可能比 JSON 慢。", MICRO, p.text3));
        }
        for pair in c.features.chunks(2) {
            rows.push(self.responsive_columns(
                700.,
                S3,
                S3,
                CrossAxisAlignment::Start,
                pair.iter().map(|f| (1., self.release_feature(f))).collect(),
            ));
        }
        rows.push(self.button_icon(
            "release-back-overview",
            "返回重点更新",
            UiIcon::ArrowLeft,
            Action::DesktopUpdate(Update::Category(String::new())),
            Btn::Secondary,
        ));
        self.vstack(S4, rows)
    }
    fn release_overview(&self, notes: &ReleaseNotes) -> El {
        let p = self.p();
        let total: usize = notes.categories.iter().map(|c| c.features.len()).sum();
        let hero = Container::new(self.vstack(
            S3,
            vec![
                self.hstack(
                    S2,
                    vec![
                        self.section_icon_badge(UiIcon::Sparkles, 23., 44.),
                        self.pill_badge(
                            format!("{total} 项更新 · {} 类场景", notes.categories.len()),
                            p.blue,
                            p.accent_soft,
                        ),
                    ],
                ),
                self.text_w(&notes.title, 20., p.text, Weight::Semibold),
                self.text(&notes.summary, BODY, p.text2),
                self.wrap(vec![
                    self.button_icon(
                        "release-watch-demo",
                        "看 39 秒操作实录",
                        UiIcon::Play,
                        Action::DesktopUpdate(Update::ExpandVideo),
                        Btn::Primary,
                    ),
                    self.line(
                        format!("公开版 {} → 开发版 {}", notes.baseline, notes.version),
                        CAPTION,
                        p.text3,
                    ),
                ]),
            ],
        ))
        .with_uniform_padding(S5)
        .with_background_color(lerp_color(
            p.surface,
            p.blue,
            if p.dark { 0.12 } else { 0.05 },
        ))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();
        #[cfg(feature = "ui-capture")]
        let hero = super::scenario_check::control("release-hero".into(), hero);
        let mut rows = vec![
            hero,
            self.section_heading(UiIcon::Bolt, "这四块，最值得先看"),
        ];
        for pair in notes.highlights.chunks(2) {
            rows.push(
                self.responsive_columns(
                    700.,
                    S3,
                    S3,
                    CrossAxisAlignment::Start,
                    pair.iter()
                        .map(|item| {
                            let card = Container::new(self.vstack(
                                S3,
                                vec![
                                    self.release_picture(&item.image, 155.),
                                    self.text(&item.caption, MICRO, p.text3),
                                    self.section_heading(
                                        category_icon(&item.category),
                                        &item.title,
                                    ),
                                    self.text(&item.summary, CAPTION, p.text2),
                                    self.button_icon(
                                        &format!("release-focus-{}", item.id),
                                        &item.action_label,
                                        UiIcon::ArrowRight,
                                        Action::DesktopUpdate(Update::Category(
                                            item.category.clone(),
                                        )),
                                        Btn::Secondary,
                                    ),
                                ],
                            ))
                            .with_uniform_padding(S3)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish();
                            (1., card)
                        })
                        .collect(),
                ),
            );
        }
        rows.push(self.panel(vec![self.section_heading(UiIcon::Chart, "性能也做了大量改进"),
            self.text("连接复用、中文与商店索引、缓存绘制、长列表分页、后台并行和增量保存，分别减少等待与重复工作。", BODY, p.text2),
            self.text_w("批量热快照 44.054 → 8.545 ms", 22., p.blue, Weight::Semibold),
            self.text("0.2.154 同机 release · 20,000 条中改 1,000 条 · 中位数；完整快照仍可能慢于 JSON，不代表整机提速。", MICRO, p.text3),
            self.button_icon("release-performance-focus", "看全部性能改进与实测", UiIcon::ArrowRight, Action::DesktopUpdate(Update::Category("performance".into())), Btn::Secondary),
        ]));
        rows.push(self.section_heading(UiIcon::Apps, "按你感兴趣的事情看"));
        for pair in notes.categories.chunks(2) {
            rows.push(
                self.responsive_columns(
                    670.,
                    S3,
                    S3,
                    CrossAxisAlignment::Start,
                    pair.iter()
                        .map(|c| {
                            (
                                1.,
                                self.panel(vec![
                                    self.section_heading(category_icon(&c.id), &c.title),
                                    self.text(&c.outcome, CAPTION, p.text2),
                                    self.button_icon(
                                        &format!("release-directory-{}", c.id),
                                        format!("查看 {} 项更新", c.features.len()),
                                        UiIcon::ArrowRight,
                                        Action::DesktopUpdate(Update::Category(c.id.clone())),
                                        Btn::Ghost,
                                    ),
                                ]),
                            )
                        })
                        .collect(),
                ),
            );
        }
        rows.push(self.release_video_card(notes));
        rows.push(self.section_heading(UiIcon::Info, "版本与使用条件"));
        rows.extend(
            notes
                .boundaries
                .iter()
                .map(|s| self.text(s, CAPTION, p.text3)),
        );
        rows.push(self.wrap(vec![
            self.release_explore(
                "release-full-history",
                "逐版本完整记录",
                Destination::History,
                Btn::Secondary,
            ),
            self.button_icon(
                "release-public-baseline",
                "GitHub 公开版",
                UiIcon::ExternalLink,
                Action::StudioOpenUrl(notes.baseline_url.clone()),
                Btn::Ghost,
            ),
            self.release_explore(
                "release-open-about",
                "设置 · 关于",
                Destination::About,
                Btn::Ghost,
            ),
        ]));
        self.vstack(S5, rows)
    }
    fn release_story_content(&self, notes: &ReleaseNotes) -> El {
        let content = if self.desktop_update.video_expanded {
            self.release_video_card(notes)
        } else if let Some(category) = notes
            .categories
            .iter()
            .find(|c| c.id == self.desktop_update.release_category)
        {
            self.release_category_content(category)
        } else {
            self.release_overview(notes)
        };
        Container::new(content)
            .with_padding_top(S2)
            .with_padding_bottom(S5)
            .with_horizontal_padding(S1)
            .finish()
    }
    pub(super) fn release_story_overlay(&self) -> Option<El> {
        if !self.desktop_update.whats_new_open {
            return None;
        }
        let p = self.p();
        let content = if self.desktop_update.release_home {
            self.release_home()
        } else if self.desktop_update.release_history {
            let history = release_notes::history();
            let limit = self.desktop_update.release_history_limit.min(history.len());
            let mut rows = vec![
                self.button_icon(
                    "release-history-back",
                    "返回重点更新",
                    UiIcon::ArrowLeft,
                    Action::DesktopUpdate(Update::BackToStory),
                    Btn::Ghost,
                ),
                self.text_w("逐版本完整记录", 26., p.text, Weight::Semibold),
                self.text(
                    format!("已显示 {limit} / {} 个版本", history.len()),
                    CAPTION,
                    p.text3,
                ),
            ];
            for section in history.iter().take(limit) {
                rows.push(self.text_w(&section.heading, SUBHEAD, p.text, Weight::Semibold));
                for line in &section.lines {
                    rows.push(self.text(line, CAPTION, p.text2));
                }
                rows.push(self.divider());
            }
            if limit < history.len() {
                rows.push(self.button(
                    "release-history-more",
                    "加载更早版本",
                    Action::DesktopUpdate(Update::MoreHistory),
                    Btn::Secondary,
                ));
            }
            self.vstack(S3, rows)
        } else {
            match release_notes::current() {
                Ok(notes) => self.release_story_content(notes),
                Err(_) => self.vstack(
                    S3,
                    vec![
                        self.text("更新说明暂不可用，仍可查看完整文字记录。", BODY, p.text2),
                        self.release_explore(
                            "release-fallback-history",
                            "查看完整文字记录",
                            Destination::History,
                            Btn::Primary,
                        ),
                    ],
                ),
            }
        };
        let header = self.between(
            self.wrap(vec![
                self.text_w("StepX Companion 更新", 16., p.text, Weight::Semibold),
                self.pill_badge(
                    format!("v{}", stepx_companion::build_info::VERSION),
                    p.text2,
                    p.surface,
                ),
            ]),
            self.icon_action(
                "release-story-close",
                "关闭更新说明",
                UiIcon::Close,
                vec![Action::DesktopUpdate(Update::CloseWhatsNew)],
                false,
            ),
        );
        let range = release_notes::current()
            .map(|notes| format!("累计更新：v{} → v{}", notes.baseline, notes.version))
            .unwrap_or_else(|_| format!("当前版本：v{}", stepx_companion::build_info::VERSION));
        let range = self.text_w(range, SUBHEAD, p.blue, Weight::Semibold);
        #[cfg(feature = "ui-capture")]
        let range = super::scenario_check::control("release-version-range".into(), range);
        let revisit_hint = self.hstack(
            S2,
            vec![
                self.ui_icon(UiIcon::Info, 16., p.blue),
                self.text("稍后在「设置 → 关于」回看", SUBHEAD, p.text),
            ],
        );
        #[cfg(feature = "ui-capture")]
        let revisit_hint =
            super::scenario_check::control("release-revisit-hint".into(), revisit_hint);
        let mut parts = vec![header, self.bounded_wrap(vec![range, revisit_hint])];
        let mut buttons = vec![self.button_icon(
            "release-show-author",
            "首页",
            UiIcon::Briefcase,
            Action::DesktopUpdate(Update::ShowAuthor),
            if self.desktop_update.release_home {
                Btn::Primary
            } else {
                Btn::Ghost
            },
        )];
        if let Ok(notes) = release_notes::current() {
            let reading_updates =
                !self.desktop_update.release_home && !self.desktop_update.release_history;
            buttons.push(self.button_icon(
                "release-category-overview",
                "重点更新",
                UiIcon::Bolt,
                Action::DesktopUpdate(Update::Category(String::new())),
                if reading_updates && self.desktop_update.release_category.is_empty() {
                    Btn::Primary
                } else {
                    Btn::Ghost
                },
            ));
            buttons.extend(notes.categories.iter().map(|c| {
                self.button_icon(
                    &format!("release-category-{}", c.id),
                    category_label(&c.id),
                    category_icon(&c.id),
                    Action::DesktopUpdate(Update::Category(c.id.clone())),
                    if reading_updates && self.desktop_update.release_category == c.id {
                        Btn::Primary
                    } else {
                        Btn::Ghost
                    },
                )
            }));
        }
        let directory = self.bounded_wrap(buttons);
        #[cfg(feature = "ui-capture")]
        let directory =
            super::scenario_check::control("release-update-directory-region".into(), directory);
        parts.push(directory);
        let scroll = self.pane_scroll(self.desktop_update.whats_new_scroll.clone(), content);
        #[cfg(feature = "ui-capture")]
        let scroll = super::scenario_check::control("release-story-scroll".into(), scroll);
        parts.extend(vec![
            self.divider(),
            Expanded::new(1., scroll).finish(),
            self.divider(),
            self.hstack(
                S2,
                vec![
                    self.spacer(),
                    self.button(
                        "whats-new-ack",
                        "知道了",
                        Action::DesktopUpdate(Update::CloseWhatsNew),
                        Btn::Primary,
                    ),
                ],
            ),
        ]);
        let sheet = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(S3)
                .with_children(parts)
                .finish(),
        )
        .with_uniform_padding(S4)
        .finish();
        let sheet = ConstrainedBox::new(popover_surface(sheet, p))
            .with_max_width(1080.)
            .with_max_height(880.)
            .finish();
        let sheet = super::motion::enter(
            sheet,
            self.desktop_update.started,
            !self.settings.value.reduced_motion,
        );
        Some(
            Container::new(Align::new(sheet).finish())
                .with_uniform_padding(S4)
                .with_background_color(if p.dark {
                    rgba(0, 0, 0, 152)
                } else {
                    rgba(15, 23, 42, 112)
                })
                .finish(),
        )
    }
}

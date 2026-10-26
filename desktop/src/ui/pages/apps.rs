use super::super::{
    icons::UiIcon, settings_state::SettingsAction, theme::*, widgets::*, Action, Companion,
    ToolPage, F_APP_SEARCH,
};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use stepx_companion::{apps::AppInfo, storage::format_bytes};

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

impl Companion {
    pub(in super::super) fn apps_actions(&self) -> Vec<El> {
        vec![
            self.button("apps-refresh", "刷新列表", Action::Apps, Btn::Secondary),
            self.button(
                "apps-apk-tool",
                "APK 工具箱",
                Action::OpenTool(Some(ToolPage::Apk)),
                Btn::Secondary,
            ),
            self.button_icon(
                "apps-install",
                "安装 APK…",
                UiIcon::Plus,
                Action::InstallApk,
                Btn::Primary,
            ),
        ]
    }

    fn installed_app_icon(&self, app: &AppInfo) -> El {
        let icon = if let Some(path) = app.icon_path.as_ref() {
            Image::new(
                facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish()
        } else {
            Align::new(self.ui_icon(
                if app.package == "com.android.calendar" {
                    UiIcon::Calendar
                } else {
                    UiIcon::Package
                },
                26.,
                self.p().text3,
            ))
            .finish()
        };
        ConstrainedBox::new(icon)
            .with_width(40.)
            .with_height(40.)
            .finish()
    }

    fn installed_app_row(&self, app: &AppInfo) -> El {
        let p = self.p();
        let package = &app.package;
        let version = if !app.version_name.trim().is_empty() {
            format!("版本 {}", app.version_name.trim())
        } else if app.version_code > 0 {
            format!("版本代码 {}", app.version_code)
        } else {
            String::new()
        };
        let mut meta_parts = Vec::new();
        if !version.is_empty() {
            meta_parts.push(version);
        }
        if app.apk_bytes() > 0 {
            meta_parts.push(format!("安装包 {}", format_bytes(app.apk_bytes())));
        }
        if app.apk_files.len() > 1 {
            meta_parts.push(format!("{} 个 APK", app.apk_files.len()));
        }
        if app.icon_path.is_none() && app.icon_error.is_some() {
            meta_parts.push("手机图标暂不可用".to_string());
        }
        let kind = if !app.enabled {
            "已停用"
        } else if app.updated_system {
            "系统应用 · 已更新"
        } else if app.system
            || app.package == "android"
            || app.package.starts_with("com.android.")
            || app.package.starts_with("android.")
            || app.package.starts_with("vendor.")
            || app.package.starts_with("com.qti.")
            || app.package.starts_with("com.qualcomm.")
            || app.package.starts_with("com.stepos.")
            || app.package.starts_with("com.stepx.")
            || (!app.launchable && app.package.starts_with("com.google."))
        {
            "系统应用"
        } else {
            "用户应用"
        };
        let display_label = stepx_companion::apps::clean_app_label(&app.package, &app.label);
        let mut info_lines = vec![
            self.line_w(&display_label, BODY, p.text, Weight::Semibold),
            self.mono(package, MICRO, p.text2),
        ];
        if !meta_parts.is_empty() {
            info_lines.push(self.line(meta_parts.join(" · "), MICRO, p.text3));
        }
        let info = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S3)
            .with_child(self.installed_app_icon(app))
            .with_child(Expanded::new(1., self.vstack(S1, info_lines)).finish())
            .finish();
        let heading = self.between(info, self.pill_badge(kind, p.text2, p.surface));
        let mut actions = Vec::new();
        if app.launchable && app.enabled {
            actions.push(self.button(
                format!("app-open:{package}"),
                "打开",
                Action::OpenApp(package.clone()),
                Btn::Secondary,
            ));
            actions.push(self.button_icon(
                format!("app-open-mirror:{package}"),
                "投屏打开",
                UiIcon::Monitor,
                Action::OpenAppInMirror(package.clone()),
                Btn::Secondary,
            ));
        } else {
            actions.push(self.disabled_button(if app.enabled {
                "无启动入口"
            } else {
                "已停用"
            }));
        }
        actions.push(self.button(
            format!("app-export:{package}"),
            "导出 APK",
            Action::ExportApp(package.clone()),
            Btn::Secondary,
        ));
        actions.push(self.button(
            format!("app-settings:{package}"),
            "应用详情",
            Action::AppSettings(package.clone()),
            Btn::Secondary,
        ));
        actions.push(self.button_icon(
            format!("app-copy:{package}"),
            "复制包名",
            UiIcon::Clipboard,
            Action::CopyText(package.clone(), "已复制应用包名".into()),
            Btn::Secondary,
        ));
        if app.can_uninstall() {
            actions.push(self.button(
                format!("app-stop:{package}"),
                "强行停止",
                Action::ForceStopApp(package.clone()),
                Btn::Ghost,
            ));
            actions.push(self.button(
                format!("app-uninstall:{package}"),
                "卸载",
                Action::RequestUninstallApp(package.clone()),
                Btn::Danger,
            ));
        }
        let mut contents = vec![heading, self.wrap(actions)];
        if self.pending_app_uninstall.as_deref() == Some(package.as_str()) {
            let confirmation = self.wrap(vec![
                self.text(
                    "卸载后，此应用在手机当前用户中的数据会被删除。",
                    CAPTION,
                    p.danger,
                ),
                self.button(
                    format!("app-cancel-uninstall:{package}"),
                    "取消",
                    Action::CancelUninstallApp,
                    Btn::Secondary,
                ),
                self.button(
                    format!("app-confirm-uninstall:{package}"),
                    "确认卸载",
                    Action::ConfirmUninstallApp(package.clone()),
                    Btn::DangerSolid,
                ),
            ]);
            contents.push(
                Container::new(confirmation)
                    .with_uniform_padding(S2)
                    .with_background_color(p.surface)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            );
        }
        let content = self.vstack(S3, contents);
        let package_dbl = package.clone();
        let package_rc = package.clone();
        let can_open = app.launchable && app.enabled;
        Hoverable::new(
            self.mouse_state(format!("app-row:{package}")),
            move |mouse| {
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
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                Container::new(content)
                    .with_horizontal_padding(S3)
                    .with_vertical_padding(S2 + 2.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            },
        )
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_double_click(move |ctx, _, _| {
            if can_open {
                ctx.dispatch_typed_action(Action::OpenApp(package_dbl.clone()));
            } else {
                ctx.dispatch_typed_action(Action::AppSettings(package_dbl.clone()));
            }
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::App(package_rc.clone()),
                at,
            ));
        })
        .finish()
    }

    pub(in super::super) fn apps_page(&self) -> El {
        let p = self.p();
        let is_stepos_or_system = |app: &AppInfo| {
            app.system
                || app.updated_system
                || app.package == "android"
                || app.package.starts_with("com.android.")
                || app.package.starts_with("android.")
                || app.package.starts_with("vendor.")
                || app.package.starts_with("com.qti.")
                || app.package.starts_with("com.qualcomm.")
                || app.package.starts_with("com.stepos.")
                || app.package.starts_with("com.stepx.")
                || (!app.launchable && app.package.starts_with("com.google."))
        };
        let query = self.fields[F_APP_SEARCH].text.trim();
        let mut filtered: Vec<&AppInfo> = match self.settings.apps_filter {
            1 => self
                .apps
                .iter()
                .filter(|app| !is_stepos_or_system(app) && app.matches_query(query))
                .collect(),
            2 => self
                .apps
                .iter()
                .filter(|app| is_stepos_or_system(app) && app.matches_query(query))
                .collect(),
            3 => self
                .apps
                .iter()
                .filter(|app| !app.enabled && app.matches_query(query))
                .collect(),
            _ => self.filtered_apps(),
        };
        if self.settings.apps_sort_by_name {
            filtered.sort_by(|a, b| {
                a.label
                    .to_lowercase()
                    .cmp(&b.label.to_lowercase())
                    .then_with(|| a.package.cmp(&b.package))
            });
        } else {
            filtered.sort_by(|a, b| {
                b.version_code
                    .cmp(&a.version_code)
                    .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            });
        }
        let common_count = self
            .apps
            .iter()
            .filter(|app| app.in_default_scope())
            .count();
        let third_party_count = self
            .apps
            .iter()
            .filter(|app| !is_stepos_or_system(app))
            .count();
        let stepos_sys_count = self
            .apps
            .iter()
            .filter(|app| is_stepos_or_system(app))
            .count();
        let disabled_count = self.apps.iter().filter(|app| !app.enabled).count();
        let mut filter_items = vec![self.segmented(
            "app-source-filter",
            vec![
                (
                    "全部来源".into(),
                    self.settings.apps_filter == 0,
                    Action::Settings(SettingsAction::SetAppsFilter(0)),
                ),
                (
                    format!("第三方应用 ({third_party_count})"),
                    self.settings.apps_filter == 1,
                    Action::Settings(SettingsAction::SetAppsFilter(1)),
                ),
                (
                    format!("StepOS 与系统组件 ({stepos_sys_count})"),
                    self.settings.apps_filter == 2,
                    Action::Settings(SettingsAction::SetAppsFilter(2)),
                ),
                (
                    format!("已停用 ({disabled_count})"),
                    self.settings.apps_filter == 3,
                    Action::Settings(SettingsAction::SetAppsFilter(3)),
                ),
            ],
        )];
        if self.settings.apps_filter == 0
            && (self.apps.len() > common_count || self.include_system_apps)
        {
            filter_items.push(self.segmented(
                "app-scope",
                vec![
                    (
                        format!("常用应用 ({common_count})"),
                        !self.include_system_apps,
                        Action::ToggleSystemApps(false),
                    ),
                    (
                        format!("含系统组件 ({})", self.apps.len()),
                        self.include_system_apps,
                        Action::ToggleSystemApps(true),
                    ),
                ],
            ));
        }
        let action_items = vec![
            self.button_icon(
                "app-sort-toggle",
                if self.settings.apps_sort_by_name {
                    "排序：按名称"
                } else {
                    "排序：按版本代码"
                },
                UiIcon::Connect,
                Action::Settings(SettingsAction::ToggleAppsSort),
                Btn::Secondary,
            ),
            self.button_icon(
                "app-toolbar-install-apk",
                "安装 APK",
                UiIcon::Plus,
                Action::InstallApk,
                Btn::Primary,
            ),
            self.input(F_APP_SEARCH, "搜索应用名称或包名…", Some(240.)),
        ];
        let toolbar = self.between(self.wrap(filter_items), self.wrap(action_items));
        let filter_label = match self.settings.apps_filter {
            1 => "仅第三方应用",
            2 => "StepOS 与系统组件",
            3 => "已停用应用",
            _ => {
                if self.include_system_apps {
                    "含系统组件"
                } else {
                    "用户安装、可启动系统应用及 Amoo"
                }
            }
        };
        let sort_label = if self.settings.apps_sort_by_name {
            "按名称排序"
        } else {
            "按版本代码降序"
        };
        let description = self.between(
            self.line(
                format!(
                    "显示 {} 项 · {filter_label} · {sort_label} · 导出包含完整安装包，不包含应用数据",
                    filtered.len(),
                ),
                MICRO,
                p.text3,
            ),
            if !filtered.is_empty() {
                let all_pkgs = filtered
                    .iter()
                    .map(|a| format!("{} ({})", a.label, a.package))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.button_icon(
                    "apps-copy-list",
                    "复制当前列表包名",
                    UiIcon::Clipboard,
                    Action::CopyText(all_pkgs, "已复制当前筛选列表的应用包名".into()),
                    Btn::Ghost,
                )
            } else {
                Empty::new().finish()
            },
        );
        let mut rows = Vec::new();
        for app in &filtered {
            if !rows.is_empty() {
                rows.push(self.divider());
            }
            rows.push(self.installed_app_row(app));
        }
        if rows.is_empty() {
            rows.push(
                Container::new(
                    Align::new(self.vstack(
                        S2,
                        vec![
                            self.line_w(
                                if self.apps_error.is_some() {
                                    "应用列表读取失败"
                                } else if !self.apps_loaded {
                                    "尚未读取应用"
                                } else if self.apps.is_empty() {
                                    "手机当前用户没有已安装应用"
                                } else {
                                    "没有匹配的应用"
                                },
                                HEADING,
                                p.text,
                                Weight::Medium,
                            ),
                            self.line(
                                if let Some(error) = self.apps_error.as_deref() {
                                    error
                                } else if !self.apps_loaded {
                                    "连接手机后点击刷新列表。"
                                } else if self.apps.is_empty() {
                                    "可以安装 APK，或刷新列表重新读取。"
                                } else {
                                    "调整搜索内容，或切换到全部应用。"
                                },
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ))
                    .finish(),
                )
                .with_uniform_padding(S6)
                .finish(),
            );
        }
        let scroll = self.pane_scroll(
            self.scroll.clone(),
            Container::new(self.vstack(0., rows))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2)
                .finish(),
        );
        let card = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Expanded::new(1., scroll).finish())
                .finish(),
        )
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S3)
            .with_child(toolbar)
            .with_child(description)
            .with_child(Expanded::new(1., self.sub_enter(card)).finish())
            .finish()
    }
}

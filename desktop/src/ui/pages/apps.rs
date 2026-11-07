use super::super::{
    app_store_state::AppStoreAction, icons::UiIcon, settings_state::SettingsAction, theme::*,
    widgets::*, Action, Companion, Page, ToolPage, F_APP_SEARCH,
};
use crate::app_store::{check_store_install_state, StoreInstallState};
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
            Align::new(self.content_icon(
                if app.package == "com.android.calendar" {
                    UiIcon::Calendar
                } else {
                    UiIcon::Package
                },
                26.,
                self.p().text2,
            ))
            .finish()
        };
        ConstrainedBox::new(icon)
            .with_width(40.)
            .with_height(40.)
            .finish()
    }

    fn app_spec_cell(&self, label: &str, value: String, mono_val: bool) -> El {
        let p = self.p();
        let val_el = if mono_val {
            self.mono(value, MICRO, p.text)
        } else {
            self.line_w(value, CAPTION, p.text, Weight::Medium)
        };
        Container::new(self.vstack(2., vec![self.line(label, MICRO, p.text3), val_el]))
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(S1 + 2.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
    }

    fn app_details_inspector(&self, app: &AppInfo) -> El {
        let p = self.p();
        let package = &app.package;
        let details = self
            .selected_app_details
            .as_ref()
            .filter(|d| d.package == *package)
            .cloned()
            .unwrap_or_else(|| stepx_companion::apps::demo_details(package));

        // 1. Inspector Header
        let proc_badge = if !details.running_pids.is_empty() {
            let pids = details
                .running_pids
                .iter()
                .map(|pid| pid.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let label = if let Some(kb) = details.mem_pss_kb {
                format!("运行中 · PID {pids} · 内存 {:.1} MB", kb as f64 / 1024.0)
            } else {
                format!("运行中 · PID {pids}")
            };
            self.pill_badge(label, p.success_text, p.success_soft)
        } else {
            self.pill_badge("未在后台运行", p.text3, p.tag_bg)
        };

        let bucket_str = if details.standby_bucket.trim().is_empty() {
            "活跃 (ACTIVE)"
        } else {
            details.standby_bucket.trim()
        };
        let mut header_left = vec![
            self.pill_badge_icon(
                "桌面应用详情与权限管理",
                UiIcon::Shield,
                p.accent,
                p.accent_soft,
            ),
            proc_badge,
            self.pill_badge(format!("待机分组：{bucket_str}"), p.text2, p.tag_bg),
        ];
        if self.selected_app_loading {
            header_left.push(self.pill_badge_icon(
                "正在同步实时状态…",
                UiIcon::Refresh,
                p.blue,
                rgba(p.blue.r, p.blue.g, p.blue.b, if p.dark { 38 } else { 24 }),
            ));
        }

        let header = self.between(
            self.wrap(header_left),
            self.button_icon(
                "app-inspector-close",
                "收起详情",
                UiIcon::Close,
                Action::InspectApp(None),
                Btn::Ghost,
            ),
        );

        // 2. Metadata Grid (Desktop-Native)
        let ver_text = if !app.version_name.trim().is_empty() && app.version_code > 0 {
            format!(
                "v{} (versionCode {})",
                app.version_name.trim(),
                app.version_code
            )
        } else if !app.version_name.trim().is_empty() {
            format!("v{}", app.version_name.trim())
        } else if app.version_code > 0 {
            format!("versionCode {}", app.version_code)
        } else {
            "系统默认版本".to_string()
        };

        let sdk_text = format!(
            "targetSdk {} · minSdk {} · {}",
            details.target_sdk.max(1),
            details.min_sdk.max(1),
            if details.primary_abi.trim().is_empty() {
                "arm64-v8a"
            } else {
                details.primary_abi.trim()
            }
        );

        let uid_text = if details.uid > 0 {
            format!("UID {} · 用户 {}", details.uid, app.user_id)
        } else {
            format!("用户 {}", app.user_id)
        };

        let installer_text = if details.installer_package.trim().is_empty() {
            "手动安装 / 系统预装".to_string()
        } else {
            details.installer_package.clone()
        };

        let time_text =
            if !details.first_install_time.is_empty() || !details.last_update_time.is_empty() {
                format!(
                    "安装 {} · 更新 {}",
                    if details.first_install_time.is_empty() {
                        "—"
                    } else {
                        &details.first_install_time
                    },
                    if details.last_update_time.is_empty() {
                        "—"
                    } else {
                        &details.last_update_time
                    }
                )
            } else {
                "系统预置或未知时间".to_string()
            };

        let apk_paths_summary = if app.apk_files.is_empty() {
            format!("数据目录 {}", details.data_dir)
        } else {
            let first_path = &app.apk_files[0].path;
            let size_label = format_bytes(app.apk_bytes());
            if app.apk_files.len() > 1 {
                format!(
                    "{size_label} ({} 个分包) · {first_path} · 数据 {}",
                    app.apk_files.len(),
                    details.data_dir
                )
            } else {
                format!("{size_label} · {first_path} · 数据 {}", details.data_dir)
            }
        };

        let meta_row_1 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(S2)
            .with_child(
                Expanded::new(1.1, self.app_spec_cell("应用包名", package.clone(), true)).finish(),
            )
            .with_child(
                Expanded::new(1.0, self.app_spec_cell("版本信息", ver_text, false)).finish(),
            )
            .with_child(
                Expanded::new(1.1, self.app_spec_cell("目标 SDK 与架构", sdk_text, false)).finish(),
            )
            .finish();

        let meta_row_2 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(S2)
            .with_child(
                Expanded::new(0.7, self.app_spec_cell("应用 UID", uid_text, false)).finish(),
            )
            .with_child(
                Expanded::new(0.8, self.app_spec_cell("安装来源", installer_text, false)).finish(),
            )
            .with_child(
                Expanded::new(1.3, self.app_spec_cell("安装与更新时间", time_text, false)).finish(),
            )
            .with_child(
                Expanded::new(
                    1.2,
                    self.app_spec_cell("安装包体积与路径", apk_paths_summary, true),
                )
                .finish(),
            )
            .finish();

        // 3. Interactive Desktop Runtime Permissions (2-column grid)
        let granted_count = details
            .permissions
            .iter()
            .filter(|perm| perm.granted)
            .count();
        let perm_header = self.between(
            self.hstack(
                S2,
                vec![
                    self.line_w(
                        "应用权限开关 · 桌面端直接授权/撤销",
                        CAPTION,
                        p.text,
                        Weight::Semibold,
                    ),
                    self.pill_badge(
                        format!("已授权 {granted_count} / {}", details.permissions.len()),
                        p.text2,
                        p.tag_bg,
                    ),
                ],
            ),
            self.line(
                "无需跳转手机设置，点击即可通过 ADB 实时变更权限",
                MICRO,
                p.text3,
            ),
        );

        let mut perm_cards = Vec::with_capacity(details.permissions.len());
        for perm in &details.permissions {
            let zh_label = stepx_companion::apps::permission_chinese_label(&perm.name)
                .unwrap_or(perm.label.as_str());
            let short_suffix = perm.name.rsplit('.').next().unwrap_or(&perm.name);
            let status_badge = if perm.granted {
                self.pill_badge("已授权", p.success_text, p.success_soft)
            } else {
                self.pill_badge("未授权", p.text3, p.tag_bg)
            };
            let toggle_btn = self.button(
                format!("app-perm-toggle:{}:{}", app.package, perm.name),
                if perm.granted {
                    "撤销权限"
                } else {
                    "授予权限"
                },
                Action::ToggleAppPermission {
                    package: app.package.clone(),
                    permission: perm.name.clone(),
                    grant: !perm.granted,
                },
                if perm.granted {
                    Btn::Secondary
                } else {
                    Btn::Primary
                },
            );
            let left = self.hstack(
                S2,
                vec![
                    status_badge,
                    self.line_w(zh_label, CAPTION, p.text, Weight::Medium),
                    self.mono(short_suffix, MICRO, p.text3),
                ],
            );
            perm_cards.push(
                Container::new(self.between(left, toggle_btn))
                    .with_horizontal_padding(S2 + 2.)
                    .with_vertical_padding(S1)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            );
        }
        let mut perm_rows = Vec::new();
        let mut iter = perm_cards.into_iter();
        while let Some(first) = iter.next() {
            if let Some(second) = iter.next() {
                perm_rows.push(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(S2)
                        .with_child(Expanded::new(1.0, first).finish())
                        .with_child(Expanded::new(1.0, second).finish())
                        .finish(),
                );
            } else {
                perm_rows.push(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(S2)
                        .with_child(Expanded::new(1.0, first).finish())
                        .with_child(Expanded::new(1.0, Empty::new().finish()).finish())
                        .finish(),
                );
            }
        }

        // 4. Desktop Action Bar
        let action_bar = self.between(
            self.wrap(vec![
                self.button(
                    "app-inspector-force-stop",
                    "强行停止",
                    Action::ForceStopApp(package.clone()),
                    Btn::Secondary,
                ),
                self.button(
                    "app-inspector-toggle-enable",
                    if app.enabled {
                        "停用应用"
                    } else {
                        "启用应用"
                    },
                    Action::ToggleAppEnabled {
                        package: package.clone(),
                        enable: !app.enabled,
                    },
                    Btn::Secondary,
                ),
                self.button(
                    "app-inspector-clear-cache",
                    "清理缓存",
                    Action::ClearAppStorage {
                        package: package.clone(),
                        clear_data: false,
                    },
                    Btn::Secondary,
                ),
                self.button(
                    "app-inspector-export",
                    "导出 APK",
                    Action::ExportApp(package.clone()),
                    Btn::Secondary,
                ),
            ]),
            self.button_icon(
                "app-inspector-phone-settings",
                "在手机系统设置中打开",
                UiIcon::Smartphone,
                Action::AppSettings(package.clone()),
                Btn::Ghost,
            ),
        );

        let inspector = Container::new(self.vstack(
            S2,
            vec![
                header,
                self.divider(),
                meta_row_1,
                meta_row_2,
                self.divider(),
                perm_header,
                self.vstack(S1 + 1., perm_rows),
                self.divider(),
                action_bar,
            ],
        ))
        .with_horizontal_padding(S3)
        .with_vertical_padding(S2 + 4.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.accent))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        #[cfg(feature = "ui-capture")]
        let inspector =
            crate::ui::scenario_check::control("app-inspector-card".to_string(), inspector);

        inspector
    }

    fn installed_app_row(&self, app: &AppInfo) -> El {
        let p = self.p();
        let package = &app.package;
        let is_selected = self.selected_app.as_deref() == Some(package.as_str());
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
        let store_match = self
            .app_store
            .catalog
            .iter()
            .find(|s| s.package == *package);
        let store_state = store_match
            .map(|s| check_store_install_state(std::slice::from_ref(app), s))
            .unwrap_or(StoreInstallState::NotInstalled);
        let mut right_badges = Vec::new();
        if let StoreInstallState::Updatable { latest_version, .. } = &store_state {
            right_badges.push(self.pill_badge_icon(
                format!("可更新至 v{latest_version}"),
                UiIcon::Upload,
                p.blue,
                rgba(p.blue.r, p.blue.g, p.blue.b, if p.dark { 38 } else { 24 }),
            ));
        }
        right_badges.push(self.pill_badge(kind, p.text2, p.surface));
        let heading = self.between(info, self.hstack(S1 + 2., right_badges));
        // Visible: primary (更新 / 打开) + 更多 + subtle 卸载. Everything else lives in
        // the expanded row (toggled by 更多 via the existing selected_app state) or the
        // right-click menu.
        let can_open = app.launchable && app.enabled;
        let updatable = matches!(store_state, StoreInstallState::Updatable { .. });
        let mut actions = Vec::new();
        let mut more_actions = Vec::new();
        if let StoreInstallState::Updatable { latest_version, .. } = &store_state {
            actions.push(self.button_icon(
                format!("app-store-upd:{package}"),
                format!("更新至 v{latest_version}"),
                UiIcon::Download,
                Action::AppStore(AppStoreAction::InstallOrUpdate {
                    package: package.clone(),
                }),
                Btn::Primary,
            ));
        }
        if can_open {
            let mirror = self.button_icon(
                format!("app-open-mirror:{package}"),
                "投屏打开",
                UiIcon::Monitor,
                Action::OpenAppInMirror(package.clone()),
                if updatable {
                    Btn::Secondary
                } else {
                    Btn::Primary
                },
            );
            if updatable {
                more_actions.push(mirror);
            } else {
                actions.push(mirror);
            }
        } else if !updatable {
            actions.push(self.disabled_button(if app.enabled {
                "无启动入口"
            } else {
                "已停用"
            }));
        }
        actions.push(self.button(
            format!("app-settings:{package}"),
            if is_selected { "收起" } else { "更多" },
            if is_selected {
                Action::InspectApp(None)
            } else {
                Action::InspectApp(Some(package.clone()))
            },
            Btn::Ghost,
        ));
        if app.can_uninstall() {
            actions.push(self.button(
                format!("app-uninstall:{package}"),
                "卸载",
                Action::RequestUninstallApp(package.clone()),
                Btn::Ghost,
            ));
        }
        // 强行停止 / 导出 APK live in the inspector action bar below; only add the
        // remaining row-level actions here to avoid duplicates.
        if is_selected {
            if can_open {
                more_actions.push(self.button(
                    format!("app-open:{package}"),
                    "手机打开",
                    Action::OpenApp(package.clone()),
                    Btn::Secondary,
                ));
            }
            if store_match.is_some() {
                more_actions.push(self.chip(
                    format!("app-store-detail:{package}"),
                    "商店详情",
                    Some(UiIcon::Package),
                    vec![
                        Action::Page(Page::AppStore),
                        Action::AppStore(AppStoreAction::SelectApp(package.clone())),
                    ],
                    false,
                    false,
                ));
            }
            more_actions.push(self.button_icon(
                format!("app-copy:{package}"),
                "复制包名",
                UiIcon::Clipboard,
                Action::CopyText(package.clone(), "已复制应用包名".into()),
                Btn::Secondary,
            ));
        }
        let mut contents = vec![heading, self.wrap(actions)];
        if is_selected {
            contents.push(self.wrap(more_actions));
            contents.push(self.app_details_inspector(app));
        }
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
        let hoverable = Hoverable::new(
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
                let bg = if mouse.is_clicked() && !is_selected {
                    p.accent_soft
                } else if is_selected {
                    mix_color(p.surface, p.accent_soft, 0.35)
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                let border_col = if is_selected {
                    p.accent
                } else {
                    ColorU::transparent_black()
                };
                Container::new(content)
                    .with_horizontal_padding(S3)
                    .with_vertical_padding(S2 + 2.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border_col))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            },
        )
        .with_defer_events_to_children()
        .with_cursor(if is_selected {
            Cursor::Arrow
        } else {
            Cursor::PointingHand
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::App(package_rc.clone()),
                at,
            ));
        });
        let el = if is_selected {
            hoverable.finish()
        } else {
            hoverable
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::InspectApp(Some(package_dbl.clone())));
                })
                .finish()
        };
        #[cfg(feature = "ui-capture")]
        let el = crate::ui::scenario_check::control(format!("app-row:{package}"), el);
        el
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
                    format!("系统组件 ({stepos_sys_count})"),
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
            self.input(F_APP_SEARCH, "搜索应用名称或包名…", Some(240.)),
        ];
        let toolbar = self.between(self.wrap(filter_items), self.wrap(action_items));
        let description = self.between(
            self.line(format!("{} 个应用", filtered.len()), MICRO, p.text3),
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
        if let Some(sel_pkg) = self.selected_app.as_deref() {
            if !filtered.iter().any(|a| a.package == sel_pkg) {
                let fallback_app = self
                    .apps
                    .iter()
                    .find(|a| a.package == sel_pkg)
                    .cloned()
                    .unwrap_or_else(|| AppInfo {
                        package: sel_pkg.to_string(),
                        label: stepx_companion::apps::clean_app_label(sel_pkg, sel_pkg),
                        enabled: true,
                        launchable: true,
                        ..AppInfo::default()
                    });
                rows.push(self.app_details_inspector(&fallback_app));
            }
        }
        let page_key = format!("installed-apps:{}:{}:{}:{query}",self.settings.apps_filter,self.settings.apps_sort_by_name,self.include_system_apps);
        let range = self.list_page_range(&page_key,filtered.len(),40);
        for app in &filtered[range] {
            if !rows.is_empty() {
                rows.push(self.divider());
            }
            rows.push(self.installed_app_row(app));
        }
        if rows.is_empty() {
            let store_btn_label = if query.is_empty() {
                "去应用商店浏览安装".to_string()
            } else {
                format!("去应用商店搜索「{query}」")
            };
            let store_btn_actions = if query.is_empty() {
                vec![Action::Page(Page::AppStore)]
            } else {
                vec![
                    Action::Page(Page::AppStore),
                    Action::AppStore(AppStoreAction::QuickSearch(query.to_string())),
                ]
            };
            rows.push(
                Container::new(
                    Align::new(self.vstack(
                        S2,
                        vec![
                            Align::new(self.line_w(
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
                            ))
                            .finish(),
                            Align::new(self.line(
                                if let Some(error) = self.apps_error.as_deref() {
                                    error
                                } else if !self.apps_loaded {
                                    "连接手机后点击刷新列表。"
                                } else if self.apps.is_empty() {
                                    "可以安装 APK，或前往应用商店下载安装。"
                                } else {
                                    "本机未安装匹配应用，可前往应用商店多源检索并一键安装。"
                                },
                                CAPTION,
                                p.text2,
                            ))
                            .finish(),
                            Align::new(
                                Container::new(self.chip(
                                    "apps-empty-goto-store",
                                    store_btn_label,
                                    Some(UiIcon::Search),
                                    store_btn_actions,
                                    true,
                                    false,
                                ))
                                .with_margin_top(S2)
                                .finish(),
                            )
                            .finish(),
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
            .with_child(self.list_page_controls(&page_key,filtered.len(),40))
            .with_child(Expanded::new(1., self.sub_enter(card)).finish())
            .finish()
    }
}

//! Multi-Source App Store (`Page::AppStore`).
//! Aggregates Tencent MyApp (应用宝实时源), Official Vendor Direct (群晖/绿联/极空间/飞牛官方直连),
//! Global Store, GitHub Releases, F-Droid, and StepOS Store into a single deduplicated catalog
//! ("多源显示一个"), with a clean 2-page architecture:
//! - Page 1: Full-width 2-column paginated catalog grid (`store_catalog_full_page`)
//! - Page 2: Full-page rich app detail view (`store_detail_full_page`)

use super::super::{
    app_store_state::{AppStoreAction, StoreLightboxState, StoreTaskStage, StoreTaskState},
    icons::UiIcon,
    theme::*,
    widgets::*,
    Action, Companion, F_STORE_SEARCH,
};
use crate::app_store::{
    category_sub_tags, check_store_install_state, compare_android_versions, HistoricalVersion,
    RelatedAppItem, StoreApp, StoreCategory, StoreInstallState, StoreSource, StoreSourceOffer,
};
use facloud_ui::{
    assets::asset_cache::AssetSource, color::ColorU, elements::*, fonts::Weight, platform::Cursor,
};
use stepx_companion::storage::format_bytes;

fn blue_soft(p: &Palette) -> ColorU {
    rgba(p.blue.r, p.blue.g, p.blue.b, if p.dark { 38 } else { 24 })
}

fn indigo_soft(p: &Palette) -> ColorU {
    rgba(
        p.indigo.r,
        p.indigo.g,
        p.indigo.b,
        if p.dark { 38 } else { 24 },
    )
}

fn clean_version_display(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches(|c| c == 'v' || c == 'V')
        .trim();
    if trimmed.is_empty()
        || trimmed == "最新版"
        || trimmed == "开源最新版"
        || trimmed == "未知"
    {
        return None;
    }
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        Some(format!("v{trimmed}"))
    } else {
        Some(trimmed.to_string())
    }
}

fn source_badge_colors(source: StoreSource, p: &Palette) -> (ColorU, ColorU) {
    match source {
        StoreSource::Tencent => (p.blue, blue_soft(p)),
        StoreSource::Global => (p.indigo, indigo_soft(p)),
        StoreSource::Official => (p.success_text, p.success_soft),
        StoreSource::GitHub => (p.indigo, indigo_soft(p)),
        StoreSource::FDroid => (p.warning, p.warning_soft),
        StoreSource::StepOs => (p.text2, p.tag_bg),
    }
}

fn source_verify_badge(source: StoreSource, p: &Palette) -> (&'static str, ColorU, ColorU) {
    match source {
        StoreSource::Official => ("安装前校验", p.success_text, p.success_soft),
        StoreSource::Tencent => ("安装前校验", p.blue, blue_soft(p)),
        StoreSource::Global => ("第三方来源", p.indigo, indigo_soft(p)),
        StoreSource::GitHub => ("开源发布", p.indigo, indigo_soft(p)),
        StoreSource::FDroid => ("F-Droid 可复现构建", p.warning, p.warning_soft),
        StoreSource::StepOs => ("平台来源", p.text2, p.tag_bg),
    }
}

impl Companion {
    fn store_source_badge(&self, source: StoreSource) -> El {
        let p = self.p();
        let (fg, bg) = source_badge_colors(source, &p);
        self.pill_badge(source.short_label(), fg, bg)
    }

    /// Count how many apps in the current store catalog have an update available on the connected phone.
    pub(in super::super) fn store_updatable_count(&self) -> usize {
        if self.apps.is_empty() {
            return 0;
        }
        self.app_store
            .catalog
            .iter()
            .filter(|app| {
                matches!(
                    check_store_install_state(&self.apps, app),
                    StoreInstallState::Updatable { .. }
                )
            })
            .count()
    }

    /// Count how many apps in the store catalog are currently installed on the connected phone.
    pub(in super::super) fn store_installed_count(&self) -> usize {
        if self.apps.is_empty() {
            return 0;
        }
        self.app_store
            .catalog
            .iter()
            .filter(|app| {
                !matches!(
                    check_store_install_state(&self.apps, app),
                    StoreInstallState::NotInstalled
                )
            })
            .count()
    }

    /// Count how many apps in the catalog belong to a given `StoreCategory`.
    fn store_category_count(&self, cat: StoreCategory) -> usize {
        match cat {
            StoreCategory::All => self.app_store.catalog.len(),
            StoreCategory::Updatable => self.store_updatable_count(),
            StoreCategory::Installed => self.store_installed_count(),
            other => self
                .app_store
                .catalog
                .iter()
                .filter(|app| app.category == other)
                .count(),
        }
    }

    pub(in super::super) fn app_store_actions(&self) -> Vec<El> {
        let updatable_count = self.store_updatable_count();
        let mut items = Vec::new();

        if self.selected.is_some() {
            items.push(self.button_icon(
                "store-hdr-install-local",
                "安装本地 APK",
                UiIcon::Plus,
                Action::InstallApk,
                Btn::Secondary,
            ));
        }

        items.push(self.button_icon(
            "store-hdr-refresh-catalog",
            if self.app_store.syncing_catalog {
                "正在同步应用库…"
            } else {
                "刷新应用库"
            },
            UiIcon::Refresh,
            Action::AppStore(AppStoreAction::RefreshCatalog),
            Btn::Secondary,
        ));

        items.push(self.button_icon(
            "store-hdr-check-updates",
            if self.app_store.checking_updates {
                "正在检测更新…"
            } else {
                "检测更新"
            },
            UiIcon::Refresh,
            Action::AppStore(AppStoreAction::CheckUpdates),
            if updatable_count > 0 {
                Btn::Ghost
            } else {
                Btn::Secondary
            },
        ));

        if updatable_count > 0 {
            items.push(self.button_icon(
                "store-hdr-update-all",
                format!("全部更新 ({updatable_count})"),
                UiIcon::Download,
                Action::AppStore(AppStoreAction::UpdateAll),
                Btn::Primary,
            ));
        }

        items
    }

    fn find_store_app<'a>(&'a self, package: &str) -> Option<&'a StoreApp> {
        if let Some(results) = &self.app_store.search_results {
            if let Some(found) = results.iter().find(|a| a.package == package) {
                return Some(found);
            }
        }
        self.app_store.catalog.iter().find(|a| a.package == package)
    }

    fn store_app_display_name(&self, package: &str) -> String {
        if let Some(app) = self.find_store_app(package) {
            return app.name.clone();
        }
        if let Some(installed) = self.apps.iter().find(|a| a.package == package) {
            return stepx_companion::apps::clean_app_label(&installed.package, &installed.label);
        }
        package.to_string()
    }

    fn store_app_icon(&self, app: &StoreApp, size: f32) -> El {
        let p = self.p();
        let cached_path = app
            .cached_icon
            .as_deref()
            .filter(|path| cached_is_file(path))
            .or_else(|| {
                self.apps
                    .iter()
                    .find(|installed| installed.package == app.package)
                    .and_then(|installed| installed.icon_path.as_deref())
                    .filter(|path| cached_is_file(path))
            });

        if let Some(path) = cached_path {
            let img = Image::new(
                AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish();
            ConstrainedBox::new(
                Container::new(img)
                    .with_uniform_padding(4.)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(p.surface_border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish()
        } else {
            let hash = app
                .package
                .bytes()
                .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize));
            let palettes = [
                (blue_soft(&p), p.blue),
                (indigo_soft(&p), p.indigo),
                (p.success_soft, p.success_text),
                (p.warning_soft, p.warning),
            ];
            let (bg, fg) = palettes[hash % palettes.len()];
            let first_char = app
                .name
                .chars()
                .next()
                .unwrap_or('A')
                .to_uppercase()
                .to_string();
            let glyph = Align::new(self.line_w(first_char, TITLE, fg, Weight::Semibold)).finish();
            ConstrainedBox::new(
                Container::new(glyph)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(p.surface_border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish()
        }
    }

    fn store_related_app_icon(&self, item: &RelatedAppItem, size: f32) -> El {
        let p = self.p();
        let cached_path = item
            .cached_icon
            .as_deref()
            .filter(|path| cached_is_file(path))
            .or_else(|| {
                self.find_store_app(&item.package)
                    .and_then(|a| a.cached_icon.as_deref())
                    .filter(|path| cached_is_file(path))
            })
            .or_else(|| {
                self.apps
                    .iter()
                    .find(|installed| installed.package == item.package)
                    .and_then(|installed| installed.icon_path.as_deref())
                    .filter(|path| cached_is_file(path))
            });

        let inner = if let Some(path) = cached_path {
            Image::new(
                AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish()
        } else {
            let ch = item.name.chars().next().unwrap_or('A').to_string();
            Align::new(self.line_w(ch, BODY, p.blue, Weight::Semibold)).finish()
        };

        ConstrainedBox::new(
            Container::new(inner)
                .with_uniform_padding(3.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        )
        .with_width(size)
        .with_height(size)
        .finish()
    }

    fn store_banners(&self) -> Vec<El> {
        let p = self.p();
        let mut banners = Vec::new();

        // 1. Status banner
        if let Some((msg, is_err)) = &self.app_store.status_banner {
            let fg = if *is_err { p.danger } else { p.text };
            let bg = if *is_err { p.danger_soft } else { p.surface };
            let border = if *is_err { p.danger } else { p.border };
            let icon = if *is_err {
                UiIcon::AlertTriangle
            } else {
                UiIcon::Check
            };
            banners.push(
                Container::new(self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(
                                icon,
                                16.,
                                if *is_err { p.danger } else { p.success },
                            ),
                            self.text(msg, BODY, fg),
                        ],
                    ),
                    self.button(
                        "store-banner-dismiss",
                        "知道了",
                        Action::AppStore(AppStoreAction::DismissBanner),
                        Btn::Secondary,
                    ),
                ))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        }

        // 2. Pending uninstall confirmation bar
        if let Some(pkg) = &self.app_store.pending_uninstall {
            let name = self.store_app_display_name(pkg);
            banners.push(self.confirm_bar(
                format!("确定要从手机卸载「{name}」({pkg}) 吗？卸载将清除该应用数据。"),
                true,
                vec![
                    self.button(
                        "store-uninstall-cancel",
                        "取消",
                        Action::AppStore(AppStoreAction::CancelUninstall),
                        Btn::Secondary,
                    ),
                    self.button(
                        "store-uninstall-confirm",
                        "确认卸载",
                        Action::AppStore(AppStoreAction::ConfirmUninstall(pkg.clone())),
                        Btn::DangerSolid,
                    ),
                ],
            ));
        }

        // 3. Pending downgrade clean-reinstall confirmation bar
        if let Some((pkg, ver_idx)) = &self.app_store.pending_downgrade_reinstall {
            let name = self.store_app_display_name(pkg);
            let ver_label = self
                .find_store_app(pkg)
                .and_then(|a| a.historical_versions.get(*ver_idx))
                .and_then(|v| clean_version_display(&v.version_name))
                .unwrap_or_else(|| "历史旧版".to_string());
            banners.push(
                Container::new(self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::AlertTriangle, 16., p.warning),
                            self.text(
                                format!(
                                    "Android 系统限制直接覆盖降级已安装的正式版「{name}」。安装 {ver_label} 可能需要清除数据。请先自行备份并在手机卸载，客户端不会自动卸载。"
                                ),
                                BODY,
                                p.text,
                            ),
                        ],
                    ),
                    self.hstack(
                        S2,
                        vec![
                            self.button(
                                "store-downgrade-cancel",
                                "取消",
                                Action::AppStore(AppStoreAction::DismissBanner),
                                Btn::Secondary,
                            ),
                            self.button(
                                "store-downgrade-confirm",
                                "已了解",
                                    Action::AppStore(AppStoreAction::DismissBanner),
                                Btn::Primary,
                            ),
                        ],
                    ),
                ))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2)
                .with_background_color(p.warning_soft)
                .with_border(Border::all(1.).with_border_color(p.warning_border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        }

        // 4. Active batch update progress strip (ONLY while actively running)
        let batch_running = !self.app_store.batch_queue.is_empty()
            || (self.app_store.batch_total > 0
                && self.app_store.batch_done < self.app_store.batch_total);

        if batch_running {
            let total = self.app_store.batch_total.max(1);
            let done = self.app_store.batch_done.min(total);
            let pct = ((done as f32 / total as f32) * 100.0).round() as u8;
            banners.push(
                Container::new(self.vstack(
                    S2,
                    vec![
                        self.between(
                            self.hstack(
                                S2,
                                vec![
                                    self.pill_badge_icon(
                                        "批量更新进行中",
                                        UiIcon::Download,
                                        p.blue,
                                        blue_soft(&p),
                                    ),
                                    self.line_w(
                                        format!(
                                            "已完成 {done} / {total} 款应用（剩余队列 {} 款）",
                                            self.app_store.batch_queue.len()
                                        ),
                                        BODY,
                                        p.text,
                                        Weight::Medium,
                                    ),
                                ],
                            ),
                            self.button(
                                "store-batch-view-updatable",
                                "查看待更新列表",
                                Action::AppStore(AppStoreAction::SelectCategory(
                                    StoreCategory::Updatable,
                                )),
                                Btn::Secondary,
                            ),
                        ),
                        self.progress(Some(pct.max(8)), p.blue),
                    ],
                ))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2 + 2.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        }

        banners
    }

    /// Clean Top Toolbar for Page 1 (`store_catalog_full_page`):
    /// - 6 human category pills with counts (`全部应用`, `社交通讯`, `影音娱乐`, `办公效率`, `实用工具`, `生活出行`)
    /// - `vdivider(16.)`
    /// - 2 status filter pills with counts (`可更新`, `已安装`)
    /// - Right: search box (`220.`) + `搜索` button + conditional `清空` button
    fn store_toolbar(&self) -> El {
        let p = self.p();
        let query = self.fields[F_STORE_SEARCH].text.trim();

        let standard_categories = [
            StoreCategory::All,
            StoreCategory::Social,
            StoreCategory::Media,
            StoreCategory::Productivity,
            StoreCategory::Tools,
            StoreCategory::Lifestyle,
        ];
        let status_categories = [StoreCategory::Updatable, StoreCategory::Installed];

        let mut left_pills: Vec<El> = Vec::with_capacity(9);
        for cat in standard_categories {
            let label = cat.label().to_string();
            let active =
                self.app_store.search_results.is_none() && self.app_store.category == cat;
            left_pills.push(self.chip(
                format!("store-cat-{:?}", cat),
                label,
                None,
                vec![Action::AppStore(AppStoreAction::SelectCategory(cat))],
                active,
                false,
            ));
        }

        left_pills.push(self.vdivider(16.));

        for cat in status_categories {
            let count = self.store_category_count(cat);
            let label = format!("{} · {count}", cat.label());
            let active =
                self.app_store.search_results.is_none() && self.app_store.category == cat;
            left_pills.push(self.chip(
                format!("store-cat-{:?}", cat),
                label,
                None,
                vec![Action::AppStore(AppStoreAction::SelectCategory(cat))],
                active,
                false,
            ));
        }

        let categories_wrap = Wrap::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S1)
            .with_run_spacing(S1)
            .with_children(left_pills)
            .finish();

        let mut search_bar = vec![
            self.input(
                F_STORE_SEARCH,
                "搜索应用、包名或开发者…",
                Some(172.),
            ),
            self.button_icon(
                "store-search-btn",
                if self.app_store.searching {
                    "搜索中"
                } else {
                    "搜索"
                },
                UiIcon::Search,
                Action::AppStore(AppStoreAction::SearchSubmit),
                Btn::Secondary,
            ),
        ];
        if !query.is_empty() || self.app_store.search_results.is_some() {
            search_bar.push(self.button_icon(
                "store-search-clear",
                "清空",
                UiIcon::Close,
                Action::AppStore(AppStoreAction::ClearSearch),
                Btn::Ghost,
            ));
        }

        let row1 = self.between(categories_wrap, self.hstack(S1 + 2., search_bar));
        let mut rows = vec![row1];

        let sub_tags = category_sub_tags(self.app_store.category);
        if self.app_store.search_results.is_none() && !sub_tags.is_empty() {
            rows.push(self.divider());

            let mut sub_pills: Vec<El> = Vec::with_capacity(sub_tags.len() + 2);
            sub_pills.push(self.line_w("细分专题", MICRO, p.text3, Weight::Semibold));
            sub_pills.push(self.chip(
                "store-subtag-all",
                "全部",
                None,
                vec![Action::AppStore(AppStoreAction::SelectSubTag(None))],
                self.app_store.sub_tag.is_none(),
                false,
            ));

            for &tag in sub_tags {
                let active = self.app_store.sub_tag.as_deref() == Some(tag);
                sub_pills.push(self.chip(
                    format!("store-subtag-{tag}"),
                    tag.to_string(),
                    None,
                    vec![Action::AppStore(AppStoreAction::SelectSubTag(Some(
                        tag.to_string(),
                    )))],
                    active,
                    false,
                ));
            }

            let sub_wrap = Wrap::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(S1 + 2.)
                .with_run_spacing(S1 + 2.)
                .with_children(sub_pills)
                .finish();

            let cache_badge = if self.app_store.syncing_catalog {
                self.pill_badge_icon(
                    "正在从在线源拉取应用…",
                    UiIcon::Refresh,
                    p.blue,
                    blue_soft(&p),
                )
            } else if let Some(sync_str) = self
                .app_store
                .catalog_last_sync
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                self.pill_badge(
                    format!("多源实时在线 · {sync_str}"),
                    p.text3,
                    p.surface,
                )
            } else {
                self.pill_badge("多源实时在线", p.text3, p.surface)
            };

            rows.push(self.between(sub_wrap, cache_badge));
        }

        if self.app_store.searching {
            rows.push(self.hstack(
                S1 + 2.,
                vec![
                    self.ui_icon(UiIcon::Refresh, 12., p.blue),
                    self.line(
                        "正在并发检索应用宝、国际源、官方直连、GitHub 与 F-Droid…",
                        MICRO,
                        p.blue,
                    ),
                ],
            ));
        } else if self.app_store.search_results.is_some() {
            rows.push(self.between(
                self.line(
                    format!(
                        "全网搜索「{}」· 已聚合多源结果",
                        self.app_store.last_search_query
                    ),
                    MICRO,
                    p.text2,
                ),
                self.link_muted(
                    "store-back-catalog",
                    "返回分类目录",
                    Action::AppStore(AppStoreAction::ClearSearch),
                ),
            ));
        }

        Container::new(self.vstack(S2, rows))
            .with_horizontal_padding(S3 + 2.)
            .with_vertical_padding(S2 + 2.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    /// Clean Bottom Pagination Bar docked at the bottom of the catalog panel.
    fn store_pagination_bar(
        &self,
        total_apps: usize,
        cur_page: usize,
        total_pages: usize,
        _start: usize,
        _end: usize,
    ) -> El {
        let p = self.p();
        let query = self.fields[F_STORE_SEARCH].text.trim();

        let base_cat_label = if let Some(sub) = self.app_store.sub_tag.as_deref() {
            format!("{} / {sub}", self.app_store.category.label())
        } else {
            self.app_store.category.label().to_string()
        };

        let scope_label = if self.app_store.search_results.is_some() {
            if self.app_store.last_search_query.trim().is_empty() {
                "搜索结果".to_string()
            } else {
                format!("搜索「{}」", self.app_store.last_search_query.trim())
            }
        } else if !query.is_empty() {
            format!("{base_cat_label} · 筛选「{query}」")
        } else {
            base_cat_label
        };

        let is_local_phone_cat = self.app_store.search_results.is_none()
            && matches!(
                self.app_store.category,
                StoreCategory::Updatable | StoreCategory::Installed
            );

        let left_summary = if is_local_phone_cat {
            self.line(
                format!(
                    "{scope_label} · 共 {total_apps} 款 · 第 {} / {total_pages} 页",
                    cur_page + 1
                ),
                CAPTION,
                p.text2,
            )
        } else {
            self.line(
                format!(
                    "{scope_label} · 第 {} 页{}",
                    cur_page + 1,
                    if self.app_store.syncing_catalog || self.app_store.searching {
                        " · 正在同步更多在线应用…"
                    } else {
                        ""
                    }
                ),
                CAPTION,
                p.text2,
            )
        };

        let mut page_controls: Vec<El> = Vec::with_capacity(10);

        // 首页 button
        if cur_page == 0 {
            page_controls.push(self.disabled_button("首页"));
        } else {
            page_controls.push(self.button(
                "store-page-first",
                "首页",
                Action::AppStore(AppStoreAction::SetPage(0)),
                Btn::Secondary,
            ));
        }

        // PrevPage button
        if cur_page == 0 {
            page_controls.push(self.disabled_button("上一页"));
        } else {
            page_controls.push(self.button_icon(
                "store-page-prev",
                "上一页",
                UiIcon::ChevronLeft,
                Action::AppStore(AppStoreAction::PrevPage),
                Btn::Secondary,
            ));
        }

        // Sliding window of up to 5 page buttons around cur_page
        let end_p = (cur_page + 3).min(total_pages.saturating_sub(1));
        let start_p = cur_page.saturating_sub(1).min(end_p.saturating_sub(4));
        for idx in start_p..=end_p {
            let is_cur = idx == cur_page;
            page_controls.push(self.chip(
                format!("store-page-num-{idx}"),
                format!("{}", idx + 1),
                None,
                vec![Action::AppStore(AppStoreAction::SetPage(idx))],
                is_cur,
                false,
            ));
        }

        // NextPage button
        if cur_page + 1 >= total_pages {
            page_controls.push(self.disabled_button("下一页"));
        } else {
            page_controls.push(self.button_icon(
                "store-page-next",
                "下一页",
                UiIcon::ChevronRight,
                Action::AppStore(AppStoreAction::NextPage),
                Btn::Secondary,
            ));
        }

        Container::new(self.between(left_summary, self.hstack(S1 + 2., page_controls)))
            .with_horizontal_padding(S3 + 2.)
            .with_vertical_padding(S2 + 2.)
            .with_background_color(p.bg)
            .finish()
    }

    fn store_lightbox_overlay(&self, lb: &StoreLightboxState) -> Option<El> {
        let p = self.p();
        let app = self.find_store_app(&lb.package)?;
        let total = app
            .cached_screenshots
            .len()
            .max(app.screenshots.len())
            .max(1);
        let idx = lb.screenshot_idx.min(total.saturating_sub(1));
        let zoom_pct = lb.zoom_pct.clamp(50, 300);
        let canvas_h = (340.0 * (zoom_pct as f32 / 100.0)).clamp(200.0, 520.0);

        let shot_path = app
            .cached_screenshots
            .get(idx)
            .map(|p| p.as_path())
            .filter(|path| cached_is_file(path));

        let main_image = if let Some(path) = shot_path {
            Image::new(
                AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .contain()
            .finish()
        } else {
            Align::new(self.vstack(
                S2,
                vec![
                    Align::new(self.ui_icon(UiIcon::Image, 28., p.text3)).finish(),
                    Align::new(self.line("正在缓存高清应用截图…", BODY, p.text2)).finish(),
                ],
            ))
            .finish()
        };

        let controls = self.wrap(vec![
            self.button_icon(
                "store-lb-prev",
                "上一张",
                UiIcon::ArrowLeft,
                Action::AppStore(AppStoreAction::LightboxPrev),
                Btn::Secondary,
            ),
            self.pill_badge(format!("{} / {}", idx + 1, total), p.text2, p.surface),
            self.button_icon(
                "store-lb-next",
                "下一张",
                UiIcon::ArrowRight,
                Action::AppStore(AppStoreAction::LightboxNext),
                Btn::Secondary,
            ),
            self.button(
                "store-lb-zoom-out",
                "− 缩小",
                Action::AppStore(AppStoreAction::LightboxZoom(-25)),
                Btn::Secondary,
            ),
            self.pill_badge(format!("{zoom_pct}%"), p.accent, p.accent_soft),
            self.button(
                "store-lb-zoom-in",
                "+ 放大",
                Action::AppStore(AppStoreAction::LightboxZoom(25)),
                Btn::Secondary,
            ),
            self.button_icon(
                "store-lb-close",
                "关闭预览",
                UiIcon::Close,
                Action::AppStore(AppStoreAction::CloseLightbox),
                Btn::Secondary,
            ),
        ]);

        let mut thumbs = Vec::new();
        for (i, path) in app.cached_screenshots.iter().enumerate() {
            if !cached_is_file(path) {
                continue;
            }
            let is_cur = i == idx;
            let pkg = app.package.clone();
            let path_str = path.to_string_lossy().into_owned();
            let thumb_el = Hoverable::new(
                self.mouse_state(format!("store-lb-thumb-{pkg}-{i}")),
                move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let border_col = if is_cur {
                        p.blue
                    } else if hovered {
                        p.accent
                    } else {
                        p.border
                    };
                    ConstrainedBox::new(
                        Container::new(
                            Image::new(
                                AssetSource::LocalFile {
                                    path: path_str.clone(),
                                },
                                CacheOption::BySize,
                            )
                            .contain()
                            .finish(),
                        )
                        .with_uniform_padding(3.)
                        .with_background_color(if is_cur { p.accent_soft } else { p.surface })
                        .with_border(Border::all(1.).with_border_color(border_col))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                    )
                    .with_width(58.)
                    .with_height(96.)
                    .finish()
                },
            )
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::AppStore(AppStoreAction::OpenLightbox {
                    package: pkg.clone(),
                    idx: i,
                }))
            })
            .finish();
            thumbs.push(thumb_el);
        }

        let mut body = vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.heading(format!("应用图片预览 · {}", lb.app_name)),
                        self.pill_badge(&app.developer, p.text2, p.tag_bg),
                    ],
                ),
                controls,
            ),
            Container::new(ConstrainedBox::new(main_image).with_height(canvas_h).finish())
                .with_uniform_padding(S2)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        ];
        if !thumbs.is_empty() {
            body.push(self.wrap(thumbs));
        }

        Some(self.panel(body))
    }

    fn store_task_progress_widget(&self, task: &StoreTaskState) -> El {
        let p = self.p();
        match &task.stage {
            StoreTaskStage::Queued => {
                let ver_hint = clean_version_display(&task.target_version)
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default();
                self.hstack(
                    S2,
                    vec![
                        self.pill_badge("排队中", p.text2, p.tag_bg),
                        self.line(format!("等待下载{ver_hint}…"), MICRO, p.text3),
                    ],
                )
            }
            StoreTaskStage::Downloading {
                percent,
                downloaded_bytes,
                total_bytes,
            } => {
                let detail = if *total_bytes > 0 {
                    format!(
                        "正在下载 {}% ({} / {})",
                        percent,
                        format_bytes(*downloaded_bytes),
                        format_bytes(*total_bytes)
                    )
                } else {
                    format!("正在下载 ({})…", format_bytes(*downloaded_bytes))
                };
                self.vstack(
                    S1,
                    vec![
                        self.progress(Some((*percent).max(5)), p.blue),
                        self.line(detail, MICRO, p.blue),
                    ],
                )
            }
            StoreTaskStage::Verifying => self.vstack(
                S1,
                vec![
                    self.progress(Some(92), p.indigo),
                    self.line("正在校验 APK 安装包完整性…", MICRO, p.indigo),
                ],
            ),
            StoreTaskStage::Installing => self.vstack(
                S1,
                vec![
                    self.progress(Some(96), p.indigo),
                    self.line(
                        if task.is_downgrade {
                            "正在向手机安装历史版本 (ADB)…"
                        } else {
                            "正在安装到手机 (ADB)…"
                        },
                        MICRO,
                        p.indigo,
                    ),
                ],
            ),
            StoreTaskStage::Uninstalling => self.vstack(
                S1,
                vec![
                    self.progress(Some(80), p.warning),
                    self.line("正在从手机卸载应用…", MICRO, p.warning),
                ],
            ),
            StoreTaskStage::Completed(msg) => {
                self.pill_badge_icon(msg.clone(), UiIcon::Check, p.success_text, p.success_soft)
            }
            StoreTaskStage::Failed(err) => self.hstack(
                S2,
                vec![
                    self.line(format!("失败：{err}"), MICRO, p.danger),
                    self.button(
                        format!("store-task-retry:{}", task.package),
                        "重试",
                        Action::AppStore(AppStoreAction::InstallOrUpdate {
                            package: task.package.clone(),
                        }),
                        Btn::Secondary,
                    ),
                ],
            ),
        }
    }

    /// Catalog card in the 2-column grid:
    /// - `48×48` rounded icon
    /// - App Name + Category badge (`app.category.label()`) + Source badge + Installed/Update badge
    /// - 1-line summary
    /// - Version / Size / Developer subtitle
    /// - Right: subtle `详情` button (`SelectApp`) + primary `安装` / `更新` / `打开` button
    /// - Clicking the card body dispatches `Action::AppStore(AppStoreAction::SelectApp(pkg.clone()))`
    fn store_app_card(&self, app: &StoreApp) -> El {
        let p = self.p();
        let pkg = &app.package;
        let install_state = check_store_install_state(&self.apps, app);
        let primary_source = app.primary_source();

        // Line 1: App Name + Category badge + Source badge + Installed/Update badge
        let cat_badge_label = if self.app_store.category != StoreCategory::All
            && !app.category_label.trim().is_empty()
        {
            app.category_label.trim()
        } else {
            app.category.label()
        };
        let display_title = {
            let mut units = 0usize;
            let mut cut_idx = app.name.len();
            let mut truncated = false;
            for (idx, ch) in app.name.char_indices() {
                let w = if ch.is_ascii() { 1 } else { 2 };
                if units + w > 20 {
                    cut_idx = idx;
                    truncated = true;
                    break;
                }
                units += w;
            }
            if truncated {
                format!("{}…", app.name[..cut_idx].trim_end())
            } else {
                app.name.clone()
            }
        };
        let mut title_items = vec![
            self.line_w(display_title, HEADING, p.text, Weight::Semibold),
            self.pill_badge(cat_badge_label, p.text2, p.tag_bg),
            self.store_source_badge(primary_source),
        ];
        match &install_state {
            StoreInstallState::Updatable { .. } => {
                title_items.push(self.pill_badge("可更新", p.blue, blue_soft(&p)));
            }
            StoreInstallState::Installed { .. } => {
                title_items.push(self.pill_badge("已安装", p.success_text, p.success_soft));
            }
            StoreInstallState::NotInstalled => {
                if app.sources.len() > 1 && app.name.chars().count() <= 7 {
                    title_items.push(self.pill_badge(
                        format!("+{}源", app.sources.len() - 1),
                        p.text3,
                        p.surface,
                    ));
                }
            }
        }
        let line1 = self.hstack(S1 + 2., title_items);

        // Line 2: 1-line summary
        let summary_text = if !app.tagline.trim().is_empty() {
            app.tagline.trim()
        } else if !app.description.trim().is_empty() {
            app.description.trim()
        } else {
            app.developer.trim()
        };
        let line2 = self.line(summary_text, CAPTION, p.text2);

        // Line 3: Version / Size / Developer subtitle
        let mut meta_parts = Vec::new();
        if let Some(v) = clean_version_display(app.version_name()) {
            meta_parts.push(v);
        } else {
            meta_parts.push("正式版".to_string());
        }
        if app.size_bytes() > 0 {
            meta_parts.push(format_bytes(app.size_bytes()));
        }
        let dev = app.developer.trim();
        let src_short = primary_source.short_label();
        if !dev.is_empty()
            && !dev.eq_ignore_ascii_case(src_short)
            && dev != "F-Droid Open Source"
            && dev != "F-Droid"
        {
            meta_parts.push(dev.to_string());
        }
        let line3 = self.line(meta_parts.join(" · "), MICRO, p.text3);

        let center_lines = vec![line1, line2, line3];

        // Right: subtle `详情` button + primary `安装` / `更新` / `打开` button (or active task progress)
        let active_task = self.app_store.tasks.get(pkg).filter(|t| t.stage.is_active());
        let right_action = if let Some(task) = active_task {
            let (label, pct, color) = match &task.stage {
                StoreTaskStage::Queued => ("排队中…".to_string(), 8, p.text3),
                StoreTaskStage::Downloading { percent, .. } => (
                    format!("下载 {}%", percent),
                    (*percent).max(6),
                    p.blue,
                ),
                StoreTaskStage::Verifying => ("校验中…".to_string(), 92, p.indigo),
                StoreTaskStage::Installing => ("安装中…".to_string(), 96, p.indigo),
                StoreTaskStage::Uninstalling => ("卸载中…".to_string(), 85, p.warning),
                _ => ("处理中…".to_string(), 50, p.blue),
            };
            self.fixed_w(
                116.,
                self.vstack(
                    4.,
                    vec![
                        self.line_w(label, MICRO, color, Weight::Medium),
                        self.progress(Some(pct), color),
                    ],
                ),
            )
        } else {
            let detail_btn = self.button(
                format!("store-card-detail:{pkg}"),
                "详情",
                Action::AppStore(AppStoreAction::SelectApp(pkg.clone())),
                Btn::Ghost,
            );
            let primary_btn = match &install_state {
                StoreInstallState::Updatable { .. } => self.button_icon(
                    format!("store-card-upd:{pkg}"),
                    "更新",
                    UiIcon::Download,
                    Action::AppStore(AppStoreAction::InstallOrUpdate {
                        package: pkg.clone(),
                    }),
                    Btn::Primary,
                ),
                StoreInstallState::Installed { .. } => self.button(
                    format!("store-card-open:{pkg}"),
                    "打开",
                    Action::OpenApp(pkg.clone()),
                    Btn::Secondary,
                ),
                StoreInstallState::NotInstalled => self.button(
                    format!("store-card-inst:{pkg}"),
                    "安装",
                    Action::AppStore(AppStoreAction::InstallOrUpdate {
                        package: pkg.clone(),
                    }),
                    Btn::Primary,
                ),
            };
            self.hstack(S1 + 2., vec![detail_btn, primary_btn])
        };

        let card_content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S3)
            .with_child(self.store_app_icon(app, 48.))
            .with_child(Expanded::new(1., self.vstack(3., center_lines)).finish())
            .with_child(right_action)
            .finish();

        let pkg_click = pkg.clone();
        let state = self.mouse_state(format!("store-card:{pkg}"));

        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if clicked {
                p.press_color(p.hover)
            } else {
                lerp_color(p.bg, p.hover, t * 0.65)
            };
            let border_col = lerp_color(p.border, p.border_strong, t);
            Container::new(card_content)
                .with_horizontal_padding(S3 + 2.)
                .with_vertical_padding(S2 + 4.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border_col))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::AppStore(AppStoreAction::SelectApp(
                pkg_click.clone(),
            )))
        })
        .finish()
    }

    /// Page 1: Full-Page Catalog Grid View (`store_catalog_full_page`)
    fn store_catalog_full_page(&self) -> El {
        let p = self.p();
        let query = self.fields[F_STORE_SEARCH].text.trim();

        let filtered = self.filtered_store_apps();
        let active_list: Vec<&StoreApp> = filtered.iter().collect();

        let total_apps = active_list.len();
        let page_size = self.app_store.page_size.max(1);
        let total_pages = total_apps.div_ceil(page_size).max(1);
        let cur_page = self.app_store.page.min(total_pages.saturating_sub(1));
        let start = (cur_page * page_size).min(total_apps);
        let end = ((cur_page + 1) * page_size).min(total_apps);
        let page_items = &active_list[start..end];

        let mut catalog_rows = Vec::new();
        if page_items.is_empty() {
            let empty_el = if self.app_store.searching {
                self.empty(
                    "正在多源检索应用…",
                    "正在并发查询应用宝、国际源、官方直连、GitHub 与 F-Droid",
                    None,
                )
            } else if !query.is_empty() && self.app_store.search_results.is_none() {
                self.empty(
                    format!("当前分类目录未匹配「{query}」"),
                    "点击下方按钮在线检索应用宝全量库、国际源、GitHub 与 F-Droid",
                    Some(self.button_icon(
                        "store-empty-online-search",
                        format!("全网搜索「{query}」"),
                        UiIcon::Search,
                        Action::AppStore(AppStoreAction::SearchSubmit),
                        Btn::Primary,
                    )),
                )
            } else if let Some(sub) = &self.app_store.sub_tag {
                self.empty(
                    format!("「{sub}」专题下暂无匹配应用"),
                    "可切换回本分类全部应用，或刷新在线应用库拉取更多同专题应用",
                    Some(self.hstack(
                        S2,
                        vec![
                            self.button(
                                "store-empty-subtag-reset",
                                "查看本分类全部应用",
                                Action::AppStore(AppStoreAction::SelectSubTag(None)),
                                Btn::Secondary,
                            ),
                            self.button_icon(
                                "store-empty-subtag-refresh",
                                "刷新在线应用库",
                                UiIcon::Refresh,
                                Action::AppStore(AppStoreAction::RefreshCatalog),
                                Btn::Primary,
                            ),
                        ],
                    )),
                )
            } else if self.app_store.category == StoreCategory::Updatable {
                self.empty(
                    "所有已安装应用均为最新版本",
                    "可点击右上角「检测更新」重新扫描手机上的已安装应用",
                    Some(self.button_icon(
                        "store-empty-check-upd",
                        "立即检测更新",
                        UiIcon::Refresh,
                        Action::AppStore(AppStoreAction::CheckUpdates),
                        Btn::Secondary,
                    )),
                )
            } else if self.app_store.category == StoreCategory::Installed {
                self.empty(
                    "暂未检测到已安装的商店应用",
                    "连接手机后可自动匹配已安装应用，也可直接在上方分类浏览并安装",
                    Some(self.button(
                        "store-empty-all-apps",
                        "浏览推荐应用",
                        Action::AppStore(AppStoreAction::SelectCategory(StoreCategory::All)),
                        Btn::Secondary,
                    )),
                )
            } else {
                self.empty(
                    "未找到匹配的应用",
                    "尝试更换搜索关键词，或切换上方应用分类",
                    Some(self.button(
                        "store-empty-reset",
                        "返回推荐应用",
                        Action::AppStore(AppStoreAction::ClearSearch),
                        Btn::Secondary,
                    )),
                )
            };
            catalog_rows.push(empty_el);
        } else {
            for pair in page_items.chunks(2) {
                let left = Expanded::new(1., self.store_app_card(pair[0])).finish();
                let right = if pair.len() == 2 {
                    Expanded::new(1., self.store_app_card(pair[1])).finish()
                } else {
                    Expanded::new(1., Empty::new().finish()).finish()
                };
                catalog_rows.push(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(S3)
                        .with_child(left)
                        .with_child(right)
                        .finish(),
                );
            }
        }

        let catalog_scroll = self.pane_scroll(
            self.scroll.clone(),
            Container::new(self.vstack(S2 + 2., catalog_rows))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S3)
                .finish(),
        );

        let catalog_pane = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Expanded::new(1., catalog_scroll).finish())
                .with_child(self.divider())
                .with_child(self.store_pagination_bar(
                    total_apps,
                    cur_page,
                    total_pages,
                    start,
                    end,
                ))
                .finish(),
        )
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S2 + 2.)
            .with_child(self.store_toolbar())
            .with_child(Expanded::new(1., catalog_pane).finish())
            .finish()
    }

    fn store_source_offer_row(
        &self,
        package: &str,
        idx: usize,
        offer: &StoreSourceOffer,
        selected: bool,
    ) -> El {
        let p = self.p();
        let (src_fg, src_bg) = source_badge_colors(offer.source, &p);
        let (ver_label, ver_fg, ver_bg) = source_verify_badge(offer.source, &p);

        let mut meta = Vec::new();
        if let Some(v) = clean_version_display(&offer.version_name) {
            meta.push(format!("版本 {v}"));
        } else {
            meta.push("最新正式版".to_string());
        }
        if offer.size_bytes > 0 {
            meta.push(format!("大小 {}", format_bytes(offer.size_bytes)));
        } else {
            meta.push("安装时获取大小".to_string());
        }
        if !offer.updated_at.trim().is_empty() {
            meta.push(format!("更新于 {}", offer.updated_at));
        }

        let left = self.vstack(
            3.,
            vec![
                self.hstack(
                    S2,
                    vec![
                        self.pill_badge(offer.source.label(), src_fg, src_bg),
                        self.pill_badge_icon(ver_label, UiIcon::Shield, ver_fg, ver_bg),
                        if selected {
                            self.pill_badge_icon(
                                "当前使用源",
                                UiIcon::Check,
                                p.accent,
                                p.accent_soft,
                            )
                        } else {
                            self.pill_badge("点击切换", p.text3, p.tag_bg)
                        },
                    ],
                ),
                self.line_w(
                    meta.join(" · "),
                    CAPTION,
                    if selected { p.text } else { p.text2 },
                    if selected {
                        Weight::Medium
                    } else {
                        Weight::Normal
                    },
                ),
            ],
        );

        let pkg_btn = package.to_string();
        let right = self.button_icon(
            format!("store-src-install:{package}:{idx}"),
            "立即安装",
            UiIcon::Download,
            Action::AppStore(AppStoreAction::InstallOrUpdate { package: pkg_btn }),
            if selected {
                Btn::Primary
            } else {
                Btn::Secondary
            },
        );

        let row_inner = self.between(left, right);
        let pkg = package.to_string();
        let state = self.mouse_state(format!("store-src:{package}:{idx}"));

        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let bg = if selected {
                p.accent_soft
            } else {
                lerp_color(p.surface, p.hover, t)
            };
            let border = if selected {
                p.accent
            } else {
                lerp_color(p.border, p.border_strong, t)
            };
            Container::new(row_inner)
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2 + 2.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::AppStore(AppStoreAction::SelectSource {
                package: pkg.clone(),
                source_idx: idx,
            }))
        })
        .finish()
    }

    fn store_historical_version_row(
        &self,
        app: &StoreApp,
        idx: usize,
        ver: &HistoricalVersion,
    ) -> El {
        let p = self.p();
        let (src_fg, src_bg) = source_badge_colors(ver.source, &p);
        let mut meta_parts = Vec::new();
        if !ver.size_label.trim().is_empty() && ver.size_label.trim() != "大小待获取" {
            meta_parts.push(ver.size_label.trim().to_string());
        } else if ver.size_bytes > 0 {
            meta_parts.push(format_bytes(ver.size_bytes));
        } else {
            meta_parts.push("安装时获取".to_string());
        }
        if !ver.release_date.trim().is_empty() {
            meta_parts.push(ver.release_date.clone());
        }

        let is_installed_newer = self
            .apps
            .iter()
            .find(|a| a.package == app.package)
            .is_some_and(|inst| {
                compare_android_versions(&inst.version_name, &ver.version_name)
                    == std::cmp::Ordering::Greater
            });

        let btn_label = if is_installed_newer {
            "降级安装"
        } else {
            "安装旧版"
        };

        let ver_display =
            clean_version_display(&ver.version_name).unwrap_or_else(|| ver.version_name.clone());

        let left = self.vstack(
            2.,
            vec![
                self.hstack(
                    S2,
                    vec![
                        self.line_w(ver_display, BODY, p.text, Weight::Semibold),
                        self.pill_badge(ver.source.short_label(), src_fg, src_bg),
                    ],
                ),
                self.line(meta_parts.join(" · "), MICRO, p.text3),
            ],
        );

        let right = self.button_icon(
            format!("store-hist:{}:{idx}", app.package),
            btn_label,
            UiIcon::Download,
            Action::AppStore(AppStoreAction::InstallHistorical {
                package: app.package.clone(),
                version_idx: idx,
                force_reinstall: false,
            }),
            Btn::Secondary,
        );

        Container::new(self.between(left, right))
            .with_horizontal_padding(S3)
            .with_vertical_padding(S2)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
    }

    fn store_recommended_app_row(&self, item: &RelatedAppItem, badge_hint: &str) -> El {
        let p = self.p();
        let installed = self.apps.iter().find(|a| a.package == item.package);
        let status_pill = if let Some(inst) = installed {
            let ver_suffix = clean_version_display(&inst.version_name)
                .map(|v| format!(" {v}"))
                .unwrap_or_default();
            self.pill_badge(
                format!("已装{ver_suffix}"),
                p.success_text,
                p.success_soft,
            )
        } else {
            let (fg, bg) = source_badge_colors(item.source, &p);
            self.pill_badge(item.source.short_label(), fg, bg)
        };

        let title_row = vec![
            self.line_w(&item.name, BODY, p.text, Weight::Semibold),
            status_pill,
        ];
        let base_sub = if item.tagline.trim().is_empty() {
            item.developer.as_str()
        } else {
            item.tagline.as_str()
        };
        let sub_text = match clean_version_display(&item.version_name) {
            Some(v) => format!("{badge_hint} · {v} · {base_sub}"),
            None => format!("{badge_hint} · {base_sub}"),
        };

        let info = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S2 + 2.)
            .with_child(self.store_related_app_icon(item, 38.))
            .with_child(
                Expanded::new(
                    1.,
                    self.vstack(
                        2.,
                        vec![
                            self.hstack(S1 + 2., title_row),
                            self.line(sub_text, MICRO, p.text2),
                        ],
                    ),
                )
                .finish(),
            )
            .finish();

        let in_catalog = self.find_store_app(&item.package).is_some();
        let nav_action = if in_catalog {
            Action::AppStore(AppStoreAction::SelectApp(item.package.clone()))
        } else {
            Action::AppStore(AppStoreAction::OpenDeveloperApp(item.clone()))
        };

        let btn = self.button(
            format!("store-rec-app:{}", item.package),
            "详情",
            nav_action.clone(),
            Btn::Secondary,
        );

        let row_inner = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S2)
            .with_child(Expanded::new(1., info).finish())
            .with_child(btn)
            .finish();
        let state = self.mouse_state(format!("store-rec-row:{}", item.package));

        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let bg = lerp_color(p.surface, p.hover, t);
            let border = lerp_color(p.border, p.border_strong, t);
            Container::new(row_inner)
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(nav_action.clone()))
        .finish()
    }

    fn store_spec_row(&self, key: &str, value_el: El) -> El {
        let p = self.p();
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S2)
            .with_child(self.fixed_w(84., self.line(key, CAPTION, p.text3)))
            .with_child(Expanded::new(1., value_el).finish())
            .finish()
    }

    /// Page 2: Full-Page App Detail View (`store_detail_full_page`)
    fn store_detail_full_page(&self, app: &StoreApp) -> El {
        let p = self.p();
        let pkg = &app.package;
        let install_state = check_store_install_state(&self.apps, app);
        let loading_detail =
            self.app_store.loading_detail_pkg.as_deref() == Some(pkg.as_str());

        // 1. Top Navigation Bar
        let mut breadcrumb_items = vec![
            self.button_icon(
                "store-detail-back",
                "返回应用列表",
                UiIcon::ArrowLeft,
                Action::AppStore(AppStoreAction::CloseDetail),
                Btn::Secondary,
            ),
            self.vdivider(16.),
            self.link(
                "store-bc-root",
                "应用商店",
                Action::AppStore(AppStoreAction::CloseDetail),
            ),
            self.line("/", CAPTION, p.text3),
            self.link(
                "store-bc-cat",
                app.category.label(),
                Action::AppStore(AppStoreAction::SelectCategory(app.category)),
            ),
            self.line("/", CAPTION, p.text3),
            self.line_w(&app.name, BODY, p.text, Weight::Semibold),
        ];
        if loading_detail {
            breadcrumb_items.push(self.pill_badge_icon(
                "正在同步详情…",
                UiIcon::Refresh,
                p.blue,
                blue_soft(&p),
            ));
        }

        let nav_right = self.hstack(
            S2,
            vec![
                self.button_icon(
                    format!("store-det-refresh:{pkg}"),
                    if loading_detail {
                        "正在刷新…"
                    } else {
                        "刷新详情"
                    },
                    UiIcon::Refresh,
                    Action::AppStore(AppStoreAction::RefreshDetail(pkg.clone())),
                    Btn::Ghost,
                ),
                self.button_icon(
                    format!("store-det-dl-apk:{pkg}"),
                    "仅下载 APK",
                    UiIcon::Folder,
                    Action::AppStore(AppStoreAction::DownloadApkOnly {
                        package: pkg.clone(),
                    }),
                    Btn::Secondary,
                ),
            ],
        );

        let top_nav_bar = Container::new(self.between(
            self.hstack(S2, breadcrumb_items),
            nav_right,
        ))
        .with_horizontal_padding(S3 + 2.)
        .with_vertical_padding(S2 + 2.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        // 2. Hero Card
        let install_badge = match &install_state {
            StoreInstallState::Updatable { latest_version, .. } => {
                let ver = clean_version_display(latest_version)
                    .unwrap_or_else(|| latest_version.clone());
                self.pill_badge(format!("可更新至 {ver}"), p.blue, blue_soft(&p))
            }
            StoreInstallState::Installed {
                installed_version, ..
            } => {
                let ver = clean_version_display(installed_version)
                    .map(|v| format!("已安装 {v}"))
                    .unwrap_or_else(|| "已安装".to_string());
                self.pill_badge(ver, p.success_text, p.success_soft)
            }
            StoreInstallState::NotInstalled => self.pill_badge("未安装", p.text3, p.tag_bg),
        };

        let mut hero_badges = vec![
            self.line_w(&app.name, TITLE, p.text, Weight::Semibold),
            self.pill_badge(app.category.label(), p.text2, p.tag_bg),
            self.store_source_badge(app.primary_source()),
            install_badge,
        ];
        if app.sources.len() > 1 {
            hero_badges.push(self.pill_badge(
                format!("{} 个可用源", app.sources.len()),
                p.indigo,
                indigo_soft(&p),
            ));
        }

        let tagline_str = if !app.tagline.trim().is_empty() {
            app.tagline.trim()
        } else if !app.description.trim().is_empty() {
            app.description.trim()
        } else {
            pkg.as_str()
        };

        let ver_str = clean_version_display(app.version_name())
            .unwrap_or_else(|| "最新正式版".to_string());
        let size_str = if app.size_bytes() > 0 {
            format_bytes(app.size_bytes())
        } else {
            "安装时获取".to_string()
        };
        let meta_line = format!(
            "开发者：{}  ·  版本：{ver_str}  ·  大小：{size_str}  ·  包名：{pkg}",
            app.developer
        );

        let mut hero_actions = Vec::new();
        match &install_state {
            StoreInstallState::Updatable { can_uninstall, .. } => {
                hero_actions.push(self.button_icon(
                    format!("store-det-upd:{pkg}"),
                    "升级到最新版",
                    UiIcon::Download,
                    Action::AppStore(AppStoreAction::InstallOrUpdate {
                        package: pkg.clone(),
                    }),
                    Btn::Primary,
                ));
                hero_actions.push(self.button(
                    format!("store-det-open:{pkg}"),
                    "在手机上打开",
                    Action::OpenApp(pkg.clone()),
                    Btn::Secondary,
                ));
                hero_actions.push(self.button_icon(
                    format!("store-det-mirror:{pkg}"),
                    "投屏打开",
                    UiIcon::Monitor,
                    Action::OpenAppInMirror(pkg.clone()),
                    Btn::Secondary,
                ));
                if *can_uninstall {
                    hero_actions.push(self.button_icon(
                        format!("store-det-uninst:{pkg}"),
                        "卸载",
                        UiIcon::Trash,
                        Action::AppStore(AppStoreAction::RequestUninstall(pkg.clone())),
                        Btn::Danger,
                    ));
                }
            }
            StoreInstallState::Installed { can_uninstall, .. } => {
                hero_actions.push(self.button(
                    format!("store-det-open:{pkg}"),
                    "在手机上打开",
                    Action::OpenApp(pkg.clone()),
                    Btn::Primary,
                ));
                hero_actions.push(self.button_icon(
                    format!("store-det-mirror:{pkg}"),
                    "投屏打开",
                    UiIcon::Monitor,
                    Action::OpenAppInMirror(pkg.clone()),
                    Btn::Secondary,
                ));
                if *can_uninstall {
                    hero_actions.push(self.button_icon(
                        format!("store-det-uninst:{pkg}"),
                        "卸载",
                        UiIcon::Trash,
                        Action::AppStore(AppStoreAction::RequestUninstall(pkg.clone())),
                        Btn::Danger,
                    ));
                }
            }
            StoreInstallState::NotInstalled => {
                hero_actions.push(self.button_icon(
                    format!("store-det-inst:{pkg}"),
                    "安装到手机",
                    UiIcon::Download,
                    Action::AppStore(AppStoreAction::InstallOrUpdate {
                        package: pkg.clone(),
                    }),
                    Btn::Primary,
                ));
            }
        }

        let hero_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S3 + 2.)
            .with_child(self.store_app_icon(app, 68.))
            .with_child(
                Expanded::new(
                    1.,
                    self.vstack(
                        4.,
                        vec![
                            self.wrap(hero_badges),
                            self.line(tagline_str, BODY, p.text2),
                            self.line(meta_line, CAPTION, p.text3),
                        ],
                    ),
                )
                .finish(),
            )
            .with_child(self.wrap(hero_actions))
            .finish();

        let mut hero_children = vec![hero_row];
        if let Some(task) = self.app_store.tasks.get(pkg) {
            hero_children.push(self.divider());
            hero_children.push(self.store_task_progress_widget(task));
        }
        let hero_card = self.panel(hero_children);

        // 3. Left/Main Column Cards
        // (1) 应用介绍与更新说明 (including screenshots preview strip if present)
        let desc_text = if app.description.trim().is_empty() {
            &app.tagline
        } else {
            &app.description
        };
        let mut intro_children = vec![
            self.between(
                self.heading("应用介绍与更新说明"),
                self.pill_badge(app.category.label(), p.text2, p.tag_bg),
            ),
            self.text(desc_text, BODY, p.text2),
        ];

        if !app.changelog.trim().is_empty() {
            intro_children.push(self.divider());
            intro_children.push(self.heading("本次更新说明"));
            intro_children.push(self.text(&app.changelog, BODY, p.text2));
        }

        if !app.cached_screenshots.is_empty() || !app.screenshots.is_empty() || loading_detail {
            intro_children.push(self.divider());
            intro_children.push(self.between(
                self.heading("应用预览截图"),
                self.line("点击任意截图可打开高清大图预览", MICRO, p.text3),
            ));

            let screenshots_body = if !app.cached_screenshots.is_empty() {
                let mut shots = Vec::new();
                for (idx, path) in app.cached_screenshots.iter().enumerate() {
                    if !cached_is_file(path) {
                        continue;
                    }
                    let path_str = path.to_string_lossy().into_owned();
                    let pkg_shot = pkg.clone();
                    let shot_card = Hoverable::new(
                        self.mouse_state(format!("store-det-shot:{pkg}:{idx}")),
                        move |mouse| {
                            let hovered = mouse.is_mouse_over_element();
                            let border_col = if hovered { p.accent } else { p.border };
                            ConstrainedBox::new(
                                Container::new(
                                    Image::new(
                                        AssetSource::LocalFile {
                                            path: path_str.clone(),
                                        },
                                        CacheOption::BySize,
                                    )
                                    .contain()
                                    .finish(),
                                )
                                .with_uniform_padding(3.)
                                .with_background_color(p.surface)
                                .with_border(Border::all(1.).with_border_color(border_col))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish(),
                            )
                            .with_width(96.)
                            .with_height(168.)
                            .finish()
                        },
                    )
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::AppStore(
                            AppStoreAction::OpenLightbox {
                                package: pkg_shot.clone(),
                                idx,
                            },
                        ))
                    })
                    .finish();
                    shots.push(shot_card);
                }
                if shots.is_empty() {
                    self.line("正在缓存应用预览截图…", CAPTION, p.text3)
                } else {
                    self.wrap(shots)
                }
            } else {
                Container::new(self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Image, 16., p.text3),
                        self.line("正在同步高清应用预览截图…", CAPTION, p.text2),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
            };
            intro_children.push(screenshots_body);
        }
        let intro_card = self.panel(intro_children);

        // (2) 下载源切换与多源镜像
        let mut source_rows = Vec::with_capacity(app.sources.len());
        for (idx, offer) in app.sources.iter().enumerate() {
            source_rows.push(self.store_source_offer_row(
                pkg,
                idx,
                offer,
                idx == app.selected_source_idx,
            ));
        }
        let sources_card = self.panel(vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.heading("下载源切换与多源镜像"),
                        self.line("支持一键切换官方直连、应用宝、国际源与开源镜像", MICRO, p.text3),
                    ],
                ),
                self.pill_badge(
                    format!("{} 个可用源", app.sources.len()),
                    p.blue,
                    blue_soft(&p),
                ),
            ),
            self.vstack(S2, source_rows),
        ]);

        // (3) 历史版本与旧版安装 (when non-empty)
        let hist_card = if !app.historical_versions.is_empty() {
            let rows: Vec<El> = app
                .historical_versions
                .iter()
                .enumerate()
                .map(|(idx, ver)| self.store_historical_version_row(app, idx, ver))
                .collect();
            Some(self.panel(vec![
                self.between(
                    self.heading("历史版本与旧版安装"),
                    self.pill_badge(
                        format!("{} 个历史版", app.historical_versions.len()),
                        p.text2,
                        p.tag_bg,
                    ),
                ),
                self.line(
                    "历史版本能否覆盖安装由 Android 决定；客户端不会自动卸载或清除应用数据。",
                    MICRO,
                    p.text3,
                ),
                self.vstack(S2, rows),
            ]))
        } else {
            None
        };

        let mut left_col_cards = Vec::with_capacity(3);
        if cfg!(feature = "ui-capture")
            && std::env::var_os("STEPX_STORE_DETAIL_BOTTOM").is_some()
        {
            if let Some(hc) = hist_card {
                left_col_cards.push(hc);
            }
            left_col_cards.push(sources_card);
            left_col_cards.push(intro_card);
        } else {
            left_col_cards.push(intro_card);
            left_col_cards.push(sources_card);
            if let Some(hc) = hist_card {
                left_col_cards.push(hc);
            }
        }

        // 4. Right/Side Column Cards
        // (1) 版本与包体信息 (clean key-value specification table)
        let cur_ver_display = match clean_version_display(app.version_name()) {
            Some(v) if app.version_code() > 0 => format!("{v} (Build {})", app.version_code()),
            Some(v) => v,
            None => app.version_name().to_string(),
        };
        let status_display = match &install_state {
            StoreInstallState::Updatable {
                installed_version,
                latest_version,
                ..
            } => {
                let iv = clean_version_display(installed_version)
                    .unwrap_or_else(|| installed_version.clone());
                let lv = clean_version_display(latest_version)
                    .unwrap_or_else(|| latest_version.clone());
                self.pill_badge(format!("可更新 ({iv} → {lv})"), p.blue, blue_soft(&p))
            }
            StoreInstallState::Installed {
                installed_version, ..
            } => {
                let iv = clean_version_display(installed_version)
                    .unwrap_or_else(|| installed_version.clone());
                self.pill_badge(format!("已安装 ({iv})"), p.success_text, p.success_soft)
            }
            StoreInstallState::NotInstalled => {
                self.line("手机未安装", CAPTION, p.text2)
            }
        };

        let pkg_row_val = self.between(
            self.mono(pkg, MICRO, p.text),
            self.button_icon(
                format!("store-det-copy-pkg:{pkg}"),
                "复制",
                UiIcon::Clipboard,
                Action::CopyText(pkg.clone(), "已复制应用包名".into()),
                Btn::Ghost,
            ),
        );

        let icp_or_sig = if !app.icp_number.trim().is_empty() {
            app.icp_number.trim().to_string()
        } else {
            "未提供备案信息".to_string()
        };

        let spec_rows = vec![
            self.store_spec_row(
                "当前版本",
                self.line_w(cur_ver_display, CAPTION, p.text, Weight::Semibold),
            ),
            self.divider(),
            self.store_spec_row("本机状态", status_display),
            self.divider(),
            self.store_spec_row("应用包名", pkg_row_val),
            self.divider(),
            self.store_spec_row(
                "安装包大小",
                self.line(size_str, CAPTION, p.text),
            ),
            self.divider(),
            self.store_spec_row(
                "适用架构",
                self.mono("arm64-v8a", MICRO, p.text2),
            ),
            self.divider(),
            self.store_spec_row(
                "当前下载源",
                self.line(app.primary_source().label(), CAPTION, p.text),
            ),
            self.divider(),
            self.store_spec_row(
                "开发者",
                self.line(&app.developer, CAPTION, p.text),
            ),
            self.divider(),
            self.store_spec_row(
                "备案信息",
                self.line(icp_or_sig, CAPTION, p.text2),
            ),
        ];

        let specs_card = self.panel(vec![
            self.heading("版本与包体信息"),
            Container::new(self.vstack(S2, spec_rows))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2 + 2.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        ]);

        // (2) 同分类与同开发者推荐:
        // Combine `app.developer_apps` + up to 6 apps from `self.app_store.catalog` in the same `app.category`
        let mut rec_rows: Vec<El> = Vec::new();
        let mut seen_pkgs = std::collections::HashSet::new();
        seen_pkgs.insert(app.package.clone());

        for dev_item in &app.developer_apps {
            if seen_pkgs.insert(dev_item.package.clone()) {
                rec_rows.push(self.store_recommended_app_row(dev_item, "同开发者"));
            }
        }

        let mut same_cat_added = 0usize;
        for other in self
            .app_store
            .catalog
            .iter()
            .filter(|other| other.category == app.category && other.package != app.package)
        {
            if same_cat_added >= 6 {
                break;
            }
            if seen_pkgs.insert(other.package.clone()) {
                let related = RelatedAppItem {
                    package: other.package.clone(),
                    name: other.name.clone(),
                    version_name: other.version_name().to_string(),
                    size_bytes: other.size_bytes(),
                    icon_url: other.icon_url.clone(),
                    cached_icon: other.cached_icon.clone(),
                    developer: other.developer.clone(),
                    tagline: other.tagline.clone(),
                    source: other.primary_source(),
                    download_url: other.download_url().to_string(),
                };
                rec_rows.push(self.store_recommended_app_row(&related, app.category.label()));
                same_cat_added += 1;
            }
        }

        let mut rec_children = vec![self.between(
            self.heading("同分类与同开发者推荐"),
            self.button_icon(
                format!("store-det-search-dev:{pkg}"),
                "搜索同厂",
                UiIcon::Search,
                Action::AppStore(AppStoreAction::SearchDeveloper(app.developer.clone())),
                Btn::Ghost,
            ),
        )];
        if rec_rows.is_empty() {
            rec_children.push(self.line(
                "点击右上角「搜索同厂」可在线检索该开发者更多应用。",
                CAPTION,
                p.text3,
            ));
        } else {
            rec_children.push(self.vstack(S2, rec_rows));
        }
        let rec_card = self.panel(rec_children);

        let right_col_cards = vec![specs_card, rec_card];

        let two_col_body = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S3)
            .with_child(Expanded::new(1.55, self.vstack(S3, left_col_cards)).finish())
            .with_child(Expanded::new(1.0, self.vstack(S3, right_col_cards)).finish())
            .finish();

        let scroll_content = self.pane_scroll(
            self.app_store.detail_scroll.clone(),
            Container::new(self.vstack(S3, vec![hero_card, two_col_body]))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S3)
                .finish(),
        );

        let detail_body_pane = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Expanded::new(1., scroll_content).finish())
                .finish(),
        )
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S2 + 2.)
            .with_child(top_nav_bar)
            .with_child(Expanded::new(1., detail_body_pane).finish())
            .finish()
    }

    pub(in super::super) fn app_store_page(&self) -> El {
        if let Some(lb) = &self.app_store.lightbox {
            if let Some(lb_el) = self.store_lightbox_overlay(lb) {
                return Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_spacing(S3)
                    .with_child(Expanded::new(1., self.sub_enter(lb_el)).finish())
                    .finish();
            }
        }

        let page_view = if let Some(detail_app) = self
            .app_store
            .selected_package
            .as_deref()
            .and_then(|pkg| self.find_store_app(pkg))
        {
            self.sub_enter(self.store_detail_full_page(detail_app))
        } else {
            self.sub_enter(self.store_catalog_full_page())
        };

        let mut children = self.store_banners();
        children.push(Expanded::new(1., page_view).finish());

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S2 + 2.)
            .with_children(children)
            .finish()
    }
}

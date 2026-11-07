//! Background state, worker channel, and action dispatcher for `Page::AppStore`.

use super::{Action, Companion, Page, F_APP_SEARCH, F_STORE_SEARCH};
use facloud_ui::{elements::*, ViewContext};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
#[allow(unused_imports)]
pub use stepx_companion::app_store::{
    app_matches_sub_tag, category_sub_tags, fetch_online_catalog_from_upstream,
    load_startup_catalog,
};
use stepx_companion::{
    adb::Adb,
    app_store::{
        check_store_install_state, check_updates_for_installed, curated_catalog,
        download_apk_with_progress, expand_category_catalog_batch, fetch_app_full_detail,
        get_cached_app_detail, is_blacklisted_store_item, merge_and_deduplicate_apps,
        search_instant_local, search_multi_source, stream_batch_icons, stream_batch_screenshots,
        PreparedExpandedQuery, RelatedAppItem, StoreApp, StoreAppDetailPayload, StoreCategory,
        StoreInstallState, StoreSource, StoreSourceOffer,
    },
    apps::{self, AppInfo},
};

pub const STORE_DEFAULT_PAGE_SIZE: usize = 36;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreTaskStage {
    Queued,
    Downloading {
        percent: u8,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Verifying,
    Installing,
    Uninstalling,
    Completed(String),
    Failed(String),
}

impl StoreTaskStage {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Downloading { .. }
                | Self::Verifying
                | Self::Installing
                | Self::Uninstalling
        )
    }
}

#[derive(Clone, Debug)]
pub struct StoreTaskState {
    pub package: String,
    pub app_name: String,
    pub target_version: String,
    pub stage: StoreTaskStage,
    pub is_downgrade: bool,
}

#[derive(Clone, Debug)]
pub struct StoreLightboxState {
    pub package: String,
    pub app_name: String,
    pub screenshot_idx: usize,
    pub zoom_pct: u16,
}

#[derive(Clone, Debug)]
pub enum AppStoreAction {
    SelectCategory(StoreCategory),
    SelectSubTag(Option<String>),
    SelectApp(String),
    CloseDetail,
    SetPage(usize),
    PrevPage,
    NextPage,
    RefreshDetail(String),
    SelectSource {
        package: String,
        source_idx: usize,
    },
    SearchSubmit,
    QuickSearch(String),
    ClearSearch,
    RefreshCatalog,
    SyncOnlineCatalog {
        force: bool,
    },
    CheckUpdates,
    InstallOrUpdate {
        package: String,
    },
    InstallFromSource {
        package: String,
        source_idx: usize,
    },
    InstallHistorical {
        package: String,
        version_idx: usize,
        force_reinstall: bool,
    },
    DownloadApkOnly {
        package: String,
    },
    UpdateAll,
    RequestUninstall(String),
    CancelUninstall,
    ConfirmUninstall(String),
    OpenInstalledManager(String),
    OpenDeveloperApp(RelatedAppItem),
    SearchDeveloper(String),
    OpenLightbox {
        package: String,
        idx: usize,
    },
    LightboxPrev,
    LightboxNext,
    LightboxZoom(i16),
    CloseLightbox,
    DismissBanner,
}

#[derive(Debug)]
pub enum StoreWorkerEvent {
    IconCached {
        package: String,
        path: PathBuf,
    },
    ScreenshotCached {
        package: String,
        index: usize,
        path: PathBuf,
    },
    SearchFinished {
        query: String,
        result: Result<Vec<StoreApp>, String>,
    },
    DebouncedSearchTrigger {
        query: String,
    },
    UpdatesChecked {
        apps: Vec<StoreApp>,
    },
    CatalogSyncFinished {
        result: Result<(Vec<StoreApp>, String, usize), String>,
    },
    DetailLoaded {
        package: String,
        result: Result<StoreAppDetailPayload, String>,
    },
    TaskProgress {
        package: String,
        stage: StoreTaskStage,
    },
    TaskFinished {
        package: String,
        app_name: String,
        target_version: String,
        is_downgrade: bool,
        version_idx: Option<usize>,
        result: Result<String, String>,
        refreshed_apps: Option<Vec<AppInfo>>,
    },
    ApkDownloadedOnly {
        package: String,
        app_name: String,
        target_version: String,
        result: Result<PathBuf, String>,
    },
    UninstallFinished {
        package: String,
        app_name: String,
        result: Result<String, String>,
        refreshed_apps: Option<Vec<AppInfo>>,
    },
}

pub struct AppStoreUiState {
    pub catalog: Vec<StoreApp>,
    pub search_results: Option<Vec<StoreApp>>,
    pub last_search_query: String,
    pub searching: bool,
    pub checking_updates: bool,
    pub update_checked_once: bool,
    pub loading_detail_pkg: Option<String>,
    pub detail_fetched_pkgs: HashSet<String>,
    pub category: StoreCategory,
    pub sub_tag: Option<String>,
    pub syncing_catalog: bool,
    pub catalog_last_sync: Option<String>,
    pub online_total_count: usize,
    pub catalog_is_fresh: bool,
    pub catalog_auto_sync_triggered: bool,
    pub expansion_cursor: HashMap<String, usize>,
    pub auto_chain_remaining: usize,
    pub page: usize,
    pub page_size: usize,
    pub selected_package: Option<String>,
    pub tasks: HashMap<String, StoreTaskState>,
    pub batch_queue: Vec<String>,
    pub batch_total: usize,
    pub batch_done: usize,
    pub pending_uninstall: Option<String>,
    pub pending_downgrade_reinstall: Option<(String, usize)>,
    pub lightbox: Option<StoreLightboxState>,
    pub status_banner: Option<(String, bool)>,
    pub detail_scroll: ClippedScrollStateHandle,
    pub rx: Option<Receiver<StoreWorkerEvent>>,
    pub tx: Sender<StoreWorkerEvent>,
}

impl AppStoreUiState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (catalog, catalog_last_sync, online_total_count, catalog_is_fresh) =
            load_startup_catalog();
        let sub_tag = std::env::var("STEPX_STORE_SUB_TAG")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let selected_package = std::env::var("STEPX_STORE_DETAIL")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let init_query = std::env::var("STEPX_STORE_QUERY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut searching = false;
        let search_results = init_query.as_deref().map(|q| {
            let (hits, cache_is_fresh) = search_instant_local(q);
            if !cache_is_fresh {
                searching = true;
                let tx_search = tx.clone();
                let q_owned = q.to_string();
                std::thread::spawn(move || {
                    let result = search_multi_source(&q_owned).map_err(|e| format!("{e:#}"));
                    let _ = tx_search.send(StoreWorkerEvent::SearchFinished {
                        query: q_owned,
                        result,
                    });
                });
            }
            hits
        });
        let last_search_query = init_query.unwrap_or_default();
        let first_n = catalog.len().min(STORE_DEFAULT_PAGE_SIZE);
        if first_n > 0 {
            Self::spawn_icon_prefetcher(&catalog[..first_n], tx.clone());
        }
        if let Some(ref res) = search_results {
            let n = res.len().min(STORE_DEFAULT_PAGE_SIZE);
            if n > 0 {
                Self::spawn_icon_prefetcher(&res[..n], tx.clone());
            }
        }
        if let Some(ref st) = sub_tag {
            let sub_apps: Vec<&StoreApp> = catalog
                .iter()
                .filter(|a| app_matches_sub_tag(a, st))
                .take(STORE_DEFAULT_PAGE_SIZE)
                .collect();
            if !sub_apps.is_empty() {
                Self::spawn_icon_prefetcher_refs(&sub_apps, tx.clone());
            }
        }
        let init_category = match std::env::var("STEPX_STORE_CATEGORY")
            .unwrap_or_default()
            .trim()
        {
            "social" | "社交通讯" => StoreCategory::Social,
            "media" | "影音娱乐" => StoreCategory::Media,
            "productivity" | "办公效率" => StoreCategory::Productivity,
            "tools" | "实用工具" => StoreCategory::Tools,
            "lifestyle" | "生活出行" => StoreCategory::Lifestyle,
            _ => StoreCategory::All,
        };
        if init_category != StoreCategory::All {
            let cat_apps: Vec<&StoreApp> = catalog
                .iter()
                .filter(|a| a.category == init_category)
                .take(STORE_DEFAULT_PAGE_SIZE)
                .collect();
            if !cat_apps.is_empty() {
                Self::spawn_icon_prefetcher_refs(&cat_apps, tx.clone());
            }
        }
        Self {
            catalog,
            search_results,
            last_search_query,
            searching,
            checking_updates: false,
            update_checked_once: false,
            loading_detail_pkg: None,
            detail_fetched_pkgs: HashSet::new(),
            category: init_category,
            sub_tag,
            syncing_catalog: false,
            catalog_last_sync,
            online_total_count,
            catalog_is_fresh,
            catalog_auto_sync_triggered: false,
            expansion_cursor: HashMap::new(),
            auto_chain_remaining: 0,
            page: 0,
            page_size: STORE_DEFAULT_PAGE_SIZE,
            selected_package,
            tasks: HashMap::new(),
            batch_queue: Vec::new(),
            batch_total: 0,
            batch_done: 0,
            pending_uninstall: None,
            pending_downgrade_reinstall: None,
            lightbox: None,
            status_banner: None,
            detail_scroll: ClippedScrollStateHandle::default(),
            rx: Some(rx),
            tx,
        }
    }

    pub fn spawn_icon_prefetcher(apps: &[StoreApp], tx: Sender<StoreWorkerEvent>) {
        let refs: Vec<&StoreApp> = apps.iter().collect();
        Self::spawn_icon_prefetcher_refs(&refs, tx);
    }

    pub fn spawn_icon_prefetcher_refs(apps: &[&StoreApp], tx: Sender<StoreWorkerEvent>) {
        let mut pending: Vec<(String, String)> = Vec::new();
        for a in apps {
            if a.cached_icon.is_none() && !a.icon_url.trim().is_empty() {
                pending.push((a.package.clone(), a.icon_url.clone()));
            }
        }
        for a in apps {
            for rel in &a.developer_apps {
                if rel.cached_icon.is_none() && !rel.icon_url.trim().is_empty() {
                    pending.push((rel.package.clone(), rel.icon_url.clone()));
                }
            }
        }
        if pending.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            stream_batch_icons(&pending, |package, path| {
                let _ = tx.send(StoreWorkerEvent::IconCached { package, path });
            });
        });
    }

    pub fn spawn_detail_fetcher(app: &StoreApp, tx: Sender<StoreWorkerEvent>) {
        let app_clone = app.clone();
        let pkg = app_clone.package.clone();
        std::thread::spawn(move || {
            let result = fetch_app_full_detail(&app_clone).map_err(|e| format!("{e:#}"));
            let _ = tx.send(StoreWorkerEvent::DetailLoaded {
                package: pkg,
                result,
            });
        });
    }

    pub fn spawn_screenshot_prefetcher(package: &str, urls: &[String], tx: Sender<StoreWorkerEvent>) {
        if urls.is_empty() || urls.iter().all(|u| u.trim().is_empty()) {
            return;
        }
        let pkg = package.to_string();
        let urls_vec = urls.to_vec();
        std::thread::spawn(move || {
            stream_batch_screenshots(&pkg, &urls_vec, |package, index, path| {
                let _ = tx.send(StoreWorkerEvent::ScreenshotCached {
                    package,
                    index,
                    path,
                });
            });
        });
    }

    pub fn has_active_work(&self) -> bool {
        self.searching
            || self.checking_updates
            || self.syncing_catalog
            || self.loading_detail_pkg.is_some()
            || !self.batch_queue.is_empty()
            || self.tasks.values().any(|t| t.stage.is_active())
    }

    pub fn active_task_count(&self) -> usize {
        self.tasks.values().filter(|t| t.stage.is_active()).count()
    }

    pub fn find_app(&self, package: &str) -> Option<&StoreApp> {
        if let Some(results) = &self.search_results {
            if let Some(found) = results.iter().find(|a| a.package == package) {
                return Some(found);
            }
        }
        self.catalog.iter().find(|a| a.package == package)
    }

    pub fn find_app_for_install(&self, package: &str) -> Option<StoreApp> {
        let from_search = self
            .search_results
            .as_ref()
            .and_then(|res| res.iter().find(|a| a.package == package))
            .cloned();
        let from_catalog = self.catalog.iter().find(|a| a.package == package).cloned();
        let mut resolved = match (from_search, from_catalog) {
            (Some(mut s), Some(c)) => {
                Self::preserve_runtime_fields(
                    std::slice::from_mut(&mut s),
                    std::slice::from_ref(&c),
                );
                Some(s)
            }
            (Some(s), None) => Some(s),
            (None, Some(c)) => Some(c),
            (None, None) => curated_catalog()
                .into_iter()
                .find(|a| a.package == package),
        };
        if let Some(ref mut app) = resolved {
            enrich_app_with_curated(app);
            ensure_valid_selected_source(app);
        }
        resolved
    }

    pub fn mutate_app<F: FnMut(&mut StoreApp)>(&mut self, package: &str, mut f: F) {
        if let Some(app) = self.catalog.iter_mut().find(|a| a.package == package) {
            f(app);
        }
        if let Some(results) = &mut self.search_results {
            if let Some(app) = results.iter_mut().find(|a| a.package == package) {
                f(app);
            }
        }
    }

    pub fn apply_cached_icon(&mut self, package: &str, path: PathBuf) {
        for app in &mut self.catalog {
            if app.package == package {
                app.cached_icon = Some(path.clone());
            }
            for rel in &mut app.developer_apps {
                if rel.package == package {
                    rel.cached_icon = Some(path.clone());
                }
            }
        }
        if let Some(results) = &mut self.search_results {
            for app in results {
                if app.package == package {
                    app.cached_icon = Some(path.clone());
                }
                for rel in &mut app.developer_apps {
                    if rel.package == package {
                        rel.cached_icon = Some(path.clone());
                    }
                }
            }
        }
    }

    pub fn preserve_runtime_fields(target: &mut [StoreApp], previous: &[StoreApp]) {
        for app in target.iter_mut() {
            if let Some(old) = previous.iter().find(|p| p.package == app.package) {
                if app.cached_icon.is_none() {
                    app.cached_icon = old.cached_icon.clone();
                }
                if app.cached_screenshots.is_empty() && !old.cached_screenshots.is_empty() {
                    app.cached_screenshots = old.cached_screenshots.clone();
                }
                if old.detail_loaded && !app.detail_loaded {
                    app.detail_loaded = true;
                    if app.description.trim().is_empty() {
                        app.description = old.description.clone();
                    }
                    if app.changelog.trim().is_empty() {
                        app.changelog = old.changelog.clone();
                    }
                    if app.screenshots.is_empty() {
                        app.screenshots = old.screenshots.clone();
                    }
                    if app.historical_versions.is_empty() {
                        app.historical_versions = old.historical_versions.clone();
                    }
                    if app.developer_apps.is_empty() {
                        app.developer_apps = old.developer_apps.clone();
                    }
                }
                let old_selected_source = old
                    .sources
                    .get(old.selected_source_idx)
                    .filter(|s| !s.download_url.trim().is_empty())
                    .map(|s| s.source);
                for old_offer in &old.sources {
                    if old_offer.download_url.trim().is_empty() {
                        continue;
                    }
                    if let Some(existing) =
                        app.sources.iter_mut().find(|s| s.source == old_offer.source)
                    {
                        if existing.download_url.trim().is_empty()
                            || (!is_direct_apk_url(&existing.download_url)
                                && is_direct_apk_url(&old_offer.download_url))
                        {
                            existing.download_url = old_offer.download_url.clone();
                            if existing.version_name.trim().is_empty() {
                                existing.version_name = old_offer.version_name.clone();
                            }
                            if existing.version_code == 0 {
                                existing.version_code = old_offer.version_code;
                            }
                            if existing.size_bytes == 0 {
                                existing.size_bytes = old_offer.size_bytes;
                            }
                            if existing.md5.trim().is_empty() {
                                existing.md5 = old_offer.md5.clone();
                            }
                        }
                    } else {
                        app.sources.push(old_offer.clone());
                    }
                }
                if let Some(sel_src) = old_selected_source {
                    if let Some(idx) = app.sources.iter().position(|s| s.source == sel_src) {
                        app.selected_source_idx = idx;
                    }
                } else if old.selected_source_idx < app.sources.len() {
                    app.selected_source_idx = old.selected_source_idx;
                }
            }
            enrich_app_with_curated(app);
            ensure_valid_selected_source(app);
        }
    }
}

pub fn is_direct_apk_url(url: &str) -> bool {
    let u = url.trim();
    (u.starts_with("http://") || u.starts_with("https://"))
        && !u.contains("sj.qq.com/appdetail")
        && !u.contains("appgallery.huawei.com/app/")
        && !u.contains("wandoujia.com/")
        && !(u.contains("f-droid.org") && u.contains("/packages/"))
}

pub fn huawei_appgallery_page_url(package: &str) -> String {
    let appid = match package.trim() {
        "app.huawei.auto" => Some("C104871165"),
        "app.huawei.motor" => Some("C108819761"),
        "com.yinwang.qiankunapp" => Some("C114128593"),
        "com.huawei.smarthome" => Some("C10406921"),
        "com.huawei.health" => Some("C10414141"),
        "com.vmall.client" => Some("C10049053"),
        "com.huawei.phoneservice" => Some("C10067631"),
        "com.huawei.hidisk" => Some("C10055832"),
        "com.huawei.himovie" => Some("C10393298"),
        "com.huawei.music" => Some("C101444301"),
        "com.huawei.browser" => Some("C100170981"),
        _ => None,
    };
    match appid {
        Some(id) => format!("https://appgallery.huawei.com/app/{id}"),
        None => "https://appgallery.huawei.com/app/package".to_string(),
    }
}

pub fn enrich_app_with_curated(app: &mut StoreApp) {
    let pkg = app.package.trim();
    if pkg.is_empty() {
        return;
    }
    let Some(curated) = curated_catalog().into_iter().find(|c| c.package == pkg) else {
        return;
    };
    for cur_offer in curated.sources {
        let cur_url = cur_offer.download_url.trim();
        if cur_url.is_empty() {
            continue;
        }
        if let Some(existing) = app.sources.iter_mut().find(|s| s.source == cur_offer.source) {
            if existing.download_url.trim().is_empty()
                || (!is_direct_apk_url(&existing.download_url) && is_direct_apk_url(cur_url))
            {
                existing.download_url = cur_url.to_string();
                if existing.version_name.trim().is_empty() {
                    existing.version_name = cur_offer.version_name;
                }
                if existing.version_code == 0 {
                    existing.version_code = cur_offer.version_code;
                }
                if existing.size_bytes == 0 {
                    existing.size_bytes = cur_offer.size_bytes;
                }
                if existing.md5.trim().is_empty() {
                    existing.md5 = cur_offer.md5;
                }
            }
        } else {
            app.sources.push(cur_offer);
        }
    }
    if app.developer_apps.is_empty() && !curated.developer_apps.is_empty() {
        app.developer_apps = curated.developer_apps;
    }
}

pub fn ensure_valid_selected_source(app: &mut StoreApp) {
    if app.sources.is_empty() {
        enrich_app_with_curated(app);
    }
    if app.sources.is_empty() {
        app.selected_source_idx = 0;
        return;
    }
    if app.selected_source_idx >= app.sources.len() {
        app.selected_source_idx = 0;
    }
    let cur_url = app.sources[app.selected_source_idx].download_url.trim();
    let needs_upgrade = cur_url.is_empty()
        || (app.selected_source_idx == 0
            && !is_direct_apk_url(cur_url)
            && app.sources.iter().any(|s| is_direct_apk_url(&s.download_url)));
    if needs_upgrade {
        if let Some(idx) = app
            .sources
            .iter()
            .position(|s| is_direct_apk_url(&s.download_url))
            .or_else(|| {
                app.sources
                    .iter()
                    .position(|s| !s.download_url.trim().is_empty())
            })
        {
            app.selected_source_idx = idx;
        }
    }
}

pub fn collect_download_candidates(
    app: &StoreApp,
    preferred_source_idx: Option<usize>,
) -> Vec<StoreSourceOffer> {
    let mut out: Vec<StoreSourceOffer> = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    let push_offer = |out: &mut Vec<StoreSourceOffer>,
                      seen_urls: &mut HashSet<String>,
                      offer: &StoreSourceOffer| {
        let trimmed = offer.download_url.trim();
        if trimmed.is_empty() {
            return;
        }
        if seen_urls.insert(trimmed.to_string()) {
            let mut cloned = offer.clone();
            cloned.download_url = trimmed.to_string();
            if cloned.version_name.trim().is_empty() {
                cloned.version_name = app.version_name().to_string();
            }
            if cloned.size_bytes == 0 {
                cloned.size_bytes = app.size_bytes();
            }
            out.push(cloned);
        }
    };

    if let Some(pref_idx) = preferred_source_idx {
        if let Some(primary) = app.sources.get(pref_idx) {
            push_offer(&mut out, &mut seen_urls, primary);
        }
    } else if let Some(selected) = app
        .sources
        .get(app.selected_source_idx)
        .or_else(|| app.active_offer())
    {
        if is_direct_apk_url(&selected.download_url)
            || !app.sources.iter().any(|s| is_direct_apk_url(&s.download_url))
        {
            push_offer(&mut out, &mut seen_urls, selected);
        }
    }

    for offer in &app.sources {
        if is_direct_apk_url(&offer.download_url) {
            push_offer(&mut out, &mut seen_urls, offer);
        }
    }

    let pkg = app.package.trim();
    if !pkg.is_empty() {
        if let Some(curated_app) = curated_catalog().into_iter().find(|c| c.package == pkg) {
            for offer in &curated_app.sources {
                if is_direct_apk_url(&offer.download_url) {
                    push_offer(&mut out, &mut seen_urls, offer);
                }
            }
        }
    }

    for offer in &app.sources {
        push_offer(&mut out, &mut seen_urls, offer);
    }

    if !pkg.is_empty() {
        if let Some(curated_app) = curated_catalog().into_iter().find(|c| c.package == pkg) {
            for offer in &curated_app.sources {
                push_offer(&mut out, &mut seen_urls, offer);
            }
        }
    }

    let is_huawei_related = app.sources.iter().any(|s| {
        s.source == StoreSource::Huawei
            || s.download_url.contains("dbankcdn.com")
            || s.download_url.contains("dbankcloud.com")
            || s.download_url.contains("appgallery.huawei.com")
    }) || pkg.starts_with("app.huawei.")
        || pkg.starts_with("com.huawei.")
        || pkg.starts_with("com.yinwang.")
        || pkg.starts_with("com.vmall.")
        || pkg.starts_with("com.hihonor.")
        || pkg.starts_with("com.seres.")
        || pkg.starts_with("com.wj.");

    if is_huawei_related
        && !pkg.is_empty()
        && !seen_urls
            .iter()
            .any(|u| u.contains("appgallery.huawei.com/app/"))
    {
        push_offer(
            &mut out,
            &mut seen_urls,
            &StoreSourceOffer {
                source: StoreSource::Huawei,
                version_name: app.version_name().to_string(),
                version_code: app.version_code(),
                size_bytes: app.size_bytes(),
                download_url: huawei_appgallery_page_url(pkg),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
        );
    }

    let all_global_or_oss = !app.sources.is_empty()
        && app.sources.iter().all(|s| {
            matches!(
                s.source,
                StoreSource::Global | StoreSource::FDroid | StoreSource::GitHub
            )
        });

    if !pkg.is_empty()
        && !pkg.starts_with("com.google.")
        && !all_global_or_oss
        && !seen_urls
            .iter()
            .any(|u| u.contains("sj.qq.com/appdetail/"))
    {
        push_offer(
            &mut out,
            &mut seen_urls,
            &StoreSourceOffer {
                source: StoreSource::Tencent,
                version_name: app.version_name().to_string(),
                version_code: app.version_code(),
                size_bytes: app.size_bytes(),
                download_url: format!("https://sj.qq.com/appdetail/{pkg}"),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
        );
    }

    out
}

pub fn download_with_multi_offer_fallback<D>(
    package: &str,
    candidates: &[StoreSourceOffer],
    mut downloader: D,
) -> Result<(PathBuf, StoreSourceOffer), String>
where
    D: FnMut(&StoreSourceOffer) -> Result<PathBuf, String>,
{
    let valid_candidates: Vec<&StoreSourceOffer> = candidates
        .iter()
        .filter(|c| !c.download_url.trim().is_empty())
        .collect();
    if valid_candidates.is_empty() {
        return Err(format!("应用 ({package}) 暂未提供可用的下载源链接"));
    }

    let mut first_err: Option<String> = None;
    for offer in valid_candidates {
        match downloader(offer) {
            Ok(path) => return Ok((path, offer.clone())),
            Err(err) => {
                if first_err.is_none() {
                    first_err = Some(format!("[{}] {}", offer.source.short_label(), err));
                }
            }
        }
    }
    Err(first_err.unwrap_or_else(|| "所有下载源均失败".into()))
}

pub fn apply_detail_to_app(app: &mut StoreApp, payload: &StoreAppDetailPayload) {
    if !payload.description.trim().is_empty() {
        app.description = payload.description.clone();
    }
    if !payload.changelog.trim().is_empty() {
        app.changelog = payload.changelog.clone();
    }
    if !payload.developer.trim().is_empty() {
        app.developer = payload.developer.clone();
    }
    if !payload.icon_url.trim().is_empty() {
        app.icon_url = payload.icon_url.clone();
    }
    if !payload.screenshots.is_empty() {
        app.screenshots = payload.screenshots.clone();
    }
    if payload.rating.is_some() {
        app.rating = payload.rating;
    }
    if !payload.download_count_label.trim().is_empty() {
        app.download_count_label = payload.download_count_label.clone();
    }
    if !payload.icp_number.trim().is_empty() {
        app.icp_number = payload.icp_number.clone();
    }
    if !payload.extra_sources.is_empty() {
        for offer in &payload.extra_sources {
            let trimmed_dl = offer.download_url.trim();
            if trimmed_dl.is_empty() {
                continue;
            }
            if let Some(existing) = app.sources.iter_mut().find(|s| s.source == offer.source) {
                let is_older_offer = !existing.version_name.trim().is_empty()
                    && !offer.version_name.trim().is_empty()
                    && stepx_companion::app_store::compare_android_versions(
                        &existing.version_name,
                        &offer.version_name,
                    ) == std::cmp::Ordering::Greater;
                if is_older_offer && is_direct_apk_url(&existing.download_url) {
                    continue;
                }
                if !offer.version_name.trim().is_empty() {
                    existing.version_name = offer.version_name.clone();
                }
                if offer.version_code > 0 {
                    existing.version_code = offer.version_code;
                }
                if offer.size_bytes > 0 {
                    existing.size_bytes = offer.size_bytes;
                }
                if !is_direct_apk_url(&existing.download_url) || is_direct_apk_url(trimmed_dl) {
                    existing.download_url = trimmed_dl.to_string();
                }
                if !offer.md5.trim().is_empty() {
                    existing.md5 = offer.md5.clone();
                }
                if !offer.updated_at.trim().is_empty() {
                    existing.updated_at = offer.updated_at.clone();
                }
                if !offer.changelog.trim().is_empty() {
                    existing.changelog = offer.changelog.clone();
                }
            } else {
                let mut clean_offer = offer.clone();
                clean_offer.download_url = trimmed_dl.to_string();
                app.sources.push(clean_offer);
            }
        }
        if let Some(tc_offer) = payload.extra_sources.iter().find(|s| {
            s.source == StoreSource::Tencent && !s.download_url.trim().is_empty()
        }) {
            for official in app
                .sources
                .iter_mut()
                .filter(|s| s.source == StoreSource::Official)
            {
                if official.download_url.contains("imtt.dd.qq.com/sjy.00008")
                    || official.download_url.contains("imtt.dd.qq.com/sjy.00009")
                    || official.download_url.contains("sjy.00008")
                    || official.download_url.contains("sjy.00009")
                {
                    if !tc_offer.version_name.trim().is_empty() {
                        official.version_name = tc_offer.version_name.clone();
                    }
                    if tc_offer.version_code > 0 {
                        official.version_code = tc_offer.version_code;
                    }
                    if tc_offer.size_bytes > 0 {
                        official.size_bytes = tc_offer.size_bytes;
                    }
                    official.download_url = tc_offer.download_url.clone();
                    official.md5 = tc_offer.md5.clone();
                }
            }
        }
        if let Some(hw_offer) = payload.extra_sources.iter().find(|s| {
            s.source == StoreSource::Huawei && is_direct_apk_url(&s.download_url)
        }) {
            for official in app
                .sources
                .iter_mut()
                .filter(|s| s.source == StoreSource::Official)
            {
                let should_sync = official.download_url.trim().is_empty()
                    || official.download_url.contains("appgallery.huawei.com")
                    || ((official.download_url.contains("dbankcdn.com")
                        || official.download_url.contains("dbankcloud.com"))
                        && stepx_companion::app_store::compare_android_versions(
                            &official.version_name,
                            &hw_offer.version_name,
                        ) == std::cmp::Ordering::Less);
                if should_sync {
                    if !hw_offer.version_name.trim().is_empty() {
                        official.version_name = hw_offer.version_name.clone();
                    }
                    if hw_offer.version_code > 0 {
                        official.version_code = hw_offer.version_code;
                    }
                    if hw_offer.size_bytes > 0 {
                        official.size_bytes = hw_offer.size_bytes;
                    }
                    official.download_url = hw_offer.download_url.trim().to_string();
                    if !hw_offer.md5.trim().is_empty() {
                        official.md5 = hw_offer.md5.clone();
                    }
                }
            }
        }
    }
    if !payload.historical_versions.is_empty() {
        for hist in &payload.historical_versions {
            if !app
                .historical_versions
                .iter()
                .any(|h| h.version_name == hist.version_name && h.source == hist.source)
            {
                app.historical_versions.push(hist.clone());
            }
        }
    }
    if !payload.developer_apps.is_empty() {
        for dev_app in &payload.developer_apps {
            if dev_app.package != app.package
                && !app
                    .developer_apps
                    .iter()
                    .any(|d| d.package == dev_app.package)
            {
                app.developer_apps.push(dev_app.clone());
            }
        }
    }
    ensure_valid_selected_source(app);
    app.detail_loaded = true;
}

fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "app".into()
    } else {
        trimmed.to_string()
    }
}

fn user_downloads_dir() -> PathBuf {
    if let Some(home) = stepx_companion::resources::user_home() {
        let dl = PathBuf::from(&home).join("Downloads");
        if dl.is_dir() || std::fs::create_dir_all(&dl).is_ok() {
            return dl;
        }
        return PathBuf::from(home);
    }
    std::env::temp_dir()
}

impl Companion {
    pub(super) fn filtered_store_apps(&self) -> Vec<&StoreApp> {
        let query = self.fields[F_STORE_SEARCH].text.trim();
        if let Some(results) = &self.app_store.search_results {
            results.iter().collect()
        } else {
            let sub_tag = self
                .app_store
                .sub_tag
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "全部");
            let prepared_query = (!query.is_empty()).then(|| PreparedExpandedQuery::new(query));
            let installed_by_pkg: Option<
                std::collections::HashMap<&str, &stepx_companion::apps::AppInfo>,
            > = matches!(
                self.app_store.category,
                StoreCategory::Updatable | StoreCategory::Installed
            )
            .then(|| self.apps.iter().map(|a| (a.package.as_str(), a)).collect());
            let mut list: Vec<&StoreApp> = self
                .app_store
                .catalog
                .iter()
                .filter(|app| {
                    if is_blacklisted_store_item(&app.package, &app.name) {
                        return false;
                    }
                    let cat_ok = match self.app_store.category {
                        StoreCategory::All => true,
                        StoreCategory::Updatable => installed_by_pkg
                            .as_ref()
                            .and_then(|m| m.get(app.package.as_str()))
                            .is_some_and(|&entry| {
                                !is_blacklisted_store_item(&entry.package, &entry.label)
                                    && (entry.launchable || app.allow_system_update)
                                    && matches!(
                                        check_store_install_state(std::slice::from_ref(entry), app),
                                        StoreInstallState::Updatable { .. }
                                    )
                            }),
                        StoreCategory::Installed => installed_by_pkg
                            .as_ref()
                            .and_then(|m| m.get(app.package.as_str()))
                            .is_some_and(|&entry| {
                                !is_blacklisted_store_item(&entry.package, &entry.label)
                                    && (entry.launchable || app.allow_system_update)
                            }),
                        other => app.category == other,
                    };
                    let sub_ok = sub_tag.is_none_or(|tag| app_matches_sub_tag(app, tag));
                    cat_ok
                        && sub_ok
                        && prepared_query
                            .as_ref()
                            .is_none_or(|pq| pq.score_app(app) > 0)
                })
                .collect();
            if let Some(tag) = sub_tag {
                if tag != "装机必备" {
                    list.sort_by_key(|app| {
                        let exact_cat = app.category_label.trim() == tag;
                        let tagline_hit = app.tagline.contains(tag);
                        (!exact_cat, !app.featured, !tagline_hit)
                    });
                }
            }
            list
        }
    }

    pub(super) fn prefetch_current_store_page_icons(&self) {
        let filtered = self.filtered_store_apps();
        if filtered.is_empty() {
            return;
        }
        let page_size = self.app_store.page_size.max(1);
        let total_pages = (filtered.len() + page_size - 1) / page_size;
        let page = self.app_store.page.min(total_pages.saturating_sub(1));
        let start = page * page_size;
        let end = (start + page_size).min(filtered.len());
        if start < end {
            AppStoreUiState::spawn_icon_prefetcher_refs(
                &filtered[start..end],
                self.app_store.tx.clone(),
            );
        }
    }

    pub(super) fn start_online_catalog_sync(&mut self, force: bool) {
        if self.app_store.syncing_catalog {
            return;
        }
        if !force && (self.app_store.catalog_is_fresh && self.app_store.online_total_count >= 150) {
            return;
        }
        self.app_store.syncing_catalog = true;
        let tx = self.app_store.tx.clone();
        std::thread::spawn(move || {
            let result = fetch_online_catalog_from_upstream().map_err(|e| format!("{e:#}"));
            let _ = tx.send(StoreWorkerEvent::CatalogSyncFinished { result });
        });
    }

    pub(super) fn trigger_dynamic_catalog_expansion(&mut self, seed_query: &str) {
        if self.demo_mode || self.app_store.searching || self.app_store.syncing_catalog {
            return;
        }
        let seeds: Vec<String> = seed_query
            .split('|')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if seeds.is_empty() {
            return;
        }
        self.app_store.syncing_catalog = true;
        let tx = self.app_store.tx.clone();
        let target_cat = Some(self.app_store.category);
        let target_sub = self.app_store.sub_tag.clone();
        std::thread::spawn(move || {
            let result = expand_category_catalog_batch(&seeds, target_cat, target_sub)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(StoreWorkerEvent::CatalogSyncFinished { result });
        });
    }

    pub(super) fn expand_catalog_if_needed(&mut self) {
        if self.app_store.search_results.is_none()
            && matches!(
                self.app_store.category,
                StoreCategory::Updatable | StoreCategory::Installed
            )
        {
            return;
        }
        if self.app_store.search_results.is_some()
            && !self.app_store.last_search_query.trim().is_empty()
        {
            let q = self.app_store.last_search_query.clone();
            self.trigger_dynamic_catalog_expansion(&q);
            return;
        }
        let cursor_key = format!(
            "{:?}:{}",
            self.app_store.category,
            self.app_store.sub_tag.as_deref().unwrap_or("")
        );
        let bank: &[&str] = match (
            self.app_store.category,
            self.app_store.sub_tag.as_deref().unwrap_or(""),
        ) {
            (_, "AI 助手") => &[
                "chatgpt", "claude", "gemini", "deepseek", "豆包", "kimi", "perplexity", "grok",
                "copilot", "poe", "大模型", "智能助手", "文心一言", "通义千问", "腾讯元宝", "智谱清言",
            ],
            (_, "NAS 私有云") => &[
                "绿联云", "极空间", "飞牛私有云", "群晖", "synology", "immich", "jellyfin",
                "nextcloud", "tailscale", "zerotier", "alist", "webdav", "samba", "私有云",
            ],
            (StoreCategory::Social, _) => &[
                "社交", "聊天", "社区", "论坛", "微博", "小红书", "贴吧", "知乎", "豆瓣", "虎扑",
                "telegram", "discord", "whatsapp", "twitter", "instagram", "facebook", "reddit",
                "陌陌", "探探", "Soul", "语音交友", "开黑", "TT语音", "兴趣圈", "同城交友", "树洞",
                "signal", "mastodon", "messenger", "名片", "博客", "校友",
            ],
            (StoreCategory::Media, _) => &[
                "视频", "影视", "音乐", "听书", "播客", "动漫", "直播", "短视频", "小说", "漫画",
                "播放器", "修图", "相机", "剪辑", "壁纸", "电台", "K歌", "弹幕", "youtube", "spotify",
                "vlc", "netflix", "twitch", "相册", "滤镜", "投屏", "字幕", "无损音乐",
            ],
            (StoreCategory::Productivity, _) => &[
                "办公", "文档", "笔记", "网盘", "邮箱", "扫描", "翻译", "词典", "会议", "日历",
                "待办", "思维导图", "PDF", "表格", "背单词", "远程桌面", "chatgpt", "claude",
                "notion", "obsidian", "云盘", "学习", "题库", "番茄钟",
            ],
            (StoreCategory::Tools, _) => &[
                "工具", "浏览器", "输入法", "文件管理", "解压缩", "计算器", "清理", "测速", "密码管理",
                "安全", "组网", "终端", "开发者", "备份", "换机", "遥控器", "手电筒", "悬浮球",
                "自动化", "截图", "录屏", "蓝牙", "WiFi", "尺子",
            ],
            (StoreCategory::Lifestyle, _) => &[
                "购物", "外卖", "地图", "导航", "打车", "旅游", "酒店", "火车票", "银行", "记账",
                "理财", "快递", "买菜", "美食", "菜谱", "运动", "健身", "天气", "万年历", "租房",
                "汽车", "充电桩", "公交", "地铁", "医疗", "电影票",
            ],
            _ => &[
                "装机必备", "热门应用", "社交聊天", "视频音乐", "效率办公", "AI大模型", "实用工具",
                "生活出行", "网购外卖", "地图导航", "读书漫画", "拍照修图", "网盘存储", "浏览器",
                "游戏社区", "新闻资讯",
            ],
        };
        let cursor = self
            .app_store
            .expansion_cursor
            .entry(cursor_key)
            .or_insert(0);
        let start = *cursor;
        let mut batch = Vec::with_capacity(4);
        for i in 0..4 {
            batch.push(bank[(start + i) % bank.len()]);
        }
        *cursor = (start + 4) % bank.len();
        let joined = batch.join("|");
        self.trigger_dynamic_catalog_expansion(&joined);
    }

    pub(super) fn ensure_store_online_catalog_synced(&mut self) {
        if self.demo_mode || self.app_store.catalog_auto_sync_triggered {
            return;
        }
        self.app_store.catalog_auto_sync_triggered = true;
        self.start_online_catalog_sync(false);
    }

    pub(super) fn sync_app_store_with_installed(&mut self) {
        if self.demo_mode || self.apps.is_empty() {
            return;
        }
        if !self.app_store.update_checked_once && !self.app_store.checking_updates {
            self.spawn_store_update_check();
        }
    }

    fn spawn_store_update_check(&mut self) {
        if self.app_store.checking_updates || self.apps.is_empty() {
            return;
        }
        self.app_store.checking_updates = true;
        let installed = self.apps.clone();
        let existing = self.app_store.catalog.clone();
        let tx = self.app_store.tx.clone();
        std::thread::spawn(move || {
            let apps = check_updates_for_installed(&installed, &existing);
            let _ = tx.send(StoreWorkerEvent::UpdatesChecked { apps });
        });
    }

    pub(super) fn on_store_search_edited(&mut self) {
        let trimmed = self.fields[F_STORE_SEARCH].text.trim().to_string();
        self.app_store.page = 0;
        if trimmed.is_empty() {
            self.app_store.search_results = None;
            self.app_store.last_search_query.clear();
            self.app_store.searching = false;
            self.prefetch_current_store_page_icons();
            return;
        }
        self.app_store.last_search_query = trimmed.clone();
        let (mut instant_hits, cache_is_fresh) = search_instant_local(&trimmed);
        AppStoreUiState::preserve_runtime_fields(&mut instant_hits, &self.app_store.catalog);
        if !instant_hits.is_empty() {
            let page_n = instant_hits.len().min(self.app_store.page_size.max(1));
            AppStoreUiState::spawn_icon_prefetcher(
                &instant_hits[..page_n],
                self.app_store.tx.clone(),
            );
            self.app_store.search_results = Some(instant_hits);
        } else {
            self.app_store.search_results = None;
        }
        if !cache_is_fresh && trimmed.chars().count() >= 2 {
            let tx = self.app_store.tx.clone();
            let q_debounce = trimmed;
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(280));
                let _ = tx.send(StoreWorkerEvent::DebouncedSearchTrigger { query: q_debounce });
            });
        }
    }

    fn start_store_search(&mut self, query: String) {
        let trimmed = query.trim().to_string();
        if trimmed.is_empty() {
            self.fields[F_STORE_SEARCH].clear();
            self.app_store.search_results = None;
            self.app_store.last_search_query.clear();
            self.app_store.searching = false;
            self.prefetch_current_store_page_icons();
            return;
        }
        self.app_store.last_search_query = trimmed.clone();
        let (mut instant_hits, cache_is_fresh) = search_instant_local(&trimmed);
        AppStoreUiState::preserve_runtime_fields(&mut instant_hits, &self.app_store.catalog);
        if !instant_hits.is_empty() {
            let page_n = instant_hits.len().min(self.app_store.page_size.max(1));
            AppStoreUiState::spawn_icon_prefetcher(
                &instant_hits[..page_n],
                self.app_store.tx.clone(),
            );
            if instant_hits.len() > page_n {
                AppStoreUiState::spawn_icon_prefetcher(
                    &instant_hits[page_n..],
                    self.app_store.tx.clone(),
                );
            }
            self.app_store.search_results = Some(instant_hits.clone());
        }
        if cache_is_fresh && !instant_hits.is_empty() {
            self.app_store.searching = false;
            self.status = format!("已聚合「{trimmed}」多源搜索结果");
            return;
        }
        self.app_store.searching = true;
        self.status = format!("正在多源检索「{trimmed}」…");
        let tx = self.app_store.tx.clone();
        std::thread::spawn(move || {
            let result = search_multi_source(&trimmed).map_err(|e| format!("{e:#}"));
            let _ = tx.send(StoreWorkerEvent::SearchFinished {
                query: trimmed,
                result,
            });
        });
    }

    fn start_app_detail_fetch(&mut self, app: StoreApp) {
        let pkg = app.package.clone();
        if self.app_store.loading_detail_pkg.as_deref() == Some(pkg.as_str()) {
            return;
        }
        if !app.detail_loaded {
            self.app_store.loading_detail_pkg = Some(pkg.clone());
        }
        self.app_store.detail_fetched_pkgs.insert(pkg);
        AppStoreUiState::spawn_detail_fetcher(&app, self.app_store.tx.clone());
    }

    fn start_download_and_install(
        &mut self,
        package: String,
        app_name: String,
        version_name: String,
        candidates: Vec<StoreSourceOffer>,
        size_bytes: u64,
        is_downgrade: bool,
        version_idx: Option<usize>,
        force_reinstall: bool,
    ) {
        if self.app_store.tasks.get(&package).is_some_and(|t| {
            matches!(
                t.stage,
                StoreTaskStage::Downloading { .. }
                    | StoreTaskStage::Verifying
                    | StoreTaskStage::Installing
                    | StoreTaskStage::Uninstalling
            )
        }) {
            self.status = format!("「{app_name}」正在下载或安装中，请稍候…");
            return;
        }

        let valid_candidates: Vec<StoreSourceOffer> = candidates
            .into_iter()
            .filter(|c| !c.download_url.trim().is_empty())
            .collect();
        if valid_candidates.is_empty() {
            self.app_store.status_banner = Some((
                format!("「{app_name}」当前来源暂未提供直接下载链接，请尝试切换其他来源"),
                true,
            ));
            return;
        }

        let initial_device = self.selected.clone();
        if initial_device.is_none() {
            self.app_store.status_banner = Some((
                format!(
                    "正在下载「{app_name}」v{version_name}（如检测到已连接手机将自动安装，否则保存至 Downloads 目录）…"
                ),
                false,
            ));
        }

        self.app_store.tasks.insert(
            package.clone(),
            StoreTaskState {
                package: package.clone(),
                app_name: app_name.clone(),
                target_version: version_name.clone(),
                stage: StoreTaskStage::Downloading {
                    percent: 0,
                    downloaded_bytes: 0,
                    total_bytes: size_bytes,
                },
                is_downgrade,
            },
        );
        self.status = format!("正在下载 {app_name} v{version_name}…");

        let tx = self.app_store.tx.clone();
        std::thread::spawn(move || {
            let pkg_progress = package.clone();
            let tx_progress = tx.clone();
            let mut attempt_idx = 0usize;

            let dl_res = download_with_multi_offer_fallback(
                &package,
                &valid_candidates,
                |offer| {
                    let cur_attempt = attempt_idx;
                    attempt_idx += 1;
                    let offer_ver = if offer.version_name.trim().is_empty() {
                        version_name.as_str()
                    } else {
                        offer.version_name.as_str()
                    };
                    let offer_size = if offer.size_bytes > 0 {
                        offer.size_bytes
                    } else {
                        size_bytes
                    };
                    if cur_attempt > 0 {
                        let _ = tx_progress.send(StoreWorkerEvent::TaskProgress {
                            package: pkg_progress.clone(),
                            stage: StoreTaskStage::Downloading {
                                percent: 0,
                                downloaded_bytes: 0,
                                total_bytes: offer_size,
                            },
                        });
                    }
                    let mut last_emit = Instant::now() - Duration::from_secs(1);
                    let mut last_pct: Option<u8> = None;
                    download_apk_with_progress(
                        &offer.download_url,
                        &package,
                        offer_ver,
                        |downloaded, total| {
                            let effective_total = if total > 0 { total } else { offer_size };
                            if effective_total > 0 && downloaded >= effective_total {
                                if last_pct != Some(100) {
                                    last_pct = Some(100);
                                    let _ = tx_progress.send(StoreWorkerEvent::TaskProgress {
                                        package: pkg_progress.clone(),
                                        stage: StoreTaskStage::Verifying,
                                    });
                                }
                                return;
                            }
                            let pct = if effective_total > 0 {
                                ((downloaded as f64 / effective_total as f64) * 100.0)
                                    .round()
                                    .clamp(0.0, 99.0) as u8
                            } else {
                                25
                            };
                            let elapsed = last_emit.elapsed();
                            if (last_pct != Some(pct) && elapsed >= Duration::from_millis(60))
                                || elapsed >= Duration::from_millis(180)
                            {
                                last_pct = Some(pct);
                                last_emit = Instant::now();
                                let _ = tx_progress.send(StoreWorkerEvent::TaskProgress {
                                    package: pkg_progress.clone(),
                                    stage: StoreTaskStage::Downloading {
                                        percent: pct,
                                        downloaded_bytes: downloaded,
                                        total_bytes: effective_total,
                                    },
                                });
                            }
                        },
                    )
                    .map_err(|e| format!("{e:#}"))
                },
            );

            let (apk_path, used_offer) = match dl_res {
                Ok(res) => res,
                Err(err) => {
                    let _ = tx.send(StoreWorkerEvent::TaskFinished {
                        package,
                        app_name,
                        target_version: version_name,
                        is_downgrade,
                        version_idx,
                        result: Err(format!("下载失败：{err}")),
                        refreshed_apps: None,
                    });
                    return;
                }
            };

            let effective_ver = if used_offer.version_name.trim().is_empty() {
                version_name
            } else {
                used_offer.version_name
            };

            let resolved_device = initial_device.or_else(|| {
                let adb = Adb::discover().ok()?;
                let raw = adb
                    .run_timeout(&["devices", "-l"], Duration::from_secs(3))
                    .ok()?;
                for t in stepx_companion::device::parse_devices(&raw) {
                    if t.state == "device" {
                        if let Ok(dev) = stepx_companion::device::Device::inspect(&adb, &t.id) {
                            return Some(dev);
                        }
                    }
                }
                None
            });

            let Some(device) = resolved_device else {
                let dest_dir = user_downloads_dir();
                let filename = format!(
                    "{}_v{}.apk",
                    sanitize_filename(&package),
                    sanitize_filename(&effective_ver)
                );
                let dest = dest_dir.join(filename);
                if apk_path != dest {
                    let _ = std::fs::copy(&apk_path, &dest);
                }
                let final_path = if dest.is_file() { dest } else { apk_path };
                let _ = tx.send(StoreWorkerEvent::ApkDownloadedOnly {
                    package,
                    app_name,
                    target_version: effective_ver,
                    result: Ok(final_path),
                });
                return;
            };

            let _ = tx.send(StoreWorkerEvent::TaskProgress {
                package: package.clone(),
                stage: StoreTaskStage::Verifying,
            });
            let _ = tx.send(StoreWorkerEvent::TaskProgress {
                package: package.clone(),
                stage: StoreTaskStage::Installing,
            });

            let install_res = Adb::discover().and_then(|adb| {
                let msg = device.install_apk_with_options(
                    &adb,
                    &apk_path,
                    is_downgrade,
                    &package,
                    force_reinstall.then_some(package.as_str()),
                )?;
                let refreshed = apps::catalog(&adb, &device).ok();
                Ok((msg, refreshed))
            });

            match install_res {
                Ok((_msg, refreshed_apps)) => {
                    let done_label = if is_downgrade {
                        format!("已安装旧版 v{effective_ver}")
                    } else {
                        format!("已安装 v{effective_ver}")
                    };
                    let _ = tx.send(StoreWorkerEvent::TaskFinished {
                        package,
                        app_name,
                        target_version: effective_ver,
                        is_downgrade,
                        version_idx,
                        result: Ok(done_label),
                        refreshed_apps,
                    });
                }
                Err(err) => {
                    let mut err_str = format!("{err:#}");
                    if !is_downgrade || !err_str.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
                        let dest_dir = user_downloads_dir();
                        let filename = format!(
                            "{}_v{}.apk",
                            sanitize_filename(&package),
                            sanitize_filename(&effective_ver)
                        );
                        let dest = dest_dir.join(filename);
                        if apk_path != dest {
                            let _ = std::fs::copy(&apk_path, &dest);
                        }
                        if dest.is_file() {
                            err_str = format!("{err_str}（APK 已保存至 {}）", dest.display());
                        }
                    }
                    let _ = tx.send(StoreWorkerEvent::TaskFinished {
                        package,
                        app_name,
                        target_version: effective_ver,
                        is_downgrade,
                        version_idx,
                        result: Err(err_str),
                        refreshed_apps: None,
                    });
                }
            }
        });
    }

    fn start_download_apk_only(
        &mut self,
        package: String,
        app_name: String,
        version_name: String,
        candidates: Vec<StoreSourceOffer>,
        size_bytes: u64,
    ) {
        if self.app_store.tasks.get(&package).is_some_and(|t| {
            matches!(
                t.stage,
                StoreTaskStage::Downloading { .. }
                    | StoreTaskStage::Verifying
                    | StoreTaskStage::Installing
                    | StoreTaskStage::Uninstalling
            )
        }) {
            self.status = format!("「{app_name}」正在下载中，请稍候…");
            return;
        }

        let valid_candidates: Vec<StoreSourceOffer> = candidates
            .into_iter()
            .filter(|c| !c.download_url.trim().is_empty())
            .collect();
        if valid_candidates.is_empty() {
            self.app_store.status_banner = Some((
                format!("「{app_name}」当前来源暂未提供直接下载链接"),
                true,
            ));
            return;
        }
        self.app_store.tasks.insert(
            package.clone(),
            StoreTaskState {
                package: package.clone(),
                app_name: app_name.clone(),
                target_version: version_name.clone(),
                stage: StoreTaskStage::Downloading {
                    percent: 0,
                    downloaded_bytes: 0,
                    total_bytes: size_bytes,
                },
                is_downgrade: false,
            },
        );
        self.status = format!("正在下载 {app_name} v{version_name} APK 到本机…");

        let tx = self.app_store.tx.clone();
        std::thread::spawn(move || {
            let pkg_progress = package.clone();
            let tx_progress = tx.clone();
            let mut attempt_idx = 0usize;

            let dl_res = download_with_multi_offer_fallback(
                &package,
                &valid_candidates,
                |offer| {
                    let cur_attempt = attempt_idx;
                    attempt_idx += 1;
                    let offer_ver = if offer.version_name.trim().is_empty() {
                        version_name.as_str()
                    } else {
                        offer.version_name.as_str()
                    };
                    let offer_size = if offer.size_bytes > 0 {
                        offer.size_bytes
                    } else {
                        size_bytes
                    };
                    if cur_attempt > 0 {
                        let _ = tx_progress.send(StoreWorkerEvent::TaskProgress {
                            package: pkg_progress.clone(),
                            stage: StoreTaskStage::Downloading {
                                percent: 0,
                                downloaded_bytes: 0,
                                total_bytes: offer_size,
                            },
                        });
                    }
                    let mut last_emit = Instant::now() - Duration::from_secs(1);
                    let mut last_pct: Option<u8> = None;
                    download_apk_with_progress(
                        &offer.download_url,
                        &package,
                        offer_ver,
                        |downloaded, total| {
                            let effective_total = if total > 0 { total } else { offer_size };
                            if effective_total > 0 && downloaded >= effective_total {
                                if last_pct != Some(100) {
                                    last_pct = Some(100);
                                    let _ = tx_progress.send(StoreWorkerEvent::TaskProgress {
                                        package: pkg_progress.clone(),
                                        stage: StoreTaskStage::Verifying,
                                    });
                                }
                                return;
                            }
                            let pct = if effective_total > 0 {
                                ((downloaded as f64 / effective_total as f64) * 100.0)
                                    .round()
                                    .clamp(0.0, 99.0) as u8
                            } else {
                                25
                            };
                            let elapsed = last_emit.elapsed();
                            if (last_pct != Some(pct) && elapsed >= Duration::from_millis(60))
                                || elapsed >= Duration::from_millis(180)
                            {
                                last_pct = Some(pct);
                                last_emit = Instant::now();
                                let _ = tx_progress.send(StoreWorkerEvent::TaskProgress {
                                    package: pkg_progress.clone(),
                                    stage: StoreTaskStage::Downloading {
                                        percent: pct,
                                        downloaded_bytes: downloaded,
                                        total_bytes: effective_total,
                                    },
                                });
                            }
                        },
                    )
                    .map_err(|e| format!("{e:#}"))
                },
            );

            let mut final_ver = version_name.clone();
            let res = dl_res.and_then(|(cached_path, used_offer)| {
                if !used_offer.version_name.trim().is_empty() {
                    final_ver = used_offer.version_name;
                }
                let dest_dir = user_downloads_dir();
                let filename = format!(
                    "{}_v{}.apk",
                    sanitize_filename(&package),
                    sanitize_filename(&final_ver)
                );
                let dest = dest_dir.join(filename);
                if cached_path != dest {
                    let _ = std::fs::copy(&cached_path, &dest);
                }
                if dest.is_file() {
                    Ok(dest)
                } else {
                    Ok(cached_path)
                }
            });

            let _ = tx.send(StoreWorkerEvent::ApkDownloadedOnly {
                package,
                app_name,
                target_version: final_ver,
                result: res,
            });
        });
    }

    fn advance_batch_queue(&mut self) {
        if let Some(next_pkg) = self.app_store.batch_queue.first().cloned() {
            self.app_store.batch_queue.remove(0);
            if let Some(app) = self.app_store.find_app_for_install(&next_pkg) {
                let candidates = collect_download_candidates(&app, None);
                if let Some(primary) = candidates.first().cloned() {
                    self.start_download_and_install(
                        app.package,
                        app.name,
                        primary.version_name,
                        candidates,
                        primary.size_bytes,
                        false,
                        None,
                        false,
                    );
                }
            }
        }
    }

    pub(super) fn handle_app_store_action(
        &mut self,
        action: AppStoreAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AppStoreAction::SelectCategory(cat) => {
                let _ = category_sub_tags(cat);
                self.app_store.category = cat;
                self.app_store.sub_tag = None;
                self.app_store.page = 0;
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                if self.app_store.search_results.is_some() {
                    self.app_store.search_results = None;
                    self.app_store.last_search_query.clear();
                    self.fields[F_STORE_SEARCH].clear();
                } else {
                    self.app_store.search_results = None;
                }
                self.scroll = Default::default();
                self.prefetch_current_store_page_icons();
                if cat == StoreCategory::Updatable
                    && self.selected.is_some()
                    && !self.app_store.update_checked_once
                    && !self.app_store.checking_updates
                {
                    if !self.apps_loaded {
                        self.handle_device_action(&Action::Apps);
                    } else {
                        self.spawn_store_update_check();
                    }
                } else if !matches!(cat, StoreCategory::Updatable | StoreCategory::Installed) {
                    self.app_store.auto_chain_remaining = 2;
                    self.expand_catalog_if_needed();
                }
            }
            AppStoreAction::SelectSubTag(sub_tag) => {
                let seed_tag = sub_tag.clone();
                self.app_store.sub_tag = sub_tag;
                self.app_store.page = 0;
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                if self.app_store.search_results.is_some() {
                    self.app_store.search_results = None;
                    self.app_store.last_search_query.clear();
                    self.fields[F_STORE_SEARCH].clear();
                }
                self.scroll = Default::default();
                self.prefetch_current_store_page_icons();
                if seed_tag.is_some() {
                    self.app_store.auto_chain_remaining = 2;
                    self.expand_catalog_if_needed();
                }
            }
            AppStoreAction::SelectApp(package) => {
                if !self
                    .app_store
                    .catalog
                    .iter()
                    .any(|a| a.package == package)
                {
                    if let Some(app_from_search) = self
                        .app_store
                        .search_results
                        .as_ref()
                        .and_then(|res| res.iter().find(|a| a.package == package))
                        .cloned()
                        .or_else(|| curated_catalog().into_iter().find(|a| a.package == package))
                    {
                        self.app_store.catalog.push(app_from_search);
                    }
                }
                self.app_store.mutate_app(&package, |app| {
                    enrich_app_with_curated(app);
                    ensure_valid_selected_source(app);
                });
                let mut cache_fresh = false;
                if let Some((cached_payload, is_fresh)) = get_cached_app_detail(&package) {
                    cache_fresh = is_fresh;
                    self.app_store.mutate_app(&package, |app| {
                        apply_detail_to_app(app, &cached_payload);
                    });
                }
                self.app_store.selected_package = Some(package.clone());
                self.app_store.lightbox = None;
                self.app_store.detail_scroll = ClippedScrollStateHandle::default();
                self.trigger_subtab_switch();
                if let Some(app) = self.app_store.find_app(&package).cloned() {
                    let mut related_to_prefetch = vec![app.clone()];
                    for other in &self.app_store.catalog {
                        if other.package != app.package && other.category == app.category {
                            related_to_prefetch.push(other.clone());
                            if related_to_prefetch.len() >= 8 {
                                break;
                            }
                        }
                    }
                    AppStoreUiState::spawn_icon_prefetcher(
                        &related_to_prefetch,
                        self.app_store.tx.clone(),
                    );
                    if app.cached_screenshots.is_empty() && !app.screenshots.is_empty() {
                        AppStoreUiState::spawn_screenshot_prefetcher(
                            &app.package,
                            &app.screenshots,
                            self.app_store.tx.clone(),
                        );
                    }
                    let has_complete_detail = app.detail_loaded && !app.screenshots.is_empty();
                    if !has_complete_detail || !cache_fresh {
                        self.start_app_detail_fetch(app);
                    }
                }
            }
            AppStoreAction::CloseDetail => {
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                self.trigger_subtab_switch();
            }
            AppStoreAction::SetPage(p) => {
                let total_apps = self.filtered_store_apps().len();
                let page_size = self.app_store.page_size.max(1);
                let total_pages = total_apps.div_ceil(page_size).max(1);
                self.app_store.page = p;
                self.scroll = Default::default();
                self.prefetch_current_store_page_icons();
                if p + 3 >= total_pages {
                    self.app_store.auto_chain_remaining = 1;
                    self.expand_catalog_if_needed();
                }
            }
            AppStoreAction::PrevPage => {
                if self.app_store.page > 0 {
                    self.app_store.page -= 1;
                    self.scroll = Default::default();
                    self.prefetch_current_store_page_icons();
                }
            }
            AppStoreAction::NextPage => {
                let total_apps = self.filtered_store_apps().len();
                let page_size = self.app_store.page_size.max(1);
                let total_pages = total_apps.div_ceil(page_size).max(1);
                let max_page = total_pages.saturating_sub(1);
                let next_p = self.app_store.page.saturating_add(1).min(max_page);
                self.app_store.page = next_p;
                self.scroll = Default::default();
                self.prefetch_current_store_page_icons();
                if next_p + 3 >= total_pages {
                    self.app_store.auto_chain_remaining = 1;
                    self.expand_catalog_if_needed();
                }
            }
            AppStoreAction::RefreshDetail(pkg) => {
                self.app_store.detail_fetched_pkgs.remove(&pkg);
                self.app_store.loading_detail_pkg = Some(pkg.clone());
                self.app_store.mutate_app(&pkg, |app| {
                    app.detail_loaded = false;
                });
                if let Some(app) = self.app_store.find_app(&pkg).cloned() {
                    self.app_store.detail_fetched_pkgs.insert(pkg);
                    AppStoreUiState::spawn_detail_fetcher(&app, self.app_store.tx.clone());
                }
            }
            AppStoreAction::SelectSource {
                package,
                source_idx,
            } => {
                self.app_store.mutate_app(&package, |app| {
                    if source_idx < app.sources.len() {
                        app.selected_source_idx = source_idx;
                    }
                });
            }
            AppStoreAction::SearchSubmit => {
                self.app_store.page = 0;
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                self.scroll = Default::default();
                let q = self.fields[F_STORE_SEARCH].text.trim().to_string();
                self.start_store_search(q);
            }
            AppStoreAction::QuickSearch(kw) => {
                self.app_store.page = 0;
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                self.scroll = Default::default();
                self.fields[F_STORE_SEARCH].set(&kw);
                self.start_store_search(kw);
            }
            AppStoreAction::ClearSearch => {
                self.fields[F_STORE_SEARCH].clear();
                self.app_store.search_results = None;
                self.app_store.last_search_query.clear();
                self.app_store.searching = false;
                self.app_store.page = 0;
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                self.scroll = Default::default();
                self.prefetch_current_store_page_icons();
            }
            AppStoreAction::RefreshCatalog => {
                let mut fresh = curated_catalog();
                fresh.extend(self.app_store.catalog.clone());
                let mut merged = merge_and_deduplicate_apps(fresh);
                AppStoreUiState::preserve_runtime_fields(&mut merged, &self.app_store.catalog);
                self.app_store.catalog = merged;
                self.prefetch_current_store_page_icons();
                self.start_online_catalog_sync(true);
                if self.selected.is_some() {
                    if !self.apps_loaded {
                        self.handle_device_action(&Action::Apps);
                    } else {
                        self.spawn_store_update_check();
                    }
                } else {
                    self.status = "正在从多源在线应用库实时同步最新应用…".into();
                }
            }
            AppStoreAction::SyncOnlineCatalog { force } => {
                let _ = force;
                self.start_online_catalog_sync(true);
            }
            AppStoreAction::CheckUpdates => {
                if self.selected.is_none() {
                    self.app_store.status_banner = Some((
                        "当前处于离线浏览模式，请先通过 USB 或无线连接手机后再检测应用更新".into(),
                        false,
                    ));
                    self.status = "请先连接手机后再检测更新".into();
                    return;
                }
                if !self.apps_loaded {
                    self.app_store.update_checked_once = false;
                    self.handle_device_action(&Action::Apps);
                    return;
                }
                self.spawn_store_update_check();
            }
            AppStoreAction::InstallOrUpdate { package } => {
                let Some(app) = self.app_store.find_app_for_install(&package) else {
                    return;
                };
                let candidates = collect_download_candidates(&app, None);
                let Some(primary) = candidates.first().cloned() else {
                    self.app_store.status_banner = Some((
                        format!("「{}」当前来源暂未提供直接下载链接", app.name),
                        true,
                    ));
                    return;
                };
                self.start_download_and_install(
                    app.package,
                    app.name,
                    primary.version_name,
                    candidates,
                    primary.size_bytes,
                    false,
                    None,
                    false,
                );
            }
            AppStoreAction::InstallFromSource {
                package,
                source_idx,
            } => {
                self.app_store.mutate_app(&package, |app| {
                    if source_idx < app.sources.len() {
                        app.selected_source_idx = source_idx;
                    }
                });
                let Some(app) = self.app_store.find_app_for_install(&package) else {
                    return;
                };
                let candidates = collect_download_candidates(&app, Some(source_idx));
                let Some(primary) = candidates.first().cloned() else {
                    self.app_store.status_banner = Some((
                        format!("「{}」当前来源暂未提供直接下载链接", app.name),
                        true,
                    ));
                    return;
                };
                self.start_download_and_install(
                    app.package,
                    app.name,
                    primary.version_name,
                    candidates,
                    primary.size_bytes,
                    false,
                    None,
                    false,
                );
            }
            AppStoreAction::InstallHistorical {
                package,
                version_idx,
                force_reinstall,
            } => {
                self.app_store.pending_downgrade_reinstall = None;
                let Some(app) = self.app_store.find_app_for_install(&package) else {
                    return;
                };
                let Some(hist) = app.historical_versions.get(version_idx).cloned() else {
                    return;
                };
                let hist_offer = StoreSourceOffer {
                    source: hist.source,
                    version_name: hist.version_name.clone(),
                    version_code: hist.version_code,
                    size_bytes: hist.size_bytes,
                    download_url: hist.download_url.clone(),
                    updated_at: hist.release_date.clone(),
                    md5: String::new(),
                    changelog: String::new(),
                };
                self.start_download_and_install(
                    app.package,
                    app.name,
                    hist.version_name,
                    vec![hist_offer],
                    hist.size_bytes,
                    true,
                    Some(version_idx),
                    force_reinstall,
                );
            }
            AppStoreAction::DownloadApkOnly { package } => {
                let Some(app) = self.app_store.find_app_for_install(&package) else {
                    return;
                };
                let candidates = collect_download_candidates(&app, None);
                let Some(primary) = candidates.first().cloned() else {
                    self.app_store.status_banner = Some((
                        format!("「{}」当前来源暂未提供直接下载链接", app.name),
                        true,
                    ));
                    return;
                };
                self.start_download_apk_only(
                    app.package,
                    app.name,
                    primary.version_name,
                    candidates,
                    primary.size_bytes,
                );
            }
            AppStoreAction::UpdateAll => {
                if self.selected.is_none() {
                    self.app_store.status_banner =
                        Some(("请先连接手机后再执行一键全部更新".into(), true));
                    return;
                }
                let updatable: Vec<StoreApp> = self
                    .app_store
                    .catalog
                    .iter()
                    .filter(|app| {
                        matches!(
                            check_store_install_state(&self.apps, app),
                            StoreInstallState::Updatable { .. }
                        )
                    })
                    .cloned()
                    .collect();
                if updatable.is_empty() {
                    self.app_store.status_banner =
                        Some(("手机上的所有已安装应用均为最新版本".into(), false));
                    return;
                }
                self.app_store.batch_total = updatable.len();
                self.app_store.batch_done = 0;
                self.app_store.batch_queue.clear();
                for app in &updatable {
                    self.app_store.tasks.insert(
                        app.package.clone(),
                        StoreTaskState {
                            package: app.package.clone(),
                            app_name: app.name.clone(),
                            target_version: app.version_name().to_string(),
                            stage: StoreTaskStage::Queued,
                            is_downgrade: false,
                        },
                    );
                    self.app_store.batch_queue.push(app.package.clone());
                }
                self.advance_batch_queue();
            }
            AppStoreAction::RequestUninstall(pkg) => {
                self.app_store.pending_uninstall = Some(pkg);
            }
            AppStoreAction::CancelUninstall => {
                self.app_store.pending_uninstall = None;
            }
            AppStoreAction::ConfirmUninstall(pkg) => {
                self.app_store.pending_uninstall = None;
                let Some(device) = self.selected.clone() else {
                    self.app_store.status_banner =
                        Some(("请先连接手机后再卸载应用".into(), true));
                    return;
                };
                let app_name = self
                    .app_store
                    .find_app(&pkg)
                    .map(|a| a.name.clone())
                    .or_else(|| {
                        self.apps
                            .iter()
                            .find(|a| a.package == pkg)
                            .map(|a| apps::clean_app_label(&a.package, &a.label))
                    })
                    .unwrap_or_else(|| pkg.clone());

                self.app_store.tasks.insert(
                    pkg.clone(),
                    StoreTaskState {
                        package: pkg.clone(),
                        app_name: app_name.clone(),
                        target_version: String::new(),
                        stage: StoreTaskStage::Uninstalling,
                        is_downgrade: false,
                    },
                );
                self.status = format!("正在从手机卸载「{app_name}」…");
                let tx = self.app_store.tx.clone();
                let app_name_for_msg = app_name.clone();
                std::thread::spawn(move || {
                    let res = Adb::discover().and_then(|adb| {
                        apps::uninstall(&adb, &device, &pkg)?;
                        let refreshed = apps::catalog(&adb, &device).ok();
                        Ok((format!("已卸载「{app_name_for_msg}」"), refreshed))
                    });
                    match res {
                        Ok((msg, refreshed_apps)) => {
                            let _ = tx.send(StoreWorkerEvent::UninstallFinished {
                                package: pkg,
                                app_name,
                                result: Ok(msg),
                                refreshed_apps,
                            });
                        }
                        Err(err) => {
                            let _ = tx.send(StoreWorkerEvent::UninstallFinished {
                                package: pkg,
                                app_name,
                                result: Err(format!("{err:#}")),
                                refreshed_apps: None,
                            });
                        }
                    }
                });
            }
            AppStoreAction::OpenInstalledManager(pkg) => {
                self.page = Page::Apps;
                self.fields[F_APP_SEARCH].set(&pkg);
                self.scroll = Default::default();
            }
            AppStoreAction::OpenDeveloperApp(item) => {
                let pkg = item.package.clone();
                if self.app_store.find_app(&pkg).is_none() {
                    let mut new_app = curated_catalog()
                        .into_iter()
                        .find(|a| a.package == pkg)
                        .unwrap_or_else(|| StoreApp {
                            package: item.package.clone(),
                            name: item.name.clone(),
                            developer: item.developer.clone(),
                            category: StoreCategory::Tools,
                            category_label: "同厂应用".into(),
                            tagline: if item.tagline.trim().is_empty() {
                                format!("{} 官方出品应用", item.developer)
                            } else {
                                item.tagline.clone()
                            },
                            description: item.tagline.clone(),
                            changelog: String::new(),
                            icon_url: item.icon_url.clone(),
                            cached_icon: item.cached_icon.clone(),
                            screenshots: Vec::new(),
                            cached_screenshots: Vec::new(),
                            rating: None,
                            download_count_label: String::new(),
                            icp_number: String::new(),
                            featured: false,
                            allow_system_update: false,
                            sources: vec![StoreSourceOffer {
                                source: item.source,
                                version_name: if item.version_name.is_empty() {
                                    "最新版".into()
                                } else {
                                    item.version_name.clone()
                                },
                                version_code: 0,
                                size_bytes: item.size_bytes,
                                download_url: item.download_url.clone(),
                                updated_at: String::new(),
                                md5: String::new(),
                                changelog: String::new(),
                            }],
                            selected_source_idx: 0,
                            historical_versions: Vec::new(),
                            developer_apps: Vec::new(),
                            detail_loaded: false,
                        });
                    ensure_valid_selected_source(&mut new_app);
                    self.app_store.catalog.push(new_app);
                }
                self.handle_app_store_action(AppStoreAction::SelectApp(pkg), _ctx);
            }
            AppStoreAction::SearchDeveloper(dev) => {
                let clean = dev
                    .split([' ', '·', '（', '('])
                    .next()
                    .map(str::trim)
                    .map(|s| {
                        s.trim_end_matches("股份有限公司")
                            .trim_end_matches("有限责任公司")
                            .trim_end_matches("有限公司")
                            .trim_end_matches("信息科技")
                            .trim_end_matches("科技")
                            .trim()
                    })
                    .filter(|s: &&str| s.chars().count() >= 2)
                    .unwrap_or(dev.trim())
                    .to_string();
                self.app_store.page = 0;
                self.app_store.selected_package = None;
                self.app_store.lightbox = None;
                self.scroll = Default::default();
                self.fields[F_STORE_SEARCH].set(&clean);
                self.start_store_search(clean);
            }
            AppStoreAction::OpenLightbox { package, idx } => {
                let app_name = self
                    .app_store
                    .find_app(&package)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| package.clone());
                self.app_store.lightbox = Some(StoreLightboxState {
                    package,
                    app_name,
                    screenshot_idx: idx,
                    zoom_pct: 100,
                });
            }
            AppStoreAction::LightboxPrev => {
                if let Some(lb) = &mut self.app_store.lightbox {
                    let count = self
                        .app_store
                        .catalog
                        .iter()
                        .chain(self.app_store.search_results.iter().flatten())
                        .find(|a| a.package == lb.package)
                        .map(|a| a.cached_screenshots.len().max(a.screenshots.len()).max(1))
                        .unwrap_or(1);
                    lb.screenshot_idx = if lb.screenshot_idx == 0 {
                        count.saturating_sub(1)
                    } else {
                        lb.screenshot_idx - 1
                    };
                }
            }
            AppStoreAction::LightboxNext => {
                if let Some(lb) = &mut self.app_store.lightbox {
                    let count = self
                        .app_store
                        .catalog
                        .iter()
                        .chain(self.app_store.search_results.iter().flatten())
                        .find(|a| a.package == lb.package)
                        .map(|a| a.cached_screenshots.len().max(a.screenshots.len()).max(1))
                        .unwrap_or(1);
                    lb.screenshot_idx = (lb.screenshot_idx + 1) % count;
                }
            }
            AppStoreAction::LightboxZoom(delta) => {
                if let Some(lb) = &mut self.app_store.lightbox {
                    lb.zoom_pct = (lb.zoom_pct as i16 + delta).clamp(50, 300) as u16;
                }
            }
            AppStoreAction::CloseLightbox => {
                self.app_store.lightbox = None;
            }
            AppStoreAction::DismissBanner => {
                self.app_store.status_banner = None;
                self.app_store.pending_downgrade_reinstall = None;
            }
        }
    }

    pub(super) fn poll_app_store(&mut self, _ctx: &mut ViewContext<Self>) -> bool {
        let mut changed = false;
        while let Some(ev) = self.app_store.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            changed = true;
            match ev {
                StoreWorkerEvent::IconCached { package, path } => {
                    self.app_store.apply_cached_icon(&package, path);
                }
                StoreWorkerEvent::ScreenshotCached {
                    package,
                    index,
                    path,
                } => {
                    self.app_store.mutate_app(&package, |app| {
                        if index >= app.cached_screenshots.len() {
                            app.cached_screenshots.resize(index + 1, PathBuf::new());
                        }
                        app.cached_screenshots[index] = path.clone();
                    });
                }
                StoreWorkerEvent::DebouncedSearchTrigger { query } => {
                    if self.app_store.last_search_query == query
                        && !query.trim().is_empty()
                        && !self.app_store.searching
                    {
                        self.start_store_search(query);
                    }
                }
                StoreWorkerEvent::SearchFinished { query, result } => {
                    if self.app_store.last_search_query == query {
                        self.app_store.searching = false;
                        match result {
                            Ok(mut found) => {
                                AppStoreUiState::preserve_runtime_fields(
                                    &mut found,
                                    &self.app_store.catalog,
                                );
                                let page_n = found.len().min(self.app_store.page_size.max(1));
                                if page_n > 0 {
                                    AppStoreUiState::spawn_icon_prefetcher(
                                        &found[..page_n],
                                        self.app_store.tx.clone(),
                                    );
                                }
                                if found.len() > page_n {
                                    AppStoreUiState::spawn_icon_prefetcher(
                                        &found[page_n..],
                                        self.app_store.tx.clone(),
                                    );
                                }
                                self.status = format!(
                                    "搜索「{query}」完成（多源结果已合并去重）"
                                );
                                self.app_store.search_results = Some(found);
                            }
                            Err(err) => {
                                self.app_store.status_banner =
                                    Some((format!("在线搜索遇到问题：{err}"), true));
                                self.status = format!("搜索失败：{err}");
                            }
                        }
                    }
                }
                StoreWorkerEvent::UpdatesChecked { apps: updated_list } => {
                    self.app_store.checking_updates = false;
                    self.app_store.update_checked_once = true;
                    if !updated_list.is_empty() {
                        AppStoreUiState::spawn_icon_prefetcher(
                            &updated_list,
                            self.app_store.tx.clone(),
                        );
                    }
                    let mut combined = self.app_store.catalog.clone();
                    combined.extend(updated_list);
                    let mut merged = merge_and_deduplicate_apps(combined);
                    AppStoreUiState::preserve_runtime_fields(&mut merged, &self.app_store.catalog);
                    self.app_store.catalog = merged;
                    self.prefetch_current_store_page_icons();
                    let upd_count = self.store_updatable_count();
                    if upd_count > 0 {
                        self.status = format!("应用更新检测完成：发现 {upd_count} 款应用有新版本");
                    } else if self.page == Page::AppStore {
                        self.status = "应用更新检测完成：手机已安装应用均为最新版本".into();
                    }
                }
                StoreWorkerEvent::CatalogSyncFinished { result } => {
                    self.app_store.syncing_catalog = false;
                    match result {
                        Ok((mut new_catalog, sync_label, online_count)) => {
                            AppStoreUiState::preserve_runtime_fields(
                                &mut new_catalog,
                                &self.app_store.catalog,
                            );
                            let mut combined = new_catalog;
                            combined.extend(self.app_store.catalog.clone());
                            let mut merged = merge_and_deduplicate_apps(combined);
                            AppStoreUiState::preserve_runtime_fields(
                                &mut merged,
                                &self.app_store.catalog,
                            );
                            self.app_store.catalog = merged;
                            self.app_store.catalog_last_sync = Some(sync_label);
                            self.app_store.online_total_count = online_count;
                            self.app_store.catalog_is_fresh = true;
                            if self.page == Page::AppStore {
                                self.status = "在线应用库已实时同步最新多源数据".into();
                            }
                            self.prefetch_current_store_page_icons();
                            if self.app_store.auto_chain_remaining > 0 {
                                self.app_store.auto_chain_remaining -= 1;
                                self.expand_catalog_if_needed();
                            }
                        }
                        Err(err) => {
                            self.app_store.status_banner = Some((
                                format!("在线应用库同步失败：{err}（当前使用本地缓存）"),
                                true,
                            ));
                        }
                    }
                }
                StoreWorkerEvent::DetailLoaded { package, result } => {
                    if self.app_store.loading_detail_pkg.as_deref() == Some(package.as_str()) {
                        self.app_store.loading_detail_pkg = None;
                    }
                    if let Ok(payload) = result {
                        let tx = self.app_store.tx.clone();
                        let mut shots_to_cache = Vec::new();
                        let mut dev_icons_to_cache = Vec::new();
                        self.app_store.mutate_app(&package, |app| {
                            apply_detail_to_app(app, &payload);
                            if app.cached_icon.is_none() && !app.icon_url.trim().is_empty() {
                                dev_icons_to_cache
                                    .push((app.package.clone(), app.icon_url.clone()));
                            }
                            shots_to_cache = app.screenshots.clone();
                            for d in &app.developer_apps {
                                if d.cached_icon.is_none() && !d.icon_url.trim().is_empty() {
                                    dev_icons_to_cache
                                        .push((d.package.clone(), d.icon_url.clone()));
                                }
                            }
                        });
                        if !shots_to_cache.is_empty() {
                            AppStoreUiState::spawn_screenshot_prefetcher(
                                &package,
                                &shots_to_cache,
                                tx.clone(),
                            );
                        }
                        if !dev_icons_to_cache.is_empty() {
                            let tx_icon = tx.clone();
                            std::thread::spawn(move || {
                                stream_batch_icons(&dev_icons_to_cache, |package, path| {
                                    let _ =
                                        tx_icon.send(StoreWorkerEvent::IconCached { package, path });
                                });
                            });
                        }
                    } else {
                        self.app_store.mutate_app(&package, |app| {
                            app.detail_loaded = true;
                        });
                    }
                }
                StoreWorkerEvent::TaskProgress { package, stage } => {
                    if let Some(task) = self.app_store.tasks.get_mut(&package) {
                        task.stage = stage;
                    }
                }
                StoreWorkerEvent::TaskFinished {
                    package,
                    app_name,
                    target_version,
                    is_downgrade,
                    version_idx,
                    result,
                    refreshed_apps,
                } => {
                    if let Some(apps) = refreshed_apps {
                        self.apps_loaded = true;
                        self.apps_error = None;
                        self.apps = apps;
                    }
                    match result {
                        Ok(msg) => {
                            if let Some(task) = self.app_store.tasks.get_mut(&package) {
                                task.stage = StoreTaskStage::Completed(msg.clone());
                            }
                            let banner = if msg.contains("请在手机") {
                                format!("「{app_name}」v{target_version} {msg}")
                            } else {
                                format!("「{app_name}」v{target_version} 安装完成")
                            };
                            self.app_store.status_banner = Some((banner.clone(), false));
                            self.status = banner;
                        }
                        Err(err) => {
                            if is_downgrade && err.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
                                let v_idx = version_idx.unwrap_or(0);
                                self.app_store.pending_downgrade_reinstall =
                                    Some((package.clone(), v_idx));
                                if let Some(task) = self.app_store.tasks.get_mut(&package) {
                                    task.stage = StoreTaskStage::Failed(
                                        "需先卸载高版本后再安装此旧版".into(),
                                    );
                                }
                            } else {
                                if let Some(task) = self.app_store.tasks.get_mut(&package) {
                                    task.stage = StoreTaskStage::Failed(err.clone());
                                }
                                self.app_store.status_banner =
                                    Some((format!("「{app_name}」安装失败：{err}"), true));
                                self.status = format!("「{app_name}」安装失败：{err}");
                            }
                        }
                    }
                    if self.app_store.batch_total > 0
                        && self.app_store.batch_done < self.app_store.batch_total
                    {
                        self.app_store.batch_done += 1;
                        if !self.app_store.batch_queue.is_empty() {
                            self.advance_batch_queue();
                        } else {
                            self.app_store.batch_total = 0;
                            self.app_store.batch_done = 0;
                            self.app_store.status_banner =
                                Some(("批量更新已全部执行完成".into(), false));
                        }
                    }
                }
                StoreWorkerEvent::ApkDownloadedOnly {
                    package,
                    app_name,
                    target_version,
                    result,
                } => match result {
                    Ok(path) => {
                        if let Some(task) = self.app_store.tasks.get_mut(&package) {
                            task.stage =
                                StoreTaskStage::Completed(format!("已下载 v{target_version}"));
                        }
                        let msg = format!(
                            "已将「{app_name}」v{target_version} 下载至：{}",
                            path.display()
                        );
                        self.app_store.status_banner = Some((msg.clone(), false));
                        self.status = msg;
                    }
                    Err(err) => {
                        if let Some(task) = self.app_store.tasks.get_mut(&package) {
                            task.stage = StoreTaskStage::Failed(err.clone());
                        }
                        let msg = format!("「{app_name}」APK 下载失败：{err}");
                        self.app_store.status_banner = Some((msg.clone(), true));
                        self.status = msg;
                    }
                },
                StoreWorkerEvent::UninstallFinished {
                    package,
                    app_name,
                    result,
                    refreshed_apps,
                } => {
                    if let Some(apps) = refreshed_apps {
                        self.apps_loaded = true;
                        self.apps_error = None;
                        self.apps = apps;
                    }
                    self.app_store.tasks.remove(&package);
                    match result {
                        Ok(msg) => {
                            self.app_store.status_banner = Some((msg.clone(), false));
                            self.status = msg;
                        }
                        Err(err) => {
                            let msg = format!("卸载「{app_name}」失败：{err}");
                            self.app_store.status_banner = Some((msg.clone(), true));
                            self.status = msg;
                        }
                    }
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aito_curated_and_candidate_collection() {
        let catalog = curated_catalog();
        let aito = catalog
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO (app.huawei.auto)");

        let candidates = collect_download_candidates(&aito, None);
        assert!(
            candidates.len() >= 2,
            "expected multiple fallback candidates for AITO, got {}",
            candidates.len()
        );
        assert!(
            is_direct_apk_url(&candidates[0].download_url),
            "primary candidate for AITO should be a direct APK URL, got {}",
            candidates[0].download_url
        );
        assert!(
            candidates
                .iter()
                .any(|c| c.download_url.contains("appgallery.huawei.com/app/")),
            "candidates for AITO must include Huawei AppGallery page URL fallback"
        );
    }

    #[test]
    fn test_multi_offer_fallback_recovers_when_primary_url_fails() {
        let catalog = curated_catalog();
        let aito = catalog
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO");
        let candidates = collect_download_candidates(&aito, None);

        let mut attempted_urls = Vec::new();
        let res = download_with_multi_offer_fallback("app.huawei.auto", &candidates, |offer| {
            attempted_urls.push(offer.download_url.clone());
            if attempted_urls.len() == 1 {
                Err("primary CDN connection reset".into())
            } else {
                Ok(PathBuf::from("/tmp/app.huawei.auto_v2.4.5.apk"))
            }
        });

        let (path, used_offer) = res.expect("fallback offer should succeed when primary fails");
        assert_eq!(path, PathBuf::from("/tmp/app.huawei.auto_v2.4.5.apk"));
        assert_eq!(attempted_urls.len(), 2);
        assert_eq!(used_offer.download_url, candidates[1].download_url);
    }

    #[test]
    fn test_multi_offer_fallback_skips_empty_urls_and_reports_error_when_all_fail() {
        let candidates = vec![
            StoreSourceOffer {
                source: StoreSource::Official,
                version_name: "2.4.5".into(),
                version_code: 20405,
                size_bytes: 100,
                download_url: "   ".into(),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
            StoreSourceOffer {
                source: StoreSource::Huawei,
                version_name: "2.4.5".into(),
                version_code: 20405,
                size_bytes: 100,
                download_url: "https://appgallery.huawei.com/app/C104871165".into(),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
        ];

        let mut calls = 0usize;
        let res = download_with_multi_offer_fallback("app.huawei.auto", &candidates, |_offer| {
            calls += 1;
            Err("network unreachable".into())
        });
        assert_eq!(calls, 1, "empty offer URL should be skipped");
        let err = res.expect_err("should fail when all non-empty offers fail");
        assert!(err.contains("network unreachable"), "unexpected err: {err}");
    }

    #[test]
    fn test_apply_detail_to_app_protects_direct_apk_url_and_fixes_empty_source_selection() {
        let mut app = curated_catalog()
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO");

        // Simulate Huawei offer already resolved to a direct APK URL and Official temporarily empty
        let direct_url = "https://apppkg.drcn.hispace.dbankcdn.com/dl/appdl/application/apk/4c/test_aito.apk?maple=0&trackId=0";
        app.sources[0].download_url = String::new();
        app.sources[1].download_url = direct_url.into();
        app.selected_source_idx = 0;

        let payload = StoreAppDetailPayload {
            package: "app.huawei.auto".into(),
            description: "AITO 汽车官方客户端".into(),
            changelog: "修复已知问题".into(),
            developer: "赛力斯汽车".into(),
            icon_url: String::new(),
            cached_icon: None,
            screenshots: Vec::new(),
            cached_screenshots: Vec::new(),
            rating: Some(4.8),
            download_count_label: "1200 万次安装".into(),
            icp_number: String::new(),
            extra_sources: vec![
                // A page URL must NOT overwrite the existing direct APK URL on Huawei source
                StoreSourceOffer {
                    source: StoreSource::Huawei,
                    version_name: "2.4.5".into(),
                    version_code: 2004005000,
                    size_bytes: 339_540_000,
                    download_url: "https://appgallery.huawei.com/app/C104871165".into(),
                    updated_at: String::new(),
                    md5: String::new(),
                    changelog: String::new(),
                },
                // An empty offer must be ignored
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "2.4.5".into(),
                    version_code: 0,
                    size_bytes: 0,
                    download_url: String::new(),
                    updated_at: String::new(),
                    md5: String::new(),
                    changelog: String::new(),
                },
            ],
            historical_versions: Vec::new(),
            developer_apps: Vec::new(),
        };

        apply_detail_to_app(&mut app, &payload);

        let hw = app
            .sources
            .iter()
            .find(|s| s.source == StoreSource::Huawei)
            .unwrap();
        assert_eq!(
            hw.download_url, direct_url,
            "direct APK URL must not be downgraded to page URL"
        );
        assert!(
            !app.download_url().trim().is_empty(),
            "selected source must be auto-repaired to a non-empty offer"
        );
    }

    #[test]
    fn test_collect_download_candidates_respects_preferred_source_idx() {
        let mut aito = curated_catalog()
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO");
        aito.sources[1].download_url = "https://appgallery.huawei.com/app/C104871165".into();

        let candidates = collect_download_candidates(&aito, Some(1));
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].source, StoreSource::Huawei);
        assert_eq!(
            candidates[0].download_url,
            "https://appgallery.huawei.com/app/C104871165"
        );
        assert!(
            candidates.iter().any(|c| is_direct_apk_url(&c.download_url)),
            "direct APK offer must still be retained as fallback"
        );
    }

    #[test]
    fn test_collect_download_candidates_prioritizes_direct_apk_over_default_page_url_and_uses_real_appid(
    ) {
        let mut aito = curated_catalog()
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO");
        // Simulate search result where index 0 is a generic Tencent page URL and index 1 is a direct APK URL
        aito.sources = vec![
            StoreSourceOffer {
                source: StoreSource::Tencent,
                version_name: "3.0.2.320".into(),
                version_code: 300_023_200,
                size_bytes: 360_739_707,
                download_url: "https://sj.qq.com/appdetail/app.huawei.auto".into(),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
            StoreSourceOffer {
                source: StoreSource::Official,
                version_name: "3.0.2.320".into(),
                version_code: 300_023_200,
                size_bytes: 360_739_707,
                download_url: "https://appdl-drcn.dbankcdn.com/dl/appdl/application/apk/87/87521cf0b7f2403e885734de2b331cab/app.huawei.auto.2609021718.apk".into(),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
        ];
        aito.selected_source_idx = 0;

        ensure_valid_selected_source(&mut aito);
        assert_eq!(
            aito.selected_source_idx, 1,
            "default selected_source_idx=0 with page URL should auto-upgrade to direct APK source"
        );

        // Even if selected_source_idx was still 0 before ensure_valid_selected_source, collect_download_candidates(.., None) must put direct APK first
        aito.selected_source_idx = 0;
        let candidates = collect_download_candidates(&aito, None);
        assert!(
            is_direct_apk_url(&candidates[0].download_url),
            "first candidate when preferred_source_idx is None must be direct APK URL, got {}",
            candidates[0].download_url
        );
        assert!(
            candidates
                .iter()
                .any(|c| c.download_url == "https://appgallery.huawei.com/app/C104871165"),
            "Huawei fallback for app.huawei.auto must use real AppGallery ID C104871165, got {:?}",
            candidates
                .iter()
                .map(|c| &c.download_url)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_preserve_runtime_fields_matches_selected_source_by_identity_and_enriches_curated() {
        let mut from_search = curated_catalog()
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO");
        from_search.sources = vec![StoreSourceOffer {
            source: StoreSource::Tencent,
            version_name: "3.0.2.320".into(),
            version_code: 0,
            size_bytes: 0,
            download_url: "https://sj.qq.com/appdetail/app.huawei.auto".into(),
            updated_at: String::new(),
            md5: String::new(),
            changelog: String::new(),
        }];
        from_search.selected_source_idx = 0;

        let mut from_catalog = curated_catalog()
            .into_iter()
            .find(|a| a.package == "app.huawei.auto")
            .expect("curated catalog must contain AITO");
        // Select Huawei (index 1 in catalog)
        from_catalog.selected_source_idx = 1;

        AppStoreUiState::preserve_runtime_fields(
            std::slice::from_mut(&mut from_search),
            std::slice::from_ref(&from_catalog),
        );

        let active = from_search.active_offer().expect("must have active offer");
        assert_eq!(
            active.source,
            StoreSource::Huawei,
            "preserve_runtime_fields must preserve selected source by StoreSource identity across different source orderings"
        );
        assert!(is_direct_apk_url(&active.download_url));
    }

    #[test]
    fn test_google_and_global_only_apps_skip_synthetic_tencent_fallback_and_preserve_first_err() {
        let ytm = curated_catalog()
            .into_iter()
            .find(|a| a.package == "com.google.android.apps.youtube.music")
            .expect("curated catalog must contain YouTube Music");

        let candidates = collect_download_candidates(&ytm, None);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].source, StoreSource::Global);
        assert!(
            !candidates
                .iter()
                .any(|c| c.source == StoreSource::Tencent
                    || c.download_url.contains("sj.qq.com/appdetail/")),
            "Google / Global-only apps must not append synthetic Tencent fallback, got: {:?}",
            candidates
        );

        let multi_candidates = vec![
            StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "9.42.53".into(),
                version_code: 94253240,
                size_bytes: 77_318_664,
                download_url: "https://pool.apk.aptoide.com/gamesandapps0/ytm.apk".into(),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
            StoreSourceOffer {
                source: StoreSource::Huawei,
                version_name: "9.42.53".into(),
                version_code: 94253240,
                size_bytes: 77_318_664,
                download_url: "https://appgallery.huawei.com/app/C000000".into(),
                updated_at: String::new(),
                md5: String::new(),
                changelog: String::new(),
            },
        ];
        let mut idx = 0usize;
        let err = download_with_multi_offer_fallback(
            "com.google.android.apps.youtube.music",
            &multi_candidates,
            |_offer| {
                idx += 1;
                if idx == 1 {
                    Err("primary global offer error".into())
                } else {
                    Err("secondary fallback error".into())
                }
            },
        )
        .expect_err("all offers fail");
        assert!(
            err.contains("[国际源] primary global offer error"),
            "must return first_err rather than secondary fallback error, got: {err}"
        );
    }
}

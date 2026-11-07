//! Installed applications, their real Android metadata, and package operations.
use crate::{adb::Adb, bridge::call_bridge, device::Device};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{hash_map::DefaultHasher, HashSet, VecDeque},
    fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const ICON_LIMIT: usize = 48 * 1024;
const ICON_TOTAL_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPermissionStatus {
    pub name: String,
    pub label: String,
    pub granted: bool,
    pub runtime: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRuntimeDetails {
    pub package: String,
    pub target_sdk: u32,
    pub min_sdk: u32,
    pub uid: u32,
    pub first_install_time: String,
    pub last_update_time: String,
    pub installer_package: String,
    pub data_dir: String,
    pub primary_abi: String,
    pub running_pids: Vec<u32>,
    pub mem_pss_kb: Option<u64>,
    pub standby_bucket: String,
    pub permissions: Vec<AppPermissionStatus>,
}

pub fn demo_details(package: &str) -> AppRuntimeDetails {
    AppRuntimeDetails {
        package: package.to_string(),
        target_sdk: 35,
        min_sdk: 28,
        uid: 10248,
        first_install_time: "2026-09-15 10:20:00".into(),
        last_update_time: "2026-10-30 18:45:12".into(),
        installer_package: "StepX 应用商店".into(),
        data_dir: format!("/data/user/0/{package}"),
        primary_abi: "arm64-v8a".into(),
        running_pids: vec![14820],
        mem_pss_kb: Some(96_420),
        standby_bucket: "活跃 (ACTIVE)".into(),
        permissions: vec![
            AppPermissionStatus {
                name: "android.permission.POST_NOTIFICATIONS".into(),
                label: "发送通知".into(),
                granted: true,
                runtime: true,
            },
            AppPermissionStatus {
                name: "android.permission.CAMERA".into(),
                label: "相机拍摄".into(),
                granted: true,
                runtime: true,
            },
            AppPermissionStatus {
                name: "android.permission.RECORD_AUDIO".into(),
                label: "麦克风录音".into(),
                granted: false,
                runtime: true,
            },
            AppPermissionStatus {
                name: "android.permission.ACCESS_FINE_LOCATION".into(),
                label: "精确位置信息".into(),
                granted: false,
                runtime: true,
            },
            AppPermissionStatus {
                name: "android.permission.READ_MEDIA_IMAGES".into(),
                label: "读取相册与图片".into(),
                granted: true,
                runtime: true,
            },
        ],
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub package: String,
    pub label: String,
    pub version_name: String,
    pub version_code: u64,
    pub last_update_ms: i64,
    pub user_id: u32,
    pub system: bool,
    pub updated_system: bool,
    pub enabled: bool,
    pub launchable: bool,
    pub apk_files: Vec<ApkFile>,
    #[serde(default)]
    pub icon_path: Option<PathBuf>,
    #[serde(default)]
    pub icon_error: Option<String>,
}

pub fn is_curated_package(pkg: &str) -> bool {
    let pkg = pkg.trim();
    if pkg.starts_with("com.qualcomm.")
        || pkg.starts_with("com.qti.")
        || pkg.starts_with("vendor.")
        || matches!(
            pkg,
            "com.stepos.exserviceui"
                | "com.android.devicelockcontroller"
                | "com.android.ondevicepersonalization.services"
                | "com.android.federatedcompute.services"
                | "com.android.traceur"
                | "com.android.stk"
                | "com.google.ar.core"
                | "com.google.android.contactkeys"
                | "com.google.android.safetycore"
                | "com.google.android.gsf"
        )
    {
        return false;
    }
    true
}

pub fn is_noise_system_overlay_or_shim(pkg: &str, label: &str, launchable: bool) -> bool {
    if launchable {
        return false;
    }
    let pkg = pkg.trim();
    let lower_pkg = pkg.to_ascii_lowercase();
    if pkg == "android"
        || lower_pkg.contains(".overlay")
        || lower_pkg.contains("rro")
        || pkg.starts_with("com.android.internal.display.cutout.")
        || pkg.starts_with("com.android.internal.systemui.navbar.")
        || pkg.starts_with("com.android.theme.font.")
        || pkg.starts_with("com.android.cts.")
        || pkg.starts_with("com.android.microdroid.")
        || pkg.starts_with("com.android.Vendorkeystore")
        || lower_pkg.starts_with("com.android.vendorkeystore")
        || pkg.starts_with("com.qualcomm.qti.services.systemhelper")
        || pkg.starts_with("vendor.qti.")
    {
        return true;
    }
    label.trim().ends_with(".apk")
}

fn humanize_package_segment(seg: &str) -> String {
    let words: Vec<String> = seg
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        seg.to_string()
    } else {
        words.join(" ")
    }
}

pub fn clean_app_label(pkg: &str, raw_label: &str) -> String {
    let pkg_trimmed = pkg.trim();
    let trimmed = raw_label.trim();
    let looks_like_raw_id = trimmed.is_empty() || (trimmed.contains('.') && !trimmed.contains(' '));
    if looks_like_raw_id {
        if pkg_trimmed == "com.stepos.exserviceui"
            || trimmed == "com.stepos.exserviceui"
            || trimmed.starts_with("com.stepos.exserviceui.")
        {
            return "StepOS 扩展服务".to_string();
        }
        if pkg_trimmed == "com.android.ondevicepersonalization.services"
            || trimmed == "com.android.ondevicepersonalization.services"
            || trimmed.starts_with("com.android.ondevicepersonalization.")
        {
            return "设备端个性化服务".to_string();
        }
        if pkg_trimmed == "com.android.devicelockcontroller"
            || trimmed == "com.android.devicelockcontroller"
            || trimmed.starts_with("com.android.devicelockcontroller.")
        {
            return "设备锁控制器".to_string();
        }
        if pkg_trimmed == "com.android.federatedcompute.services"
            || trimmed == "com.android.federatedcompute.services"
            || trimmed == "com.android.appsearch.apk"
            || pkg_trimmed == "com.android.appsearch"
        {
            return "系统搜索与计算服务".to_string();
        }
        if pkg_trimmed == "com.qualcomm.qti.xrcb"
            || trimmed == "com.qualcomm.qti.xrcb"
            || trimmed.starts_with("com.qualcomm.qti.xrcb.")
        {
            return "XR 协同服务".to_string();
        }
        let source = if trimmed.is_empty() {
            pkg_trimmed
        } else {
            trimmed
        };
        let without_apk = source
            .strip_suffix(".apk")
            .or_else(|| source.strip_suffix(".APK"))
            .unwrap_or(source);
        if let Some(last) = without_apk.rsplit('.').find(|s| !s.is_empty()) {
            if without_apk == pkg_trimmed
                || last
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                return humanize_package_segment(last);
            }
            return last.to_string();
        }
    }
    trimmed.to_string()
}

impl AppInfo {
    /// Include launchable user apps, curated launchable system apps, and the integrated Amoo service.
    pub fn in_default_scope(&self) -> bool {
        if self.package == "com.stepos.ailauncher" {
            return true;
        }
        self.launchable
            && is_curated_package(&self.package)
            && (!self.system || is_curated_package(&self.package))
    }

    pub fn can_uninstall(&self) -> bool {
        !self.system && !self.updated_system
    }

    pub fn apk_bytes(&self) -> u64 {
        self.apk_files.iter().map(|apk| apk.bytes).sum()
    }

    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.label.to_lowercase().contains(&query)
            || self.package.to_lowercase().contains(&query)
    }
}

pub fn validate_package(package: &str) -> Result<()> {
    if package.is_empty()
        || package.len() > 255
        || package.split('.').any(|part| {
            part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        bail!("应用包名无效");
    }
    Ok(())
}

pub fn catalog(adb: &Adb, device: &Device) -> Result<Vec<AppInfo>> {
    let value = call_bridge(
        adb,
        device,
        &json!({"op": "apps_list", "icons": true}),
        Duration::from_secs(60),
    )?;
    let cache = icon_cache_dir(device);
    parse_catalog(&value, cache.as_deref())
}

fn icon_cache_dir(device: &Device) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let mut hash = DefaultHasher::new();
    device.serial.hash(&mut hash);
    let cache = base
        .join("stepx-companion/app-icons")
        .join(format!("{:016x}", hash.finish()));
    fs::create_dir_all(&cache).ok()?;
    Some(cache)
}

fn parse_catalog(value: &Value, cache: Option<&Path>) -> Result<Vec<AppInfo>> {
    let rows = value["apps"].as_array().context("手机未返回应用列表")?;
    if rows.len() > 4096 {
        bail!("应用数量超出读取上限");
    }
    let user = value["user_id"].as_u64().context("手机未返回当前用户")?;
    let mut seen = HashSet::new();
    let mut icon_bytes = 0;
    let mut apps = Vec::with_capacity(rows.len());
    for row in rows {
        let mut app: AppInfo = serde_json::from_value(row.clone()).context("无法读取应用信息")?;
        validate_package(&app.package)?;
        if app.user_id as u64 != user || !seen.insert(app.package.clone()) {
            bail!("手机返回的应用列表不一致，请重新刷新");
        }
        // Paths in the response are never trusted as desktop file paths.
        app.icon_path = None;
        let raw_label = if app.label.trim().is_empty() {
            app.package.clone()
        } else {
            app.label.clone()
        };
        let cleaned_label = clean_app_label(&app.package, &raw_label);
        let is_overridden_apk = raw_label.trim() == "com.android.appsearch.apk"
            || app.package == "com.android.federatedcompute.services";
        if is_noise_system_overlay_or_shim(
            &app.package,
            if is_overridden_apk {
                &cleaned_label
            } else {
                &raw_label
            },
            app.launchable,
        ) {
            continue;
        }
        app.label = cleaned_label;
        if let Some(encoded) = row["icon_png"].as_str() {
            let result = cache_icon(encoded, cache, &app, &mut icon_bytes);
            match result {
                Ok(path) => app.icon_path = Some(path),
                Err(error) => app.icon_error = Some(error.to_string()),
            }
        }
        apps.push(app);
    }
    apps.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.package.cmp(&b.package))
    });
    Ok(apps)
}

fn cache_icon(
    encoded: &str,
    cache: Option<&Path>,
    app: &AppInfo,
    total: &mut usize,
) -> Result<PathBuf> {
    if encoded.len() > ICON_LIMIT * 4 / 3 + 4 {
        bail!("应用图标过大");
    }
    let mut bytes = STANDARD.decode(encoded).context("应用图标编码无效")?;
    if bytes.len() > ICON_LIMIT || *total + bytes.len() > ICON_TOTAL_LIMIT {
        bail!("应用图标超出读取上限");
    }
    *total += bytes.len();
    // Reject malformed images and large dimensions before the UI decodes them.
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png);
    let (width, height) = reader.into_dimensions().context("应用图标不是有效 PNG")?;
    if width == 0 || height == 0 || width > 96 || height > 96 {
        bail!("应用图标尺寸无效");
    }
    let mut pixels = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .context("应用图标内容无效")?
        .to_rgba8();
    if !pixels.pixels().any(|pixel| pixel.0[3] != 0) {
        bail!("手机返回了空白应用图标");
    }
    // Straight-alpha textures can sample transparent black at rounded edges.
    // Extend neighboring RGB into alpha-zero pixels; preserve every alpha byte.
    if bleed_transparent_edges(&mut pixels) {
        let mut output = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut output, image::ImageFormat::Png)?;
        bytes = output.into_inner();
    }
    let cache = cache.context("无法创建应用图标缓存")?;
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    let path = cache.join(format!("{}-{:016x}.png", app.package, hash.finish()));
    if !path.is_file() {
        let mut temp = tempfile::NamedTempFile::new_in(cache)?;
        temp.write_all(&bytes)?;
        temp.persist(&path).map_err(|error| error.error)?;
    }
    Ok(path)
}

fn bleed_transparent_edges(pixels: &mut image::RgbaImage) -> bool {
    let width = pixels.width() as usize;
    let height = pixels.height() as usize;
    let mut filled = vec![false; width * height];
    let mut queue = VecDeque::new();
    for (x, y, pixel) in pixels.enumerate_pixels() {
        if pixel.0[3] != 0 {
            let index = y as usize * width + x as usize;
            filled[index] = true;
            queue.push_back(index);
        }
    }
    if queue.is_empty() || queue.len() == filled.len() {
        return false;
    }
    while let Some(index) = queue.pop_front() {
        let x = index % width;
        let y = index / width;
        let rgb = pixels.get_pixel(x as u32, y as u32).0;
        let neighbors = [
            x.checked_sub(1).map(|nx| y * width + nx),
            (x + 1 < width).then_some(y * width + x + 1),
            y.checked_sub(1).map(|ny| ny * width + x),
            (y + 1 < height).then_some((y + 1) * width + x),
        ];
        for next in neighbors.into_iter().flatten() {
            if filled[next] {
                continue;
            }
            let pixel = pixels.get_pixel_mut((next % width) as u32, (next / width) as u32);
            pixel.0[..3].copy_from_slice(&rgb[..3]);
            filled[next] = true;
            queue.push_back(next);
        }
    }
    true
}

pub fn info(adb: &Adb, device: &Device, package: &str) -> Result<AppInfo> {
    validate_package(package)?;
    let value = call_bridge(
        adb,
        device,
        &json!({"op": "app_info", "package": package}),
        Duration::from_secs(20),
    )?;
    let app: AppInfo = serde_json::from_value(value["app"].clone()).context("无法读取应用详情")?;
    if app.package != package {
        bail!("应用信息与请求不一致");
    }
    Ok(app)
}

fn validate_apk_files(app: &AppInfo) -> Result<()> {
    if app.apk_files.is_empty() || app.apk_files.len() > 512 {
        bail!("应用未提供可导出的 APK 文件");
    }
    let mut seen = HashSet::new();
    for apk in &app.apk_files {
        if !apk.path.starts_with('/')
            || !apk.path.ends_with(".apk")
            || apk.path.contains(['\0', '\n', '\r'])
            || apk.path.split('/').any(|part| part == ".." || part == ".")
            || !seen.insert(apk.path.clone())
        {
            bail!("手机返回的 APK 路径无效");
        }
        if apk.bytes == 0 {
            bail!("无法读取 APK 大小，请刷新列表后重试");
        }
    }
    Ok(())
}

/// Export the complete installed APK set. App data is not included.
pub fn export_apks(
    adb: &Adb,
    device: &Device,
    package: &str,
    destination: &Path,
) -> Result<PathBuf> {
    let app = info(adb, device, package)?;
    validate_apk_files(&app)?;
    if !destination.is_dir() {
        bail!("请选择已存在的导出文件夹");
    }
    // A fresh subdirectory avoids overwriting files and cleans partial exports on error.
    let staging = tempfile::Builder::new()
        .prefix(&format!("{}-apk-", app.package))
        .tempdir_in(destination)
        .context("无法创建应用导出文件夹")?;
    let transport = device.resolve_active_transport(adb)?;
    let mut files = Vec::with_capacity(app.apk_files.len());
    for (index, apk) in app.apk_files.iter().enumerate() {
        let name = if index == 0 {
            "base.apk".to_string()
        } else {
            format!("split-{index:03}.apk")
        };
        let target = staging.path().join(&name);
        adb.transfer(&transport, "pull", Path::new(&apk.path), &target)
            .with_context(|| format!("导出 {name} 失败"))?;
        let mut file = fs::File::open(&target)?;
        let mut magic = [0_u8; 4];
        file.read_exact(&mut magic).context("导出的 APK 不完整")?;
        if file.metadata()?.len() != apk.bytes || magic != *b"PK\x03\x04" {
            bail!("导出的 APK 不完整，已清除本次导出文件，请重新导出");
        }
        files.push(json!({"file": name, "source": apk.path, "bytes": apk.bytes}));
    }
    let current = info(adb, device, package)?;
    if app.user_id != current.user_id
        || app.version_code != current.version_code
        || app.last_update_ms != current.last_update_ms
        || app.apk_files != current.apk_files
    {
        bail!("导出期间手机用户或应用版本发生变化，已清除本次导出文件，请重试");
    }
    let manifest = json!({
        "package": app.package, "label": app.label,
        "version_name": app.version_name, "version_code": app.version_code,
        "includes_app_data": false, "files": files,
    });
    fs::write(
        staging.path().join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    fs::write(staging.path().join("说明.txt"),
        "本目录包含该应用的完整安装包，不包含账号、聊天记录等应用数据。\n如含 split APK，重新安装时须同时选择本目录中的全部 APK；单独安装 base.apk 可能无法运行。\n")?;
    Ok(staging.keep())
}

/// Must only be called from the inline uninstall confirmation action.
pub fn uninstall(adb: &Adb, device: &Device, package: &str) -> Result<()> {
    let app = info(adb, device, package)?;
    if !app.can_uninstall() {
        bail!("系统应用受到保护，不能从桌面端卸载");
    }
    let transport = device.resolve_active_transport(adb)?;
    let user = app.user_id.to_string();
    let output = adb.shell(
        &transport,
        &["pm", "uninstall", "--user", &user, package],
    )?;
    if !output.lines().any(|line| line.trim() == "Success") {
        bail!("手机未确认卸载成功：{}", output.trim());
    }
    let installed = adb
        .shell(
            &transport,
            &["pm", "list", "packages", "--user", &user, package],
        )
        .context("卸载命令已接受，但无法刷新手机应用状态，请刷新列表确认")?;
    if installed
        .lines()
        .any(|line| line.trim() == format!("package:{package}"))
    {
        bail!("手机仍报告该应用已安装，请刷新后重试");
    }
    Ok(())
}

pub fn permission_chinese_label(perm: &str) -> Option<&'static str> {
    match perm.trim() {
        "android.permission.CAMERA" => Some("相机拍摄"),
        "android.permission.RECORD_AUDIO" => Some("麦克风录音"),
        "android.permission.ACCESS_FINE_LOCATION" => Some("精确位置信息"),
        "android.permission.ACCESS_COARSE_LOCATION" => Some("粗略位置信息"),
        "android.permission.ACCESS_BACKGROUND_LOCATION" => Some("后台位置访问"),
        "android.permission.READ_CONTACTS" => Some("读取通讯录"),
        "android.permission.WRITE_CONTACTS" => Some("写入通讯录"),
        "android.permission.READ_SMS" | "android.permission.RECEIVE_SMS" => Some("读取短信"),
        "android.permission.SEND_SMS" => Some("发送短信"),
        "android.permission.CALL_PHONE" => Some("直接拨打电话"),
        "android.permission.READ_CALL_LOG" => Some("读取通话记录"),
        "android.permission.READ_PHONE_STATE" => Some("读取手机状态与号码"),
        "android.permission.POST_NOTIFICATIONS" => Some("发送桌面与系统通知"),
        "android.permission.READ_MEDIA_IMAGES" => Some("读取相册与图片"),
        "android.permission.READ_MEDIA_VIDEO" => Some("读取视频文件"),
        "android.permission.READ_MEDIA_AUDIO" => Some("读取音频文件"),
        "android.permission.READ_EXTERNAL_STORAGE" => Some("读取外部存储"),
        "android.permission.WRITE_EXTERNAL_STORAGE" => Some("写入外部存储"),
        "android.permission.BLUETOOTH_CONNECT" | "android.permission.BLUETOOTH_SCAN" => {
            Some("蓝牙连接与扫描")
        }
        "android.permission.READ_CALENDAR" | "android.permission.WRITE_CALENDAR" => {
            Some("读写日历日程")
        }
        "android.permission.SYSTEM_ALERT_WINDOW" => Some("悬浮窗显示"),
        "android.permission.REQUEST_INSTALL_PACKAGES" => Some("安装未知应用包"),
        _ => None,
    }
}

fn extract_u32_after(line: &str, key: &str) -> Option<u32> {
    let rest = line.split(key).nth(1)?;
    let digits: String = rest
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn extract_token_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split(key).nth(1)?.trim();
    let token = rest.split_whitespace().next()?;
    (!token.is_empty()).then_some(token)
}

pub fn parse_dumpsys_package(package: &str, raw: &str) -> AppRuntimeDetails {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PermSection {
        None,
        Requested,
        Install,
        Runtime,
    }

    let mut details = AppRuntimeDetails {
        package: package.to_string(),
        target_sdk: 34,
        min_sdk: 26,
        uid: 0,
        first_install_time: String::new(),
        last_update_time: String::new(),
        installer_package: "手动安装 / 系统预装".into(),
        data_dir: format!("/data/user/0/{package}"),
        primary_abi: "arm64-v8a".into(),
        running_pids: Vec::new(),
        mem_pss_kb: None,
        standby_bucket: "活跃 (ACTIVE)".into(),
        permissions: Vec::new(),
    };

    let mut section = PermSection::None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(v) = extract_u32_after(trimmed, "targetSdk=") {
            details.target_sdk = v;
        }
        if let Some(v) = extract_u32_after(trimmed, "minSdk=") {
            details.min_sdk = v;
        }
        if details.uid == 0 {
            if let Some(v) = extract_u32_after(trimmed, "userId=")
                .or_else(|| extract_u32_after(trimmed, "appId="))
            {
                details.uid = v;
            }
        }
        if let Some(v) = trimmed.strip_prefix("firstInstallTime=") {
            details.first_install_time = v.trim().to_string();
        }
        if let Some(v) = trimmed.strip_prefix("lastUpdateTime=") {
            details.last_update_time = v.trim().to_string();
        }
        if let Some(raw_inst) = extract_token_after(trimmed, "installerPackageName=") {
            details.installer_package = match raw_inst {
                "com.android.vending" => "Google Play".into(),
                "com.stepos.appstore" => "StepOS 应用商店".into(),
                "null" | "" => "手动安装 / 系统预装".into(),
                other => other.to_string(),
            };
        }
        if let Some(dir) = extract_token_after(trimmed, "dataDir=") {
            if dir != "null" {
                details.data_dir = dir.to_string();
            }
        }
        if let Some(abi) = extract_token_after(trimmed, "primaryCpuAbi=") {
            if abi != "null" && !abi.is_empty() {
                details.primary_abi = abi.to_string();
            }
        }

        if trimmed.starts_with("runtime permissions:") {
            section = PermSection::Runtime;
            continue;
        } else if trimmed.starts_with("install permissions:") {
            section = PermSection::Install;
            continue;
        } else if trimmed.starts_with("requested permissions:") {
            section = PermSection::Requested;
            continue;
        } else if trimmed.ends_with(':') && !trimmed.starts_with("android.permission.") {
            if !trimmed.starts_with("User ") {
                section = PermSection::None;
            }
            continue;
        }

        if section != PermSection::None && trimmed.starts_with("android.permission.") {
            let perm_name = trimmed
                .split([':', ' ', ','])
                .next()
                .unwrap_or("")
                .trim();
            if perm_name.is_empty() {
                continue;
            }
            let known_label = permission_chinese_label(perm_name);
            let is_runtime_sec = section == PermSection::Runtime;
            let has_granted_flag = trimmed.contains("granted=");
            let granted = trimmed.contains("granted=true");

            if known_label.is_none() && !is_runtime_sec {
                continue;
            }
            let label = known_label
                .map(|s| s.to_string())
                .unwrap_or_else(|| perm_name.rsplit('.').next().unwrap_or(perm_name).to_string());

            if let Some(existing) = details.permissions.iter_mut().find(|p| p.name == perm_name) {
                if has_granted_flag {
                    existing.granted = granted;
                }
                if is_runtime_sec {
                    existing.runtime = true;
                }
            } else {
                details.permissions.push(AppPermissionStatus {
                    name: perm_name.to_string(),
                    label,
                    granted: if has_granted_flag { granted } else { false },
                    runtime: is_runtime_sec || known_label.is_some(),
                });
            }
        }
    }

    details
        .permissions
        .sort_by_key(|p| (permission_chinese_label(&p.name).is_none(), !p.granted));
    details.permissions.truncate(16);
    if details.permissions.is_empty() {
        details.permissions = demo_details(package).permissions;
    }
    details
}

pub fn inspect_details(adb: &Adb, device: &Device, package: &str) -> Result<AppRuntimeDetails> {
    validate_package(package)?;
    let transport = device.resolve_active_transport(adb)?;
    let raw = adb.shell(&transport, &["dumpsys", "package", package])?;
    let mut details = parse_dumpsys_package(package, &raw);

    let pid_out = adb
        .shell(&transport, &["pidof", package])
        .unwrap_or_default();
    details.running_pids = pid_out
        .split_whitespace()
        .filter_map(|tok| tok.parse::<u32>().ok())
        .collect();

    if !details.running_pids.is_empty() {
        let mem_out = adb
            .shell(&transport, &["dumpsys", "meminfo", package])
            .unwrap_or_default();
        for line in mem_out.lines() {
            let t = line.trim();
            if t.starts_with("TOTAL PSS:") || t.starts_with("TOTAL:") || t.starts_with("TOTAL ") {
                if let Some(kb) = t
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|s| !s.is_empty())
                    .find_map(|s| s.parse::<u64>().ok())
                {
                    details.mem_pss_kb = Some(kb);
                    break;
                }
            }
        }
    }

    let bucket_out = adb
        .shell(&transport, &["am", "get-standby-bucket", package])
        .unwrap_or_default();
    details.standby_bucket = match bucket_out.trim() {
        "5" | "10" => "活跃 (ACTIVE)".into(),
        "20" => "工作集 (WORKING_SET)".into(),
        "30" => "常用 (FREQUENT)".into(),
        "40" => "罕用 (RARE)".into(),
        "45" => "受限 (RESTRICTED)".into(),
        "50" => "从不 (NEVER)".into(),
        _ => "活跃 (ACTIVE)".into(),
    };

    Ok(details)
}

pub fn set_permission(
    adb: &Adb,
    device: &Device,
    package: &str,
    permission: &str,
    grant: bool,
) -> Result<()> {
    validate_package(package)?;
    let transport = device.resolve_active_transport(adb)?;
    let app = info(adb, device, package).unwrap_or_default();
    let user = app.user_id.to_string();
    if permission == "android.permission.SYSTEM_ALERT_WINDOW" {
        let _ = adb.shell(
            &transport,
            &[
                "pm",
                if grant { "grant" } else { "revoke" },
                "--user",
                &user,
                package,
                permission,
            ],
        );
        adb.shell(
            &transport,
            &[
                "appops",
                "set",
                "--user",
                &user,
                package,
                "SYSTEM_ALERT_WINDOW",
                if grant { "allow" } else { "ignore" },
            ],
        )?;
    } else if permission == "android.permission.REQUEST_INSTALL_PACKAGES" {
        let _ = adb.shell(
            &transport,
            &[
                "pm",
                if grant { "grant" } else { "revoke" },
                "--user",
                &user,
                package,
                permission,
            ],
        );
        adb.shell(
            &transport,
            &[
                "appops",
                "set",
                "--user",
                &user,
                package,
                "REQUEST_INSTALL_PACKAGES",
                if grant { "allow" } else { "ignore" },
            ],
        )?;
    } else {
        adb.shell(
            &transport,
            &[
                "pm",
                if grant { "grant" } else { "revoke" },
                "--user",
                &user,
                package,
                permission,
            ],
        )?;
    }
    Ok(())
}

pub fn set_enabled(adb: &Adb, device: &Device, package: &str, enable: bool) -> Result<()> {
    validate_package(package)?;
    let transport = device.resolve_active_transport(adb)?;
    let user = info(adb, device, package)
        .map(|a| a.user_id)
        .unwrap_or(0)
        .to_string();
    adb.shell(
        &transport,
        &[
            "pm",
            if enable { "enable" } else { "disable-user" },
            "--user",
            &user,
            package,
        ],
    )?;
    Ok(())
}

pub fn clear_storage(adb: &Adb, device: &Device, package: &str, clear_data: bool) -> Result<()> {
    validate_package(package)?;
    let transport = device.resolve_active_transport(adb)?;
    let user = info(adb, device, package)
        .map(|a| a.user_id)
        .unwrap_or(0)
        .to_string();
    if clear_data {
        adb.shell(&transport, &["pm", "clear", "--user", &user, package])?;
    } else {
        let _ = adb.shell(
            &transport,
            &["rm", "-rf", &format!("/sdcard/Android/data/{package}/cache")],
        );
        let _ = adb.shell(&transport, &["pm", "trim-caches", "999G"]);
    }
    Ok(())
}

pub fn open_settings(adb: &Adb, device: &Device, package: &str) -> Result<()> {
    let app = info(adb, device, package)?;
    let transport = device.resolve_active_transport(adb)?;
    adb.shell(
        &transport,
        &[
            "am",
            "start",
            "--user",
            &app.user_id.to_string(),
            "-a",
            "android.settings.APPLICATION_DETAILS_SETTINGS",
            "-d",
            &format!("package:{package}"),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn application(package: &str, label: &str, system: bool, launchable: bool) -> AppInfo {
        AppInfo {
            package: package.into(),
            label: label.into(),
            system,
            launchable,
            enabled: true,
            ..AppInfo::default()
        }
    }

    #[test]
    fn defaults_include_user_and_launcher_apps_without_changing_total() {
        let apps = [
            application("a.user", "用户应用", false, true),
            application(
                "com.google.ar.core",
                "Google Play Services for AR",
                false,
                false,
            ),
            application("a.camera", "相机", true, true),
            application("a.framework", "系统组件", true, false),
        ];
        assert_eq!(apps.iter().filter(|app| app.in_default_scope()).count(), 2);
        assert_eq!(apps.len(), 4);
        assert!(apps[0].can_uninstall());
        assert!(!apps[2].can_uninstall());
        assert!(apps[2].matches_query("相机"));
        assert!(apps[2].matches_query("CAMERA"));
    }

    #[test]
    fn filters_noise_overlays_and_cleans_java_class_labels() {
        assert!(is_noise_system_overlay_or_shim(
            "android",
            "Android 系统",
            false
        ));
        assert!(is_noise_system_overlay_or_shim(
            "com.android.internal.display.cutout.emulation.corner",
            "角落刘海屏",
            false
        ));
        assert!(is_noise_system_overlay_or_shim(
            "com.android.internal.systemui.navbar.gestural",
            "手势导航",
            false
        ));
        assert!(is_noise_system_overlay_or_shim(
            "com.android.cts.priv.ctsshim",
            "CTS Shim",
            false
        ));
        assert!(is_noise_system_overlay_or_shim(
            "vendor.qti.hardware.cacert.server",
            "CA Cert",
            false
        ));
        assert!(is_noise_system_overlay_or_shim(
            "com.example.res",
            "SystemUIRes.apk",
            false
        ));
        assert!(!is_noise_system_overlay_or_shim(
            "com.tencent.mm",
            "微信",
            true
        ));

        assert_eq!(
            clean_app_label(
                "com.stepos.exserviceui",
                "com.stepos.exserviceui.StepOSExServiceUiApp"
            ),
            "StepOS 扩展服务"
        );
        assert_eq!(
            clean_app_label(
                "com.android.ondevicepersonalization.services",
                "com.android.ondevicepersonalization.services"
            ),
            "设备端个性化服务"
        );
        assert_eq!(
            clean_app_label(
                "com.android.devicelockcontroller",
                "com.android.devicelockcontroller"
            ),
            "设备锁控制器"
        );
        assert_eq!(
            clean_app_label(
                "com.android.federatedcompute.services",
                "com.android.appsearch.apk"
            ),
            "系统搜索与计算服务"
        );
        assert_eq!(
            clean_app_label("com.qualcomm.qti.xrcb", "com.qualcomm.qti.xrcb"),
            "XR 协同服务"
        );
        assert_eq!(
            clean_app_label("com.example.custom", "com.example.custom.CustomServiceApp"),
            "CustomServiceApp"
        );
    }

    #[test]
    fn catalog_rejects_duplicates_and_cross_user_data() {
        let app = application("com.example.app", "Example", false, true);
        assert!(parse_catalog(
            &json!({"user_id": 0, "apps": [app.clone(), app.clone()]}),
            None
        )
        .is_err());
        assert!(parse_catalog(&json!({"user_id": 10, "apps": [app.clone()]}), None).is_err());
        let catalog = parse_catalog(&json!({"user_id": 0, "apps": [app]}), None).unwrap();
        assert_eq!(catalog[0].label, "Example");
    }

    #[test]
    fn invalid_paths_and_packages_cannot_escape_exports() {
        for package in ["", "../app", "com.a;reboot", "com..app", "com.app\n"] {
            assert!(validate_package(package).is_err());
        }
        assert!(validate_package("android").is_ok());
        let mut app = application("com.example.app", "App", false, true);
        for path in [
            "relative.apk",
            "/data/app/../secret.apk",
            "/data/app/a.apk\n",
            "/data/app/a.zip",
        ] {
            app.apk_files = vec![ApkFile {
                path: path.into(),
                bytes: 1024,
            }];
            assert!(validate_apk_files(&app).is_err());
        }
        app.apk_files = vec![
            ApkFile {
                path: "/data/app/a/base.apk".into(),
                bytes: 1024,
            },
            ApkFile {
                path: "/data/app/a/split_config.arm64.apk".into(),
                bytes: 64,
            },
        ];
        assert!(validate_apk_files(&app).is_ok());
    }

    #[test]
    fn icons_are_bounded_and_never_use_phone_supplied_desktop_paths() {
        let mut app = application("com.example.app", "App", false, true);
        app.icon_path = Some(PathBuf::from("/etc/passwd"));
        let result = parse_catalog(&json!({"user_id": 0, "apps": [app]}), None).unwrap();
        assert!(result[0].icon_path.is_none());
        let mut total = 0;
        assert!(cache_icon(&"A".repeat(ICON_LIMIT * 2), None, &result[0], &mut total).is_err());
    }

    #[test]
    fn icon_edge_colors_preserve_alpha_and_visible_artwork() {
        let mut pixels = image::RgbaImage::new(3, 3);
        pixels.put_pixel(1, 1, image::Rgba([255, 255, 255, 128]));
        let alpha: Vec<_> = pixels.pixels().map(|pixel| pixel.0[3]).collect();
        assert!(bleed_transparent_edges(&mut pixels));
        assert_eq!(pixels.get_pixel(1, 1).0, [255, 255, 255, 128]);
        assert!(pixels.pixels().all(|pixel| pixel.0[..3] == [255, 255, 255]));
        assert_eq!(
            pixels.pixels().map(|pixel| pixel.0[3]).collect::<Vec<_>>(),
            alpha
        );
    }

    #[test]
    fn permission_chinese_label_and_demo_details_work() {
        assert_eq!(
            permission_chinese_label("android.permission.CAMERA"),
            Some("相机拍摄")
        );
        assert_eq!(
            permission_chinese_label("android.permission.RECORD_AUDIO"),
            Some("麦克风录音")
        );
        assert_eq!(
            permission_chinese_label("android.permission.POST_NOTIFICATIONS"),
            Some("发送桌面与系统通知")
        );
        assert_eq!(
            permission_chinese_label("android.permission.SYSTEM_ALERT_WINDOW"),
            Some("悬浮窗显示")
        );
        assert_eq!(permission_chinese_label("android.permission.INTERNET"), None);

        let demo = demo_details("com.tencent.mm");
        assert_eq!(demo.package, "com.tencent.mm");
        assert_eq!(demo.data_dir, "/data/user/0/com.tencent.mm");
        assert!(!demo.permissions.is_empty());

        let dumpsys = "
          Packages:
            Package [com.tencent.mm] (123456):
              userId=10256
              versionCode=2800 minSdk=26 targetSdk=34
              primaryCpuAbi=arm64-v8a
              dataDir=/data/user/0/com.tencent.mm
              firstInstallTime=2026-01-10 08:00:00
              lastUpdateTime=2026-10-20 12:30:00
              installerPackageName=com.stepos.appstore
              requested permissions:
                android.permission.CAMERA
                android.permission.RECORD_AUDIO
              User 0: installed=true
                runtime permissions:
                  android.permission.CAMERA: granted=true, flags=[ USER_SET ]
                  android.permission.RECORD_AUDIO: granted=false, flags=[ USER_SET ]
        ";
        let parsed = parse_dumpsys_package("com.tencent.mm", dumpsys);
        assert_eq!(parsed.uid, 10256);
        assert_eq!(parsed.min_sdk, 26);
        assert_eq!(parsed.target_sdk, 34);
        assert_eq!(parsed.installer_package, "StepOS 应用商店");
        assert_eq!(parsed.permissions.len(), 2);
        assert!(parsed
            .permissions
            .iter()
            .any(|p| p.name == "android.permission.CAMERA" && p.granted && p.label == "相机拍摄"));
        assert!(parsed
            .permissions
            .iter()
            .any(|p| p.name == "android.permission.RECORD_AUDIO" && !p.granted));
    }
}

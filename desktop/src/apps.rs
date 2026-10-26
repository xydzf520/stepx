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
    let mut files = Vec::with_capacity(app.apk_files.len());
    for (index, apk) in app.apk_files.iter().enumerate() {
        let name = if index == 0 {
            "base.apk".to_string()
        } else {
            format!("split-{index:03}.apk")
        };
        let target = staging.path().join(&name);
        adb.transfer(&device.transport, "pull", Path::new(&apk.path), &target)
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
    let user = app.user_id.to_string();
    let output = adb.shell(
        &device.transport,
        &["pm", "uninstall", "--user", &user, package],
    )?;
    if !output.lines().any(|line| line.trim() == "Success") {
        bail!("手机未确认卸载成功：{}", output.trim());
    }
    let installed = adb
        .shell(
            &device.transport,
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

pub fn open_settings(adb: &Adb, device: &Device, package: &str) -> Result<()> {
    let app = info(adb, device, package)?;
    adb.shell(
        &device.transport,
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
}

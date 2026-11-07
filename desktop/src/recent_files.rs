//! Recent shared files. Metadata discovery and verified copies never open phone UI.
use crate::{
    adb::{shell_quote, Adb},
    device::{sha256_file, Device},
    resources, storage,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const RETAIN: usize = 160;
const VISIT_LIMIT: usize = 12000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Image,
    Pdf,
    Document,
    Media,
    Archive,
    Other,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "图片",
            Self::Pdf => "PDF",
            Self::Document => "文档",
            Self::Media => "影音",
            Self::Archive => "压缩包",
            Self::Other => "文件",
        }
    }
    pub fn for_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "svg" => Self::Image,
            "pdf" => Self::Pdf,
            "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "rtf" | "txt"
            | "md" | "csv" | "html" => Self::Document,
            "mp4" | "mov" | "mkv" | "webm" | "mp3" | "wav" | "m4a" | "flac" | "ogg" => Self::Media,
            "zip" | "rar" | "7z" | "gz" | "tar" => Self::Archive,
            _ => Self::Other,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFile {
    pub path: PathBuf,
    /// Physical device identity, not the currently selected transport address.
    pub device: Option<String>,
    pub bytes: u64,
    pub at: i64,
    pub modified: i64,
    pub kind: Kind,
    pub folder: String,
    #[serde(default)]
    pub thumbnail: Option<PathBuf>,
}
impl RecentFile {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
    pub fn id(&self) -> String {
        format!(
            "{}:{}",
            self.device.as_deref().unwrap_or("desktop"),
            self.path.display()
        )
    }
}
#[derive(Default, Debug)]
pub struct Scan {
    pub files: Vec<RecentFile>,
    pub limited: bool,
    pub unavailable: Vec<PathBuf>,
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub fn now() -> i64 {
    millis(SystemTime::now())
}
fn usable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.starts_with('.')
        && !name.starts_with("~$")
        && !lower.contains(".crypt")
        && !lower.contains(".db-")
        && !lower.ends_with(".log")
        && ![".db", ".sqlite", ".sqlite3", ".db-wal", ".db-shm", ".lock"]
            .iter()
            .any(|e| lower.ends_with(e))
        && !matches!(
            lower.as_str(),
            "manifest.json"
                | "desktop-link.json"
                | "package-lock.json"
                | "yarn.lock"
                | "cargo.lock"
        )
        && !name.chars().any(char::is_control)
        && ![
            ".part",
            ".partial",
            ".crdownload",
            ".download",
            ".tmp",
            ".temp",
            ".swp",
            "~",
        ]
        .iter()
        .any(|e| name.to_ascii_lowercase().ends_with(e))
}
fn excluded_dir(name: &str) -> bool {
    name.eq_ignore_ascii_case("mmkv")
        || name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "vendor"
                | "build"
                | "__pycache__"
                | "cache"
                | "thumbnails"
                | "db_storage"
                | "apm_record"
                | "logs"
                | "Databases"
                | "Backups"
                | "venv"
        )
}
// Some chat clients store mutable account state beside user documents. Keep
// those exact machine files out of the discovery inbox without restricting
// explicit copies or hiding user-authored JSON/configuration documents.
fn discoverable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    usable_name(name)
        && !lower.ends_with(".crc")
        && !matches!(lower.as_str(), "login_configv2" | "rs_config" | "global_config")
}
fn finish(scan: &mut Scan) {
    scan.files
        .sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.path.cmp(&b.path)));
    scan.files
        .dedup_by(|a, b| a.path == b.path && a.device == b.device);
    scan.files.truncate(RETAIN);
}

/// XDG user folders are data, never sourced/executed as a shell script.
pub fn default_roots() -> Vec<PathBuf> {
    let home = resources::user_home()
        .map(PathBuf::from)
        .unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let raw = fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let mut roots = Vec::new();
    for (key, fallback) in [
        ("DOWNLOAD", "Downloads"),
        ("DESKTOP", "Desktop"),
        ("DOCUMENTS", "Documents"),
        ("PICTURES", "Pictures"),
    ] {
        let prefix = format!("XDG_{key}_DIR=");
        let configured = raw
            .lines()
            .find_map(|line| line.trim().strip_prefix(&prefix))
            .map(|s| {
                s.trim_matches('"')
                    .replace("$HOME", &home.to_string_lossy())
            })
            .map(PathBuf::from);
        let path = configured.unwrap_or_else(|| home.join(fallback));
        // Disabled XDG folders can point at HOME; never recurse through the whole account.
        if path.is_absolute() && path != home && !roots.contains(&path) {
            roots.push(path);
        }
    }
    roots.push(crate::settings::downloads());
    roots
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub folders: Vec<PathBuf>,
    #[serde(default)]
    pub hidden: Vec<PathBuf>,
}
impl Preferences {
    fn path() -> PathBuf {
        resources::user_config().join("stepx/recent-files.json")
    }
    pub fn load() -> Self {
        fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        serde_json::to_writer(tmp.as_file(), self)?;
        tmp.as_file().sync_all()?;
        tmp.persist(path)?;
        Ok(())
    }
    pub fn roots(&self, workspace: Option<&Path>) -> Vec<PathBuf> {
        let mut roots = default_roots();
        roots.extend(self.folders.iter().cloned());
        if let Some(p) = workspace {
            roots.push(p.to_path_buf());
        }
        roots.retain(|p| !self.hidden.contains(p));
        roots.sort();
        roots.dedup();
        roots
    }
}

/// A bounded walk of selected folders, metadata only. Incomplete writes are deferred.
pub fn desktop_files(roots: &[PathBuf]) -> Scan {
    let mut scan = Scan::default();
    let mut visited = HashSet::new();
    let started = Instant::now();
    let mut count = 0;
    let now = now();
    // Breadth first: each selected folder gets a turn before deep archive trees.
    let mut queue = std::collections::VecDeque::from_iter(roots.iter().cloned().map(|p| (p, 0)));
    while let Some((dir, depth)) = queue.pop_front() {
        if started.elapsed() > Duration::from_secs(3) || count >= VISIT_LIMIT {
            scan.limited = true;
            break;
        }
        let canonical = match dir.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                scan.unavailable.push(dir);
                continue;
            }
        };
        if !visited.insert(canonical) {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                scan.unavailable.push(dir);
                continue;
            }
        };
        for entry in entries.flatten() {
            count += 1;
            if count > VISIT_LIMIT || started.elapsed() > Duration::from_secs(3) {
                scan.limited = true;
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if !excluded_dir(&name) {
                    if depth < 6 {
                        queue.push_back((entry.path(), depth + 1));
                    } else {
                        scan.limited = true;
                    }
                }
                continue;
            }
            if !kind.is_file() || !discoverable_name(&name) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().map(millis).unwrap_or(0);
            if meta.len() == 0 || modified > now - 2000 {
                continue;
            }
            let path = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            scan.files.push(RecentFile {
                kind: Kind::for_path(&path),
                path,
                device: None,
                bytes: meta.len(),
                at: meta.created().map(millis).unwrap_or(modified).max(modified),
                modified,
                folder: dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                thumbnail: None,
            });
        }
    }
    finish(&mut scan);
    // Decode only tiny previews off the UI thread; large/unsupported files keep a type icon.
    for file in scan.files.iter_mut().take(24) {
        file.thumbnail = thumbnail(file);
    }
    scan
}

fn thumbnail(file: &RecentFile) -> Option<PathBuf> {
    thumbnail_in(file, &resources::user_state().join("recent-thumbnails"))
}
fn thumbnail_in(file: &RecentFile, root: &Path) -> Option<PathBuf> {
    if file.kind != Kind::Image || file.bytes > 20 * 1024 * 1024 {
        return None;
    }
    let meta = fs::symlink_metadata(&file.path).ok()?;
    if !meta.is_file() || meta.len() != file.bytes
        || millis(meta.modified().ok()?) != file.modified {
        return None;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    file.path.hash(&mut h);
    meta.modified().ok()?.hash(&mut h);
    meta.len().hash(&mut h);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ctime().hash(&mut h);
        meta.ctime_nsec().hash(&mut h);
    }
    let out = root.join(format!("{:x}.png", h.finish()));
    if fs::symlink_metadata(&out).is_ok_and(|m| m.is_file()) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(0o600)).ok()?;
        }
        return Some(out);
    }
    let mut reader = image::ImageReader::open(&file.path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(48 * 1024 * 1024);
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(96, 96);
    fs::create_dir_all(root).ok()?;
    if !fs::symlink_metadata(root).ok()?.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).ok()?;
    }
    let mut pending = tempfile::Builder::new().prefix(".thumbnail-pending-").tempfile_in(root).ok()?;
    image.write_to(pending.as_file_mut(), image::ImageFormat::Png).ok()?;
    // A concurrent scan may already have published this identical cache key.
    match pending.persist_noclobber(&out) {
        Ok(_) => {},
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists
            && fs::symlink_metadata(&out).is_ok_and(|m| m.is_file()) => {},
        Err(_) => return None,
    }
    Some(out)
}

/// Generated previews only. Keep at most 512 recent entries, with a seven-day
/// lifetime. Use a bounded heap rather than collecting the whole cache in RAM.
pub fn clean_thumbnails(root: &Path, now: SystemTime) -> Result<(usize, u64)> {
    let mut retained = std::collections::BinaryHeap::new();
    let (mut removed, mut bytes) = (0, 0u64);
    if !root.exists() { return Ok((removed, bytes)); }
    ensure!(!fs::symlink_metadata(root)?.file_type().is_symlink(), "缩略图缓存目录不能是链接");
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue; };
        let Some(stem) = name.strip_suffix(".png") else { continue; };
        if stem.is_empty() || stem.len() > 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !entry.file_type()?.is_file() { continue; }
        let meta = entry.metadata()?;
        let modified = meta.modified()?;
        let expired = now.duration_since(modified).unwrap_or_default() > Duration::from_secs(7 * 86400);
        let discard = if expired {
            Some((path, meta.len()))
        } else {
            retained.push(std::cmp::Reverse((modified, path, meta.len())));
            (retained.len() > 512).then(|| {
                let std::cmp::Reverse((_, path, size)) = retained.pop().unwrap();
                (path, size)
            })
        };
        if let Some((path, size)) = discard {
            fs::remove_file(path)?;
            removed += 1;
            bytes = bytes.saturating_add(size);
        }
    }
    Ok((removed, bytes))
}

pub fn phone_files(adb: &Adb, device: &Device) -> Result<Scan> {
    let transport = device.resolve_active_transport(adb)?;
    // Native filemanager_search_file launches a phone screen, not a metadata query.
    // Public shared folders only; no private app data, no wake/screen automation.
    let roots = [
        "Download",
        "DCIM",
        "Pictures",
        "Documents",
        "Movies",
        "Music",
        "Bluetooth",
        "Android/media",
        "tencent/MicroMsg/Download",
    ];
    let roots = roots
        .iter()
        .map(|s| shell_quote(&format!("/sdcard/{s}")))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!("for d in {roots}; do if [ -d \"$d\" ]; then find \"$d\" -maxdepth 6 \\( -name '.*' -o -name cache -o -name thumbnails -o -name Databases -o -name Backups \\) -prune -o -type f -exec stat -c '%Y %Z %s %n' '{{}}' +; fi; done | head -n {}", VISIT_LIMIT+1);
    let output = adb.run_timeout(
        &["-s", &transport, "shell", &script],
        Duration::from_secs(12),
    )?;
    Ok(parse_phone(&output, &device.serial, now()))
}
fn parse_phone(raw: &str, device: &str, now: i64) -> Scan {
    let mut scan = Scan {
        limited: raw.lines().count() > VISIT_LIMIT,
        ..Default::default()
    };
    for line in raw.lines().take(VISIT_LIMIT) {
        let mut parts = line.splitn(4, ' ');
        let (Some(m), Some(c), Some(b), Some(path)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(m), Ok(c), Ok(bytes)) = (m.parse::<i64>(), c.parse::<i64>(), b.parse::<u64>())
        else {
            continue;
        };
        let modified = m.saturating_mul(1000);
        let at = modified.max(c.saturating_mul(1000));
        let path = PathBuf::from(path);
        if storage::validate_path(&path.to_string_lossy()).is_err()
            || !discoverable_name(&path.file_name().unwrap_or_default().to_string_lossy())
            || bytes == 0
            || at > now - 2000
        {
            continue;
        }
        scan.files.push(RecentFile {
            kind: Kind::for_path(&path),
            folder: path
                .parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path,
            device: Some(device.into()),
            bytes,
            at,
            modified,
            thumbnail: None,
        });
    }
    finish(&mut scan);
    scan
}

/// A dedicated directory per delivery keeps original filenames without replacing anything.
pub fn send_to_phone(adb: &Adb, device: &Device, file: &RecentFile) -> Result<String> {
    ensure!(file.device.is_none(), "请选择电脑文件");
    let meta = fs::symlink_metadata(&file.path).context("文件已移动或删除，请刷新")?;
    ensure!(
        meta.is_file()
            && meta.len() == file.bytes
            && meta.modified().map(millis).unwrap_or(0) == file.modified,
        "文件仍在变化，请刷新后重试"
    );
    let name = file.name();
    ensure!(usable_name(&name), "文件名不可用");
    let hash = sha256_file(&file.path)?;
    let transport = device.resolve_active_transport(adb)?;
    let base = if file.kind == Kind::Image {
        "Pictures"
    } else {
        "Download"
    };
    let dir = format!(
        "/sdcard/{base}/StepX/{}-{:08x}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        rand::random::<u32>()
    );
    adb.shell(
        &transport,
        &["mkdir", "-p", &format!("/sdcard/{base}/StepX")],
    )?;
    adb.shell(&transport, &["mkdir", &dir])?;
    let pending = format!("{dir}/.receiving");
    let target = format!("{dir}/{name}");
    let result = (|| -> Result<String> {
        adb.transfer(&transport, "push", &file.path, Path::new(&pending))?;
        let remote = adb.shell(&transport, &["sha256sum", &pending])?;
        ensure!(
            remote.split_whitespace().next() == Some(hash.as_str())
                && sha256_file(&file.path)? == hash,
            "文件在传输期间变化，未完成导入，请重试"
        );
        adb.shell(&transport, &["mv", &pending, &target])?;
        // Index the final path for Gallery/Files. This broadcast never brings an app forward.
        let uri = format!(
            "file://{}",
            target.replace("/sdcard/", "/storage/emulated/0/")
        );
        let _ = adb.shell(
            &transport,
            &[
                "am",
                "broadcast",
                "-a",
                "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                "-d",
                &uri,
            ],
        );
        Ok(target.clone())
    })();
    if result.is_err() {
        let _ = adb.shell(&transport, &["rm", "-f", &pending]);
        let _ = adb.shell(&transport, &["rmdir", &dir]);
    }
    result
}

pub fn save_to_desktop(
    adb: &Adb,
    device: &Device,
    file: &RecentFile,
    destination: &Path,
) -> Result<PathBuf> {
    ensure!(
        file.device.as_deref() == Some(device.serial.as_str()),
        "手机已切换，请重新选择文件"
    );
    let remote = file.path.to_str().context("文件路径不可用")?;
    storage::validate_path(remote)?;
    let transport = device.resolve_active_transport(adb)?;
    let canonical = adb.shell(&transport, &["readlink", "-f", remote])?;
    let canonical = canonical.trim();
    ensure!(
        canonical.starts_with("/storage/emulated/0/"),
        "文件不在手机共享目录"
    );
    let before = adb.shell(&transport, &["sha256sum", canonical])?;
    let hash = before
        .split_whitespace()
        .next()
        .context("无法校验手机文件")?;
    fs::create_dir_all(destination)?;
    let temp = tempfile::tempdir_in(destination)?;
    let local = temp.path().join(file.name());
    adb.transfer(&transport, "pull", Path::new(canonical), &local)?;
    let after = adb.shell(&transport, &["sha256sum", canonical])?;
    ensure!(
        sha256_file(&local)? == hash && after.split_whitespace().next() == Some(hash),
        "手机文件在传输期间变化，请重试"
    );
    let final_path = destination.join(file.name());
    // persist_noclobber is atomic even when another app writes the same filename.
    let mut output = tempfile::NamedTempFile::new_in(destination)?;
    std::io::copy(&mut fs::File::open(&local)?, output.as_file_mut())?;
    output.as_file().sync_all()?;
    let final_path = if final_path.exists() {
        destination.join(format!(
            "{}-{:08x}-{}",
            chrono::Local::now().format("%H%M%S"),
            rand::random::<u32>(),
            file.name()
        ))
    } else {
        final_path
    };
    output
        .persist_noclobber(&final_path)
        .context("目标文件已存在，未覆盖，请重试")?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn thumbnail_cache_is_atomic_private_and_detects_submillisecond_image_changes() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("picture.png");
        let cache = dir.path().join("cache");
        let original_time = UNIX_EPOCH + Duration::new(1000, 1);
        image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0])).save(&image_path).unwrap();
        fs::File::options().write(true).open(&image_path).unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_time)).unwrap();
        let file = RecentFile {
            path:image_path.clone(), device:None, bytes:fs::metadata(&image_path).unwrap().len(),
            at:1_000_000, modified:1_000_000, kind:Kind::Image, folder:String::new(), thumbnail:None,
        };
        let first = thumbnail_in(&file, &cache).unwrap();
        assert_eq!(image::open(&first).unwrap().to_rgb8().get_pixel(0,0).0, [255,0,0]);
        assert_eq!(thumbnail_in(&file, &cache), Some(first.clone()));
        image::RgbImage::from_pixel(1, 1, image::Rgb([0, 0, 255])).save(&image_path).unwrap();
        assert_eq!(fs::metadata(&image_path).unwrap().len(), file.bytes);
        fs::File::options().write(true).open(&image_path).unwrap()
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::new(1000, 2))).unwrap();
        let second = thumbnail_in(&file, &cache).unwrap();
        assert_ne!(first, second);
        assert_eq!(image::open(&second).unwrap().to_rgb8().get_pixel(0,0).0, [0,0,255]);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 2, "no partially written PNGs");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&cache).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(&second).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
    #[test]
    fn cache_maintenance_keeps_recent_entries_and_never_follows_links_or_touches_originals() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000);
        for i in 0..520u64 {
            let path = dir.path().join(format!("{i:016x}.png"));
            fs::write(&path, b"generated preview").unwrap();
            fs::File::options().write(true).open(path).unwrap().set_times(
                fs::FileTimes::new().set_modified(now - Duration::from_secs(i + 1)),
            ).unwrap();
        }
        let old = dir.path().join("f000000000000000.png");
        fs::write(&old, b"expired preview").unwrap();
        fs::File::options().write(true).open(&old).unwrap().set_times(
            fs::FileTimes::new().set_modified(now - Duration::from_secs(8 * 86400)),
        ).unwrap();
        let original = dir.path().join("user-photo.png");
        fs::write(&original, b"original photo").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&original, dir.path().join("ffffffffffffffff.png")).unwrap();
        let (removed, bytes) = clean_thumbnails(dir.path(), now).unwrap();
        assert_eq!(removed, 9);
        assert!(bytes > 0);
        assert!(!old.exists());
        for i in 0..520u64 {
            assert_eq!(dir.path().join(format!("{i:016x}.png")).exists(), i < 512);
        }
        assert_eq!(fs::read(&original).unwrap(), b"original photo");
        #[cfg(unix)]
        assert!(dir.path().join("ffffffffffffffff.png").is_symlink());
        assert_eq!(clean_thumbnails(dir.path(), now).unwrap(), (0,0));
    }
    #[test]
    fn discovery_omits_machine_state_but_keeps_authored_configuration_files() {
        for name in ["login_configv2", "rs_config", "global_config", "rs_config.crc"] {
            assert!(!super::discoverable_name(name));
            assert!(super::usable_name(name), "explicit file copies still allowed");
        }
        for name in ["项目配置.json", "config.toml", "global_config.md", "部署说明.pdf", "README"] {
            assert!(super::discoverable_name(name));
        }
    }
    use super::*;
    #[test]
    fn discovery_excludes_partial_hidden_symlink_and_deduplicates_roots() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        for n in [
            "报告.pdf",
            "设计.png",
            ".hidden.pdf",
            "草稿.docx.crdownload",
            "~$文档.docx",
            "msgstore.db.crypt15",
            "contacts.sqlite3",
            "message.db-incremental.material",
            "worker.log",
            "manifest.json",
        ] {
            fs::write(dir.join(n), b"ok").unwrap();
        }
        fs::create_dir(dir.join("node_modules")).unwrap();
        fs::write(dir.join("node_modules/ignore.txt"), b"x").unwrap();
        fs::create_dir(dir.join("db_storage")).unwrap();
        fs::write(dir.join("db_storage/internal.material"), b"x").unwrap();
        fs::create_dir(dir.join("mmkv")).unwrap();
        fs::write(dir.join("mmkv/account_state"), b"x").unwrap();
        fs::create_dir(dir.join("apm_record")).unwrap();
        fs::write(dir.join("apm_record/memory"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("报告.pdf"), dir.join("link.pdf")).unwrap();
        // No sleeps: use std FileTimes to make files stable.
        for entry in fs::read_dir(dir).unwrap().flatten() {
            if entry.file_type().unwrap().is_file() {
                fs::File::options()
                    .write(true)
                    .open(entry.path())
                    .unwrap()
                    .set_times(
                        fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1000)),
                    )
                    .unwrap();
            }
        }
        let scan = desktop_files(&[dir.into(), dir.into()]);
        assert_eq!(scan.files.len(), 2);
        assert!(scan.files.iter().any(|f| f.kind == Kind::Pdf));
        fs::remove_file(dir.join("报告.pdf")).unwrap();
        assert_eq!(desktop_files(&[dir.into()]).files.len(), 1);
    }
    #[test]
    fn phone_metadata_preserves_spaces_and_orders_by_arrival_not_filename() {
        let scan=parse_phone("100 210 24 /sdcard/Download/新 报告.pdf\n200 200 80 /sdcard/Pictures/a.png\n210 210 90 /data/private.pdf\n210 210 50 /sdcard/Download/a.tmp\n220 220 80 /sdcard/Android/media/backup.db.crypt15\n", "phone-a", 300000);
        assert_eq!(scan.files.len(), 2);
        assert_eq!(scan.files[0].name(), "新 报告.pdf");
        assert_eq!(scan.files[0].at, 210000);
        assert_eq!(scan.files[0].device.as_deref(), Some("phone-a"));
    }
    #[test]
    fn pending_writes_do_not_appear_and_explicit_folder_preferences_round_trip() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("writing.pdf"), b"writing").unwrap();
        assert!(desktop_files(&[t.path().into()]).files.is_empty());
        let _scope = resources::override_config_dir_for_current_thread(t.path().into());
        let p = Preferences {
            folders: vec![t.path().join("exports")],
            hidden: vec![t.path().join("private")],
        };
        p.save().unwrap();
        assert_eq!(Preferences::load().folders, p.folders);
    }
}

//! Public community QR updates. Expiry belongs to the published QR, never to a download.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

pub const MANIFEST_PATH: &str = "community/group-qr.json";
pub const SOURCES: [&str; 2] = [
    "https://raw.githubusercontent.com/xydzf520/stepx/main/",
    "https://cdn.jsdelivr.net/gh/xydzf520/stepx@main/",
];
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub image: String,
    pub sha256: String,
    pub expires_at: String,
}

impl Manifest {
    pub fn expires_at_ms(&self) -> Result<i64> {
        ensure!(self.schema == 1, "不支持的群码更新格式");
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "群码校验值无效"
        );
        ensure!(
            self.image == format!("group-{}.png", self.sha256),
            "群码图片路径无效"
        );
        Ok(chrono::DateTime::parse_from_rfc3339(&self.expires_at)
            .context("群码缺少有效的到期时间")?
            .timestamp_millis())
    }
}

#[derive(Clone, Debug)]
pub struct CachedQr {
    pub path: PathBuf,
    pub expires_at_ms: i64,
    pub sha256: String,
}

pub fn cache_dir() -> PathBuf {
    crate::resources::user_config().join("stepx-companion/community")
}

pub fn bundled_manifest() -> Manifest {
    serde_json::from_str(include_str!("../../community/group-qr.json"))
        .expect("bundled group QR manifest")
}

pub fn validate_image(bytes: &[u8], manifest: &Manifest) -> Result<()> {
    manifest.expires_at_ms()?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
        "群码图片大小无效"
    );
    ensure!(
        crate::device::sha256_reader(bytes)? == manifest.sha256,
        "群码图片校验失败"
    );
    ensure!(
        image::guess_format(bytes)? == image::ImageFormat::Png,
        "群码图片格式无效"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().context("群码图片无法读取")?;
    ensure!(
        decoded.width() >= 128 && decoded.height() >= 128,
        "群码图片分辨率不足"
    );
    Ok(())
}

fn read_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("group-qr.json");
    ensure!(
        std::fs::metadata(&path)?.len() <= MAX_MANIFEST_BYTES as u64,
        "群码更新文件过大"
    );
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(path)?)?;
    manifest.expires_at_ms()?;
    Ok(manifest)
}

pub fn load_cached(dir: &Path) -> Option<CachedQr> {
    let manifest = read_manifest(dir).ok()?;
    let path = dir.join(&manifest.image);
    if std::fs::metadata(&path).ok()?.len() > MAX_IMAGE_BYTES as u64 {
        return None;
    }
    validate_image(&std::fs::read(&path).ok()?, &manifest).ok()?;
    Some(CachedQr {
        path,
        expires_at_ms: manifest.expires_at_ms().ok()?,
        sha256: manifest.sha256,
    })
}

fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(dir.join(name)).map_err(|e| e.error)?;
    Ok(())
}

fn sync_from(
    dir: &Path,
    now_ms: i64,
    mut fetch: impl FnMut(&str, usize) -> Result<Vec<u8>>,
) -> Result<CachedQr> {
    let bytes = fetch(MANIFEST_PATH, MAX_MANIFEST_BYTES)?;
    ensure!(bytes.len() <= MAX_MANIFEST_BYTES, "群码更新文件过大");
    let manifest: Manifest = serde_json::from_slice(&bytes).context("群码更新文件无法读取")?;
    let expires_at_ms = manifest.expires_at_ms()?;
    ensure!(expires_at_ms > now_ms, "发布的群码已过期");
    // Downloading the same QR again must never restart its validity window.
    for old in [read_manifest(dir).ok(), Some(bundled_manifest())]
        .into_iter()
        .flatten()
    {
        ensure!(
            old.sha256 != manifest.sha256 || expires_at_ms <= old.expires_at_ms()?,
            "相同群码不能通过重复发布延长有效期"
        );
        ensure!(
            old.sha256 == manifest.sha256 || expires_at_ms >= old.expires_at_ms()?,
            "忽略来源中较旧的群码"
        );
    }
    let cached = load_cached(dir);
    if cached.as_ref().is_none_or(|c| c.sha256 != manifest.sha256) {
        let image = fetch(&format!("community/{}", manifest.image), MAX_IMAGE_BYTES)?;
        validate_image(&image, &manifest)?;
        // Content-addressed files keep the previous usable image intact even if
        // downloading or saving the replacement fails halfway through.
        atomic_write(dir, &manifest.image, &image)?;
    }
    atomic_write(dir, "group-qr.json", &serde_json::to_vec_pretty(&manifest)?)?;
    Ok(CachedQr {
        path: dir.join(manifest.image),
        expires_at_ms,
        sha256: manifest.sha256,
    })
}

fn download(url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let file = tempfile::NamedTempFile::new()?;
    let mut command = Command::new("curl");
    crate::resources::quiet_command(&mut command);
    let output = command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "4",
            "--max-time",
            "8",
            "--max-filesize",
            &max_bytes.to_string(),
            "--header",
            "Cache-Control: no-cache",
            "--output",
        ])
        .arg(file.path())
        .arg(url)
        .output()
        .context("无法启动群码下载")?;
    ensure!(
        output.status.success(),
        "群码来源暂不可用（{}）",
        output.status
    );
    ensure!(
        file.as_file().metadata()?.len() <= max_bytes as u64,
        "群码下载内容过大"
    );
    Ok(std::fs::read(file.path())?)
}

/// Runs only on the background worker. A failed source never changes the cache.
pub fn sync(dir: &Path, now_ms: i64) -> Result<CachedQr> {
    let mut errors = Vec::new();
    for source in SOURCES {
        match sync_from(dir, now_ms, |path, limit| {
            download(&format!("{source}{path}"), limit)
        }) {
            Ok(qr) => return Ok(qr),
            Err(error) => errors.push(error.to_string()),
        }
    }
    bail!("{}", errors.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;
    const IMAGE: &[u8] = include_bytes!("../assets/qr_group.png");
    fn fetch(path: &str, _: usize) -> Result<Vec<u8>> {
        Ok(if path == MANIFEST_PATH {
            serde_json::to_vec(&bundled_manifest())?
        } else {
            IMAGE.to_vec()
        })
    }
    #[test]
    fn remote_update_installs_validated_image_and_unchanged_checks_do_not_redownload() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = bundled_manifest();
        let now = manifest.expires_at_ms().unwrap() - 86400000;
        let first = sync_from(dir.path(), now, fetch).unwrap();
        assert_eq!(std::fs::read(&first.path).unwrap(), IMAGE);
        let second = sync_from(dir.path(), now + 1000, |path, limit| {
            assert_eq!(path, MANIFEST_PATH, "unchanged image must use cache");
            fetch(path, limit)
        })
        .unwrap();
        assert_eq!(first.expires_at_ms, second.expires_at_ms);
        assert_eq!(load_cached(dir.path()).unwrap().sha256, manifest.sha256);
    }
    #[test]
    fn failures_keep_previous_image_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let now = bundled_manifest().expires_at_ms().unwrap() - 86400000;
        let first = sync_from(dir.path(), now, fetch).unwrap();
        let before = std::fs::read(dir.path().join("group-qr.json")).unwrap();
        for bad in [b"<html>404</html>".as_slice(), b"{}".as_slice()] {
            assert!(sync_from(dir.path(), now, |_, _| Ok(bad.to_vec())).is_err());
        }
        assert!(sync_from(dir.path(), now, |_, _| bail!("offline")).is_err());
        assert_eq!(
            std::fs::read(dir.path().join("group-qr.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read(&first.path).unwrap(), IMAGE);
    }
    #[test]
    fn expiry_is_not_extended_by_redownloading_or_republishing_old_image() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = bundled_manifest();
        let expiry = manifest.expires_at_ms().unwrap();
        sync_from(dir.path(), expiry - 1, fetch).unwrap();
        assert!(sync_from(dir.path(), expiry, fetch).is_err());
        manifest.expires_at = "2026-12-01T00:00:00+08:00".into();
        assert!(sync_from(dir.path(), expiry, |_, _| Ok(serde_json::to_vec(
            &manifest
        )?))
        .is_err());
        assert_eq!(load_cached(dir.path()).unwrap().expires_at_ms, expiry);
        let fresh_install = tempfile::tempdir().unwrap();
        assert!(
            sync_from(fresh_install.path(), expiry, |_, _| {
                Ok(serde_json::to_vec(&manifest)?)
            })
            .is_err(),
            "bundled QR also pins its true expiry on a fresh install"
        );
    }
    #[test]
    fn rejects_corrupt_images_and_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = bundled_manifest();
        let now = manifest.expires_at_ms().unwrap() - 1;
        assert!(sync_from(dir.path(), now, |path, limit| {
            if path == MANIFEST_PATH {
                fetch(path, limit)
            } else {
                Ok(vec![0; 2048])
            }
        })
        .is_err());
        assert!(load_cached(dir.path()).is_none());
        manifest.image = "../qr_group.png".into();
        assert!(manifest.expires_at_ms().is_err());
    }

    #[test]
    fn new_image_replaces_cache_and_stale_mirror_cannot_roll_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = bundled_manifest();
        let now = manifest.expires_at_ms().unwrap() - 1;
        sync_from(dir.path(), now, fetch).unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(128, 128)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        manifest.sha256 = crate::device::sha256_reader(bytes.as_slice()).unwrap();
        manifest.image = format!("group-{}.png", manifest.sha256);
        manifest.expires_at = "2026-11-12T00:00:00+08:00".into();
        let installed = sync_from(dir.path(), now, |path, _| {
            Ok(if path == MANIFEST_PATH {
                serde_json::to_vec(&manifest)?
            } else {
                bytes.clone()
            })
        })
        .unwrap();
        assert_eq!(installed.sha256, manifest.sha256);
        assert!(sync_from(dir.path(), now, fetch).is_err());
        assert_eq!(load_cached(dir.path()).unwrap().sha256, manifest.sha256);
    }
}

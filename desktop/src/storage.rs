use crate::{
    adb::{shell_quote, Adb},
    device::Device,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub directory: bool,
    pub symlink: bool,
}

impl Entry {
    pub fn sort_time_key(&self) -> u64 {
        let digits: String = self.name.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 8 {
            digits
                .chars()
                .take(14)
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        } else {
            0
        }
    }

    pub fn date_label(&self) -> Option<String> {
        let digits: Vec<char> = self.name.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 8 {
            let y: String = digits[0..4].iter().collect();
            let m: String = digits[4..6].iter().collect();
            let d: String = digits[6..8].iter().collect();
            if let (Ok(yv), Ok(mv), Ok(dv)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>())
            {
                if (2015..=2035).contains(&yv) && (1..=12).contains(&mv) && (1..=31).contains(&dv) {
                    return Some(format!("{yv:04}-{mv:02}-{dv:02}"));
                }
            }
        }
        None
    }

    pub fn kind_badge(&self) -> &'static str {
        if self.symlink {
            "链接"
        } else if self.directory {
            "文件夹"
        } else {
            let lower = self.name.to_ascii_lowercase();
            if lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".png")
                || lower.ends_with(".webp")
                || lower.ends_with(".gif")
                || lower.ends_with(".heic")
            {
                "图片"
            } else if lower.ends_with(".mp4")
                || lower.ends_with(".mkv")
                || lower.ends_with(".mov")
                || lower.ends_with(".webm")
            {
                "视频"
            } else if lower.ends_with(".mp3")
                || lower.ends_with(".flac")
                || lower.ends_with(".wav")
                || lower.ends_with(".m4a")
                || lower.ends_with(".ogg")
            {
                "音频"
            } else if lower.ends_with(".apk") {
                "安装包"
            } else if lower.ends_with(".zip")
                || lower.ends_with(".tar")
                || lower.ends_with(".gz")
                || lower.ends_with(".7z")
                || lower.ends_with(".rar")
            {
                "压缩包"
            } else if lower.ends_with(".pdf")
                || lower.ends_with(".doc")
                || lower.ends_with(".docx")
                || lower.ends_with(".xlsx")
                || lower.ends_with(".xls")
                || lower.ends_with(".pptx")
                || lower.ends_with(".ppt")
                || lower.ends_with(".txt")
                || lower.ends_with(".md")
                || lower.ends_with(".json")
                || lower.ends_with(".csv")
                || lower.ends_with(".html")
                || lower.ends_with(".htm")
                || lower.ends_with(".xml")
                || lower.ends_with(".log")
                || lower.ends_with(".yaml")
                || lower.ends_with(".yml")
                || lower.ends_with(".toml")
                || lower.ends_with(".rtf")
            {
                "文档"
            } else {
                "文件"
            }
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.2} GB", b / GB)
    }
}

pub fn validate_path(path: &str) -> Result<()> {
    if path.contains('\0') || path.contains('\n') || path.contains('\r') {
        bail!("路径包含不支持的字符");
    }
    if path != "/sdcard" && !path.starts_with("/sdcard/") {
        bail!("文件管理仅限手机共享存储 /sdcard");
    }
    if Path::new(path)
        .components()
        .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
        || path.split('/').any(|p| p == "." || p == "..")
    {
        bail!("路径不能包含 . 或 ..");
    }
    Ok(())
}

fn verified_path(adb: &Adb, device: &Device, path: &str) -> Result<String> {
    validate_path(path)?;
    device.verify(adb)?;
    resolve_storage_path(adb, device, path)
}

fn resolve_storage_path(adb: &Adb, device: &Device, path: &str) -> Result<String> {
    validate_path(path)?;
    let canonical = adb.shell(&device.transport, &["readlink", "-f", path])?;
    let canonical = canonical.trim().to_string();
    if canonical != "/storage/emulated/0" && !canonical.starts_with("/storage/emulated/0/") {
        bail!("该路径不属于手机共享存储（符号链接或存储用户不受支持）");
    }
    Ok(canonical)
}

pub fn list(adb: &Adb, device: &Device, path: &str) -> Result<Vec<Entry>> {
    let canonical = verified_path(adb, device, path)?;
    let script = format!("find {} -mindepth 1 -maxdepth 1 -exec sh -c 'for f do stat -c \"%f\n%s\" \"$f\" && printf \"%s\\0\" \"$f\"; done' sh {{}} +", shell_quote(&canonical));
    let text = adb.script(&device.transport, &script)?;
    parse_entries(&text, &canonical, path)
}

pub fn parse_entries(text: &str, canonical: &str, logical: &str) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for record in text.split('\0').filter(|r| !r.is_empty()) {
        let mut parts = record.splitn(3, '\n');
        let mode = u32::from_str_radix(parts.next().context("文件类型缺失")?.trim(), 16)?;
        let bytes: u64 = parts.next().context("文件大小缺失")?.trim().parse()?;
        let path = parts.next().context("文件名缺失")?;
        let name = path
            .strip_prefix(&format!("{}/", canonical.trim_end_matches('/')))
            .context("文件不在当前目录")?;
        if name.contains('/') || name.is_empty() {
            bail!("无效的目录项");
        }
        entries.push(Entry {
            name: name.into(),
            path: format!("{}/{name}", logical.trim_end_matches('/')),
            bytes,
            directory: mode & 0xf000 == 0x4000,
            symlink: mode & 0xf000 == 0xa000,
        });
    }
    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    Ok(entries)
}

pub fn pull(adb: &Adb, device: &Device, remote: &str, destination: &Path) -> Result<()> {
    let canonical = verified_path(adb, device, remote)?;
    if destination.exists() {
        bail!("本地目标已存在，请选择新文件名以避免覆盖");
    }
    adb.transfer(
        &device.transport,
        "pull",
        Path::new(&canonical),
        destination,
    )
}

pub fn push(adb: &Adb, device: &Device, local: &Path, directory: &str) -> Result<()> {
    if !local.is_file() {
        bail!("请选择一个本地文件");
    }
    let canonical = verified_path(adb, device, directory)?;
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .context("文件名必须是 UTF-8")?;
    let target = format!("{canonical}/{name}");
    adb.script(
        &device.transport,
        &format!(
            "test ! -e {} && test ! -L {}",
            shell_quote(&target),
            shell_quote(&target)
        ),
    )
    .context("手机中已存在同名文件，请先重命名本地文件")?;
    adb.transfer(&device.transport, "push", local, Path::new(&target))
}

#[derive(Clone, Debug)]
pub struct Trashed {
    pub original: String,
    pub trashed: String,
    pub device_serial: String,
}

pub fn trash(adb: &Adb, device: &Device, remote: &str) -> Result<Trashed> {
    if remote.trim_end_matches('/') == "/sdcard" || remote.starts_with("/sdcard/.stepx-trash") {
        bail!("不能移动共享存储根目录或回收站");
    }
    let canonical = verified_path(adb, device, remote)?;
    if canonical == "/storage/emulated/0" {
        bail!("不能移动共享存储根目录");
    }
    let trash_root = "/sdcard/.stepx-trash";
    let q_remote = shell_quote(remote);
    let q_trash = shell_quote(trash_root);
    let root = adb
        .script(
            &device.transport,
            &format!(
                "test ! -L {q_remote} && test ! -L {q_trash} && mkdir -p {q_trash} && readlink -f {q_trash}"
            ),
        )
        .context("不移动符号链接，请选择实际文件")?
        .trim()
        .to_string();
    if root != "/storage/emulated/0" && !root.starts_with("/storage/emulated/0/") {
        bail!("该路径不属于手机共享存储（符号链接或存储用户不受支持）");
    }
    let name = Path::new(remote)
        .file_name()
        .and_then(|s| s.to_str())
        .context("文件名无效")?;
    let target = format!("{root}/{:016x}-{name}", rand::random::<u64>());
    let q_canonical = shell_quote(&canonical);
    let q_target = shell_quote(&target);
    adb.script(
        &device.transport,
        &format!(
            "mv -n -- {q_canonical} {q_target} && test ! -e {q_canonical} && test -e {q_target}"
        ),
    )?;
    Ok(Trashed {
        original: remote.into(),
        trashed: target.replacen("/storage/emulated/0", "/sdcard", 1),
        device_serial: device.serial.clone(),
    })
}

pub fn restore(adb: &Adb, device: &Device, record: &Trashed) -> Result<()> {
    if device.serial != record.device_serial {
        bail!("请连接被移入回收站文件所在的手机");
    }
    let source = verified_path(adb, device, &record.trashed)?;
    let parent = PathBuf::from(&record.original)
        .parent()
        .context("原目录无效")?
        .to_string_lossy()
        .to_string();
    let parent = resolve_storage_path(adb, device, &parent)?;
    let name = Path::new(&record.original)
        .file_name()
        .context("原文件名无效")?
        .to_string_lossy();
    let target = format!("{parent}/{name}");
    adb.script(
        &device.transport,
        &format!(
            "test ! -e {} && test ! -L {} && mv -n {} {} && test ! -e {}",
            shell_quote(&target),
            shell_quote(&target),
            shell_quote(&source),
            shell_quote(&target),
            shell_quote(&source)
        ),
    )
    .context("恢复失败：原位置可能已有同名文件")?;
    Ok(())
}

pub fn validate_entry_name(name: &str) -> Result<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        bail!("请输入有效的名称（不能是 . 或 ..）");
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.contains('\n')
        || trimmed.contains('\r')
    {
        bail!("名称不能包含路径分隔符或控制字符");
    }
    Ok(trimmed)
}

pub fn mkdir(adb: &Adb, device: &Device, directory: &str, name: &str) -> Result<String> {
    let name = validate_entry_name(name)?;
    let canonical_dir = verified_path(adb, device, directory)?;
    let target = format!("{canonical_dir}/{name}");
    adb.script(
        &device.transport,
        &format!(
            "test ! -e {t} && test ! -L {t} && mkdir -- {t}",
            t = shell_quote(&target),
        ),
    )
    .context("创建文件夹失败：当前目录下可能已存在同名文件或文件夹")?;
    Ok(format!("{}/{name}", directory.trim_end_matches('/')))
}

pub fn rename(adb: &Adb, device: &Device, remote: &str, new_name: &str) -> Result<String> {
    if remote.trim_end_matches('/') == "/sdcard" || remote.starts_with("/sdcard/.stepx-trash") {
        bail!("不能重命名共享存储根目录或回收站");
    }
    let new_name = validate_entry_name(new_name)?;
    let canonical = verified_path(adb, device, remote)?;
    if canonical == "/storage/emulated/0" {
        bail!("不能重命名共享存储根目录");
    }
    adb.shell(&device.transport, &["test", "!", "-L", remote])
        .context("不支持重命名符号链接")?;
    let parent_logical = PathBuf::from(remote)
        .parent()
        .context("原文件父目录无效")?
        .to_string_lossy()
        .to_string();
    let parent_canonical = resolve_storage_path(adb, device, &parent_logical)?;
    let target_canonical = format!("{parent_canonical}/{new_name}");
    if target_canonical == canonical {
        return Ok(remote.to_string());
    }
    adb.script(
        &device.transport,
        &format!(
            "test ! -e {dst} && test ! -L {dst} && mv -n -- {src} {dst} && test ! -e {src} && test -e {dst}",
            src = shell_quote(&canonical),
            dst = shell_quote(&target_canonical),
        ),
    )
    .context("重命名失败：目标名称可能已存在")?;
    Ok(format!(
        "{}/{new_name}",
        parent_logical.trim_end_matches('/')
    ))
}

/// Only called after explicit confirmation in the desktop UI.
pub fn delete(adb: &Adb, device: &Device, remote: &str) -> Result<()> {
    if remote.trim_end_matches('/') == "/sdcard" {
        bail!("不能删除共享存储根目录");
    }
    let canonical = verified_path(adb, device, remote)?;
    if canonical == "/storage/emulated/0" {
        bail!("不能删除共享存储根目录");
    }
    adb.script(
        &device.transport,
        &format!(
            "test ! -L {} && rm -rf -- {}",
            shell_quote(remote),
            shell_quote(&canonical)
        ),
    )
    .context("不删除符号链接，请选择实际文件")?;
    Ok(())
}

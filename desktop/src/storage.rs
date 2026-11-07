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

fn verified_path(adb: &Adb, device: &Device, path: &str) -> Result<(String, String)> {
    validate_path(path)?;
    let transport = device.resolve_active_transport(adb)?;
    let canonical = resolve_storage_path(adb, &transport, path)?;
    Ok((transport, canonical))
}

fn resolve_storage_path(adb: &Adb, transport: &str, path: &str) -> Result<String> {
    validate_path(path)?;
    let canonical = adb.shell(transport, &["readlink", "-f", path])?;
    let canonical = canonical.trim().to_string();
    if canonical != "/storage/emulated/0" && !canonical.starts_with("/storage/emulated/0/") {
        bail!("该路径不属于手机共享存储（符号链接或存储用户不受支持）");
    }
    Ok(canonical)
}

pub fn list(adb: &Adb, device: &Device, path: &str) -> Result<Vec<Entry>> {
    let (transport, canonical) = verified_path(adb, device, path)?;
    let script = format!("find {} -mindepth 1 -maxdepth 1 -exec sh -c 'for f do stat -c \"%f\n%s\" \"$f\" && printf \"%s\\0\" \"$f\"; done' sh {{}} +", shell_quote(&canonical));
    let text = adb.script(&transport, &script)?;
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
    let (transport, canonical) = verified_path(adb, device, remote)?;
    if destination.exists() {
        bail!("本地目标已存在，请选择新文件名以避免覆盖");
    }
    adb.transfer(
        &transport,
        "pull",
        Path::new(&canonical),
        destination,
    )
}

pub fn push(adb: &Adb, device: &Device, local: &Path, directory: &str) -> Result<()> {
    if !local.is_file() {
        bail!("请选择一个本地文件");
    }
    let (transport, canonical) = verified_path(adb, device, directory)?;
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .context("文件名必须是 UTF-8")?;
    let target = format!("{canonical}/{name}");
    adb.script(
        &transport,
        &format!(
            "test ! -e {} && test ! -L {}",
            shell_quote(&target),
            shell_quote(&target)
        ),
    )
    .context("手机中已存在同名文件，请先重命名本地文件")?;
    adb.transfer(&transport, "push", local, Path::new(&target))
}

#[derive(Clone, Debug)]
pub struct Trashed {
    pub original: String,
    pub trashed: String,
    pub device_serial: String,
    pub native_mcp: bool,
}

pub fn trash(adb: &Adb, device: &Device, remote: &str) -> Result<Trashed> {
    if remote.trim_end_matches('/') == "/sdcard" || remote.starts_with("/sdcard/.stepx-trash") {
        bail!("不能移动共享存储根目录或回收站");
    }
    let (transport, canonical) = verified_path(adb, device, remote)?;
    if canonical == "/storage/emulated/0" {
        bail!("不能移动共享存储根目录");
    }
    adb.shell(&transport, &["test", "!", "-L", remote])
        .context("不移动符号链接，请选择实际文件")?;
    // Recycle in the phone's own file/gallery trash instead of hiding the file
    // in an app-specific directory that cannot be restored from the phone UI.
    if crate::native_mcp::try_call(adb, device, "file_safe_delete",
        serde_json::json!({"path":canonical,"recursive":true}))?.is_some() {
        adb.script(&transport, &format!("test ! -e {} && test ! -L {}",shell_quote(&canonical),shell_quote(&canonical)))
            .context("手机已收到回收请求，但原路径仍存在；未重复删除，请在手机回收站核对")?;
        return Ok(Trashed {original:remote.to_owned(),trashed:String::new(),device_serial:device.serial.clone(),native_mcp:true});
    }
    let trash_root = "/sdcard/.stepx-trash";
    let q_remote = shell_quote(remote);
    let q_trash = shell_quote(trash_root);
    let root = adb
        .script(
            &transport,
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
        &transport,
        &format!(
            "mv -n -- {q_canonical} {q_target} && test ! -e {q_canonical} && test -e {q_target}"
        ),
    )?;
    Ok(Trashed {
        original: remote.into(),
        trashed: target.replacen("/storage/emulated/0", "/sdcard", 1),
        device_serial: device.serial.clone(),
        native_mcp: false,
    })
}

pub fn restore(adb: &Adb, device: &Device, record: &Trashed) -> Result<()> {
    if device.serial != record.device_serial {
        bail!("请连接被移入回收站文件所在的手机");
    }
    if record.native_mcp {
        let transport=device.resolve_active_transport(adb)?;
        let parent=Path::new(&record.original).parent().context("原目录无效")?;
        let parent=resolve_storage_path(adb,&transport,&parent.to_string_lossy())?;
        let name=Path::new(&record.original).file_name().context("原文件名无效")?.to_string_lossy();
        let target=format!("{parent}/{name}");
        adb.script(&transport,&format!("test ! -e {} && test ! -L {}",shell_quote(&target),shell_quote(&target)))
            .context("原位置已有同名文件，未恢复或覆盖；请先保留该文件")?;
        let receipt=crate::native_mcp::try_call(adb,device,"file_restore",serde_json::json!({"path":target,"recursive":true}))?;
        anyhow::ensure!(receipt.is_some(),"当前手机未提供原生恢复能力，请在手机文件管理/图库回收站恢复");
        adb.script(&transport,&format!("test -e {} && test ! -L {}",shell_quote(&target),shell_quote(&target)))
            .context("手机已收到恢复请求，但原路径尚未出现；未重复提交，请在手机回收站核对")?;
        return Ok(());
    }
    let (transport, source) = verified_path(adb, device, &record.trashed)?;
    let parent = PathBuf::from(&record.original)
        .parent()
        .context("原目录无效")?
        .to_string_lossy()
        .to_string();
    let parent = resolve_storage_path(adb, &transport, &parent)?;
    let name = Path::new(&record.original)
        .file_name()
        .context("原文件名无效")?
        .to_string_lossy();
    let target = format!("{parent}/{name}");
    adb.script(
        &transport,
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
    let (transport, canonical_dir) = verified_path(adb, device, directory)?;
    let target = format!("{canonical_dir}/{name}");
    // Firmware create_folder is explicitly root-only. Nested directories keep
    // the checked storage API; passing a path as folderName would misplace files.
    if canonical_dir == "/storage/emulated/0" {
        if crate::native_mcp::try_call(adb, device, "filemanager_create_folder", serde_json::json!({"folderName":name}))?.is_some() {
            adb.shell(&transport, &["test", "-d", &target]).context("原生建文件夹已提交，但目标目录未读回；未重试")?;
            return Ok(format!("{}/{name}",directory.trim_end_matches('/')));
        }
    }
    adb.script(
        &transport,
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
    let (transport, canonical) = verified_path(adb, device, remote)?;
    if canonical == "/storage/emulated/0" {
        bail!("不能重命名共享存储根目录");
    }
    adb.shell(&transport, &["test", "!", "-L", remote])
        .context("不支持重命名符号链接")?;
    let parent_logical = PathBuf::from(remote)
        .parent()
        .context("原文件父目录无效")?
        .to_string_lossy()
        .to_string();
    let parent_canonical = resolve_storage_path(adb, &transport, &parent_logical)?;
    let target_canonical = format!("{parent_canonical}/{new_name}");
    if target_canonical == canonical {
        return Ok(remote.to_string());
    }
    let is_directory = adb.shell(&transport, &["test", "-d", &canonical]).is_ok();
    // File MCP preserves the original suffix when no extension is supplied;
    // that differs from an explicit extension-removing rename.
    if is_directory || std::path::Path::new(new_name).extension().is_some() {
        let method = if is_directory {"filemanager_edit_folder_name"} else {"filemanager_edit_file_name"};
        if crate::native_mcp::try_call(adb, device, method, serde_json::json!({"file_uri":canonical,"newName":new_name}))?.is_some() {
            adb.shell(&transport, &["test", "-e", &target_canonical]).context("原生重命名目标尚未读回；未重试")?;
            adb.shell(&transport, &["test", "!", "-e", &canonical]).context("原生重命名后源路径仍存在；未确认完成")?;
            return Ok(format!("{}/{new_name}", parent_logical.trim_end_matches('/')));
        }
    }
    adb.script(
        &transport,
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
    let (transport, canonical) = verified_path(adb, device, remote)?;
    if canonical == "/storage/emulated/0" {
        bail!("不能删除共享存储根目录");
    }
    adb.script(
        &transport,
        &format!(
            "test ! -L {} && rm -rf -- {}",
            shell_quote(remote),
            shell_quote(&canonical)
        ),
    )
    .context("不删除符号链接，请选择实际文件")?;
    Ok(())
}

#[cfg(all(test,unix))]
mod native_recycle_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejected_recycle_never_falls_back_to_mv_or_permanent_deletion() {
        let schemas:serde_json::Value=serde_json::from_str(include_str!("../tests/fixtures/native-mcp-contracts.json")).unwrap();
        let fixture=crate::native_mcp::test_support::Fixture::new(vec![json!({"name":"file_safe_delete","callingMethod":"LOCAL","callType":"MCP","inputSchema":schemas["schemas"]["file_safe_delete"]})],Some(json!({"ok":false,"error":"rejected"})));
        let adb=fixture.adb("case \"$*\" in *readlink*) echo /storage/emulated/0/Download/owned.txt;; *test*) exit 0;; *) exit 19;; esac");
        assert!(trash(&adb,&fixture.device(),"/sdcard/Download/owned.txt").is_err());
        assert_eq!(fixture.calls.lock().unwrap().len(),1);
        let calls=std::fs::read_to_string(fixture.path().join("calls")).unwrap();
        assert!(!calls.contains("mv ")&&!calls.contains("rm "));
    }
    #[test]
    fn restore_collision_is_rejected_before_native_call() {
        let fixture=crate::native_mcp::test_support::Fixture::new(vec![],None);
        let adb=fixture.adb("case \"$*\" in *readlink*) echo /storage/emulated/0/Download;; *test*) exit 1;; *) exit 19;; esac");
        let record=Trashed {original:"/sdcard/Download/owned.txt".into(),trashed:String::new(),device_serial:fixture.device().serial,native_mcp:true};
        let error=restore(&adb,&fixture.device(),&record).unwrap_err();
        assert!(error.to_string().contains("同名文件"));
        assert!(fixture.calls.lock().unwrap().is_empty());
    }
}

//! Small private snapshots. Large histories belong in indexed stores, not here.
use anyhow::{ensure, Context, Result};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "数据文件必须是普通文件，不能是链接或设备"
    );
    ensure!(meta.len() <= limit, "数据文件超过读取上限，原文件已保留");
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A file may be replaced after lstat. Never follow a new link or wait
        // indefinitely while opening a FIFO that took a regular file's place.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    ensure!(opened.is_file(), "数据文件必须是普通文件，不能是链接或设备");
    ensure!(opened.len() <= limit, "数据文件超过读取上限，原文件已保留");
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "数据文件超过读取上限，原文件已保留"
    );
    Ok(bytes)
}

/// Atomic replacement, unique temporary file, durable contents and directory entry.
/// Returns false for an unchanged snapshot; does not rewrite it on each UI tick.
pub fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "数据文件不能是链接或目录"
        );
        if meta.len() == bytes.len() as u64 && read_bounded(path, meta.len())? == bytes {
            return Ok(false);
        }
    }
    let parent = path.parent().context("数据目录无效")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_snapshots_skip_identical_writes_and_bound_reads() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        assert!(write_if_changed(&p, b"first").unwrap());
        let at = fs::metadata(&p).unwrap().modified().unwrap();
        assert!(!write_if_changed(&p, b"first").unwrap());
        assert_eq!(at, fs::metadata(&p).unwrap().modified().unwrap());
        assert!(read_bounded(&p, 4).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"first");
        assert!(write_if_changed(&p, b"second").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn concurrent_replacements_are_whole_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        std::thread::scope(|s| {
            for byte in 0..8u8 {
                let p = &p;
                s.spawn(move || {
                    for _ in 0..12 {
                        write_if_changed(p, &vec![byte; 8192]).unwrap();
                    }
                });
            }
        });
        let saved = fs::read(p).unwrap();
        assert_eq!(saved.len(), 8192);
        assert!(saved.iter().all(|b| *b == saved[0]));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn reads_reject_links_and_fifos_without_waiting_for_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, "source").unwrap();
        let link = dir.path().join("linked.txt");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read_bounded(&link, 100).is_err());
        let fifo = dir.path().join("pipe.txt");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(read_bounded(&fifo, 100).is_err());
        assert_eq!(read_bounded(&file, u64::MAX).unwrap(), b"source");
    }
}

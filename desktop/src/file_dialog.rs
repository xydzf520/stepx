//! File selectors must never create an AppKit panel in the client process.
//! UI callers use the asynchronous adapter in `ui::file_dialog_state`; the
//! synchronous methods here are for worker threads only.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DialogKind {
    OpenFile,
    OpenFiles,
    OpenFolder,
    SaveFile,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FileDialog {
    title: String,
    filename: String,
    location: Option<PathBuf>,
    filters: Vec<(String, Vec<String>)>,
}

pub const MACOS_SCRIPT: &str = include_str!("file_dialog_macos.js");

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_title(mut self, title: &str) -> Self {
        self.title = title.into();
        self
    }
    pub fn set_filename(mut self, name: &str) -> Self {
        self.filename = name.into();
        self
    }
    pub fn set_location(mut self, path: impl AsRef<Path>) -> Self {
        self.location = Some(path.as_ref().to_path_buf());
        self
    }
    pub fn add_filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.filters.push((
            name.into(),
            extensions.iter().map(|s| (*s).into()).collect(),
        ));
        self
    }
    pub fn show_open_single_file(self) -> Result<Option<PathBuf>> {
        self.show(DialogKind::OpenFile)
            .map(|v| v.into_iter().next())
    }
    pub fn show_open_multiple_file(self) -> Result<Vec<PathBuf>> {
        self.show(DialogKind::OpenFiles)
    }
    pub fn show_open_single_dir(self) -> Result<Option<PathBuf>> {
        self.show(DialogKind::OpenFolder)
            .map(|v| v.into_iter().next())
    }
    pub fn show_save_single_file(self) -> Result<Option<PathBuf>> {
        self.show(DialogKind::SaveFile)
            .map(|v| v.into_iter().next())
    }

    pub fn show(self, kind: DialogKind) -> Result<Vec<PathBuf>> {
        // Background operations and UI requests share this gate. Never open
        // several independent native windows after repeated clicks.
        static ACTIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ACTIVE
            .try_lock()
            .map_err(|_| anyhow::anyhow!("文件选择窗口已打开，请先选择文件或取消"))?;
        #[cfg(target_os = "macos")]
        {
            let output = self
                .macos_command(kind)?
                .output()
                .context("无法打开 macOS 文件选择器（/usr/bin/osascript）")?;
            parse_macos_result(
                kind,
                output.status.success(),
                &output.stdout,
                &output.stderr,
            )
        }
        #[cfg(all(not(target_os = "macos"), feature = "desktop-ui"))]
        {
            let mut dialog = native_dialog::FileDialog::new();
            if !self.title.is_empty() {
                dialog = dialog.set_title(&self.title);
            }
            if !self.filename.is_empty() {
                dialog = dialog.set_filename(&self.filename);
            }
            if let Some(location) = &self.location {
                dialog = dialog.set_location(location);
            }
            let filters: Vec<Vec<&str>> = self
                .filters
                .iter()
                .map(|(_, exts)| exts.iter().map(String::as_str).collect())
                .collect();
            for ((name, _), exts) in self.filters.iter().zip(&filters) {
                dialog = dialog.add_filter(name, exts);
            }
            match kind {
                DialogKind::OpenFile => dialog
                    .show_open_single_file()
                    .map(|v| v.into_iter().collect()),
                DialogKind::OpenFiles => dialog.show_open_multiple_file(),
                DialogKind::OpenFolder => dialog
                    .show_open_single_dir()
                    .map(|v| v.into_iter().collect()),
                DialogKind::SaveFile => dialog
                    .show_save_single_file()
                    .map(|v| v.into_iter().collect()),
            }
            .context("无法打开文件选择器")
        }
        #[cfg(all(not(target_os = "macos"), not(feature = "desktop-ui")))]
        {
            let _ = kind;
            bail!("此构建未包含桌面文件选择器")
        }
    }

    /// The script is constant. Prompts, filenames and paths are JSON data in a
    /// separate argv entry, never AppleScript, JavaScript or shell source.
    pub fn macos_command(&self, kind: DialogKind) -> Result<std::process::Command> {
        let request = serde_json::json!({
            "kind": kind,
            "title": self.title,
            "filename": self.filename,
            "location": self.location.as_ref().filter(|p| p.is_dir()),
            "extensions": self.filters.iter().flat_map(|(_, exts)| exts).collect::<Vec<_>>(),
        });
        let mut command = std::process::Command::new("/usr/bin/osascript");
        command
            .args(["-l", "JavaScript", "-e", MACOS_SCRIPT, "--"])
            .arg(serde_json::to_string(&request)?);
        Ok(command)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MacResponse {
    status: String,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    message: String,
}

pub fn parse_macos_result(
    kind: DialogKind,
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Vec<PathBuf>> {
    if !success {
        let diagnostic = String::from_utf8_lossy(stderr);
        bail!(
            "macOS 文件选择器异常退出：{}",
            diagnostic.trim().chars().take(600).collect::<String>()
        );
    }
    let response: MacResponse =
        serde_json::from_slice(stdout).context("macOS 文件选择器返回了无效结果，请重试")?;
    match response.status.as_str() {
        "cancelled" => {
            anyhow::ensure!(response.paths.is_empty(), "取消选择时返回了异常文件路径");
            Ok(Vec::new())
        }
        "error" => bail!("macOS 文件选择失败：{}", response.message),
        "selected" => {
            anyhow::ensure!(!response.paths.is_empty(), "文件选择结果为空，请重试");
            anyhow::ensure!(
                kind == DialogKind::OpenFiles || response.paths.len() == 1,
                "文件选择器返回了多余路径"
            );
            response
                .paths
                .into_iter()
                .map(|path| {
                    anyhow::ensure!(
                        path.starts_with('/') && !path.contains('\0'),
                        "文件选择器返回了无效路径"
                    );
                    Ok(PathBuf::from(path))
                })
                .collect()
        }
        _ => bail!("macOS 文件选择器返回了未知状态"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_dialog_arguments_are_data_not_script() {
        let hostile = "中文 \"; do shell script \"touch /tmp/never\"\n$(echo no)`no`\\";
        let command = FileDialog::new()
            .set_title(hostile)
            .set_filename(hostile)
            .macos_command(DialogKind::SaveFile)
            .unwrap();
        assert_eq!(command.get_program(), "/usr/bin/osascript");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args.len(), 6);
        assert_eq!(args[3], MACOS_SCRIPT);
        let request: serde_json::Value = serde_json::from_str(args[5].to_str().unwrap()).unwrap();
        assert_eq!(request["title"], hostile);
        assert_eq!(request["filename"], hostile);
    }
    #[test]
    fn macos_dialog_preserves_multiple_unicode_and_newline_paths() {
        let paths = vec!["/tmp/空 格\n换行.pdf", "/tmp/a\"b\\c.png"];
        let bytes =
            serde_json::to_vec(&serde_json::json!({"status":"selected","paths":paths})).unwrap();
        assert_eq!(
            parse_macos_result(DialogKind::OpenFiles, true, &bytes, b"").unwrap(),
            paths.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
        assert!(parse_macos_result(DialogKind::OpenFile, true, &bytes, b"").is_err());
    }
    #[test]
    fn macos_dialog_cancel_is_not_error_or_selection() {
        assert!(parse_macos_result(
            DialogKind::SaveFile,
            true,
            br#"{"status":"cancelled"}"#,
            b""
        )
        .unwrap()
        .is_empty());
        assert!(parse_macos_result(
            DialogKind::OpenFile,
            true,
            br#"{"status":"cancelled","paths":["/tmp/a"]}"#,
            b""
        )
        .is_err());
    }
    #[test]
    fn macos_dialog_errors_and_malformed_results_are_visible() {
        for bad in [
            b"not json".as_slice(),
            br#"{"status":"selected","paths":["relative"]}"#,
            br#"{"status":"selected","paths":[]}"#,
            br#"{"status":"mystery"}"#,
        ] {
            assert!(parse_macos_result(DialogKind::OpenFile, true, bad, b"").is_err());
        }
        assert!(
            parse_macos_result(DialogKind::OpenFile, false, b"", b"permission denied")
                .unwrap_err()
                .to_string()
                .contains("permission denied")
        );
        assert!(parse_macos_result(
            DialogKind::OpenFile,
            true,
            br#"{"status":"error","message":"not authorized"}"#,
            b""
        )
        .unwrap_err()
        .to_string()
        .contains("not authorized"));
    }
}

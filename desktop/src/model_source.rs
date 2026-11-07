//! One explicit billing/model origin shared by every desktop entry point.
//! This file contains no credentials and never migrates or edits API settings.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    PhoneAccount,
    IndependentApi,
}
impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::PhoneAccount => "手机账号",
            Self::IndependentApi => "独立 API",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub source: Source,
}
fn path() -> PathBuf {
    crate::resources::user_config().join("stepx-companion/model-source.json")
}
fn load_at(path: &Path) -> Result<Preferences> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Preferences::default())
        }
        Err(error) => return Err(error).context("无法读取模型来源设置"),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "模型来源设置必须是本机普通文件"
    );
    ensure!(metadata.len() <= 8192, "模型来源设置过大，请重新选择来源");
    let mut bytes = vec![];
    fs::File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 8192, "模型来源设置过大，请重新选择来源");
    serde_json::from_slice(&bytes).context("模型来源设置无效，请重新选择手机账号或独立 API")
}
fn save_at(path: &Path, source: Source) -> Result<Preferences> {
    let parent = path.parent().context("模型来源设置目录无效")?;
    fs::create_dir_all(parent)?;
    ensure!(
        !fs::symlink_metadata(parent)?.file_type().is_symlink(),
        "模型来源设置目录不能是符号链接"
    );
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "模型来源设置不能是符号链接或目录"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let preferences = Preferences { source };
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(&serde_json::to_vec_pretty(&preferences)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .context("无法保存模型来源设置")?;
    Ok(preferences)
}
pub fn load() -> Result<Preferences> {
    load_at(&path())
}
pub fn save(source: Source) -> Result<Preferences> {
    save_at(&path(), source)
}
pub fn prefers_phone() -> Result<bool> {
    Ok(load()?.source == Source::PhoneAccount)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_phone_and_explicit_switch_never_edit_saved_api_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model-source.json");
        let api = directory.path().join("model.json");
        let original = br#"{"key":"private-test-sentinel","model":"saved-model"}"#;
        fs::write(&api, original).unwrap();
        assert_eq!(load_at(&path).unwrap().source, Source::PhoneAccount);
        save_at(&path, Source::IndependentApi).unwrap();
        assert_eq!(load_at(&path).unwrap().source, Source::IndependentApi);
        save_at(&path, Source::PhoneAccount).unwrap();
        assert_eq!(fs::read(&api).unwrap(), original);
        let body = fs::read_to_string(&path).unwrap();
        assert!(!body.contains("key") && !body.contains("private-test-sentinel"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn invalid_sources_are_errors_and_cannot_silently_select_api() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model-source.json");
        for body in [
            "broken",
            r#"{"source":"unknown"}"#,
            r#"{"source":"phone_account","key":"not-allowed"}"#,
        ] {
            fs::write(&path, body).unwrap();
            assert!(load_at(&path).is_err());
        }
        fs::write(&path, vec![b' '; 8193]).unwrap();
        assert!(load_at(&path).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn source_preferences_do_not_follow_a_replacement_symlink() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let external = directory.path().join("existing.json");
        fs::write(&external, r#"{"source":"independent_api"}"#).unwrap();
        let link = directory.path().join("model-source.json");
        symlink(&external, &link).unwrap();
        assert!(load_at(&link).is_err());
        assert!(save_at(&link, Source::PhoneAccount).is_err());
        assert!(fs::read_to_string(external)
            .unwrap()
            .contains("independent_api"));
    }
}

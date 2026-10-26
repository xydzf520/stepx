//! Explicit, read-only desktop release checks. An absent channel is never “up to date”.
use crate::{
    adb::{run_process, Adb},
    build_info,
    device::Device,
    pairing::Preferences,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::BTreeMap, path::PathBuf, time::Duration};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateConfig {
    pub manifest_url: String,
}
impl UpdateConfig {
    pub fn path() -> PathBuf {
        Preferences::path().with_file_name("updates.json")
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = std::fs::read(path).context("无法读取更新源设置")?;
        ensure!(bytes.len() <= 8192, "更新源设置过大");
        serde_json::from_slice(&bytes).context("更新源设置格式无效，请重新保存地址")
    }
    pub fn save(&self) -> Result<()> {
        if !self.manifest_url.is_empty() {
            validate_https(&self.manifest_url)?;
        }
        let path = Self::path();
        std::fs::create_dir_all(path.parent().context("缺少设置目录")?)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub notes: String,
    /// Allows releases with the same semver to distinguish a newer compiled build.
    #[serde(default)]
    pub build_unix_seconds: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    product: String,
    releases: BTreeMap<String, Release>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Unconfigured,
    Available,
    UpToDate,
    Ahead,
}
#[derive(Clone, Debug)]
pub struct UpdateReport {
    pub status: UpdateStatus,
    pub checked_at: String,
    pub release: Option<Release>,
}
impl UpdateReport {
    pub fn title(&self) -> &'static str {
        match self.status {
            UpdateStatus::Unconfigured => "尚未设置桌面更新源",
            UpdateStatus::Available => "有新版本可用",
            UpdateStatus::UpToDate => "已是此更新源的最新版本",
            UpdateStatus::Ahead => "当前版本高于更新源版本",
        }
    }
}
pub fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
fn checked_at() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Require unambiguous HTTPS URLs. Fetcher performs a second standards-based URL validation.
pub fn validate_https(url: &str) -> Result<()> {
    ensure!(
        url.len() <= 4096
            && !url.chars().any(|c| c.is_control() || c.is_whitespace())
            && !url.contains('\\'),
        "更新链接格式无效"
    );
    let rest = url
        .strip_prefix("https://")
        .context("更新源和下载页必须使用 HTTPS 地址")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    ensure!(
        !host.is_empty() && !host.contains('@') && !host.starts_with('-'),
        "更新链接缺少有效主机名，或包含账号信息"
    );
    ensure!(!url.contains('#'), "更新链接不能包含片段标记");
    Ok(())
}

pub fn check(config: &UpdateConfig) -> Result<UpdateReport> {
    check_with(
        config,
        build_info::VERSION,
        build_info::COMPILED_UNIX_SECONDS.parse().unwrap_or(0),
        &target(),
        fetch,
    )
}
fn check_with(
    config: &UpdateConfig,
    current: &str,
    build: u64,
    target: &str,
    fetcher: impl FnOnce(&str) -> Result<Vec<u8>>,
) -> Result<UpdateReport> {
    if config.manifest_url.is_empty() {
        return Ok(UpdateReport {
            status: UpdateStatus::Unconfigured,
            checked_at: checked_at(),
            release: None,
        });
    }
    validate_https(&config.manifest_url)?;
    let bytes = fetcher(&config.manifest_url)?;
    ensure!(bytes.len() <= 512 * 1024, "更新清单过大");
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("更新源没有返回有效的 StepX 更新清单")?;
    ensure!(
        manifest.schema == 1 && manifest.product == "stepx-companion",
        "此更新清单不适用于 StepX Desktop"
    );
    let release = manifest
        .releases
        .get(target)
        .context("更新源尚未提供适用于当前系统的版本")?
        .clone();
    validate_https(&release.url)?;
    ensure!(release.notes.len() <= 32 * 1024, "更新说明过长");
    let comparison = compare_versions(&release.version, current)?;
    let status = match comparison {
        Ordering::Greater => UpdateStatus::Available,
        Ordering::Less => UpdateStatus::Ahead,
        Ordering::Equal => match release.build_unix_seconds {
            Some(remote) if remote > build => UpdateStatus::Available,
            Some(remote) if remote < build => UpdateStatus::Ahead,
            _ => UpdateStatus::UpToDate,
        },
    };
    Ok(UpdateReport {
        status,
        checked_at: checked_at(),
        release: Some(release),
    })
}
fn fetch(url: &str) -> Result<Vec<u8>> {
    let mut cmd = crate::resources::python_command();
    cmd.args(["-c", include_str!("updates_fetch.py")]);
    let output = run_process(
        cmd,
        Some(serde_json::to_vec(&serde_json::json!({"url":url}))?),
        Duration::from_secs(22),
    )
    .context("检查更新超时或网络工具无法启动，请稍后重试")?;
    if !output.status.success() {
        // The helper emits only fixed error labels; never echo credentials or response bodies.
        let code = String::from_utf8_lossy(&output.stderr);
        let message = match code.trim() {
            "HTTP404" => "更新源不存在或尚未发布清单（404）",
            "HTTP403" | "HTTP401" => "更新源需要访问权限",
            "TOO_LARGE" => "更新清单超过大小限制",
            "INVALID_URL" => "更新源或重定向地址不是有效的 HTTPS 链接",
            "INVALID_CONTENT" => "更新源返回的内容类型不受支持",
            _ => "无法连接更新源，请检查网络和地址后重试",
        };
        bail!(message);
    }
    Ok(output.stdout)
}

#[derive(Debug, PartialEq, Eq)]
struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}
fn version(input: &str) -> Result<Version> {
    ensure!(input.len() <= 100, "更新版本号无效");
    let input = input.strip_prefix('v').unwrap_or(input);
    let (base, metadata) = input
        .split_once('+')
        .map_or((input, None), |(base, metadata)| (base, Some(metadata)));
    if let Some(metadata) = metadata {
        ensure!(
            metadata.split('.').all(|item| !item.is_empty()
                && item.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')),
            "构建版本号无效"
        );
    }
    let (core, pre) = base
        .split_once('-')
        .map_or((base, None), |(c, p)| (c, Some(p)));
    let mut parsed = [0; 3];
    let pieces = core.split('.').collect::<Vec<_>>();
    ensure!(pieces.len() == 3, "更新版本号须为 major.minor.patch 格式");
    for (i, piece) in pieces.into_iter().enumerate() {
        ensure!(
            !piece.is_empty()
                && piece.bytes().all(|c| c.is_ascii_digit())
                && (piece.len() == 1 || !piece.starts_with('0')),
            "更新版本号无效"
        );
        parsed[i] = piece.parse().context("更新版本号超出范围")?;
    }
    let mut prerelease = Vec::new();
    if let Some(pre) = pre {
        for item in pre.split('.') {
            ensure!(
                !item.is_empty() && item.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                "预发布版本号无效"
            );
            ensure!(
                !item.bytes().all(|c| c.is_ascii_digit())
                    || item.len() == 1
                    || !item.starts_with('0'),
                "预发布版本号无效"
            );
            prerelease.push(item.to_owned());
        }
    }
    Ok(Version {
        core: parsed,
        pre: prerelease,
    })
}
pub fn compare_versions(a: &str, b: &str) -> Result<Ordering> {
    let (a, b) = (version(a)?, version(b)?);
    let core = a.core.cmp(&b.core);
    if core != Ordering::Equal {
        return Ok(core);
    }
    if a.pre.is_empty() || b.pre.is_empty() {
        return Ok(b.pre.is_empty().cmp(&a.pre.is_empty()).reverse());
    }
    for (a, b) in a.pre.iter().zip(&b.pre) {
        let (an, bn) = (
            a.bytes().all(|c| c.is_ascii_digit()),
            b.bytes().all(|c| c.is_ascii_digit()),
        );
        let order = match (an, bn) {
            (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.cmp(b),
        };
        if order != Ordering::Equal {
            return Ok(order);
        }
    }
    Ok(a.pre.len().cmp(&b.pre.len()))
}

#[derive(Clone, Debug)]
pub struct PhoneVersion {
    pub serial: String,
    pub firmware: String,
    pub security_patch: String,
    pub has_update_page: bool,
}
pub fn phone_version(adb: &Adb, device: &Device) -> Result<PhoneVersion> {
    device.verify(adb)?;
    let raw = adb.script(
        &device.transport,
        "v=$(getprop ro.build.aios_version); if [ -z \"$v\" ]; then v=$(getprop ro.build.display.id); fi; printf '%s\\n' \"$v\"; getprop ro.build.version.security_patch; cmd package resolve-activity --brief -a stepos.settings.SYSTEM_UPDATE_SETTINGS 2>/dev/null || true",
    )?;
    let mut lines = raw.lines();
    let firmware = lines.next().unwrap_or("").trim().to_owned();
    let security_patch = lines.next().unwrap_or("").trim().to_owned();
    Ok(PhoneVersion {
        serial: device.serial.clone(),
        firmware,
        security_patch,
        has_update_page: lines.any(|line| line.trim().starts_with("com.stepos.update/")),
    })
}
pub fn open_phone_update(adb: &Adb, device: &Device) -> Result<()> {
    device.verify(adb)?;
    let result = adb.shell(
        &device.transport,
        &[
            "am",
            "start",
            "-a",
            "stepos.settings.SYSTEM_UPDATE_SETTINGS",
            "-p",
            "com.stepos.update",
        ],
    )?;
    ensure!(
        !result.contains("Error")
            && !result.contains("Exception")
            && !result.contains("unable to resolve"),
        "手机没有可打开的系统更新页面"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> UpdateConfig {
        UpdateConfig {
            manifest_url: "https://updates.example.com/stable.json".into(),
        }
    }
    fn manifest(version: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"schema":1,"product":"stepx-companion","releases":{"linux-x86_64":{"version":version,"url":"https://updates.example.com/download","notes":"修复与改进"}}})).unwrap()
    }
    #[test]
    fn semver_orders_numbers_and_prereleases() {
        for (a, b) in [
            ("0.10.0", "0.9.0"),
            ("1.0.0", "1.0.0-rc.1"),
            ("1.0.0-rc.10", "1.0.0-rc.2"),
            ("1.0.0-beta", "1.0.0-2"),
            ("1.0.0-rc.1.1", "1.0.0-rc.1"),
        ] {
            assert_eq!(
                compare_versions(a, b).unwrap(),
                Ordering::Greater,
                "{a} vs {b}"
            );
            assert_eq!(compare_versions(b, a).unwrap(), Ordering::Less);
        }
        assert_eq!(
            compare_versions("v1.0.0+abc", "1.0.0").unwrap(),
            Ordering::Equal
        );
        for bad in [
            "1.0",
            "1.0.01",
            "x.y.z",
            "1.0.0-",
            "1.0.0-01",
            "1.0.0+",
            "1.0.0+a+b",
        ] {
            assert!(compare_versions(bad, "1.0.0").is_err());
        }
    }
    #[test]
    fn no_source_is_not_latest_and_does_not_fetch() {
        let report = check_with(&UpdateConfig::default(), "0.1.0", 1, "linux-x86_64", |_| {
            panic!("must not fetch")
        })
        .unwrap();
        assert_eq!(report.status, UpdateStatus::Unconfigured);
    }
    #[test]
    fn checks_release_and_platform() {
        assert_eq!(
            check_with(&config(), "0.1.0", 1, "linux-x86_64", |_| Ok(manifest(
                "0.2.0"
            )))
            .unwrap()
            .status,
            UpdateStatus::Available
        );
        assert_eq!(
            check_with(&config(), "0.1.0", 1, "linux-x86_64", |_| Ok(manifest(
                "0.1.0"
            )))
            .unwrap()
            .status,
            UpdateStatus::UpToDate
        );
        assert_eq!(
            check_with(&config(), "0.2.0", 1, "linux-x86_64", |_| Ok(manifest(
                "0.1.0"
            )))
            .unwrap()
            .status,
            UpdateStatus::Ahead
        );
        assert!(
            check_with(&config(), "0.1.0", 1, "windows-x86_64", |_| Ok(manifest(
                "0.2.0"
            )))
            .is_err()
        );
    }
    #[test]
    fn rejects_failures_wrong_product_and_insecure_links() {
        assert!(check_with(&config(), "0.1.0", 1, "linux-x86_64", |_| bail!("network")).is_err());
        assert!(check_with(
            &config(),
            "0.1.0",
            1,
            "linux-x86_64",
            |_| Ok(b"[]".to_vec())
        )
        .is_err());
        let bad = String::from_utf8(manifest("0.2.0"))
            .unwrap()
            .replace("stepx-companion", "other-product")
            .into_bytes();
        assert!(check_with(&config(), "0.1.0", 1, "linux-x86_64", |_| Ok(bad)).is_err());
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https:///foo",
            "https://example.com/\nx",
            "https://example.com/#token",
        ] {
            assert!(validate_https(url).is_err(), "{url}");
        }
    }
    #[test]
    fn newer_build_same_version_is_update() {
        let mut data: serde_json::Value = serde_json::from_slice(&manifest("0.1.0")).unwrap();
        data["releases"]["linux-x86_64"]["build_unix_seconds"] = 20.into();
        assert_eq!(
            check_with(&config(), "0.1.0", 10, "linux-x86_64", |_| Ok(
                serde_json::to_vec(&data).unwrap()
            ))
            .unwrap()
            .status,
            UpdateStatus::Available
        );
    }
}

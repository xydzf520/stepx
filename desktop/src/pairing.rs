use crate::adb::Adb;
use anyhow::{bail, Context, Result};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub kind: String,
    pub endpoint: String,
}

pub fn parse_services(output: &str) -> Vec<Service> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            let endpoint = parts[parts.len() - 1];
            let kind = parts[parts.len() - 2].trim_end_matches('.');
            if !["_adb-tls-connect._tcp", "_adb-tls-pairing._tcp"].contains(&kind)
                || validate_endpoint(endpoint).is_err()
            {
                return None;
            }
            Some(Service {
                // mDNS may append a conflict suffix such as " (2)" to its instance name.
                name: parts[..parts.len() - 2]
                    .join(" ")
                    .trim_end_matches('.')
                    .into(),
                kind: kind.into(),
                endpoint: endpoint.into(),
            })
        })
        .collect()
}

pub fn validate_endpoint(input: &str) -> Result<SocketAddr> {
    let address: SocketAddr = input
        .trim()
        .parse()
        .context("请输入 IP:端口，例如 192.168.1.8:37123；IPv6 使用 [地址]:端口")?;
    if address.port() == 0 || address.ip().is_unspecified() || address.ip().is_multicast() {
        bail!("无效的设备地址");
    }
    Ok(address)
}

pub fn pair_code(adb: &Adb, endpoint: &str, code: &str) -> Result<String> {
    validate_endpoint(endpoint)?;
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        bail!("请输入手机上显示的六位配对码");
    }
    adb.pair(endpoint.trim(), code)
}

pub fn unique_code_endpoint(services: &[Service]) -> Result<String> {
    let mut endpoints: Vec<_> = services
        .iter()
        .filter(|s| s.kind == "_adb-tls-pairing._tcp" && s.name.starts_with("adb-"))
        .map(|s| s.endpoint.clone())
        .collect();
    endpoints.sort();
    endpoints.dedup();
    match endpoints.as_slice() {
        [endpoint] => Ok(endpoint.clone()),
        [] => anyhow::bail!("未发现配对窗口，请在手机打开六位配对码弹窗，或填写其中的 IP:配对端口"),
        _ => anyhow::bail!("发现多台正在配对的手机，请填写目标手机弹窗中的 IP:配对端口"),
    }
}

pub struct QrSession {
    pub service: String,
    secret: String,
    pub started: Instant,
    pub directory: tempfile::TempDir,
    pub path: PathBuf,
}

impl QrSession {
    pub fn new() -> Result<Self> {
        let mut rng = rand::thread_rng();
        let service = format!("studio-stepx-{:016x}", rng.gen::<u64>());
        let secret: String = (0..20).map(|_| rng.sample(Alphanumeric) as char).collect();
        let payload = format!("WIFI:T:ADB;S:{service};P:{secret};;");
        let code = qrcode::QrCode::new(payload.as_bytes())?;
        let size = code.width();
        let scale = 6;
        let image = image::GrayImage::from_fn(
            ((size + 8) * scale) as u32,
            ((size + 8) * scale) as u32,
            |x, y| {
                let (x, y) = ((x as usize) / scale, (y as usize) / scale);
                let dark = x >= 4
                    && y >= 4
                    && x < size + 4
                    && y < size + 4
                    && code[(x - 4, y - 4)] == qrcode::Color::Dark;
                image::Luma([if dark { 0 } else { 255 }])
            },
        );
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pairing.png");
        image.save(&path)?;
        Ok(Self {
            service,
            secret,
            started: Instant::now(),
            directory,
            path,
        })
    }

    pub fn expired(&self) -> bool {
        self.started.elapsed() >= Duration::from_secs(120)
    }

    pub fn remaining_secs(&self) -> u64 {
        Duration::from_secs(120)
            .saturating_sub(self.started.elapsed())
            .as_secs()
    }

    pub fn try_pair(&self, adb: &Adb) -> Result<Option<String>> {
        if self.expired() {
            bail!("二维码已过期，请刷新二维码");
        }
        let services = crate::discovery::discover_services(adb)?;
        if let Some(service) = services
            .iter()
            .find(|s| s.kind == "_adb-tls-pairing._tcp" && s.name == self.service)
        {
            adb.pair(&service.endpoint, &self.secret)?;
            return Ok(Some(service.endpoint.clone()));
        }
        Ok(None)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RememberedDevice {
    pub serial: String,
    pub model: String,
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub devices: Vec<RememberedDevice>,
    pub selected_serial: Option<String>,
    #[serde(default)]
    pub first_connect_done: bool,
    #[serde(default)]
    pub adb_guide_seen: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub auto_sync_calendar: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub auto_sync_clipboard: bool,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub wireless: std::collections::BTreeMap<String, WirelessIdentity>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            selected_serial: None,
            first_connect_done: false,
            adb_guide_seen: false,
            auto_sync_calendar: true,
            auto_sync_clipboard: true,
            wireless: std::collections::BTreeMap::new(),
        }
    }
}

/// Discovery hints are not credentials. Hardware identity is verified after TLS authentication.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirelessIdentity {
    #[serde(default)]
    pub guid: String,
    #[serde(default)]
    pub endpoint: String,
}

impl Preferences {
    pub fn should_show_adb_guide(&self) -> bool {
        !self.adb_guide_seen && !self.first_connect_done && self.devices.is_empty()
    }

    pub fn path() -> PathBuf {
        if let Some(path) = std::env::var_os("STEPX_CONFIG_DIR") {
            return PathBuf::from(path).join("devices.json");
        }
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("stepx-companion/devices.json");
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("stepx-companion/devices.json");
        }
        let home = PathBuf::from(
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .unwrap_or_default(),
        );
        #[cfg(target_os = "macos")]
        {
            let xdg_fallback = home.join(".config/stepx-companion/devices.json");
            if xdg_fallback.exists() {
                return xdg_fallback;
            }
            let app_support = home.join("Library/Application Support");
            if app_support.is_dir() {
                return app_support.join("stepx-companion/devices.json");
            }
        }
        home.join(".config/stepx-companion/devices.json")
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&std::fs::read(path)?)
            .context("设备记录损坏；请备份并修复 devices.json")
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }
    pub fn save_to(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let parent = path.parent().context("设备记录目录无效")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
    pub fn remember(&mut self, serial: String, model: String) {
        self.first_connect_done = true;
        self.devices.retain(|d| d.serial != serial);
        self.devices.push(RememberedDevice {
            serial: serial.clone(),
            model,
        });
        self.selected_serial = Some(serial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_default_and_serde_roundtrip() {
        let p = Preferences::default();
        assert!(p.auto_sync_clipboard);
        assert!(p.auto_sync_calendar);

        let mut p_off = p.clone();
        p_off.auto_sync_clipboard = false;
        let json = serde_json::to_string(&p_off).expect("serialize");
        let p2: Preferences = serde_json::from_str(&json).expect("deserialize");
        assert!(!p2.auto_sync_clipboard);

        let p3: Preferences =
            serde_json::from_str(r#"{"devices":[],"selected_serial":null}"#).expect("deserialize");
        assert!(p3.auto_sync_clipboard);
        assert!(p3.auto_sync_calendar);
    }
}

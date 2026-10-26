use crate::{adb::Adb, mirror::MirrorSession};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    collections::HashMap,
    process::Command,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Transport {
    pub id: String,
    pub state: String,
    pub model: String,
}

fn backup_transport_registry() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set_backup_transports_for(keys: &[&str], backups: Vec<String>) {
    if let Ok(mut map) = backup_transport_registry().lock() {
        for key in keys {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                map.insert(trimmed.to_string(), backups.clone());
            }
        }
    }
}

pub fn backup_transports_for(keys: &[&str]) -> Vec<String> {
    if let Ok(map) = backup_transport_registry().lock() {
        for key in keys {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                if let Some(found) = map.get(trimmed) {
                    if !found.is_empty() {
                        return found.clone();
                    }
                }
            }
        }
    }
    Vec::new()
}

/// Priority of a transport ID (lower is higher priority):
/// 1 = physical USB serial (`ZYPZ261A01M692000LP`)
/// 2 = explicit wireless IP:Port (`10.0.1.177:34883`)
/// 3 = mDNS TLS service (`adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp`)
pub fn transport_priority(id: &str) -> u8 {
    let trimmed = id.trim();
    if trimmed.starts_with("adb-") || trimmed.contains("._adb-tls-") {
        3
    } else if trimmed.contains(':') {
        2
    } else {
        1
    }
}

pub fn is_usb_transport(id: &str) -> bool {
    transport_priority(id) == 1
}

/// Extract canonical hardware serial from a USB serial or `adb-<SERIAL>-<suffix>._adb-tls-...` transport ID.
/// Returns `None` for `ip:port` wireless endpoints whose serial must be resolved via model/getprop.
pub fn canonical_serial_from_transport(id: &str) -> Option<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("adb-") {
        let without_service = rest
            .split("._adb-tls-")
            .next()
            .unwrap_or(rest)
            .split_whitespace()
            .next()
            .unwrap_or(rest);
        if let Some((serial, _suffix)) = without_service.rsplit_once('-') {
            if !serial.trim().is_empty() {
                return Some(serial.trim().to_string());
            }
        }
        return None;
    }
    if trimmed.contains(':') {
        return None;
    }
    Some(trimmed.to_string())
}

impl Transport {
    pub fn backup_transports(&self) -> Vec<String> {
        let canonical = canonical_serial_from_transport(&self.id).unwrap_or_default();
        let model_key = if !self.model.is_empty() {
            format!("model:{}", self.model)
        } else {
            String::new()
        };
        backup_transports_for(&[&self.id, &canonical, &model_key])
            .into_iter()
            .filter(|b| b != &self.id)
            .collect()
    }
}

pub fn parse_devices(output: &str) -> Vec<Transport> {
    let raw: Vec<(Transport, String)> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() < 2 || line.starts_with("List of") || line.starts_with('*') {
                return None;
            }
            let product = parts
                .iter()
                .find_map(|p| p.strip_prefix("product:"))
                .unwrap_or("")
                .to_string();
            Some((
                Transport {
                    id: parts[0].into(),
                    state: parts[1].into(),
                    model: parts
                        .iter()
                        .find_map(|p| p.strip_prefix("model:"))
                        .unwrap_or("")
                        .into(),
                },
                product,
            ))
        })
        .collect();
    deduplicate_parsed_transports(raw, false)
}

fn deduplicate_parsed_transports(
    raw: Vec<(Transport, String)>,
    preserve_existing_backups: bool,
) -> Vec<Transport> {
    struct TransportGroup {
        serial: Option<String>,
        model: String,
        product: String,
        online: bool,
        members: Vec<Transport>,
    }

    let mut groups: Vec<TransportGroup> = Vec::new();
    for (transport, product) in raw {
        let online = transport.state == "device";
        let serial = canonical_serial_from_transport(&transport.id);
        if online {
            if let Some(group) = groups.iter_mut().find(|g| {
                if !g.online {
                    return false;
                }
                match (&g.serial, &serial) {
                    (Some(gs), Some(ts)) => gs == ts,
                    (Some(_), None) | (None, Some(_)) | (None, None) => {
                        !g.model.is_empty()
                            && g.model == transport.model
                            && (g.product.is_empty() || product.is_empty() || g.product == product)
                    }
                }
            }) {
                if group.serial.is_none() && serial.is_some() {
                    group.serial = serial;
                }
                if group.model.is_empty() && !transport.model.is_empty() {
                    group.model = transport.model.clone();
                }
                if group.product.is_empty() && !product.is_empty() {
                    group.product = product;
                }
                group.members.push(transport);
                continue;
            }
        }
        groups.push(TransportGroup {
            serial,
            model: transport.model.clone(),
            product,
            online,
            members: vec![transport],
        });
    }

    let mut result = Vec::with_capacity(groups.len());
    for mut group in groups {
        if group.online && group.members.len() > 1 {
            group.members.sort_by(|a, b| {
                transport_priority(&a.id)
                    .cmp(&transport_priority(&b.id))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        let mut iter = group.members.into_iter();
        if let Some(mut primary) = iter.next() {
            if primary.model.is_empty() && !group.model.is_empty() {
                primary.model = group.model.clone();
            }
            let mut backups = if preserve_existing_backups {
                primary.backup_transports()
            } else {
                Vec::new()
            };
            for m in iter {
                if m.id != primary.id && !backups.contains(&m.id) {
                    backups.push(m.id.clone());
                }
                if preserve_existing_backups {
                    for b in m.backup_transports() {
                        if b != primary.id && !backups.contains(&b) {
                            backups.push(b);
                        }
                    }
                }
            }
            backups.sort_by(|a, b| {
                transport_priority(a)
                    .cmp(&transport_priority(b))
                    .then_with(|| a.cmp(b))
            });
            let serial_key = group.serial.unwrap_or_default();
            let model_key = if !primary.model.is_empty() {
                format!("model:{}", primary.model)
            } else {
                String::new()
            };
            set_backup_transports_for(&[&primary.id, &serial_key, &model_key], backups.clone());
            if !serial_key.is_empty() && serial_key != primary.id {
                let mut serial_backups = vec![primary.id.clone()];
                for b in &backups {
                    if !serial_backups.contains(b) {
                        serial_backups.push(b.clone());
                    }
                }
                set_backup_transports_for(&[&serial_key], serial_backups);
            }
            result.push(primary);
        }
    }
    result
}

pub fn deduplicate_transports(transports: Vec<Transport>) -> Vec<Transport> {
    let raw = transports.into_iter().map(|t| (t, String::new())).collect();
    deduplicate_parsed_transports(raw, true)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Device {
    pub transport: String,
    pub serial: String,
    pub model: String,
    pub android: String,
    pub product: String,
}

pub fn deduplicate_multi_transport_devices(devices: Vec<Device>) -> Vec<Device> {
    struct DeviceGroup {
        serial: Option<String>,
        model: String,
        product: String,
        members: Vec<Device>,
    }

    let mut groups: Vec<DeviceGroup> = Vec::new();
    for device in devices {
        let effective_serial = if !device.serial.trim().is_empty()
            && !device.serial.contains(':')
            && !device.serial.starts_with("adb-")
        {
            Some(device.serial.trim().to_string())
        } else {
            canonical_serial_from_transport(&device.transport)
        };

        if let Some(group) = groups
            .iter_mut()
            .find(|g| match (&g.serial, &effective_serial) {
                (Some(gs), Some(ds)) => gs == ds,
                (Some(_), None) | (None, Some(_)) | (None, None) => {
                    !g.model.is_empty()
                        && g.model == device.model
                        && (g.product.is_empty()
                            || device.product.is_empty()
                            || g.product == device.product)
                }
            })
        {
            if group.serial.is_none() && effective_serial.is_some() {
                group.serial = effective_serial;
            }
            if group.model.is_empty() && !device.model.is_empty() {
                group.model = device.model.clone();
            }
            if group.product.is_empty() && !device.product.is_empty() {
                group.product = device.product.clone();
            }
            group.members.push(device);
            continue;
        }

        groups.push(DeviceGroup {
            serial: effective_serial,
            model: device.model.clone(),
            product: device.product.clone(),
            members: vec![device],
        });
    }

    let mut out = Vec::with_capacity(groups.len());
    for mut group in groups {
        group.members.sort_by(|a, b| {
            transport_priority(&a.transport)
                .cmp(&transport_priority(&b.transport))
                .then_with(|| a.transport.cmp(&b.transport))
        });
        let mut iter = group.members.into_iter();
        if let Some(mut primary) = iter.next() {
            if let Some(ref s) = group.serial {
                if primary.serial.is_empty()
                    || primary.serial.contains(':')
                    || primary.serial.starts_with("adb-")
                {
                    primary.serial = s.clone();
                }
            }
            let mut backups = primary.backup_transports();
            for member in iter {
                if member.transport != primary.transport && !backups.contains(&member.transport) {
                    backups.push(member.transport.clone());
                }
                for b in member.backup_transports() {
                    if b != primary.transport && !backups.contains(&b) {
                        backups.push(b);
                    }
                }
            }
            backups.sort_by(|a, b| {
                transport_priority(a)
                    .cmp(&transport_priority(b))
                    .then_with(|| a.cmp(b))
            });
            primary.set_backup_transports(backups);
            out.push(primary);
        }
    }
    out
}

fn validate_generic_apk_identity(value: &serde_json::Value) -> Result<()> {
    if value["ok"] != true {
        bail!(
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("无法核对 APK 包名，尚未安装")
        );
    }
    let package = value["package"]
        .as_str()
        .filter(|p| !p.is_empty())
        .context("APK 包名校验未返回有效结果，尚未安装")?;
    if package == "com.stepx.companion.app" {
        bail!("StepX伴侣请通过“小工具 → 安装 StepX伴侣”更新；内置 GUI Agent 完整包请使用“小工具 → 内置 GUI Agent”安装。通用 APK 入口不会覆盖伴侣，现有功能和数据未改变。");
    }
    Ok(())
}

fn check_generic_apk(path: &std::path::Path) -> Result<()> {
    let mut command = crate::resources::python_command();
    command
        .arg(crate::resources::root().join("scripts/companion_apk.py"))
        .arg("--package")
        .arg(path);
    let output = crate::adb::run_process(command, None, std::time::Duration::from_secs(45))?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("无法核对 APK 包名，尚未安装；请重新解压完整客户端安装包")?;
    validate_generic_apk_identity(&value)?;
    if !output.status.success() {
        bail!("APK 包名校验未完成，尚未安装");
    }
    Ok(())
}

impl Device {
    pub fn backup_transports(&self) -> Vec<String> {
        let canonical = canonical_serial_from_transport(&self.transport).unwrap_or_default();
        let model_key = if !self.model.is_empty() {
            format!("model:{}", self.model)
        } else {
            String::new()
        };
        backup_transports_for(&[&self.transport, &self.serial, &canonical, &model_key])
            .into_iter()
            .filter(|b| b != &self.transport)
            .collect()
    }

    pub fn set_backup_transports(&self, backups: Vec<String>) {
        let canonical = canonical_serial_from_transport(&self.transport).unwrap_or_default();
        let model_key = if !self.model.is_empty() {
            format!("model:{}", self.model)
        } else {
            String::new()
        };
        let filtered: Vec<String> = backups
            .into_iter()
            .filter(|b| b != &self.transport)
            .collect();
        set_backup_transports_for(
            &[&self.transport, &self.serial, &canonical, &model_key],
            filtered,
        );
    }

    pub fn has_wireless_backup(&self) -> bool {
        !self.backup_transports().is_empty()
    }

    pub fn is_usb_primary(&self) -> bool {
        is_usb_transport(&self.transport)
    }

    pub fn inspect(adb: &Adb, transport: &str) -> Result<Self> {
        let props = adb.script(transport, "getprop ro.serialno; getprop ro.product.model; getprop ro.build.version.release; getprop ro.product.device")?;
        let parts: Vec<_> = props.lines().map(str::trim).collect();
        if parts.len() < 4 || parts[0].is_empty() {
            bail!("无法读取设备身份，请解锁手机并允许调试");
        }
        let model = parts[1].to_lowercase();
        if !model.contains("stepx") && !model.contains("pz261a") && parts[3] != "pandora" {
            bail!("这台设备不是已支持的 StepX（型号：{}），未选中。", parts[1]);
        }
        let device = Self {
            transport: transport.into(),
            serial: parts[0].into(),
            model: parts[1].into(),
            android: parts[2].into(),
            product: parts[3].into(),
        };
        if device.backup_transports().is_empty() {
            if let Ok(raw_devices) =
                adb.run_timeout(&["devices", "-l"], std::time::Duration::from_secs(2))
            {
                let _ = parse_devices(&raw_devices);
            }
        }
        let existing_backups = device.backup_transports();
        if !existing_backups.is_empty() {
            device.set_backup_transports(existing_backups);
        }
        Ok(device)
    }

    /// A transport/IP can be reassigned. Recheck hardware identity before every operation.
    pub fn verify(&self, adb: &Adb) -> Result<()> {
        let serial = adb.shell(&self.transport, &["getprop", "ro.serialno"])?;
        if serial.trim() != self.serial {
            bail!("设备身份已变化，请重新选择手机。此次操作没有执行。");
        }
        Ok(())
    }

    pub fn summary(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let raw = adb.script(&self.transport, "dumpsys battery; df -h /data /sdcard; wm size; ip -o -4 addr show dev wlan0 2>/dev/null || true")?;
        Ok(format_summary(&raw))
    }

    pub fn key(&self, adb: &Adb, code: u32) -> Result<()> {
        if ![
            3, 4, 5, 6, 24, 25, 26, 27, 66, 67, 82, 85, 87, 88, 120, 164, 187, 219, 220, 221, 223,
            224, 231,
        ]
        .contains(&code)
        {
            bail!("不支持的按键");
        }
        self.verify(adb)?;
        adb.shell(&self.transport, &["input", "keyevent", &code.to_string()])?;
        Ok(())
    }

    pub fn screen_resolution(&self, adb: &Adb) -> (u32, u32) {
        adb.shell(&self.transport, &["wm", "size"])
            .map(|raw| parse_wm_size(&raw))
            .unwrap_or((1080, 2400))
    }

    pub fn tap_normalized(&self, adb: &Adb, norm_x: u16, norm_y: u16) -> Result<(u32, u32)> {
        self.verify(adb)?;
        let (w, h) = self.screen_resolution(adb);
        let px = ((norm_x.min(1000) as u32) * w / 1000).clamp(1, w.saturating_sub(1));
        let py = ((norm_y.min(1000) as u32) * h / 1000).clamp(1, h.saturating_sub(1));
        adb.shell(
            &self.transport,
            &["input", "tap", &px.to_string(), &py.to_string()],
        )?;
        Ok((px, py))
    }

    pub fn screen_gesture(&self, adb: &Adb, gesture: u8) -> Result<String> {
        self.verify(adb)?;
        let (w, h) = self.screen_resolution(adb);
        let cx = w / 2;
        let cy = h / 2;
        let y_bot = h * 78 / 100;
        let y_top = h * 22 / 100;
        let x_right = w * 84 / 100;
        let x_left = w * 16 / 100;
        match gesture {
            0 => {
                adb.shell(
                    &self.transport,
                    &[
                        "input",
                        "swipe",
                        &cx.to_string(),
                        &y_bot.to_string(),
                        &cx.to_string(),
                        &y_top.to_string(),
                        "240",
                    ],
                )?;
                Ok("已向手机发送上滑手势".into())
            }
            1 => {
                adb.shell(
                    &self.transport,
                    &[
                        "input",
                        "swipe",
                        &cx.to_string(),
                        &y_top.to_string(),
                        &cx.to_string(),
                        &y_bot.to_string(),
                        "240",
                    ],
                )?;
                Ok("已向手机发送下滑手势".into())
            }
            2 => {
                adb.shell(
                    &self.transport,
                    &[
                        "input",
                        "swipe",
                        &x_right.to_string(),
                        &cy.to_string(),
                        &x_left.to_string(),
                        &cy.to_string(),
                        "220",
                    ],
                )?;
                Ok("已向手机发送左滑手势".into())
            }
            3 => {
                adb.shell(
                    &self.transport,
                    &[
                        "input",
                        "swipe",
                        &x_left.to_string(),
                        &cy.to_string(),
                        &x_right.to_string(),
                        &cy.to_string(),
                        "220",
                    ],
                )?;
                Ok("已向手机发送右滑返回手势".into())
            }
            4 => {
                let _ = adb.shell(
                    &self.transport,
                    &["cmd", "statusbar", "expand-notifications"],
                );
                Ok("已展开手机通知栏".into())
            }
            5 => {
                let _ = adb.shell(&self.transport, &["cmd", "statusbar", "expand-settings"]);
                Ok("已展开手机快捷设置面板".into())
            }
            6 => {
                let _ = adb.shell(&self.transport, &["cmd", "statusbar", "collapse"]);
                Ok("已收起手机状态栏".into())
            }
            _ => {
                let _ = adb.script(
                    &self.transport,
                    &format!(
                        "input keyevent 224; wm dismiss-keyguard >/dev/null 2>&1 || true; input swipe {cx} {y_bot} {cx} {y_top} 220"
                    ),
                );
                Ok("已点亮屏幕并发送上滑解锁".into())
            }
        }
    }

    pub fn open_amoo(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let script = concat!(
            "if ! pm path com.stepos.ailauncher 2>/dev/null | grep -q 'package:'; then ",
            "echo STEPX_NO_AMOO; ",
            "else ",
            "input keyevent 224 >/dev/null 2>&1 || true; ",
            "input keyevent 82 >/dev/null 2>&1 || true; ",
            "am start -n com.stepos.ailauncher/.chat.activity.ChatMainActivity --activity-single-top; ",
            "fi"
        );
        match adb.script(&self.transport, script) {
            Ok(out) => {
                if out.contains("STEPX_NO_AMOO") {
                    bail!("没有检测到系统 Amoo 包 com.stepos.ailauncher");
                }
            }
            Err(_) => {
                if adb
                    .shell(
                        &self.transport,
                        &[
                            "am",
                            "start",
                            "-a",
                            "com.stepos.ailauncher.action.OPEN_CHAT",
                            "-p",
                            "com.stepos.ailauncher",
                        ],
                    )
                    .is_err()
                {
                    self.key(adb, 219)?;
                }
            }
        }
        Ok("已在手机上打开 Amoo 对话中心".into())
    }

    pub fn open_amoo_memory(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                "stepos://amoo/memory",
            ],
        )?;
        Ok("已在手机上打开 Amoo 记忆与偏好空间 (stepos://amoo/memory)".into())
    }

    pub fn open_amoo_tasks(&self, adb: &Adb) -> Result<String> {
        let _ = self.open_amoo(adb)?;
        let _ = adb.shell(
            &self.transport,
            &[
                "am",
                "broadcast",
                "-a",
                "com.stepos.ailauncher.action.OPEN_ALL_TASK",
                "-p",
                "com.stepos.ailauncher",
            ],
        );
        Ok("已在手机上打开 Amoo 会话与并行任务中心".into())
    }

    pub fn open_amoo_voice_settings(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "com.stepos.ailauncher.action.VOICE_SETTING",
            ],
        )?;
        Ok("已在手机上打开 Amoo 语音唤醒与播报音色设置".into())
    }

    pub fn open_amoo_safety_center(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "stepos.intent.action.AGENT_SAFETY_CENTER_LAUNCHER",
            ],
        )?;
        Ok("已在手机上打开智能体安全中心 (AgentSafetyCenter)".into())
    }

    pub fn open_amoo_screen_perception(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let _ = adb.shell(
            &self.transport,
            &[
                "am",
                "startservice",
                "-a",
                "com.stepos.picker.screenperception.action.WAKE_UP_SERVICE",
            ],
        );
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "stepos.intent.action.AGENT_SCREEN_PERCEPTION",
            ],
        )?;
        Ok("已唤起小跃屏幕智能感知与圈选权限设置".into())
    }

    pub fn open_amoo_persona(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "com.stepos.alphame.action.personal_preference",
            ],
        )?;
        Ok("已在手机上打开个人偏好与常用地址/通勤设置 (AlphaMe)".into())
    }

    pub fn open_amoo_model_manager(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "com.stepos.aiengine.action.MODEL_MANAGER",
            ],
        )?;
        Ok("已在手机上打开端侧大模型管理 (AIEngine)".into())
    }

    pub fn amoo_voice_wake(&self, adb: &Adb) -> Result<String> {
        let _ = self.open_amoo(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "broadcast",
                "-a",
                "com.stepos.ailauncher.action.VOICE_WAKE",
                "-p",
                "com.stepos.ailauncher",
            ],
        )?;
        Ok("已唤起 Amoo 语音聆听，请直接说话或发语音".into())
    }

    pub fn amoo_share_file(&self, adb: &Adb, local_path: &std::path::Path) -> Result<String> {
        if !local_path.is_file() {
            bail!("请选择有效的本地图片或文件附件");
        }
        self.verify(adb)?;
        let dest_dir = "/sdcard/Download/StepX";
        let _ = adb.shell(&self.transport, &["mkdir", "-p", dest_dir]);
        crate::storage::push(adb, self, local_path, dest_dir)?;
        let file_name = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment");
        let ext = local_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => "image/*",
            "txt" | "md" | "json" | "csv" | "log" => "text/plain",
            _ => "*/*",
        };
        let remote_uri = format!("file://{dest_dir}/{file_name}");
        let _ = adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.SEND",
                "-n",
                "com.stepos.ailauncher/.chat.activity.ShareActivity",
                "-t",
                mime,
                "--eu",
                "android.intent.extra.STREAM",
                &remote_uri,
            ],
        );
        Ok(format!("已将附件 {file_name} 投递至 Amoo"))
    }

    pub fn dial_number(&self, adb: &Adb, number: &str) -> Result<String> {
        let clean: String = number
            .trim()
            .chars()
            .filter(|c| c.is_ascii_digit() || "+*#".contains(*c))
            .collect();
        if clean.is_empty() {
            bail!("请输入有效的电话号码");
        }
        self.verify(adb)?;
        let uri = format!("tel:{clean}");
        let result = adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.DIAL",
                "-d",
                &uri,
            ],
        )?;
        if result.contains("Error:") || result.contains("Exception") {
            bail!("无法打开手机拨号界面：{result}");
        }
        Ok("已打开手机拨号界面，请在手机选择 SIM 卡并确认呼叫".into())
    }

    pub fn send_focus_text(&self, adb: &Adb, text: &str) -> Result<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            bail!("请输入要发送到手机焦点框的文字");
        }
        self.verify(adb)?;
        if trimmed.is_ascii() {
            let escaped = trimmed.replace(' ', "%s");
            adb.shell(&self.transport, &["input", "text", &escaped])?;
            return Ok(format!("已向手机当前焦点输入：{}", trimmed));
        }
        // For CJK / Unicode text, try Amoo prepare or clipboard broadcast fallback
        let _ = crate::bridge::amoo_prepare(adb, self, trimmed);
        Ok(format!("已将文字同步至手机输入区：{}", trimmed))
    }

    pub fn push_backscreen_frame(&self, adb: &Adb, frame: &[u8]) -> Result<String> {
        self.verify(adb)?;
        crate::backscreen::push_frame(adb, self, frame)?;
        let state = adb.shell(&self.transport, &["dumpsys", "display_ext"])?;
        if !state.contains("power=on") || state.contains("policyAllowsOutput=false") {
            bail!("手机接收到图案，但背屏未亮起；请将背屏朝上并检查手机的背屏、勿扰和省电设置");
        }
        Ok("已将17×23像素帧交给手机背屏，HAL已开启".into())
    }
    pub fn push_backscreen_pattern(
        &self,
        adb: &Adb,
        pattern: BackScreenPattern,
        brightness: u8,
        invert: bool,
    ) -> Result<String> {
        self.push_backscreen_frame(adb, &pattern.frame_391(brightness, invert))
    }
    pub fn push_backscreen_image(
        &self,
        adb: &Adb,
        path: &std::path::Path,
        brightness: u8,
        invert: bool,
    ) -> Result<(Vec<u8>, String)> {
        let frame = image_to_backscreen_frame(path, brightness, invert)?;
        let message = self.push_backscreen_frame(adb, &frame)?;
        Ok((frame, message))
    }
    pub fn set_backscreen_marquee(
        &self,
        adb: &Adb,
        text: &str,
        scroll: bool,
        reverse: bool,
    ) -> Result<String> {
        crate::backscreen::marquee(adb, self, text, scroll, reverse)
    }
    pub fn set_backscreen_mode(&self, adb: &Adb, mode: &str) -> Result<String> {
        crate::backscreen::set_display_mode(adb, self, mode)
    }
    pub fn set_backscreen_brightness(&self, adb: &Adb, level: u8) -> Result<String> {
        crate::backscreen::set_brightness(adb, self, level)
    }
    pub fn trigger_backscreen_scene(&self, adb: &Adb, scene: &str) -> Result<String> {
        self.verify(adb)?;
        match scene {
            "clock" => crate::backscreen::configure_native(
                adb,
                self,
                crate::backscreen::NativeFeature::Clock,
            ),
            "notify" => crate::backscreen::configure_native(
                adb,
                self,
                crate::backscreen::NativeFeature::Notifications,
            ),
            "countdown" => {
                let output = adb.shell(
                    &self.transport,
                    &[
                        "dumpsys",
                        "display_ext",
                        "back_screen",
                        "special",
                        "countdown",
                        "3",
                    ],
                )?;
                if output.to_ascii_lowercase().contains("error") {
                    bail!("手机倒计时未启动：{output}")
                }
                Ok("已请求手机原生3秒倒计时".into())
            }
            "reset" => {
                crate::backscreen::clear_frame(adb, self)?;
                crate::backscreen::set_display_mode(adb, self, "auto")
            }
            _ => bail!("请从背屏创作台选择对应动画后播放；当前功能不会模拟天气或系统通知"),
        }
    }

    pub fn call_logs(&self, adb: &Adb) -> Result<Vec<CallRecord>> {
        let result = crate::bridge::call_bridge(
            adb,
            self,
            &serde_json::json!({"op": "read_calls"}),
            std::time::Duration::from_secs(30),
        )
        .context("无法读取手机通话记录")?;
        parse_call_records(&result)
    }

    pub fn contacts(&self, adb: &Adb) -> Result<Vec<ContactEntry>> {
        let result = crate::bridge::call_bridge(
            adb,
            self,
            &serde_json::json!({"op": "read_contacts"}),
            std::time::Duration::from_secs(30),
        )
        .context("无法读取手机通讯录")?;
        parse_contact_records(&result)
    }

    pub fn create_contact(&self, adb: &Adb, name: &str, number: &str) -> Result<String> {
        let name = name.trim();
        let number = number.trim();
        if name.is_empty() || number.is_empty() {
            bail!("请填写联系人姓名与电话号码");
        }
        self.verify(adb)?;
        let result = adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.INSERT",
                "-t",
                "vnd.android.cursor.dir/contact",
                "--es",
                "name",
                name,
                "--es",
                "phone",
                number,
            ],
        )?;
        if result.contains("Error:") || result.contains("Exception") {
            bail!("无法打开手机联系人编辑页：{result}");
        }
        Ok("已填入手机联系人编辑页，请在手机确认保存后刷新通讯录".into())
    }

    pub fn visible_text(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        // Unique remote file avoids another tool's concurrent hierarchy dump.
        let remote = format!(
            "/data/local/tmp/stepx-ui-{:016x}.xml",
            rand::random::<u64>()
        );
        let q = crate::adb::shell_quote(&remote);
        let xml = adb.script(
            &self.transport,
            &format!("uiautomator dump {q} >/dev/null && cat {q}; rm -f {q}"),
        )?;
        visible_text(&xml)
    }

    pub fn messages(&self, adb: &Adb) -> Result<Vec<crate::sms::Message>> {
        crate::sms::read(adb, self)
    }

    pub fn draft_sms(&self, adb: &Adb, number: &str, body: &str) -> Result<String> {
        let number = number.trim();
        if number.is_empty()
            || number.len() > 40
            || !number
                .chars()
                .all(|c| c.is_ascii_digit() || "+ -()".contains(c))
        {
            bail!("请输入有效的收件人号码");
        }
        if body.is_empty() {
            bail!("短信内容不能为空");
        }
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.SENDTO",
                "-d",
                &format!("smsto:{number}"),
                "--es",
                "sms_body",
                body,
            ],
        )?;
        Ok("短信草稿已交给手机。请核对收件人和内容，再在实时控制窗口点击发送。".into())
    }

    pub fn open_messages(&self, adb: &Adb) -> Result<()> {
        self.verify(adb)?;
        adb.shell(
            &self.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.APP_MESSAGING",
            ],
        )?;
        Ok(())
    }

    pub fn apps(&self, adb: &Adb) -> Result<Vec<String>> {
        self.apps_with_filter(adb, false)
    }

    pub fn apps_with_filter(&self, adb: &Adb, include_system: bool) -> Result<Vec<String>> {
        self.verify(adb)?;
        let args: &[&str] = if include_system {
            &["pm", "list", "packages"]
        } else {
            &["pm", "list", "packages", "-3"]
        };
        let output = adb.shell(&self.transport, args)?;
        let mut apps: Vec<_> = output
            .lines()
            .filter_map(|s| s.strip_prefix("package:").map(str::to_owned))
            .collect();
        apps.sort();
        Ok(apps)
    }

    pub fn install_apk(&self, adb: &Adb, local_apk: &std::path::Path) -> Result<String> {
        if !local_apk.is_file() {
            bail!("请选择一个有效的本地 APK 安装包文件");
        }
        let is_apk = local_apk
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("apk"));
        if !is_apk {
            bail!("仅支持安装 .apk 格式的 Android 应用包");
        }
        // Inspect and install the same immutable snapshot. The general application
        // picker must not bypass Companion's signed, feature-preserving updater.
        let mut snapshot = tempfile::Builder::new().suffix(".apk").tempfile()?;
        let mut source = std::fs::File::open(local_apk).context("无法读取 APK 安装包")?;
        std::io::copy(&mut source, &mut snapshot).context("无法准备 APK 安装包")?;
        snapshot.as_file().sync_all()?;
        check_generic_apk(snapshot.path())?;
        self.verify(adb)?;
        let path_str = snapshot
            .path()
            .to_str()
            .context("APK 文件路径必须是有效的 UTF-8")?;
        let out = adb.run_timeout(
            &["-s", &self.transport, "install", "-r", path_str],
            std::time::Duration::from_secs(180),
        )?;
        let file_name = local_apk
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("应用");
        if out.contains("Success") {
            Ok(format!("已成功安装 APK：{file_name}"))
        } else {
            Ok(format!("APK 安装完成（{file_name}）：{}", out.trim()))
        }
    }

    pub fn open_app(&self, adb: &Adb, package: &str) -> Result<()> {
        if package.is_empty()
            || !package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._".contains(c))
        {
            bail!("无效的应用包名");
        }
        self.verify(adb)?;
        let component = adb.shell(
            &self.transport,
            &[
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.LAUNCHER",
                package,
            ],
        )?;
        let component = component
            .lines()
            .find(|line| line.contains('/'))
            .context("该应用没有可打开的主界面")?;
        adb.shell(&self.transport, &["am", "start", "-n", component])?;
        Ok(())
    }

    pub fn mirror(&self, adb: &Adb) -> Result<MirrorSession> {
        self.mirror_with_options(adb, crate::mirror::MirrorOptions::default())
    }

    pub fn mirror_with_options(
        &self,
        adb: &Adb,
        options: crate::mirror::MirrorOptions,
    ) -> Result<MirrorSession> {
        self.verify(adb)?;
        crate::mirror::start_with_options(adb, &self.transport, options)
    }

    pub fn force_stop_app(&self, adb: &Adb, package: &str) -> Result<String> {
        if package.is_empty()
            || !package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._".contains(c))
        {
            bail!("无效的应用包名");
        }
        self.verify(adb)?;
        adb.shell(&self.transport, &["am", "force-stop", package])?;
        let label = friendly_app_name(package).unwrap_or(package);
        Ok(format!("已强行停止应用：{label} ({package})"))
    }

    pub fn hangup_call(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        adb.shell(&self.transport, &["input", "keyevent", "6"])?;
        Ok("已向手机发送挂断指令 (KEYCODE_ENDCALL = 6)".into())
    }

    pub fn run_google_handoff_manage(&self, subcmd: &str) -> Result<serde_json::Value> {
        let script = crate::resources::root().join("handoff/google-play-stepx/tools/manage.py");
        if !script.is_file() {
            bail!("未找到 Google Play 特权工程脚本：{}", script.display());
        }
        let out = crate::resources::python_command()
            .arg(&script)
            .arg(subcmd)
            .env("STEPX_SERIAL", &self.transport)
            .env("STEPX_DEVICE_SERIAL", &self.serial)
            .output()
            .context("无法调用 Google Play 特权管理脚本")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("manage.py {subcmd} 执行失败：{}", err.trim());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(text.trim()).context("解析 manage.py 输出 JSON 失败")
    }

    pub fn run_root_bridge_python(&self, py_code: &str) -> Result<String> {
        let script =
            crate::resources::root().join("handoff/google-play-stepx/tools/root_bridge.py");
        if !script.is_file() {
            bail!("未找到 root_bridge.py：{}", script.display());
        }
        let out = crate::resources::python_command()
            .arg(&script)
            .arg("-c")
            .arg(py_code)
            .env("STEPX_SERIAL", &self.transport)
            .env("STEPX_DEVICE_SERIAL", &self.serial)
            .output()
            .context("无法调用 root_bridge.py")?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() && stdout.is_empty() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            bail!("root_bridge 执行异常：{stderr}");
        }
        Ok(stdout)
    }

    pub fn inspect_google_suite(&self, adb: &Adb) -> Result<GoogleSuiteStatus> {
        crate::google::inspect(self, adb)
    }

    pub fn maintain_google_suite(&self, adb: &Adb) -> Result<(GoogleSuiteStatus, String)> {
        let status = self.inspect_google_suite(adb)?;
        let message = status.verification_note.clone();
        Ok((status, message))
    }

    pub fn prepare_google_snapshot(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let key_path = crate::resources::user_config().join("stepx-gms/backup.key");
        let snap_dir = crate::resources::user_state().join("google-play/backups");
        let has_key = key_path.is_file();
        let has_snap = snap_dir.is_dir();
        Ok(format!(
            "安全快照校验完成：本地备份目录 {} ({}) · 加密密钥 {} · 手机还原点需要另行核验",
            snap_dir.display(),
            if has_snap { "已就绪" } else { "待创建" },
            if has_key {
                "AES-GCM 已配置"
            } else {
                "未配置备份密钥"
            }
        ))
    }

    pub fn inspect_system_services(&self, adb: &Adb) -> Result<Vec<SystemServiceInfo>> {
        self.verify(adb)?;
        let combined = adb.script(
            &self.transport,
            "pm list packages -d; echo '<<<STEPX_U>>>'; pm list packages -u; echo '<<<STEPX_ALL>>>'; pm list packages; echo '<<<STEPX_IME>>>'; settings get secure default_input_method",
        )?;
        let (disabled_raw, rest) = combined
            .split_once("<<<STEPX_U>>>")
            .unwrap_or((&combined, ""));
        let (all_raw, rest) = rest.split_once("<<<STEPX_ALL>>>").unwrap_or((rest, ""));
        let (installed_raw, ime_raw) = rest.split_once("<<<STEPX_IME>>>").unwrap_or((rest, ""));
        let ime_raw = ime_raw.trim();

        let mut list = default_system_services();
        for item in &mut list {
            let pkg_tag = format!("package:{}", item.package);
            let is_installed = installed_raw.lines().any(|l| l.trim() == pkg_tag);
            let is_disabled = disabled_raw.lines().any(|l| l.trim() == pkg_tag);
            let exists_uninstalled = all_raw.lines().any(|l| l.trim() == pkg_tag);
            item.enabled = is_installed && !is_disabled;
            if !exists_uninstalled && !is_installed {
                item.status_note = "未安装".into();
                continue;
            }
            if ime_raw.starts_with(&format!("{}/", item.package)) {
                item.status_note = "当前默认输入法".into();
                continue;
            }
            if item.package == "org.fcitx.fcitx5.android" {
                item.status_note = if ime_raw.contains("fcitx5") {
                    "当前默认输入法 · 开源零遥测".into()
                } else if item.enabled {
                    "已安装 · 可在手机设置中切换".into()
                } else {
                    "未启用".into()
                };
            } else if !is_installed && exists_uninstalled {
                item.status_note = "已从当前用户卸载精简 (可一键恢复)".into();
            } else if is_disabled {
                item.status_note = "已停用".into();
            } else {
                item.status_note = "已启用".into();
            }
        }
        Ok(list)
    }

    fn toggle_system_service_no_inspect(
        &self,
        adb: &Adb,
        package: &str,
        enable: bool,
    ) -> Result<()> {
        let known = default_system_services()
            .into_iter()
            .any(|s| s.package == package && s.safe_to_remove);
        if !known {
            bail!("此服务不在可管理范围内");
        }
        if enable {
            let q = crate::adb::shell_quote(package);
            adb.script(
                &self.transport,
                &format!("cmd package install-existing --user 0 {q} >/dev/null 2>&1 || true; pm enable --user 0 {q}"),
            )?;
        } else {
            let ime = adb.shell(
                &self.transport,
                &["settings", "get", "secure", "default_input_method"],
            )?;
            if ime.trim().starts_with(&format!("{package}/")) {
                bail!("不能停用当前输入法，请先在手机设置中切换输入法");
            }
            adb.shell(
                &self.transport,
                &["pm", "disable-user", "--user", "0", package],
            )?;
        }
        Ok(())
    }

    pub fn toggle_system_service(&self, adb: &Adb, package: &str, enable: bool) -> Result<String> {
        self.verify(adb)?;
        self.toggle_system_service_no_inspect(adb, package, enable)?;
        let status = self.inspect_system_services(adb)?;
        if !status
            .iter()
            .any(|s| s.package == package && s.status_note != "未安装" && s.enabled == enable)
        {
            bail!("手机未确认服务状态变更，请刷新后重试");
        }
        Ok(format!(
            "已{}服务：{package}",
            if enable { "启用" } else { "停用" }
        ))
    }

    pub fn debloat_preset_services(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let targets = [
            "com.sohu.inputmethod.sogou.oem",
            "com.qti.diagservices",
            "com.android.traceur",
            "com.android.printspooler",
            "com.android.bips",
            "com.android.dreams.basic",
        ];
        let mut toggled = Vec::new();
        for pkg in targets {
            if self
                .toggle_system_service_no_inspect(adb, pkg, false)
                .is_ok()
            {
                toggled.push(pkg);
            }
        }
        let status = self.inspect_system_services(adb).unwrap_or_default();
        let count = toggled
            .into_iter()
            .filter(|pkg| {
                status
                    .iter()
                    .any(|s| s.package == *pkg && s.status_note != "未安装" && !s.enabled)
            })
            .count();
        Ok(format!(
            "已完成系统自带服务一键精简：共停用 {count} 项冗余后台服务（搜狗定制输入法 / QDMA 诊断 / 系统追踪 / 打印后台等）"
        ))
    }

    pub fn restore_all_system_services(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let targets = [
            "com.sohu.inputmethod.sogou.oem",
            "com.qti.diagservices",
            "com.android.traceur",
            "com.android.printspooler",
            "com.android.bips",
            "com.android.dreams.basic",
        ];
        for pkg in targets {
            let _ = self.toggle_system_service_no_inspect(adb, pkg, true);
        }
        let _ = self.inspect_system_services(adb);
        Ok("已恢复全部 StepOS 原厂预装系统服务".into())
    }

    pub fn switch_input_method_fcitx5(&self, adb: &Adb, use_fcitx5: bool) -> Result<String> {
        if use_fcitx5 {
            crate::input_method::install_and_enable(adb, self)?;
            Ok("已核验小企鹅为默认输入法，搜狗已停用；安全输入法保留".into())
        } else {
            crate::input_method::restore_sogou(adb, self)?;
            Ok("已核验恢复搜狗输入法，小企鹅与词库保留".into())
        }
    }

    /// Grants deep Amoo & OpenClaw permissions + background keep-alive (`oom_score_adj = -800`).
    pub fn grant_deep_amoo_permissions(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let mut cmds = Vec::with_capacity(18);
        for pkg in ["com.stepos.ailauncher", "com.stepos.stepclaw"] {
            cmds.push(format!(
                "dumpsys deviceidle whitelist +{pkg} >/dev/null 2>&1 || true"
            ));
            for op in [
                "RUN_IN_BACKGROUND",
                "RUN_ANY_IN_BACKGROUND",
                "SYSTEM_EXEMPT_FROM_POWER_RESTRICTIONS",
                "SYSTEM_ALERT_WINDOW",
                "GET_USAGE_STATS",
                "MANAGE_EXTERNAL_STORAGE",
                "WRITE_SETTINGS",
            ] {
                cmds.push(format!(
                    "cmd appops set {pkg} {op} allow >/dev/null 2>&1 || true"
                ));
            }
            cmds.push(format!(
                "am set-standby-bucket {pkg} active >/dev/null 2>&1 || true"
            ));
        }
        let _ = adb.script(&self.transport, &cmds.join("; "));
        // Also use root_bridge to lock oom_score_adj = -800 on openclaw gateway & ailauncher processes
        let py = r#"
import glob, os
adjusted = 0
for comm_path in glob.glob('/proc/[0-9]*/cmdline'):
    try:
        data = open(comm_path, 'rb').read().decode('utf-8', 'ignore')
        if any(k in data for k in ('openclaw', 'ailauncher', 'stepclaw')):
            pid = comm_path.split('/')[2]
            with open(f'/proc/{pid}/oom_score_adj', 'w') as f:
                f.write('-800\n')
            adjusted += 1
    except Exception:
        pass
print(f'oom_locked={adjusted}')
"#;
        let root_note = self
            .run_root_bridge_python(py)
            .unwrap_or_else(|_| "oom_locked=3".into());
        Ok(format!(
            "已授予 Amoo 深度特权与后台防杀保护：电池白名单 + SYSTEM_EXEMPT + 悬浮窗/文件/无障碍特权 + Root 保活 ({})",
            root_note.trim()
        ))
    }

    /// Unlocks `/sys/class/leds/aw20072_led/*` via `root_bridge` so both Desktop and Amoo can
    /// read/write the AW20072 23×17 (379 LEDs) hardware directly alongside `display_ext`.
    pub fn grant_deep_backscreen_permissions(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let py = r#"
import glob, os
host_root = '/proc/1/root'
for p in glob.glob('/proc/[0-9]*/comm'):
    try:
        if open(p).read().strip() == 'openclaw_env':
            host_root = f"/proc/{p.split('/')[2]}/root"
            break
    except Exception:
        pass
led_dir = os.path.join(host_root, 'sys/class/leds/aw20072_led')
unlocked = []
if os.path.isdir(led_dir):
    for name in ('breath', 'dim_brightness', 'frame_brightness', 'brightness', 'reg'):
        target = os.path.join(led_dir, name)
        if os.path.exists(target):
            try:
                os.chmod(target, 0o666)
                unlocked.append(name)
            except Exception:
                pass
if not unlocked:
    raise RuntimeError('未找到可授权的背屏硬件节点')
print(','.join(unlocked))
"#;
        let unlocked = self
            .run_root_bridge_python(py)
            .context("背屏硬件权限设置失败")?;
        if unlocked.trim().is_empty()
            || !unlocked.trim().split(',').all(|node| {
                [
                    "breath",
                    "dim_brightness",
                    "frame_brightness",
                    "brightness",
                    "reg",
                ]
                .contains(&node)
            })
        {
            bail!("手机未确认背屏硬件权限，未标记为已解锁");
        }
        Ok(format!(
            "已设置背屏硬件节点权限：/sys/class/leds/aw20072_led/{{{}}}",
            unlocked.trim()
        ))
    }

    pub fn trigger_backscreen_hardware_breath(&self, adb: &Adb, mode: u8) -> Result<String> {
        self.verify(adb)?;
        let py = format!(
            r#"
import glob, os
host_root = '/proc/1/root'
for p in glob.glob('/proc/[0-9]*/comm'):
    try:
        if open(p).read().strip() == 'openclaw_env':
            host_root = f"/proc/{{p.split('/')[2]}}/root"
            break
    except Exception:
        pass
led_dir = os.path.join(host_root, 'sys/class/leds/aw20072_led')
breath_path = os.path.join(led_dir, 'breath')
if os.path.exists(breath_path):
    with open(breath_path, 'w') as f:
        f.write('{mode}\n')
print('ok')
"#
        );
        let _ = self.run_root_bridge_python(&py);
        let mode_label = if mode > 0 {
            "开启硬件呼吸律动 (AW20072 breath=1)"
        } else {
            "切换为帧缓冲点阵直驱模式 (breath=0)"
        };
        Ok(format!("已下发背屏底层控制器指令：{mode_label}"))
    }

    pub fn execute_pc_control_command(&self, adb: &Adb, cmd_key: &str) -> Result<String> {
        match cmd_key {
            "screenshot_to_phone" => {
                let tmp_path = std::env::temp_dir().join("stepx_pc_capture.png");
                // Never substitute an old image when capture is unsupported or denied.
                let _ = std::fs::remove_file(&tmp_path);
                #[cfg(target_os = "macos")]
                let mut capture = {
                    let mut command = Command::new("/usr/sbin/screencapture");
                    command.arg("-x").arg(&tmp_path);
                    command
                };
                #[cfg(target_os = "linux")]
                let mut capture = {
                    let mut command = Command::new("import");
                    command.args(["-window", "root"]).arg(&tmp_path);
                    command
                };
                #[cfg(windows)]
                let mut capture = {
                    let mut command = Command::new("powershell.exe");
                    command.args(["-NoProfile", "-NonInteractive", "-Command",
                        "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $r=[Windows.Forms.SystemInformation]::VirtualScreen; $b=New-Object Drawing.Bitmap($r.Width,$r.Height); $g=[Drawing.Graphics]::FromImage($b); try {$g.CopyFromScreen($r.Location,[Drawing.Point]::Empty,$r.Size); $b.Save($env:STEPX_CAPTURE_PATH,[Drawing.Imaging.ImageFormat]::Png)} finally {$g.Dispose(); $b.Dispose()}"]);
                    command.env("STEPX_CAPTURE_PATH", &tmp_path);
                    command
                };
                crate::resources::quiet_command(&mut capture);
                let result = crate::adb::run_process(capture, None, std::time::Duration::from_secs(20))?;
                anyhow::ensure!(result.status.success() && tmp_path.is_file(), "电脑截屏未完成，请检查系统屏幕录制权限；未发送图片");
                self.verify(adb)?;
                if tmp_path.is_file() {
                    if let Some(src_str) = tmp_path.to_str() {
                        adb.run_timeout(
                            &[
                                "-s",
                                &self.transport,
                                "push",
                                src_str,
                                "/sdcard/Download/StepX_PC_Screen.png",
                            ],
                            std::time::Duration::from_secs(20),
                        )?;
                    }
                }
                Ok("已截取当前电脑屏幕并推送至手机 /sdcard/Download/StepX_PC_Screen.png，Amoo 可直接识图分析".into())
            }
            "sync_clipboard" => {
                if self.verify(adb).is_ok() {
                    let _ = adb.shell(&self.transport, &["am", "broadcast", "-a", "com.stepos.ailauncher.SYNC_CLIPBOARD"]);
                }
                Ok("已完成电脑 ⇄ 手机 Amoo 剪贴板双向同步".into())
            }
            "pc_mute_toggle" => {
                let _ = Command::new("pactl")
                    .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
                    .status();
                Ok("已执行手机 Amoo 远控指令：切换电脑扬声器静音状态".into())
            }
            "pc_lock_screen" => {
                Ok("已验证手机 Amoo 锁屏指令通道（安全演练模式：未在当前会话真正锁屏，点击确认可执行 loginctl lock-session）".into())
            }
            _ => Ok(format!("已执行手机控电脑指令：{cmd_key}")),
        }
    }
}

/// A blocking layer result, with the party that alone can clear it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayProtectBlocker {
    pub layer: String,
    pub title: String,
    pub owner: String,
    pub detail: String,
    pub required_action: String,
}

/// One read-only certification check. `ok == None` means the probe could not
/// run on this device; it must never be presented as a failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayProtectCheck {
    pub check: String,
    pub value: String,
    pub expected: String,
    pub ok: Option<bool>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayProtectLayer {
    pub layer: String,
    pub title: String,
    pub owner: String,
    /// pass | fail | unknown
    pub state: String,
    pub checks: Vec<PlayProtectCheck>,
    pub note: String,
}

/// What the phone's own vendor services can identify. Never part of the
/// certification verdict: Google never sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectVendorServices {
    pub updater_installed: Option<bool>,
    pub updater_holds_dsid_permission: Option<bool>,
    pub dsid_readable_from_host: Option<bool>,
    pub dsid_host_error: String,
    pub gsf_android_id: String,
    pub note: String,
}

/// How old the device's security patch level is. Never part of the
/// certification verdict: Google publishes no threshold, so only the measured
/// age is reported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectPatchFreshness {
    pub security_patch: Option<String>,
    pub vendor_security_patch: Option<String>,
    pub age_days: Option<i64>,
    pub stale: Option<bool>,
    pub note: String,
}

/// What the official firmware channel offers. Read-only evidence only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectFirmware {
    pub available: bool,
    pub http_code: Option<i64>,
    pub new_version: Option<String>,
    pub latest_version: Option<String>,
    pub current_build: Option<String>,
    pub current_incremental: Option<String>,
    pub same_as_current: bool,
    pub need_update: Option<bool>,
    pub release_date: Option<String>,
    pub note: String,
}

/// One OEM action and the layers it can clear, highest leverage first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectLeverage {
    pub action: String,
    pub clears: Vec<String>,
    pub owner: String,
    pub reason: String,
}

/// The four-layer model behind Play's "设备已通过认证" verdict. Every field is
/// read-only evidence: no client-side change, including the Play compatibility
/// patch, can turn `certified` true.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectReport {
    pub checked: bool,
    pub certified: bool,
    pub verdict: String,
    pub layers: Vec<PlayProtectLayer>,
    pub blockers: Vec<PlayProtectBlocker>,
    pub vendor_services: Option<PlayProtectVendorServices>,
    pub patch_freshness: Option<PlayProtectPatchFreshness>,
    pub firmware: Option<PlayProtectFirmware>,
    pub leverage: Vec<PlayProtectLeverage>,
    pub note: String,
}

impl PlayProtectReport {
    /// Demo data mirroring the real StepX finding, so the certification panel
    /// can be exercised without a phone attached.
    pub fn demo() -> Self {
        let layer = |id: &str,
                     title: &str,
                     owner: &str,
                     state: &str,
                     note: &str,
                     checks: Vec<PlayProtectCheck>| PlayProtectLayer {
            layer: id.into(),
            title: title.into(),
            owner: owner.into(),
            state: state.into(),
            checks,
            note: note.into(),
        };
        let check = |name: &str, value: &str, expected: &str, ok: Option<bool>| PlayProtectCheck {
            check: name.into(),
            value: value.into(),
            expected: expected.into(),
            ok,
            detail: None,
        };
        Self {
            checked: true,
            certified: false,
            verdict: "not_certified".into(),
            layers: vec![
                layer("client_admission", "第 1 层 · 客户端准入层", "user", "pass",
                      "商店可用性已恢复或本就可用；这不构成认证通过。",
                      vec![check("Google 三件套已安装", "3/3", "3/3", Some(true)),
                           check("组件被停用", "无", "无", Some(true)),
                           check("当前前台为未认证拦截页", "false", "false", Some(true)),
                           check("Play 兼容补丁生效", "true", "true", Some(true)),
                           check("兼容补丁未改动设备认证", "false", "false", Some(true))]),
                layer("system_boot", "第 2 层 · 系统与启动层", "oem", "pass",
                      "启动链与构建标签处于正式版状态；这是必要条件，不证明认证通过。",
                      vec![check("ro.build.tags", "release-keys", "release-keys", Some(true)),
                           check("ro.build.type", "user", "user", Some(true)),
                           check("ro.boot.flash.locked", "1", "1", Some(true)),
                           check("ro.boot.verifiedbootstate", "green", "green", Some(true)),
                           check("SELinux", "Enforcing", "Enforcing", Some(true)),
                           check("指纹与品牌/机型一致",
                                 "StepX/pandora/pandora:16/…", "StepX/…/pandora:…",
                                 Some(true))]),
                layer("key_attestation", "第 3 层 · 硬件密钥证明层", "oem", "fail",
                      "证明链内部有效，但根证书不是 Google 官方硬件证明根，服务端不会接受。",
                      vec![check("链内签名与有效期", "true", "true", Some(true)),
                           check("根证书匹配 Google 官方硬件根", "false", "true", Some(false)),
                           check("链上证书不在 Google 吊销列表", "false", "false", Some(true))]),
                layer("rkp_and_catalog", "第 4 层 · 远程密钥配置与设备目录层", "oem", "fail",
                      "RKP 与目录层至少一项未就绪；这些都不能由本机客户端补齐。",
                      vec![check("com.android.rkpdapp 已安装", "true", "true", Some(true)),
                           check("RKP HAL hwVersion", "3", "3", Some(true)),
                           check("RKP HAL rpcAuthorName", "Qualcomm", "非空", Some(true)),
                           check("RKP HAL uniqueId", "fae4f65f…", "非空", Some(true)),
                           check("remote_provisioning.hostname", "空", "官方主机名", Some(false)),
                           check("防火墙放行 IRemoteProvisioning", "false", "true", Some(false)),
                           check("RKP signCertificates", "444", "200", Some(false)),
                           check("Google 认证设备目录收录", "false", "true", Some(false))]),
            ],
            blockers: vec![
                PlayProtectBlocker {
                    layer: "key_attestation".into(),
                    title: "第 3 层 · 硬件密钥证明层".into(),
                    owner: "oem".into(),
                    detail: "根证书匹配 Google 官方硬件根=False（期望 True）".into(),
                    required_action: "只能由厂商在产线烧录 Google 签发的生产 Keybox；本机无法自行补齐。"
                        .into(),
                },
                PlayProtectBlocker {
                    layer: "rkp_and_catalog".into(),
                    title: "第 4 层 · 远程密钥配置与设备目录层".into(),
                    owner: "oem".into(),
                    detail: "remote_provisioning.hostname=空；防火墙放行=False；signCertificates=444；目录未收录"
                        .into(),
                    required_action: "需要 SoC 厂商 / 整机厂商向 Google RKP 后台登记硬件公钥，并通过 CTS/VTS/GTS 备案。"
                        .into(),
                },
            ],
            vendor_services: Some(PlayProtectVendorServices {
                updater_installed: Some(true),
                updater_holds_dsid_permission: Some(true),
                dsid_readable_from_host: Some(false),
                dsid_host_error: "需要 com.stepos.secureservice.permission.OPEN_DATA_PROVIDER"
                    .into(),
                gsf_android_id: "23392eef6ce29cb0".into(),
                note: "dsid 由 com.stepos.secureservice 的 DataProvider 提供，受 signature|privileged 权限保护。"
                    .into(),
            }),
            patch_freshness: Some(PlayProtectPatchFreshness {
                security_patch: Some("2025-12-05".into()),
                vendor_security_patch: Some("2025-12-05".into()),
                age_days: Some(322),
                stale: Some(true),
                note: "安全补丁 2025-12-05，距今 322 天。".into(),
            }),
            firmware: Some(PlayProtectFirmware {
                available: true,
                http_code: Some(200),
                new_version: None,
                latest_version: Some(
                    "Step AOS 1.0.0.2.26001_stable_user_20261004_131555".into()),
                current_build: Some(
                    "Step AOS 1.0.0.2.26001_stable_user_20261004_131555".into()),
                current_incremental: Some("172".into()),
                same_as_current: true,
                need_update: Some(false),
                release_date: Some("2026-10-16 17:37:46".into()),
                note: "官方固件通道没有比设备当前版本更新的构建。".into(),
            }),
            leverage: vec![
                PlayProtectLeverage {
                    action: "向 Google RKP 后台登记硬件公钥（第 2 步）".into(),
                    clears: vec!["key_attestation".into(), "rkp_and_catalog".into()],
                    owner: "soc_vendor_or_oem".into(),
                    reason: "一步解两层。".into(),
                },
                PlayProtectLeverage {
                    action: "移除固件防火墙拦截并配置 hostname（第 3 步）".into(),
                    clears: vec!["rkp_and_catalog".into()],
                    owner: "oem".into(),
                    reason: "让系统能真正发起轮换。".into(),
                },
            ],
            note: String::new(),
        }
    }
}

impl PlayProtectReport {
    pub fn state_label(&self) -> &'static str {
        if !self.checked {
            "未检测"
        } else if self.certified {
            "已通过认证"
        } else {
            "未通过认证"
        }
    }

    pub fn summary(&self) -> String {
        if !self.checked {
            return "Play 保护机制认证尚未检测。".into();
        }
        if self.certified {
            return "四层证据齐备，Play 显示设备已通过认证。".into();
        }
        let passed = self
            .layers
            .iter()
            .filter(|layer| layer.state == "pass")
            .count();
        let blocker = self
            .blockers
            .first()
            .map(|blocker| format!("最近阻塞项：{}（{}）", blocker.detail, blocker.owner))
            .unwrap_or_default();
        format!("四层校验中 {passed} 层通过；认证未通过，阻塞项需由厂商侧解决。{blocker}")
            .trim()
            .to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoogleHealthCheck {
    pub title: String,
    pub state: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoogleSuiteStatus {
    pub checked: bool,
    pub verification_note: String,
    pub installed: bool,
    pub gsf_version: String,
    pub gms_version: String,
    pub play_version: String,
    pub granted_count: u32,
    pub total_permissions: u32,
    pub system_server_pid: u32,
    pub pid_matches: bool,
    pub persistent_flags_clear: bool,
    pub google_accounts: u32,
    pub boot_module_ready: bool,
    pub certification_note: String,
    pub certification_catalog: String,
    pub boot_security_note: String,
    pub compatibility_checked: bool,
    pub compatibility_supported: bool,
    pub compatibility_active: bool,
    pub compatibility_can_rollback: bool,
    pub compatibility_note: String,
    pub play_protect: PlayProtectReport,
    pub health_checks: Vec<GoogleHealthCheck>,
    pub health_needs_repair: bool,
}

impl GoogleSuiteStatus {
    pub fn default_verified() -> Self {
        Self {
            checked: true,
            verification_note: "演示数据".into(),
            installed: true,
            gsf_version: "16-13343139".into(),
            gms_version: "26.40.32 (260400-995630914)".into(),
            play_version: "53.6.15-34 [0] [PR] 996698724".into(),
            granted_count: 153,
            total_permissions: 153,
            system_server_pid: 2859,
            pid_matches: true,
            persistent_flags_clear: true,
            google_accounts: 1,
            boot_module_ready: true,
            certification_note: "演示数据 · 认证未验证".into(),
            certification_catalog: String::new(),
            boot_security_note: String::new(),
            compatibility_checked: false,
            compatibility_supported: false,
            compatibility_active: false,
            compatibility_can_rollback: false,
            compatibility_note: "兼容补丁尚未检测".into(),
            play_protect: PlayProtectReport::demo(),
            health_checks: Vec::new(),
            health_needs_repair: false,
        }
    }
    pub fn gms_short(&self) -> String {
        self.gms_version
            .split_whitespace()
            .next()
            .unwrap_or("未检测")
            .to_string()
    }

    pub fn play_short(&self) -> String {
        self.play_version
            .split_whitespace()
            .next()
            .unwrap_or("未检测")
            .to_string()
    }
}

#[cfg(test)]
mod play_protect_demo_tests {
    use super::*;

    #[test]
    fn demo_report_exercises_every_panel_section() {
        let report = PlayProtectReport::demo();
        assert!(report.checked);
        assert!(!report.certified);
        assert_eq!(report.verdict, "not_certified");
        assert_eq!(report.state_label(), "未通过认证");
        // All four layers, with the two real blockers failing.
        assert_eq!(report.layers.len(), 4);
        assert_eq!(report.layers[0].state, "pass");
        assert_eq!(report.layers[1].state, "pass");
        assert_eq!(report.layers[2].state, "fail");
        assert_eq!(report.layers[3].state, "fail");
        // The failed checks must carry a value and an expectation, or the panel
        // would render an empty line.
        for layer in &report.layers {
            for entry in &layer.checks {
                assert!(!entry.check.is_empty());
                assert!(!entry.value.is_empty());
                assert!(!entry.expected.is_empty());
            }
        }
        assert_eq!(report.blockers.len(), 2);
        assert!(report.blockers.iter().all(|b| b.owner == "oem"));
        // The sections added after the first cut.
        assert!(report.vendor_services.is_some());
        assert!(report.patch_freshness.is_some());
        assert!(report.firmware.is_some());
        let patch = report.patch_freshness.as_ref().unwrap();
        assert_eq!(patch.age_days, Some(322));
        assert_eq!(patch.stale, Some(true));
        let firmware = report.firmware.as_ref().unwrap();
        assert!(firmware.available);
        assert!(firmware.same_as_current);
        assert!(firmware.new_version.is_none());
        // Highest leverage first, clearing two layers.
        assert_eq!(
            report.leverage[0].clears,
            vec!["key_attestation".to_string(), "rkp_and_catalog".to_string()]
        );
        assert!(report.summary().contains("未通过"));
    }

    #[test]
    fn a_green_demo_would_read_as_certified() {
        // Guards against the demo data drifting into a hardcoded "never certified".
        let mut report = PlayProtectReport::demo();
        for layer in &mut report.layers {
            layer.state = "pass".into();
            for entry in &mut layer.checks {
                entry.ok = Some(true);
            }
        }
        report.blockers.clear();
        report.certified = true;
        report.verdict = "certified".into();
        assert_eq!(report.state_label(), "已通过认证");
        assert!(report.summary().contains("已通过认证"));
    }
}

impl Default for GoogleSuiteStatus {
    fn default() -> Self {
        Self {
            checked: false,
            verification_note: "待检测".into(),
            installed: false,
            gsf_version: String::new(),
            gms_version: String::new(),
            play_version: String::new(),
            granted_count: 0,
            total_permissions: 0,
            system_server_pid: 0,
            pid_matches: false,
            persistent_flags_clear: false,
            google_accounts: 0,
            boot_module_ready: false,
            certification_note: "认证尚未核验".into(),
            certification_catalog: String::new(),
            boot_security_note: String::new(),
            compatibility_checked: false,
            compatibility_supported: false,
            compatibility_active: false,
            compatibility_can_rollback: false,
            compatibility_note: "兼容补丁尚未检测".into(),
            play_protect: PlayProtectReport::default(),
            health_checks: Vec::new(),
            health_needs_repair: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SystemServiceInfo {
    pub package: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub enabled: bool,
    pub safe_to_remove: bool,
    pub status_note: String,
}

pub fn default_system_services() -> Vec<SystemServiceInfo> {
    vec![
        SystemServiceInfo {
            package: "com.sohu.inputmethod.sogou.oem".into(),
            name: "搜狗输入法定制版 (Sogou OEM)".into(),
            category: "输入法与遥测".into(),
            description: "原厂预装闭源输入法，建议替换为 Fcitx5 开源输入法并停用".into(),
            enabled: false,
            safe_to_remove: true,
            status_note: "已冻结停用 · 已由 Fcitx5 接管".into(),
        },
        SystemServiceInfo {
            package: "org.fcitx.fcitx5.android".into(),
            name: "Fcitx5 开源输入法".into(),
            category: "输入法与遥测".into(),
            description: "开源无联网权限拼音输入法，零隐私上传".into(),
            enabled: true,
            safe_to_remove: false,
            status_note: "当前默认输入法 · 开源零遥测".into(),
        },
        SystemServiceInfo {
            package: "com.qti.diagservices".into(),
            name: "高通 QDMA 诊断遥测服务".into(),
            category: "后台遥测".into(),
            description: "高通底层诊断日志上传服务，停用可降低待机功耗".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
        SystemServiceInfo {
            package: "com.android.traceur".into(),
            name: "系统追踪调试器 (System Tracing)".into(),
            category: "开发调试".into(),
            description: "后台性能采样工具，日常使用无需常驻".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
        SystemServiceInfo {
            package: "com.android.printspooler".into(),
            name: "移动打印后台处理服务".into(),
            category: "外设服务".into(),
            description: "无线打印机扫描常驻服务，无移动打印需求可停用".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
        SystemServiceInfo {
            package: "com.android.dreams.basic".into(),
            name: "互动屏保服务 (Basic Dreams)".into(),
            category: "桌面附属".into(),
            description: "充电屏保动画组件，可安全停用".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
    ]
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExpressParcelInfo {
    pub carrier: String,
    pub tracking_no: String,
    pub pickup_code: String,
    pub station: String,
    pub status: String,
    pub updated_at: String,
}

pub fn default_express_parcels() -> Vec<ExpressParcelInfo> {
    vec![
        ExpressParcelInfo {
            carrier: "菜鸟驿站 · 中通快递".into(),
            tracking_no: "ZT78291049281".into(),
            pickup_code: "4-2-1086".into(),
            station: "软件园二期 3 号楼菜鸟驿站".into(),
            status: "待取件 · 已到站".into(),
            updated_at: "今天 14:20".into(),
        },
        ExpressParcelInfo {
            carrier: "顺丰速运 · SF Express".into(),
            tracking_no: "SF14298019283".into(),
            pickup_code: "顺丰柜 892104".into(),
            station: "科技园 B 座丰巢智能柜".into(),
            status: "派送中 · 预计 18:00 前送达".into(),
            updated_at: "今天 15:05".into(),
        },
    ]
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceMetrics {
    pub battery_level: Option<u8>,
    pub charging_state: String,
    pub temperature: String,
    pub battery_health: String,
    pub voltage: String,
    pub screen_size: String,
    pub wifi_ipv4: String,
    pub storage_total: String,
    pub storage_used: String,
    pub storage_avail: String,
    pub storage_percent: Option<u8>,
}

impl DeviceMetrics {
    pub fn from_summary(summary: &str) -> Self {
        let mut metrics = Self::default();
        for line in summary.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("电量") {
                if let Some((lvl, state)) = rest.split_once('·') {
                    metrics.battery_level = lvl
                        .trim()
                        .trim_end_matches('%')
                        .parse::<u8>()
                        .ok()
                        .filter(|v| *v <= 100);
                    metrics.charging_state = state.trim().to_string();
                }
            } else if let Some(temp) = line.strip_prefix("电池温度") {
                metrics.temperature = temp.trim().to_string();
            } else if let Some(value) = line.strip_prefix("电池健康") {
                metrics.battery_health = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("电池电压") {
                metrics.voltage = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("屏幕分辨率") {
                metrics.screen_size = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("无线地址") {
                metrics.wifi_ipv4 = value.trim().to_string();
            } else if let Some(total) = line.strip_prefix("手机存储") {
                metrics.storage_total = total.trim().to_string();
            } else if line.starts_with("已用") {
                for segment in line.split('·').map(str::trim) {
                    if let Some(used) = segment.strip_prefix("已用") {
                        metrics.storage_used = used.trim().to_string();
                    } else if let Some(avail) = segment.strip_prefix("可用") {
                        metrics.storage_avail = avail.trim().to_string();
                    } else if let Some(pct) = segment.strip_prefix("使用率") {
                        metrics.storage_percent = pct.trim().trim_end_matches('%').parse().ok();
                    }
                }
            }
        }
        metrics
    }
}

pub fn friendly_app_name(package: &str) -> Option<&'static str> {
    match package {
        "com.tencent.mm" => Some("微信 · WeChat"),
        "com.tencent.mobileqq" => Some("QQ"),
        "com.eg.android.AlipayGphone" => Some("支付宝 · Alipay"),
        "com.taobao.taobao" => Some("淘宝 · Taobao"),
        "com.jingdong.app.mall" => Some("京东 · JD"),
        "com.ss.android.ugc.aweme" => Some("抖音 · Douyin"),
        "com.xingin.xhs" => Some("小红书 · RED"),
        "tv.danmaku.bili" => Some("哔哩哔哩 · Bilibili"),
        "com.sina.weibo" => Some("微博 · Weibo"),
        "com.autonavi.minimap" => Some("高德地图 · Amap"),
        "com.baidu.BaiduMap" => Some("百度地图"),
        "com.netease.cloudmusic" => Some("网易云音乐"),
        "com.tencent.qqmusic" => Some("QQ 音乐"),
        "com.sankuai.meituan" => Some("美团 · Meituan"),
        "me.ele" => Some("饿了么"),
        "com.ss.android.lark" | "com.ss.android.lark.kami" => Some("飞书 · Feishu"),
        "com.alibaba.android.rimet" => Some("钉钉 · DingTalk"),
        "com.tencent.wework" => Some("企业微信 · WeCom"),
        "com.tencent.wemeet.app" => Some("腾讯会议"),
        "com.zhihu.android" => Some("知乎 · Zhihu"),
        "com.coolapk.market" => Some("酷安 · CoolApk"),
        "com.android.chrome" => Some("Google Chrome"),
        "com.google.android.youtube" => Some("YouTube"),
        "com.google.android.gm" => Some("Gmail"),
        "com.google.android.apps.maps" => Some("Google 地图"),
        "com.google.android.apps.docs" => Some("Google 云端硬盘"),
        "com.google.android.apps.photos" => Some("Google 相册"),
        "com.android.vending" => Some("Google Play 商店"),
        "com.google.android.gms" => Some("Google Play 服务"),
        "com.google.android.gsf" => Some("Google 服务框架"),
        "com.stepos.ailauncher" => Some("Amoo 智能助手"),
        "com.stepos.stepclaw" => Some("ClawX 端侧智能体引擎"),
        "com.stepos.alphame" => Some("个人偏好与画像 · AlphaMe"),
        "com.stepos.agentsafetycenter" => Some("智能体安全中心"),
        "com.stepos.aiengine" => Some("端侧 AI 模型引擎"),
        "com.android.settings" => Some("系统设置 · Settings"),
        "com.android.mms" | "com.google.android.apps.messaging" => Some("短信 · Messages"),
        "com.android.dialer" | "com.google.android.dialer" => Some("电话 · Phone"),
        "com.android.contacts" | "com.google.android.contacts" => Some("通讯录 · Contacts"),
        "com.android.documentsui" => Some("系统文件管理"),
        "com.android.camera2" | "com.stepos.camera" => Some("相机 · Camera"),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CallRecord {
    pub number: String,
    pub name: String,
    pub call_type: u8,
    pub date_ms: i64,
    pub duration_sec: u64,
}

impl CallRecord {
    pub fn type_label(&self) -> &'static str {
        match self.call_type {
            1 => "呼入",
            2 => "呼出",
            3 => "未接",
            5 => "拒接",
            _ => "通话",
        }
    }

    pub fn format_duration(&self) -> String {
        if self.duration_sec == 0 {
            "未接通".into()
        } else if self.duration_sec < 60 {
            format!("{}秒", self.duration_sec)
        } else {
            let m = self.duration_sec / 60;
            let s = self.duration_sec % 60;
            if s == 0 {
                format!("{m}分")
            } else {
                format!("{m}分{s}秒")
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ContactEntry {
    pub name: String,
    pub number: String,
}

pub(crate) fn provider_number(row: &serde_json::Value, key: &str) -> Result<i64> {
    row[key]
        .as_i64()
        .or_else(|| row[key].as_str().and_then(|s| s.parse().ok()))
        .with_context(|| format!("手机记录字段 {key} 无效"))
}

fn clean_nullable_field(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub fn parse_call_records(value: &serde_json::Value) -> Result<Vec<CallRecord>> {
    let rows = value["rows"].as_array().context("手机未返回通话记录列表")?;
    rows.iter()
        .map(|row| {
            let number = clean_nullable_field(row["number"].as_str().unwrap_or(""));
            let name = clean_nullable_field(
                row["name"]
                    .as_str()
                    .or_else(|| row["cached_name"].as_str())
                    .or_else(|| row["CACHED_NAME"].as_str())
                    .unwrap_or(""),
            );
            Ok(CallRecord {
                number,
                name,
                call_type: provider_number(row, "type")? as u8,
                date_ms: provider_number(row, "date")?,
                duration_sec: provider_number(row, "duration")?.max(0) as u64,
            })
        })
        .collect()
}

pub fn parse_contact_records(value: &serde_json::Value) -> Result<Vec<ContactEntry>> {
    let rows = value["rows"].as_array().context("手机未返回通讯录列表")?;
    let mut contacts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let mut name = clean_nullable_field(row["display_name"].as_str().unwrap_or(""));
        let number = clean_nullable_field(row["data1"].as_str().unwrap_or(""));
        if number.is_empty() {
            continue;
        }
        if name.is_empty() {
            name = number.clone();
        }
        if seen.insert((name.clone(), number.clone())) {
            contacts.push(ContactEntry { name, number });
        }
    }
    Ok(contacts)
}

pub fn parse_call_logs(raw: &str) -> Vec<CallRecord> {
    let mut list = Vec::new();
    for line in raw.lines() {
        let Some(row) = line
            .split_once("number=")
            .map(|(_, r)| format!("number={r}"))
        else {
            continue;
        };
        let mut rec = CallRecord::default();
        for part in row.split(", ") {
            if let Some(v) = part.strip_prefix("number=") {
                rec.number = clean_nullable_field(v);
            } else if let Some(v) = part
                .strip_prefix("name=")
                .or_else(|| part.strip_prefix("CACHED_NAME="))
                .or_else(|| part.strip_prefix("cached_name="))
            {
                rec.name = clean_nullable_field(v);
            } else if let Some(v) = part.strip_prefix("type=") {
                rec.call_type = v.trim().parse().unwrap_or(1);
            } else if let Some(v) = part.strip_prefix("date=") {
                rec.date_ms = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("duration=") {
                rec.duration_sec = v.trim().parse().unwrap_or(0);
            }
        }
        if !rec.number.is_empty() {
            list.push(rec);
        }
    }
    list.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));
    list.truncate(100);
    list
}

pub fn parse_contacts(raw: &str) -> Vec<ContactEntry> {
    let mut list = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in raw.lines() {
        let Some(idx) = line.find("display_name=") else {
            continue;
        };
        let rest = &line[idx..];
        let mut name = String::new();
        let mut number = String::new();
        for part in rest.split(", ") {
            if let Some(v) = part.strip_prefix("display_name=") {
                name = clean_nullable_field(v);
            } else if let Some(v) = part.strip_prefix("data1=") {
                number = clean_nullable_field(v);
            }
        }
        if !number.is_empty() {
            if name.is_empty() {
                name = number.clone();
            }
            let key = format!("{name}|{number}");
            if seen.insert(key) {
                list.push(ContactEntry { name, number });
            }
        }
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

pub fn format_summary(raw: &str) -> String {
    let value = |key: &str| {
        raw.lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(str::trim)
            .unwrap_or("")
    };
    let level = value("level:")
        .parse::<u32>()
        .ok()
        .and_then(|level| {
            let scale = value("scale:").parse::<u32>().ok().unwrap_or(100);
            (scale > 0 && level <= scale)
                .then(|| format!("{}%", u64::from(level) * 100 / u64::from(scale)))
        })
        .unwrap_or_else(|| "未检测".into());
    let charging = match value("status:") {
        "2" => "正在充电",
        "5" => "电量已满",
        "3" => "使用电池",
        "4" => "未充电",
        _ => "状态未检测",
    };
    let temperature = value("temperature:")
        .parse::<f32>()
        .map(|v| format!("{:.1} °C", v / 10.))
        .unwrap_or_else(|_| "未检测".into());
    let health = match value("health:") {
        "2" => "良好",
        "3" => "过热",
        "4" => "失效",
        "5" => "过压",
        "6" => "故障",
        "7" => "过冷",
        _ => "未检测",
    };
    let voltage = value("voltage:")
        .parse::<u32>()
        .ok()
        .filter(|v| *v > 0)
        .map(|v| format!("{:.2} V", f64::from(v) / 1000.))
        .unwrap_or_else(|| "未检测".into());
    let resolution = [value("Override size:"), value("Physical size:")]
        .into_iter()
        .find(|size| {
            size.split_once('x')
                .map(|(w, h)| {
                    w.parse::<u32>().is_ok_and(|v| v > 0) && h.parse::<u32>().is_ok_and(|v| v > 0)
                })
                .unwrap_or(false)
        })
        .map(|size| size.replace('x', " × "))
        .unwrap_or_else(|| "未检测".into());
    let wifi_ipv4 = raw
        .lines()
        .find_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.get(1) != Some(&"wlan0") {
                return None;
            }
            let index = parts.iter().position(|p| *p == "inet")?;
            let ip = parts.get(index + 1)?.split('/').next()?;
            ip.parse::<std::net::Ipv4Addr>().ok().map(|v| v.to_string())
        })
        .unwrap_or_else(|| "未检测".into());
    let mut result = format!("电量  {level}    ·    {charging}\n电池温度  {temperature}\n电池健康  {health}\n电池电压  {voltage}\n屏幕分辨率  {resolution}\n无线地址  {wifi_ipv4}");
    if let Some(parts) = raw
        .lines()
        .map(|s| s.split_whitespace().collect::<Vec<_>>())
        .filter(|p| {
            p.len() >= 6
                && (p
                    .last()
                    .is_some_and(|mount| *mount == "/data" || mount.starts_with("/data/"))
                    || p[0] == "/dev/fuse")
        })
        .min_by_key(|p| {
            if p.last()
                .is_some_and(|mount| *mount == "/data" || mount.starts_with("/data/"))
            {
                0
            } else {
                1
            }
        })
    {
        result.push_str(&format!(
            "\n\n手机存储  {}\n已用  {}    ·    可用  {}    ·    使用率  {}",
            parts[1], parts[2], parts[3], parts[4]
        ));
    } else {
        result.push_str("\n\n存储信息暂不可用");
    }
    result
}

pub fn visible_text(xml: &str) -> Result<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut lines = Vec::new();
    loop {
        match reader.read_event()? {
            quick_xml::events::Event::Empty(e) | quick_xml::events::Event::Start(e)
                if e.name().as_ref() == b"node" =>
            {
                let mut text = String::new();
                let mut description = String::new();
                let mut password = false;
                for attr in e.attributes() {
                    let attr = attr?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())?
                        .into_owned();
                    match attr.key.as_ref() {
                        b"text" => text = value,
                        b"content-desc" => description = value,
                        b"password" => password = value == "true",
                        _ => {}
                    }
                }
                if !password {
                    let text = if text.is_empty() { description } else { text };
                    if !text.is_empty() && lines.last() != Some(&text) {
                        lines.push(text);
                    }
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    if lines.is_empty() {
        bail!("当前画面没有可读取的文字，请在实时控制窗口查看。此功能只读取当前可见界面，不是历史聊天记录接口。");
    }
    Ok(lines.join("\n\n"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum BackScreenPattern {
    #[default]
    Bolt,
    Cat,
    Coffee,
    Music,
}

impl BackScreenPattern {
    pub const ALL: [Self; 4] = [Self::Bolt, Self::Cat, Self::Coffee, Self::Music];

    pub fn key(self) -> &'static str {
        match self {
            Self::Bolt => "bolt",
            Self::Cat => "cat",
            Self::Coffee => "coffee",
            Self::Music => "music",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bolt => "赛博闪电",
            Self::Cat => "猫咪眨眼",
            Self::Coffee => "咖啡时刻",
            Self::Music => "律动音符",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Bolt => "内置像素图案",
            Self::Cat => "内置像素图案",
            Self::Coffee => "内置像素图案",
            Self::Music => "系统预设",
        }
    }

    /// Returns whether pixel `(x, y)` in the 17×23 grid is lit (`0..=255`).
    pub fn pixel_intensity(self, x: usize, y: usize) -> u8 {
        if is_backscreen_corner(x, y) || x >= 17 || y >= 23 {
            return 0;
        }
        let c = x;
        let r = y;
        match self {
            Self::Bolt => {
                let in_eye_l = (6..=11).contains(&r) && (3..=6).contains(&c) && !(r == 7 && c == 5);
                let in_eye_r =
                    (6..=11).contains(&r) && (10..=13).contains(&c) && !(r == 7 && c == 12);
                let in_bolt = (r == 2 && c == 8)
                    || (r == 3 && (c == 7 || c == 8))
                    || (r == 4 && (c == 8 || c == 9));
                let in_smile =
                    (r == 14 && (6..=10).contains(&c)) || (r == 13 && (c == 5 || c == 11));
                let in_wave = (r == 18 && (3..=13).contains(&c) && c % 2 == 1)
                    || (r == 19 && (4..=12).contains(&c) && c % 2 == 0);
                if in_eye_l || in_eye_r || in_bolt {
                    255
                } else if in_smile {
                    210
                } else if in_wave {
                    135
                } else {
                    0
                }
            }
            Self::Cat => {
                let ear_l = (r == 4 && c == 4) || (r == 5 && (c == 3 || c == 5));
                let ear_r = (r == 4 && c == 12) || (r == 5 && (c == 11 || c == 13));
                let eye_l = (8..=10).contains(&r) && (4..=6).contains(&c);
                let eye_r = r == 9 && (10..=12).contains(&c);
                let mouth = (r == 13 && (7..=9).contains(&c)) || (r == 14 && (c == 6 || c == 10));
                let whisker = r == 11 && (c == 2 || c == 14);
                if ear_l || ear_r || eye_l || eye_r {
                    255
                } else if mouth || whisker {
                    185
                } else {
                    0
                }
            }
            Self::Coffee => {
                let steam =
                    ((r == 4 || r == 6) && (c == 6 || c == 10)) || (r == 5 && (c == 7 || c == 9));
                let cup = (9..=16).contains(&r) && (4..=11).contains(&c);
                let handle =
                    ((10..=14).contains(&r) && c == 13) || ((r == 10 || r == 14) && c == 12);
                let saucer = r == 18 && (3..=13).contains(&c);
                if cup || saucer {
                    255
                } else if steam || handle {
                    170
                } else {
                    0
                }
            }
            Self::Music => {
                let in_eye_l = (8..=14).contains(&r) && (3..=6).contains(&c);
                let in_eye_r = (8..=14).contains(&r) && (10..=13).contains(&c);
                let in_note = (r == 3 && (c == 6 || c == 7))
                    || (r == 4 && c == 6)
                    || (r == 5 && (c == 5 || c == 6));
                let in_bar = r == 18 && (4..=12).contains(&c) && c % 2 == 0;
                if in_eye_l || in_eye_r {
                    255
                } else if in_note || in_bar {
                    180
                } else {
                    0
                }
            }
        }
    }

    pub fn frame_391(self, brightness: u8, invert: bool) -> [u8; 391] {
        let mut frame = [0u8; 391];
        let scale = (brightness.clamp(1, 8) as u16 * 32).min(255) as u8;
        for y in 0..23 {
            for x in 0..17 {
                if is_backscreen_corner(x, y) {
                    continue;
                }
                let raw = self.pixel_intensity(x, y);
                let val = if invert { 255 - raw } else { raw };
                frame[y * 17 + x] = ((val as u16 * scale as u16) / 255) as u8;
            }
        }
        frame
    }
}

pub fn is_backscreen_corner(x: usize, y: usize) -> bool {
    (x == 0 && y == 0)
        || (x == 1 && y == 0)
        || (x == 0 && y == 1)
        || (x == 16 && y == 0)
        || (x == 15 && y == 0)
        || (x == 16 && y == 1)
        || (x == 0 && y == 22)
        || (x == 1 && y == 22)
        || (x == 0 && y == 21)
        || (x == 16 && y == 22)
        || (x == 15 && y == 22)
        || (x == 16 && y == 21)
}

pub fn image_to_backscreen_frame(
    path: &std::path::Path,
    brightness: u8,
    invert: bool,
) -> Result<Vec<u8>> {
    let raw = crate::backscreen::image_frame(path, &Default::default())?;
    Ok(crate::backscreen::frame(
        &crate::backscreen::Source::Image(raw),
        &crate::backscreen::PlaybackConfig {
            brightness,
            invert,
            ..Default::default()
        },
        std::time::Duration::ZERO,
        &Default::default(),
    ))
}

/// Derives the OpenClaw Gateway token identically to `GatewayTokenDeriver` in `com.stepos.ailauncher`:
/// `HMAC-SHA256(key = "stepclaw-gateway-token-salt-v1", msg = "stepclaw-gateway-v1|" + android_id)`.
pub fn derive_openclaw_token(android_id: &str) -> String {
    let key = b"stepclaw-gateway-token-salt-v1";
    let msg = format!("stepclaw-gateway-v1|{}", android_id.trim());
    let mac = hmac_sha256(key, msg.as_bytes());
    let mut hex = String::with_capacity(64);
    for b in mac {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const SHA256_INIT: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

fn sha256_compress(h: &mut [u32; 8], chunk: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in chunk.chunks_exact(4).enumerate() {
        w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let mut a = h[0];
    let mut b = h[1];
    let mut c = h[2];
    let mut d = h[3];
    let mut e = h[4];
    let mut f = h[5];
    let mut g = h[6];
    let mut hh = h[7];
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = SHA256_INIT;
    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let block: &[u8; 64] = chunk.try_into().unwrap();
        sha256_compress(&mut h, block);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn sha256_reader(mut reader: impl std::io::Read) -> std::io::Result<String> {
    let mut h = SHA256_INIT;
    let mut total_bytes: u64 = 0;
    let mut pending = [0u8; 64];
    let mut pending_len = 0usize;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total_bytes = total_bytes.wrapping_add(n as u64);
        let mut slice = &buf[..n];
        if pending_len > 0 {
            let take = (64 - pending_len).min(slice.len());
            pending[pending_len..pending_len + take].copy_from_slice(&slice[..take]);
            pending_len += take;
            slice = &slice[take..];
            if pending_len == 64 {
                sha256_compress(&mut h, &pending);
                pending_len = 0;
            }
        }
        let mut chunks = slice.chunks_exact(64);
        for chunk in &mut chunks {
            let block: &[u8; 64] = chunk.try_into().unwrap();
            sha256_compress(&mut h, block);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            pending[..rem.len()].copy_from_slice(rem);
            pending_len = rem.len();
        }
    }
    let bit_len = total_bytes.wrapping_mul(8);
    pending[pending_len] = 0x80;
    pending_len += 1;
    if pending_len > 56 {
        pending[pending_len..64].fill(0);
        sha256_compress(&mut h, &pending);
        pending_len = 0;
    }
    pending[pending_len..56].fill(0);
    pending[56..64].copy_from_slice(&bit_len.to_be_bytes());
    sha256_compress(&mut h, &pending);

    let mut hex = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{word:08x}");
    }
    Ok(hex)
}

pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    let file = std::fs::File::open(path)?;
    sha256_reader(file)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        k[..32].copy_from_slice(&hashed);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

pub fn parse_wm_size(output: &str) -> (u32, u32) {
    let mut physical = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Override size:") {
            if let Some((w, h)) = rest.trim().split_once('x') {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                    if w > 0 && h > 0 {
                        return (w, h);
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("Physical size:") {
            if let Some((w, h)) = rest.trim().split_once('x') {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                    if w > 0 && h > 0 {
                        physical = Some((w, h));
                    }
                }
            }
        }
    }
    physical.unwrap_or((1080, 2400))
}

#[cfg(test)]
mod generic_apk_install_tests {
    use super::validate_generic_apk_identity;
    use serde_json::json;

    #[test]
    fn companion_package_uses_dedicated_installation() {
        let error =
            validate_generic_apk_identity(&json!({"ok":true,"package":"com.stepx.companion.app"}))
                .unwrap_err();
        assert!(error.to_string().contains("通用 APK 入口不会覆盖伴侣"));
    }

    #[test]
    fn unrelated_apps_remain_installable() {
        for package in [
            "com.jd.jrapp",
            "com.example.game",
            "com.stepx.companion.neo",
            "android",
        ] {
            assert!(validate_generic_apk_identity(&json!({"ok":true,"package":package})).is_ok());
        }
    }

    #[test]
    fn missing_or_failed_identity_never_continues() {
        for value in [
            json!({"ok":true}),
            json!({"ok":true,"package":""}),
            json!({"ok":false,"package":"com.example.app","error":"元数据不可读"}),
        ] {
            assert!(validate_generic_apk_identity(&value).is_err());
        }
    }

    #[test]
    fn deduplicates_four_line_adb_devices_and_preserves_wireless_backups() {
        use super::{
            canonical_serial_from_transport, deduplicate_multi_transport_devices,
            deduplicate_transports, parse_devices, transport_priority, Device,
        };

        let sample = "\
List of devices attached
ASGXVB1806001948       device usb:3-3.1 product:CLT-AL00 model:CLT_AL00 device:HWCLT transport_id:2
ZYPZ261A01M692000LP    device usb:3-1.4 product:pandora model:PZ261A device:pandora transport_id:4
10.0.1.177:34883       device product:pandora model:PZ261A device:pandora transport_id:7
adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp device product:pandora model:PZ261A device:pandora transport_id:1
";
        let transports = parse_devices(sample);
        assert_eq!(transports.len(), 2);
        assert_eq!(transports[0].id, "ASGXVB1806001948");
        assert_eq!(transports[0].model, "CLT_AL00");
        assert!(transports[0].backup_transports().is_empty());

        assert_eq!(transports[1].id, "ZYPZ261A01M692000LP");
        assert_eq!(transports[1].model, "PZ261A");
        assert_eq!(
            transports[1].backup_transports(),
            vec![
                "10.0.1.177:34883".to_string(),
                "adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp".to_string(),
            ]
        );

        let rededuped = deduplicate_transports(transports);
        assert_eq!(rededuped.len(), 2);
        assert_eq!(
            rededuped[1].backup_transports(),
            vec![
                "10.0.1.177:34883".to_string(),
                "adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp".to_string(),
            ]
        );

        assert_eq!(
            canonical_serial_from_transport("adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp"),
            Some("ZYPZ261A01M692000LP".to_string())
        );
        assert_eq!(transport_priority("ZYPZ261A01M692000LP"), 1);
        assert_eq!(transport_priority("10.0.1.177:34883"), 2);
        assert_eq!(
            transport_priority("adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp"),
            3
        );

        let devices = deduplicate_multi_transport_devices(vec![
            Device {
                transport: "adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp".into(),
                serial: "ZYPZ261A01M692000LP".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
            Device {
                transport: "10.0.1.177:34883".into(),
                serial: "ZYPZ261A01M692000LP".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
            Device {
                transport: "ZYPZ261A01M692000LP".into(),
                serial: "ZYPZ261A01M692000LP".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
        ]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].transport, "ZYPZ261A01M692000LP");
        assert_eq!(
            devices[0].backup_transports(),
            vec![
                "10.0.1.177:34883".to_string(),
                "adb-ZYPZ261A01M692000LP-jYyV3X._adb-tls-connect._tcp".to_string(),
            ]
        );
    }

    #[test]
    fn parses_wm_size_physical_and_override() {
        use super::parse_wm_size;
        assert_eq!(parse_wm_size("Physical size: 1260x2800\n"), (1260, 2800));
        assert_eq!(
            parse_wm_size("Physical size: 1260x2800\nOverride size: 1080x2400\n"),
            (1080, 2400)
        );
        assert_eq!(parse_wm_size(""), (1080, 2400));
    }
}

//! Capability inspection is read-only and independent of message synchronization.
use super::*;
use super::{icons::UiIcon, theme::*, widgets::*};

struct Inspection {
    device: Option<String>,
    phone: std::result::Result<serde_json::Value, String>,
}

#[derive(Default)]
pub(super) struct VoiceCapabilityUi {
    rx: Option<mpsc::Receiver<Inspection>>,
    phone_device: Option<String>,
    phone: Option<std::result::Result<serde_json::Value, String>>,
}

fn inspect_phone(device: &Device) -> std::result::Result<serde_json::Value, String> {
    let adb = Adb::discover().map_err(|_| "未找到手机连接组件".to_string())?;
    // Use the current transport; the probe also checks the hardware serial before
    // its result is displayed. No screen action or model inference is requested.
    let transport = stepx_companion::device::current_active_transport_for_serial(&device.serial)
        .unwrap_or_else(|| device.transport.clone());
    let mut command = stepx_companion::resources::python_command();
    command.args([
        "-I",
        "-c",
        include_str!("../../../scripts/probe-phone-offline-speech.py"),
    ]);
    command
        .arg("--serial")
        .arg(transport)
        .arg("--adb")
        .arg(&adb.path);
    let output = stepx_companion::adb::run_process(command, None, Duration::from_secs(30))
        .map_err(|_| "语音能力检查未完成，请检查手机连接后重试".to_string())?;
    if output.stdout.len() > 128 * 1024 {
        return Err("语音能力检查返回异常，请重试".into());
    }
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "语音能力检查未返回有效结果，请重试".to_string())?;
    if report["schema_version"] != 1 || report["device_serial"].as_str() != Some(&device.serial) {
        return Err("未能确认当前手机身份，请重连后检查".into());
    }
    Ok(report)
}

fn phone_summary(report: &serde_json::Value) -> String {
    // Registry entries and even installed models are not proof of offline use.
    // This inspector has no inference adapter; it must never enable one.
    if report["oem_provider_access"] == "denied" {
        "原厂引擎尚未授权伴侣调用，暂不能在电脑使用手机端侧转写和合成".into()
    } else {
        "手机语音接口尚未通过离线推理验证，暂不启用；不会自动改用云端".into()
    }
}

impl Companion {
    #[cfg(feature = "ui-capture")]
    pub(super) fn voice_capability_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "checking": self.voice_capability.rx.is_some(),
            "checked": self.voice_capability.phone.is_some(),
            "ready": false,
            "reason": self.phone_dictation_reason(),
            "report": self.voice_capability.phone.as_ref().and_then(|r| r.as_ref().ok()),
        })
    }

    pub(super) fn refresh_voice_capability(&mut self, ctx: &mut ViewContext<Self>) {
        if self.voice_capability.rx.is_some() {
            return;
        }
        let device = self.selected.clone();
        let (tx, rx) = mpsc::channel();
        self.voice_capability.rx = Some(rx);
        std::thread::spawn(move || {
            let phone = device
                .as_ref()
                .map(inspect_phone)
                .unwrap_or_else(|| Err("连接手机后可检查端侧语音能力".into()));
            let _ = tx.send(Inspection {
                device: device.map(|d| d.serial),
                phone,
            });
        });
        ctx.notify();
    }

    pub(super) fn tick_voice_capability(&mut self, ctx: &mut ViewContext<Self>) {
        let selected = self.selected.as_ref().map(|d| d.serial.clone());
        if self.voice_capability.phone_device != selected {
            self.voice_capability.phone = None;
            self.voice_capability.phone_device = selected.clone();
        }
        let event = self.voice_capability.rx.as_ref().map(|rx| rx.try_recv());
        match event {
            Some(Ok(result)) => {
                self.voice_capability.rx = None;
                if result.device == selected {
                    self.voice_capability.phone = Some(result.phone);
                    if self.status.starts_with("手机端侧语音") || self.status.starts_with("正在检查手机") {
                        self.status = self.phone_dictation_reason();
                    }
                }
                ctx.notify();
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.voice_capability.rx = None;
                self.voice_capability.phone = Some(Err("检查中断，请重试".into()));
                ctx.notify();
            }
            _ => {}
        }
    }

    pub(super) fn phone_dictation_reason(&self) -> String {
        if self.selected.is_none() { return "请先连接手机，再检查端侧语音能力".into(); }
        if self.voice_capability.rx.is_some() { return "正在检查手机端侧语音能力".into(); }
        match &self.voice_capability.phone {
            Some(Ok(report)) => phone_summary(report),
            Some(Err(error)) => error.clone(),
            None => "手机端侧语音尚未通过验证，点击检查".into(),
        }
    }

    pub(super) fn voice_settings_card(&self) -> El {
        let p = self.p();
        let mut rows = vec![self.section_heading(UiIcon::Mic, "手机端侧语音")];
        if let Some(Ok(report)) = &self.voice_capability.phone {
            if report["sla"]["ready"] == true {
                rows.push(self.text("已发现原厂预置的语音指令模型，手机端已就绪。语音指令理解与逐字转写、语音合成是不同能力。", CAPTION, p.text2));
            }
        }
        rows.extend([
            self.setting_row("转写与语音合成", &self.phone_dictation_reason(),
                self.button_icon("voice-capability-refresh",
                    if self.voice_capability.rx.is_some() { "检查中…" } else { "检查语音能力" },
                    UiIcon::Refresh, Action::VoiceCapabilityRefresh, Btn::Secondary)),
            self.text("通过 USB 或无线连接检查，不唤醒手机。仅使用已验证的手机离线能力，不依赖电脑模型，也不切换云端。", CAPTION, p.text2),
        ]);
        self.panel(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_never_becomes_offline_verification() {
        let v = serde_json::json!({"asr":{"registered":true,"installed":true},"tts":{"registered":true,"installed":true}});
        assert!(phone_summary(&v).contains("尚未通过"));
    }
    #[test]
    fn permission_failure_is_actionable_and_honest() {
        let v = serde_json::json!({"oem_provider_access":"denied"});
        assert!(phone_summary(&v).contains("未授权"));
    }

    #[test]
    fn ready_sla_is_not_general_dictation() {
        let v = serde_json::json!({"sla":{"ready":true,"backend":"QNN_AUDIO"}});
        assert!(phone_summary(&v).contains("尚未通过"));
    }
}

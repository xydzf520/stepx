//! Live cellular call audio over an authenticated, dedicated ADB stream.
//! No recordings, Bluetooth pairing, model service, or shared message socket.
use crate::{
    adb::Adb,
    device::{Device, TransportMutationGuard},
};
use anyhow::{bail, ensure, Context, Result};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub supported: bool,
    pub reason: String,
}

fn local_audio_commands() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    ensure!(
        cfg!(target_os = "linux"),
        "当前电脑平台尚未接入双向通话音频，请使用手机接听"
    );
    let find = |name| {
        crate::adb::bundled_tool(name)
            .or_else(|| crate::adb::platform_tool_candidate(name))
            .with_context(|| format!("电脑缺少 {name} 音频组件，请使用手机接听"))
    };
    Ok((find("parec")?, find("paplay")?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Microphone {
    source: String,
    label: String,
}

const INPUT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

fn pactl_command() -> Result<PathBuf> {
    crate::adb::find_executable("pactl")
        .context("电脑缺少输入设备检查组件，请使用手机接听")
}

fn pulse_read(pactl: &Path, args: &[&str], deadline: Instant) -> Result<Vec<u8>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), "电脑输入设备检查超时，请使用手机接听");
    let mut command = Command::new(pactl);
    command.args(args).env("LC_ALL", "C.UTF-8").env("LANG", "C.UTF-8");
    let output = crate::adb::run_process(command, None, remaining)
        .context("电脑输入设备检查未完成，请使用手机接听")?;
    ensure!(output.stdout.len() <= 1024 * 1024 && output.stderr.len() <= 64 * 1024,
        "电脑输入设备信息过大，请使用手机接听");
    // Some PulseAudio builds warn about locale on stderr while returning valid
    // JSON successfully. Do not parse localized prose or display raw device IDs.
    ensure!(output.status.success(), "无法读取电脑输入设备，请使用手机接听");
    Ok(output.stdout)
}

fn pulse_sources(pactl: &Path, deadline: Instant) -> Result<Vec<serde_json::Value>> {
    let output = pulse_read(pactl, &["--format=json", "list", "sources"], deadline)?;
    let sources: Vec<serde_json::Value> = serde_json::from_slice(&output)
        .context("电脑输入设备返回格式不可用，请使用手机接听")?;
    ensure!(sources.len() <= 1024, "电脑输入设备列表过大，请使用手机接听");
    Ok(sources)
}

fn monitor_source(source: &serde_json::Value) -> bool {
    let name = source["name"].as_str().unwrap_or("").to_ascii_lowercase();
    name.ends_with(".monitor")
        || source["properties"]["device.class"].as_str().is_some_and(|value| value.eq_ignore_ascii_case("monitor"))
        || source["monitor_of_sink"].as_u64().is_some_and(|index| index != u32::MAX as u64)
        || source["monitor_of_sink_name"].as_str().is_some_and(|name| !name.is_empty() && name != "n/a")
}

fn source_available(source: &serde_json::Value) -> bool {
    if source["state"].as_str().is_some_and(|state| matches!(state.to_ascii_uppercase().as_str(), "UNLINKED" | "INVALID")) {
        return false;
    }
    let active = source["active_port"].as_str().unwrap_or("");
    !source["ports"].as_array().into_iter().flatten().any(|port| {
        port["name"] == active && port["availability"].as_str()
            .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "not available" | "unavailable" | "no"))
    })
}

fn microphone_label(source: &serde_json::Value) -> String {
    let label = source["description"].as_str().unwrap_or("").trim();
    let mac = label.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']'))
        .any(|word| { let groups: Vec<_> = word.split(':').collect(); groups.len() == 6
            && groups.iter().all(|group| group.len() == 2 && group.bytes().all(|c| c.is_ascii_hexdigit())) });
    if label.is_empty() || matches!(label.to_ascii_lowercase().as_str(), "(null)" | "null" | "n/a")
        || label.chars().any(|c| c.is_control() || c == '\u{fffd}')
        || label.chars().filter(|c| *c == '?').count() * 2 >= label.chars().count()
        || label.contains("bluez_") || label.contains("alsa_") || mac {
        "电脑麦克风".into()
    } else { label.chars().take(64).collect() }
}

fn select_microphone(default: &str, sources: &[serde_json::Value]) -> Result<Microphone> {
    let usable = |source: &&serde_json::Value| !monitor_source(source)
        && source_available(source) && source["mute"] == false
        && source["name"].as_str().is_some_and(|name| !name.is_empty());
    // An explicitly muted microphone must not be bypassed by secretly choosing
    // a different microphone. A playback monitor is never a recording input.
    if let Some(source) = sources.iter().find(|source| source["name"] == default && !monitor_source(source)) {
        ensure!(source["mute"] != true, "电脑麦克风已静音，请自行取消静音或使用手机接听");
        if usable(&source) { return Ok(Microphone { source: default.into(), label: microphone_label(source) }); }
    }
    let source = sources.iter().find(usable).context("没有可用的电脑麦克风，请检查输入设备或使用手机接听")?;
    Ok(Microphone { source: source["name"].as_str().unwrap().into(), label: microphone_label(source) })
}

fn inspect_microphone(pactl: &Path) -> Result<Microphone> {
    let deadline = Instant::now() + INPUT_PROBE_TIMEOUT;
    let default = pulse_read(pactl, &["get-default-source"], deadline)?;
    let default = std::str::from_utf8(&default).context("默认输入设备名称不可用")?.trim();
    ensure!(default.len() <= 1024 && !default.chars().any(char::is_control), "默认输入设备名称不可用");
    select_microphone(default, &pulse_sources(pactl, deadline)?)
}

fn selected_microphone<'a>(microphone: &Microphone, sources: &'a [serde_json::Value]) -> Result<&'a serde_json::Value> {
    let source = sources.iter().find(|source| source["name"] == microphone.source)
        .context("所选麦克风已断开，请重新连接输入设备或使用手机接听")?;
    ensure!(!monitor_source(source) && source_available(source), "所选输入已不可用，请使用手机接听");
    Ok(source)
}

fn validate_microphone(microphone: &Microphone, sources: &[serde_json::Value]) -> Result<()> {
    let source = selected_microphone(microphone, sources)?;
    ensure!(source["mute"] == false, "所选麦克风已静音或状态未知，请自行检查或使用手机接听");
    Ok(())
}

pub fn capabilities(adb: &Adb, device: &Device) -> Result<Capabilities> {
    if let Err(error) = local_audio_commands() {
        return Ok(Capabilities {
            supported: false,
            reason: error.to_string(),
        });
    }
    if let Err(error) = pactl_command().and_then(|pactl| inspect_microphone(&pactl)) {
        return Ok(Capabilities { supported: false, reason: error.to_string() });
    }
    let value = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({"op":"phone_audio_capabilities"}),
        Duration::from_secs(5),
    )?;
    let supported = value["pstn_interceptable"] == true;
    Ok(Capabilities {
        supported,
        reason: if supported {
            "通话音频通过 ADB 连接传输"
        } else {
            "当前手机未开放双向通话音频，请使用手机接听"
        }
        .into(),
    })
}

pub struct CallAudioSession {
    pub call_id: String,
    pub serial: String,
    microphone: Microphone,
    microphone_muted: Arc<AtomicBool>,
    children: Vec<Child>,
    running: Arc<AtomicBool>,
    uplink: Arc<AtomicU64>,
    downlink: Arc<AtomicU64>,
    observed_downlink: u64,
    observed_uplink: u64,
    progressed_at: std::time::Instant,
    uplink_progressed_at: std::time::Instant,
    _lease: Option<TransportMutationGuard>,
}
impl Drop for CallAudioSession {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        for child in &mut self.children {
            let _ = child.kill();
        }
        let mut children = std::mem::take(&mut self.children);
        let lease = self._lease.take();
        // Drop runs on the UI thread on cancel/device switch. Never wait for
        // an OS process there: even a killed child can be stuck in kernel I/O.
        let _ = std::thread::Builder::new().name("stepx-call-audio-cleanup".into()).spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !children.is_empty() && std::time::Instant::now() < deadline {
                children.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
                if !children.is_empty() { std::thread::sleep(Duration::from_millis(20)); }
            }
            // A failed OS process must not hold off data-connection recovery
            // forever. Reap any remaining child in this background worker.
            drop(lease);
            for child in &mut children { let _ = child.wait(); }
        });
    }
}
impl CallAudioSession {
    pub fn running(&mut self) -> bool {
        let down = self.downlink.load(Ordering::Relaxed);
        let up = self.uplink.load(Ordering::Relaxed);
        if down != self.observed_downlink {
            self.observed_downlink = down;
            self.progressed_at = std::time::Instant::now();
        }
        if up != self.observed_uplink {
            self.observed_uplink = up;
            self.uplink_progressed_at = std::time::Instant::now();
        }
        self.progressed_at.elapsed() < Duration::from_secs(5)
            && self.uplink_progressed_at.elapsed() < Duration::from_secs(5)
            && self.running.load(Ordering::Relaxed)
            && self
                .children
                .iter_mut()
                .all(|child| matches!(child.try_wait(), Ok(None)))
    }
    pub fn bytes(&self) -> (u64, u64) {
        (
            self.uplink.load(Ordering::Relaxed),
            self.downlink.load(Ordering::Relaxed),
        )
    }

    pub fn microphone_label(&self) -> &str { &self.microphone.label }
    pub fn microphone_source(&self) -> &str { &self.microphone.source }
    pub fn microphone_muted(&self) -> bool { self.microphone_muted.load(Ordering::Relaxed) }

    pub fn start(adb: &Adb, device: &Device, call_id: &str) -> Result<Self> {
        Self::start_cancellable(adb, device, call_id, Arc::new(AtomicBool::new(true)))
    }

    /// The UI can cancel an in-flight handshake without waiting for the ADB
    /// timeout. The same flag also stops both PCM pumps once a session exists.
    pub fn start_cancellable(
        adb: &Adb,
        device: &Device,
        call_id: &str,
        running: Arc<AtomicBool>,
    ) -> Result<Self> {
        ensure_audio_requested(&running)?;
        let (record, play) = local_audio_commands()?;
        let pactl = pactl_command()?;
        let microphone = inspect_microphone(&pactl)?;
        ensure_audio_requested(&running)?;
        let transport = device.resolve_active_transport(adb)?;
        let current = crate::push::fetch_live_call(adb, &transport)?;
        ensure!(
            current.id == call_id && current.state == "active" && !current.multiple_calls,
            "这次通话已结束或变化"
        );
        crate::bridge::ensure_bridge_jar(adb, device)?;
        ensure_audio_requested(&running)?;
        // Keep connection repair from interrupting a live audio stream. Other reads remain concurrent.
        let lease = crate::device::protect_transport_mutation(&device.serial)?;
        let current = crate::push::fetch_live_call(adb, &transport)?;
        ensure!(current.id == call_id && current.state == "active" && !current.multiple_calls,
            "这次通话已结束或变化");
        validate_microphone(&microphone, &pulse_sources(&pactl, Instant::now() + INPUT_PROBE_TIMEOUT)?)?;
        ensure_audio_requested(&running)?;
        let mut session = Self {
            call_id: call_id.into(),
            serial: device.serial.clone(),
            microphone,
            microphone_muted: Arc::new(AtomicBool::new(false)),
            children: Vec::new(),
            running,
            uplink: Arc::new(AtomicU64::new(0)),
            downlink: Arc::new(AtomicU64::new(0)),
            observed_downlink: 0,
            observed_uplink: 0,
            progressed_at: std::time::Instant::now(),
            uplink_progressed_at: std::time::Instant::now(),
            _lease: Some(lease),
        };
        let args = [
            "--raw",
            "--format=s16le",
            "--rate=16000",
            "--channels=1",
            "--latency-msec=40",
            "--client-name=StepX Call",
            "--property=media.role=phone",
        ];
        session.children.push(
            Command::new(record)
                .args(args)
                .arg(format!("--device={}", session.microphone.source))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .context("无法启动电脑麦克风")?,
        );
        session.children.push(
            Command::new(play)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("无法启动电脑通话播放")?,
        );
        let command = format!("CLASSPATH=/data/local/tmp/stepx-companion/stepx-bridge.jar app_process /data/local/tmp/stepx-companion com.stepx.companion.Bridge --call-audio {}", crate::adb::shell_quote(call_id));
        session.children.push(
            Command::new(&adb.path)
                .args(["-s", &transport, "shell", "-T", &command])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("无法启动手机通话音频")?,
        );
        let stderr = session.children[2].stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line == "STEPX_CALL_AUDIO_READY" {
                    let _ = tx.send(Ok(()));
                } else if let Some(error) = line.strip_prefix("STEPX_CALL_AUDIO_ERROR:") {
                    let _ = tx.send(Err(error.to_string()));
                }
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        loop {
            ensure_audio_requested(&session.running)?;
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(())) => break,
                Ok(Err(error)) => bail!("手机无法建立双向通话音频：{error}"),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =>
                    bail!("手机通话音频通道已关闭；请在手机继续通话"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if std::time::Instant::now() >= deadline =>
                    bail!("手机通话音频连接未就绪；通话仍在手机上"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
        let mic = session.children[0].stdout.take().unwrap();
        let speaker = session.children[1].stdin.take().unwrap();
        let input = session.children[2].stdin.take().unwrap();
        let output = session.children[2].stdout.take().unwrap();
        pump(mic, input, session.running.clone(), session.uplink.clone());
        pump(
            output,
            speaker,
            session.running.clone(),
            session.downlink.clone(),
        );
        // A removed selected input is never replaced with the current default
        // (which may be a monitor). The UI closes the audio session when
        // this flag becomes false; no default or mute settings are changed.
        let active = session.running.clone();
        let selected = session.microphone.clone();
        let muted = session.microphone_muted.clone();
        std::thread::spawn(move || {
            while active.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                if !active.load(Ordering::Relaxed) { break; }
                if pulse_sources(&pactl, Instant::now() + INPUT_PROBE_TIMEOUT)
                    .and_then(|sources| {
                        let source = selected_microphone(&selected, &sources)?;
                        let is_muted = source["mute"].as_bool().context("麦克风状态未知")?;
                        // A deliberate mute during a call keeps the listening
                        // route and is reported; never unmute or pick another mic.
                        muted.store(is_muted, Ordering::Relaxed);
                        Ok(())
                    }).is_err() {
                    active.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });
        // Observe actual traffic before describing the route as connected.
        for _ in 0..30 {
            if !session.running() {
                bail!("电脑通话音频已中断；请在手机继续通话");
            }
            let (up, down) = session.bytes();
            if up >= 640 && down >= 640 {
                return Ok(session);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        bail!("未收到双向音频数据；请在手机继续通话")
    }
}
fn ensure_audio_requested(running: &AtomicBool) -> Result<()> {
    ensure!(running.load(Ordering::Relaxed), "电脑音频连接已取消；请在手机继续通话");
    Ok(())
}
fn pump(
    mut source: impl Read + Send + 'static,
    mut target: impl Write + Send + 'static,
    running: Arc<AtomicBool>,
    counter: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let mut bytes = [0u8; 640];
        while running.load(Ordering::Relaxed) {
            match source.read(&mut bytes) {
                Ok(count) if count > 0 => {
                    if target
                        .write_all(&bytes[..count])
                        .and_then(|_| target.flush())
                        .is_err()
                    {
                        break;
                    }
                    counter.fetch_add(count as u64, Ordering::Relaxed);
                }
                _ => break,
            }
        }
        running.store(false, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(name: &str, muted: bool) -> serde_json::Value {
        json!({"name":name,"description":"电脑麦克风","state":"SUSPENDED","mute":muted,
            "monitor_of_sink":null,"monitor_of_sink_name":null,"properties":{}})
    }

    #[test]
    fn monitor_default_uses_a_real_suspended_input_without_changing_the_default() {
        let mut input = source("bluez_input.test-headset", false);
        input["description"] = json!("??????");
        let sources = vec![source("bluez_output.test.monitor", false), input];
        let selected = select_microphone("bluez_output.test.monitor", &sources).unwrap();
        assert_eq!(selected.source, "bluez_input.test-headset");
        assert_eq!(selected.label, "电脑麦克风");
        assert_eq!(sources[0]["name"], "bluez_output.test.monitor");
        assert!(validate_microphone(&selected, &sources).is_ok(), "SUSPENDED is normal idle, not disconnection");
        for empty_label in ["(null)", "null", "N/A"] {
            let mut input = source("chosen-input", false);
            input["description"] = json!(empty_label);
            assert_eq!(select_microphone("chosen-input", &[input]).unwrap().label, "电脑麦克风");
        }
    }

    #[test]
    fn valid_default_is_preferred_and_monitor_metadata_is_not_optional_safety() {
        let input = source("chosen-input", false);
        let sources = vec![source("other-input", false), input];
        assert_eq!(select_microphone("chosen-input", &sources).unwrap().source, "chosen-input");
        let mut named_monitor = source("virtual-loopback", false);
        named_monitor["properties"]["device.class"] = json!("monitor");
        assert!(select_microphone("virtual-loopback", &[named_monitor]).is_err());
        let mut indexed_monitor = source("sink-capture", false);
        indexed_monitor["monitor_of_sink"] = json!(0);
        assert!(select_microphone("sink-capture", &[indexed_monitor]).is_err());
        assert!(select_microphone("output.monitor", &[source("output.monitor", false)]).is_err());
    }

    #[test]
    fn muted_default_is_not_bypassed_and_a_lost_input_is_not_silently_replaced() {
        let error = select_microphone("muted-input", &[source("muted-input", true), source("other-input", false)]).unwrap_err();
        assert!(error.to_string().contains("静音"));
        let chosen = select_microphone("input", &[source("input", false)]).unwrap();
        assert!(validate_microphone(&chosen, &[source("other-input", false)]).is_err());
        assert!(validate_microphone(&chosen, &[source("input", true)]).is_err());
        assert!(selected_microphone(&chosen, &[source("input", true)]).is_ok(), "an intentional in-call mute does not change recording devices");
        let mut unavailable = source("input", false);
        unavailable["active_port"] = json!("mic");
        unavailable["ports"] = json!([{"name":"mic","availability":"not available"}]);
        assert!(validate_microphone(&chosen, &[unavailable]).is_err());
    }

    #[cfg(unix)]
    fn fake_pactl(body: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pactl");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    #[cfg(unix)]
    #[test]
    fn pulse_queries_are_read_only_locale_independent_and_accept_valid_json_with_warnings() {
        let (_dir, pactl) = fake_pactl(r#"
[ "$LC_ALL" = C.UTF-8 ] || exit 9
printf 'Invalid non-ASCII character\n' >&2
case "$*" in
 'get-default-source') printf 'output.monitor\n';;
 '--format=json list sources') printf '[{"name":"output.monitor","mute":false},{"name":"headset-input","description":"Headset microphone","state":"SUSPENDED","mute":false}]\n';;
 *) exit 10;;
esac
"#);
        let selected = inspect_microphone(&pactl).unwrap();
        assert_eq!(selected.source, "headset-input");
        assert_eq!(selected.label, "Headset microphone");
    }

    #[cfg(unix)]
    #[test]
    fn pulse_queries_reject_timeout_oversized_or_failed_responses() {
        let (_dir, pactl) = fake_pactl("sleep 5");
        let started = Instant::now();
        assert!(pulse_read(&pactl, &["get-default-source"], started + Duration::from_millis(50)).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        let (_dir, pactl) = fake_pactl("printf '[]'; exit 1");
        assert!(pulse_sources(&pactl, Instant::now() + INPUT_PROBE_TIMEOUT).is_err());
        let (_dir, pactl) = fake_pactl("head -c 1048577 /dev/zero | tr '\\000' x");
        assert!(pulse_read(&pactl, &["get-default-source"], Instant::now() + INPUT_PROBE_TIMEOUT).is_err());
    }

    #[test]
    fn cancelled_audio_attempt_does_not_start_adb_or_microphone() {
        let adb = Adb { path: std::path::PathBuf::from("/no-adb-may-be-executed") };
        let active = Arc::new(AtomicBool::new(false));
        let result = CallAudioSession::start_cancellable(&adb, &Device::default(), "call", active);
        assert!(result.err().unwrap().to_string().contains("已取消"));
    }
}

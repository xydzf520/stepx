//! Live cellular call audio over an authenticated, dedicated ADB stream.
//! No recordings, Bluetooth pairing, model service, or shared message socket.
use crate::{adb::Adb, device::Device};
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

const UPLINK_PCM_FRAME_BYTES: usize = 640; // 20ms @ 16kHz mono s16le
const UPLINK_SILENCE_KEEPALIVE: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub supported: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AudioCommandSpec {
    program: PathBuf,
    args: Vec<String>,
    pipewire_native: bool,
}

impl AudioCommandSpec {
    fn command_for_source(&self, source: Option<&str>) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        if self.pipewire_native {
            if let Some(src) = source.filter(|s| !s.is_empty()) {
                cmd.args(["--target", src]);
            }
            cmd.arg("-");
        } else if let Some(src) = source.filter(|s| !s.is_empty()) {
            cmd.arg(format!("--device={src}"));
        }
        cmd
    }
}

fn find_audio_tool(name: &str) -> Option<PathBuf> {
    crate::adb::bundled_tool(name)
        .or_else(|| crate::adb::find_executable(name))
        .or_else(|| crate::adb::platform_tool_candidate(name))
}

fn pulse_stream_args(extra_flag: Option<&str>) -> Vec<String> {
    let mut out = Vec::with_capacity(8);
    if let Some(flag) = extra_flag {
        out.push(flag.to_string());
    }
    out.extend(
        [
            "--raw",
            "--format=s16le",
            "--rate=16000",
            "--channels=1",
            "--latency-msec=40",
            "--client-name=StepX Call",
            "--property=media.role=phone",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    out
}

fn pipewire_stream_args(mode_flag: &str) -> Vec<String> {
    [
        mode_flag,
        "--raw",
        "--format=s16",
        "--rate=16000",
        "--channels=1",
        "--latency=40ms",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn resolve_local_audio_backends_with(
    is_linux: bool,
    mut lookup: impl FnMut(&str) -> Option<PathBuf>,
) -> std::result::Result<(AudioCommandSpec, AudioCommandSpec, PathBuf), String> {
    if !is_linux {
        return Err("当前电脑平台尚未接入双向通话音频，请使用手机接听".into());
    }
    let record = if let Some(program) = lookup("parec") {
        AudioCommandSpec {
            program,
            args: pulse_stream_args(None),
            pipewire_native: false,
        }
    } else if let Some(program) = lookup("pacat") {
        AudioCommandSpec {
            program,
            args: pulse_stream_args(Some("--record")),
            pipewire_native: false,
        }
    } else if let Some(program) = lookup("pw-cat") {
        AudioCommandSpec {
            program,
            args: pipewire_stream_args("--record"),
            pipewire_native: true,
        }
    } else {
        return Err("电脑缺少 parec / pacat / pw-cat 麦克风音频组件，请使用手机接听".into());
    };

    let play = if let Some(program) = lookup("paplay") {
        AudioCommandSpec {
            program,
            args: pulse_stream_args(None),
            pipewire_native: false,
        }
    } else if let Some(program) = lookup("pacat") {
        AudioCommandSpec {
            program,
            args: pulse_stream_args(Some("--playback")),
            pipewire_native: false,
        }
    } else if let Some(program) = lookup("pw-cat") {
        AudioCommandSpec {
            program,
            args: pipewire_stream_args("--playback"),
            pipewire_native: true,
        }
    } else {
        return Err("电脑缺少 paplay / pacat / pw-cat 扬声器音频组件，请使用手机接听".into());
    };

    let pactl = lookup("pactl")
        .ok_or_else(|| "电脑缺少输入设备检查组件（pactl），请使用手机接听".to_string())?;

    Ok((record, play, pactl))
}

/// Fast, non-blocking preflight check verifying that desktop speaker/mic audio
/// routing binaries are installed locally without spawning any blocking IPC.
pub fn preflight_check() -> std::result::Result<(), String> {
    resolve_local_audio_backends_with(cfg!(target_os = "linux"), find_audio_tool).map(|_| ())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Microphone {
    source: String,
    label: String,
}

const INPUT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

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
    let (_, _, pactl) = match resolve_local_audio_backends_with(
        cfg!(target_os = "linux"),
        find_audio_tool,
    ) {
        Ok(backends) => backends,
        Err(reason) => {
            return Ok(Capabilities {
                supported: false,
                reason,
            });
        }
    };
    if let Err(error) = inspect_microphone(&pactl) {
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
}
impl Drop for CallAudioSession {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        for child in &mut self.children {
            let _ = child.kill();
        }
        let mut children = std::mem::take(&mut self.children);
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
            for child in &mut children { let _ = child.wait(); }
        });
    }
}
impl CallAudioSession {
    pub fn preflight_check() -> std::result::Result<(), String> {
        preflight_check()
    }

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
        let (record, play, pactl) =
            resolve_local_audio_backends_with(cfg!(target_os = "linux"), find_audio_tool)
                .map_err(|msg| anyhow::anyhow!("{msg}"))?;
        Self::start_cancellable_with_backends(adb, device, call_id, running, record, play, pactl)
    }

    fn start_cancellable_with_backends(
        adb: &Adb,
        device: &Device,
        call_id: &str,
        running: Arc<AtomicBool>,
        record: AudioCommandSpec,
        play: AudioCommandSpec,
        pactl: PathBuf,
    ) -> Result<Self> {
        ensure_audio_requested(&running)?;
        let microphone = inspect_microphone(&pactl)?;
        ensure_audio_requested(&running)?;
        let transport = device.resolve_active_transport(adb)?;
        let current = crate::push::fetch_live_call(adb, &transport)?;
        ensure!(
            current.id == call_id && current.state == "active" && !current.multiple_calls,
            "这次通话已结束或变化"
        );
        // Hold the transport mutation guard ONLY while ensuring/pushing the bridge jar
        // and completing the initial `Bridge --call-audio` handshake, then drop it before
        // entering the long-lived audio streaming loop so Hangup/SMS/other mutations are never starved.
        let startup_lease = crate::device::protect_transport_mutation(&device.serial)?;
        crate::bridge::ensure_bridge_jar(adb, device)?;
        ensure_audio_requested(&running)?;
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
        };
        session.children.push(
            record
                .command_for_source(Some(&session.microphone.source))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .context("无法启动电脑麦克风")?,
        );
        session.children.push(
            play.command_for_source(None)
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
        // Release the per-device mutation guard now that bridge/stream startup is complete.
        drop(startup_lease);

        let mic = session.children[0].stdout.take().unwrap();
        let speaker = session.children[1].stdin.take().unwrap();
        let input = session.children[2].stdin.take().unwrap();
        let output = session.children[2].stdout.take().unwrap();
        pump_uplink(mic, input, session.running.clone(), session.uplink.clone());
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

/// Uplink pump with corked/idle-mic silence keepalive:
/// whenever local capture yields no audio bytes for `>= 200ms` while the session is active,
/// sends a 20ms zero-PCM silence frame (`640` zero bytes for 16kHz mono s16le) so Android's
/// `CallAudioTools` reader never times out and drops downlink audio.
fn pump_uplink(
    mut source: impl Read + Send + 'static,
    mut target: impl Write + Send + 'static,
    running: Arc<AtomicBool>,
    counter: Arc<AtomicU64>,
) -> std::thread::JoinHandle<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(32);
    let reader_running = running.clone();
    std::thread::spawn(move || {
        let mut bytes = [0u8; UPLINK_PCM_FRAME_BYTES];
        while reader_running.load(Ordering::Relaxed) {
            match source.read(&mut bytes) {
                Ok(count) if count > 0 => {
                    if tx.send(bytes[..count].to_vec()).is_err() {
                        break;
                    }
                }
                Ok(_) => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    std::thread::spawn(move || {
        let silence = [0u8; UPLINK_PCM_FRAME_BYTES];
        while running.load(Ordering::Relaxed) {
            match rx.recv_timeout(UPLINK_SILENCE_KEEPALIVE) {
                Ok(chunk) => {
                    if target
                        .write_all(&chunk)
                        .and_then(|_| target.flush())
                        .is_err()
                    {
                        break;
                    }
                    counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    if target
                        .write_all(&silence)
                        .and_then(|_| target.flush())
                        .is_err()
                    {
                        break;
                    }
                    counter.fetch_add(silence.len() as u64, Ordering::Relaxed);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        running.store(false, Ordering::Relaxed);
    })
}

fn pump(
    mut source: impl Read + Send + 'static,
    mut target: impl Write + Send + 'static,
    running: Arc<AtomicBool>,
    counter: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let mut bytes = [0u8; UPLINK_PCM_FRAME_BYTES];
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

    #[test]
    fn preflight_check_validates_speaker_mic_and_pactl_without_blocking() {
        let bin = |name: &str| PathBuf::from(format!("/usr/bin/{name}"));

        // PulseAudio classic tools (`parec` + `paplay` + `pactl`)
        assert!(resolve_local_audio_backends_with(true, |t| {
            matches!(t, "parec" | "paplay" | "pactl").then(|| bin(t))
        })
        .is_ok());

        // Unified `pacat` (`--record` / `--playback`) + `pactl`
        let (rec, play, _) = resolve_local_audio_backends_with(true, |t| {
            matches!(t, "pacat" | "pactl").then(|| bin(t))
        })
        .unwrap();
        assert!(rec.args.iter().any(|a| a == "--record"));
        assert!(play.args.iter().any(|a| a == "--playback"));

        // PipeWire native `pw-cat` + `pactl`
        let (pw_rec, pw_play, _) = resolve_local_audio_backends_with(true, |t| {
            matches!(t, "pw-cat" | "pactl").then(|| bin(t))
        })
        .unwrap();
        assert!(pw_rec.pipewire_native && pw_play.pipewire_native);

        // Missing capture, playback, pactl, or unsupported OS must return descriptive errors
        assert!(resolve_local_audio_backends_with(true, |t| {
            matches!(t, "paplay" | "pactl").then(|| bin(t))
        })
        .is_err());
        assert!(resolve_local_audio_backends_with(true, |t| {
            matches!(t, "parec" | "pactl").then(|| bin(t))
        })
        .is_err());
        assert!(resolve_local_audio_backends_with(true, |t| {
            matches!(t, "parec" | "paplay").then(|| bin(t))
        })
        .is_err());
        assert!(resolve_local_audio_backends_with(false, |t| Some(bin(t))).is_err());
    }

    #[test]
    fn corked_mic_sends_20ms_zero_pcm_silence_keepalive_every_200ms() {
        struct CorkedMicReader {
            stop_rx: std::sync::mpsc::Receiver<()>,
        }
        impl Read for CorkedMicReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.stop_rx.recv();
                Ok(0)
            }
        }

        struct RecordingSink(std::sync::mpsc::Sender<Vec<u8>>);
        impl Write for RecordingSink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .send(buf.to_vec())
                    .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let (writes_tx, writes_rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let counter = Arc::new(AtomicU64::new(0));

        let handle = pump_uplink(
            CorkedMicReader { stop_rx },
            RecordingSink(writes_tx),
            running.clone(),
            counter.clone(),
        );

        let frames = (0..2)
            .map(|_| writes_rx.recv_timeout(Duration::from_secs(3)))
            .collect::<std::result::Result<Vec<_>, _>>();
        running.store(false, Ordering::Relaxed);
        drop(stop_tx);
        handle.join().unwrap();

        let mut frames = frames.expect("corked microphone must send consecutive keepalive frames");
        frames.extend(writes_rx.try_iter());
        assert!(
            frames.iter().all(|frame| {
                frame.len() == UPLINK_PCM_FRAME_BYTES && frame.iter().all(|&byte| byte == 0)
            }),
            "corked-mic keepalives must be 20ms frames of pure zero PCM"
        );
        assert_eq!(
            counter.load(Ordering::Relaxed),
            (frames.len() * UPLINK_PCM_FRAME_BYTES) as u64
        );
    }

    #[cfg(unix)]
    #[test]
    fn active_call_audio_session_drops_mutation_guard_after_startup_and_survives_corked_mic() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop_server = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_server.clone();
        listener.set_nonblocking(true).unwrap();
        let push_server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            while !stop_clone.load(Ordering::Acquire) && Instant::now() < deadline {
                let stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let now_ms = chrono::Utc::now().timestamp_millis();
                let resp = json!({
                    "ok": true,
                    "available": true,
                    "id": "call-guard-test",
                    "state": "active",
                    "multiple_calls": false,
                    "observed_ms": now_ms
                });
                let mut out = serde_json::to_vec(&resp).unwrap();
                out.push(b'\n');
                let _ = (&stream).write_all(&out);
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let write_exe = |name: &str, body: &str| -> PathBuf {
            let p = dir.path().join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };

        let pactl = write_exe(
            "pactl",
            r#"case "$*" in
  'get-default-source') printf 'corked-mic\n';;
  '--format=json list sources') printf '[{"name":"corked-mic","description":"Desktop Mic","state":"SUSPENDED","mute":false}]\n';;
  *) exit 1;;
esac"#,
        );
        // Corked mic: stays open without writing any PCM bytes!
        let parec = write_exe("parec", "sleep 10");
        let paplay = write_exe("paplay", "cat >/dev/null");
        let serial = "CALLAUDIOHWSERIAL01";
        let endpoint = "192.0.2.212:40101";
        let reconnect_marker = dir.path().join("reconnected");
        let fake_adb = write_exe(
            "adb",
            &format!(
                r#"case "$*" in
  "devices -l")
    printf 'List of devices attached\n{endpoint} device\n'
    exit 0
    ;;
  *"shell getprop ro.serialno"*)
    printf '{serial}\n'
    exit 0
    ;;
  *"forward tcp:0 localabstract:stepx_push_v1"*)
    printf '{port}\n'
    exit 0
    ;;
  *"stat -c %s"*)
    printf 'READY'
    exit 0
    ;;
  *"--call-audio"*)
    printf 'STEPX_CALL_AUDIO_READY\n' >&2
    head -c 1920 /dev/zero
    cat >/dev/null
    exit 0
    ;;
esac
exit 0"#,
            ),
        );

        let device = Device {
            transport: endpoint.into(),
            serial: serial.into(),
            ..Default::default()
        };
        crate::bridge::mark_bridge_jar_cached(endpoint);
        crate::bridge::mark_bridge_jar_cached(serial);

        let (record_spec, play_spec, pactl_path) = resolve_local_audio_backends_with(true, |t| {
            match t {
                "parec" => Some(parec.clone()),
                "paplay" => Some(paplay.clone()),
                "pactl" => Some(pactl.clone()),
                _ => None,
            }
        })
        .unwrap();

        let mut session = CallAudioSession::start_cancellable_with_backends(
            &Adb { path: fake_adb },
            &device,
            "call-guard-test",
            Arc::new(AtomicBool::new(true)),
            record_spec,
            play_spec,
            pactl_path,
        )
        .expect("call audio session should connect using corked-mic keepalive");

        assert!(session.running());
        let (up, down) = session.bytes();
        assert!(up >= 640 && down >= 640, "expected >=640B both ways, got ({up}, {down})");

        // CRITICAL [P0-4]: While `session` is still active, TransportMutationGuard MUST already be dropped!
        // `recover_transport` calls `begin_host_session_repair`, which returns `None` if `writes > 0`.
        let repair_adb = write_exe(
            "repair_adb",
            &format!(
                r#"case "$*" in
  "devices -l") printf 'List of devices attached\n{endpoint} device\n';;
  "-s {endpoint} shell getprop ro.serialno") if [ -f '{marker}' ]; then echo {serial}; else exit 1; fi;;
  "disconnect {endpoint}") exit 0;;
  "connect {endpoint}") touch '{marker}';;
  *) exit 1;;
esac"#,
                marker = reconnect_marker.display()
            ),
        );
        assert_eq!(
            crate::device::recover_transport(
                &Adb { path: repair_adb },
                endpoint,
                Duration::from_secs(4)
            )
            .as_deref(),
            Some(endpoint),
            "CallAudioSession must release TransportMutationGuard right after startup so mutations/recovery are not starved"
        );
        {
            let _other_mutation = crate::device::protect_transport_mutation(serial)
                .expect("Concurrent Hangup/SMS mutations must succeed during active call audio");
        }

        drop(session);
        stop_server.store(true, Ordering::Release);
        let _ = push_server.join();
    }
}

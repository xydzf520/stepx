//! Local, cancellable speech. Never uploads conversation text or uses a model API key.
use anyhow::{ensure, Context, Result};
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub fn spoken_text(source: &str) -> String {
    let mut code = false;
    source
        .lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                code = !code;
                return None;
            }
            if code {
                return None;
            }
            Some(
                line.trim_start_matches(['#', '>', ' ', '*'])
                    .replace("**", "")
                    .replace('`', ""),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .filter(|c| *c != '\0')
        .take(12_000)
        .collect()
}
pub struct Playback {
    child: Child,
    started: Instant,
    _text: tempfile::NamedTempFile,
}
impl Playback {
    pub fn start(text: &str) -> Result<Self> {
        let text = spoken_text(text);
        ensure!(!text.trim().is_empty(), "这条消息没有可朗读的文字");
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(text.as_bytes())?;
        let mut command;
        #[cfg(target_os = "linux")]
        {
            command = Command::new(std::env::current_exe()?);
            command.arg("--speech-helper");
        }
        #[cfg(target_os = "macos")]
        {
            command = Command::new("/usr/bin/say");
            command.arg("-f").arg(file.path());
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command = Command::new("powershell.exe");
            command.args(["-NoProfile","-NonInteractive","-Command","Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak([System.IO.File]::ReadAllText($env:STEPX_SPEECH_FILE)); $s.Dispose()"])
                .env("STEPX_SPEECH_FILE",file.path()).creation_flags(0x08000000);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("无法启动电脑语音服务")?;
        #[cfg(target_os = "linux")]
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(e) = stdin.write_all(text.as_bytes()) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.into());
                }
            }
        }
        Ok(Self {
            child,
            started: Instant::now(),
            _text: file,
        })
    }
    pub fn poll(&mut self) -> Result<bool> {
        if self.started.elapsed() > Duration::from_secs(20 * 60) {
            self.stop();
            anyhow::bail!("朗读超时，已停止");
        }
        match self.child.try_wait()? {
            Some(status) => {
                ensure!(status.success(),"电脑语音服务不可用。Linux 请安装 espeak-ng 与中文语音数据；Windows / macOS 请在系统设置中添加中文语音。");
                Ok(true)
            }
            None => Ok(false),
        }
    }
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "linux")]
pub fn helper() -> Result<()> {
    use std::ffi::{c_char, c_void, CString};
    let mut text = String::new();
    std::io::stdin()
        .take(128 * 1024)
        .read_to_string(&mut text)?;
    ensure!(!text.is_empty(), "empty speech");
    let c_text = CString::new(text.as_str())?;
    // Run the optional system engine in a child process: stopping it cannot affect
    // screen readers or other applications, and engine crashes cannot freeze the UI.
    unsafe {
        if let Ok(lib) = libloading::Library::new("libespeak-ng.so.1")
            .or_else(|_| libloading::Library::new("libespeak-ng.so"))
        {
            let init: libloading::Symbol<
                unsafe extern "C" fn(i32, i32, *const c_char, i32) -> i32,
            > = lib.get(b"espeak_Initialize")?;
            let voice: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> i32> =
                lib.get(b"espeak_SetVoiceByName")?;
            let synth: libloading::Symbol<
                unsafe extern "C" fn(
                    *const c_void,
                    usize,
                    u32,
                    i32,
                    u32,
                    u32,
                    *mut u32,
                    *mut c_void,
                ) -> i32,
            > = lib.get(b"espeak_Synth")?;
            let sync: libloading::Symbol<unsafe extern "C" fn() -> i32> =
                lib.get(b"espeak_Synchronize")?;
            let terminate: libloading::Symbol<unsafe extern "C" fn() -> i32> =
                lib.get(b"espeak_Terminate")?;
            ensure!(
                init(3, 0, std::ptr::null(), 0x8000) > 0,
                "speech initialization failed"
            );
            ensure!(voice(c"cmn".as_ptr()) == 0, "Chinese voice unavailable");
            let result = synth(
                c_text.as_ptr().cast(),
                c_text.as_bytes_with_nul().len(),
                0,
                1,
                0,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            let completed = sync();
            terminate();
            ensure!(result == 0 && completed == 0, "speech playback failed");
            return Ok(());
        }
    }
    for (program, args) in [
        ("spd-say", &["-w", "-l", "cmn", text.as_str()][..]),
        ("espeak-ng", &["-v", "cmn", text.as_str()][..]),
    ] {
        if let Ok(status) = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            if status.success() {
                return Ok(());
            }
        }
    }
    anyhow::bail!("Install espeak-ng")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_code_or_nul_in_speech() {
        assert_eq!(
            spoken_text("## 你好\n**世界**\n```rust\nsecret()\n```\n再见\0"),
            "你好\n世界\n再见"
        );
    }
}

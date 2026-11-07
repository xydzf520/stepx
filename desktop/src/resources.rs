//! Locate resources beside a portable executable, inside a macOS bundle, or in source.
use std::path::PathBuf;

thread_local! {
    static CONFIG_DIR_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Keeps UI test configuration isolated even when the library is compiled as a
/// normal dependency of a test binary (and therefore does not have `cfg(test)`).
#[doc(hidden)]
pub struct ConfigDirOverride {
    previous: Option<PathBuf>,
    // The override must be restored on the thread that installed it.
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[doc(hidden)]
pub fn override_config_dir_for_current_thread(path: PathBuf) -> ConfigDirOverride {
    ConfigDirOverride {
        previous: CONFIG_DIR_OVERRIDE.with(|slot| slot.borrow_mut().replace(path)),
        _thread_bound: std::marker::PhantomData,
    }
}

impl Drop for ConfigDirOverride {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _ = CONFIG_DIR_OVERRIDE.try_with(|slot| *slot.borrow_mut() = previous);
    }
}
pub fn root() -> PathBuf {
    if let Some(path) = std::env::var_os("STEPX_RESOURCE_DIR") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for path in [dir.to_path_buf(), dir.join("../Resources")] {
                if path.join("scripts").is_dir() {
                    return path;
                }
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
pub fn user_config() -> PathBuf {
    if let Some(path) = CONFIG_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return path;
    }
    std::env::var_os("STEPX_CONFIG_DIR")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        })
}

pub fn python() -> std::ffi::OsString {
    std::env::var_os("STEPX_PYTHON")
        .or_else(|| {
            let bundled = root().join("runtime/python").join(if cfg!(windows) {
                "python.exe"
            } else {
                "bin/python3"
            });
            bundled.is_file().then(|| bundled.into_os_string())
        })
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "python".into()
            } else {
                "python3".into()
            }
        })
}

pub fn user_home() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
}

/// Writable receipts and downloads stay outside the installed application.
pub fn user_state() -> PathBuf {
    if let Some(path) = std::env::var_os("STEPX_STATE_DIR") {
        return path.into();
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path).join("stepx-companion");
    }
    let home = user_home().map(PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("StepX Companion")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/StepX Companion")
    } else {
        home.join(".local/state/stepx-companion")
    }
}

pub fn python_command() -> std::process::Command {
    let mut command = std::process::Command::new(python());
    command
        .arg("-B")
        .arg("-X")
        .arg("utf8")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    command.env("STEPX_RESOURCE_DIR", root());
    command.env("STEPX_STATE_DIR", user_state());
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    if std::env::var_os("STEPX_ADB").is_none() {
        if let Some(adb) = crate::adb::bundled_tool("adb") {
            command.env("STEPX_ADB", adb);
        }
    }
    quiet_command(&mut command);
    command
}

/// Background utilities must not create console windows on Windows.
pub fn quiet_command(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_helpers_preserve_machine_readable_output() {
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "echo stepx-receipt"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "printf stepx-receipt; printf stepx-diagnostic >&2"]);
            c
        };
        quiet_command(&mut command);
        let result = command.output().unwrap();
        assert!(result.status.success());
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "stepx-receipt"
        );
        #[cfg(not(windows))]
        assert_eq!(String::from_utf8_lossy(&result.stderr), "stepx-diagnostic");
    }
}

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
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
}

pub fn root() -> PathBuf {
    if let Some(path) = std::env::var_os("STEPX_RESOURCE_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(dir) = exe_dir() {
        for path in [dir.clone(), dir.join("../Resources")] {
            if path.join("scripts").is_dir() {
                return path;
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
    }
    #[cfg(not(debug_assertions))]
    {
        exe_dir().unwrap_or_else(std::env::temp_dir)
    }
}

pub fn user_config() -> PathBuf {
    if let Some(path) = CONFIG_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return path;
    }
    std::env::var_os("STEPX_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_user_config)
}

/// System configuration root shared by all StepX launches for this user.
pub fn default_user_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("APPDATA").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            user_home()
                .map(|home| PathBuf::from(home).join(".config"))
                .unwrap_or_else(std::env::temp_dir)
        })
}

pub fn python() -> std::ffi::OsString {
    if let Some(explicit) = std::env::var_os("STEPX_PYTHON").filter(|v| !v.is_empty()) {
        return explicit;
    }
    let bundled = root().join("runtime/python").join(if cfg!(windows) {
        "python.exe"
    } else {
        "bin/python3"
    });
    if bundled.is_file() {
        return bundled.into_os_string();
    }
    #[cfg(target_os = "macos")]
    {
        use std::path::Path;
        for candidate in [
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
            "/Library/Frameworks/Python.framework/Versions/Current/bin/python3",
        ] {
            if Path::new(candidate).is_file() {
                return candidate.into();
            }
        }
        let clt_ready = Path::new("/Library/Developer/CommandLineTools/usr/bin/python3").is_file()
            || Path::new("/Applications/Xcode.app").exists();
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join("python3");
                if candidate == Path::new("/usr/bin/python3") && !clt_ready {
                    continue;
                }
                if candidate.is_file() {
                    return candidate.into_os_string();
                }
            }
        }
        if clt_ready {
            if Path::new("/usr/bin/python3").is_file() {
                return "/usr/bin/python3".into();
            }
            return "python3".into();
        }
        // Never fall back to `/usr/bin/python3` or bare `"python3"` when Xcode CLT
        // is missing; Apple's `/usr/bin/python3` stub pops up a modal installer dialog.
        return "/opt/homebrew/bin/python3".into();
    }
    #[cfg(windows)]
    {
        "python".into()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        "python3".into()
    }
}

pub fn user_home() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
}

/// Writable receipts and downloads stay outside the installed application.
pub fn user_state() -> PathBuf {
    if let Some(path) = std::env::var_os("STEPX_STATE_DIR").filter(|v| !v.is_empty()) {
        return path.into();
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(path).join("stepx-companion");
    }
    let Some(home) = user_home().map(PathBuf::from) else {
        return std::env::temp_dir().join("stepx-companion");
    };
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
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

use command::blocking::Command;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::process::Stdio;
use std::{env, fs, path};

/// Attempt to find a running process that we believe is the window compositor.
///
/// The name comes from `/proc/$pid/comm`, and so it will be truncated to the first 15 chars of the
/// actual process name.
/// https://superuser.com/questions/567648/ps-comm-format-always-cuts-the-process-name
pub(crate) fn look_for_wayland_compositor() -> Option<String> {
    // Try to determine the compositor by looking at the Wayland display
    // socket and seeing which process is listening on it via /proc.
    if let Some(compositor_name) = get_wayland_compositor_from_socket() {
        return Some(compositor_name);
    }

    // If the above method didn't work, fallback to a less precise method. Simply use `ps
    // -u` and grep for a recognized set of names among the running processes. This may
    // have false positives, like processes that name-clash with these compositors.
    let uid = nix::unistd::getuid();
    let euid = nix::unistd::geteuid();
    if let Ok(mut ps_child) = Command::new("ps")
        .args(["-u", &format!("{euid}"), "-U", &format!("{uid}")])
        .stdout(Stdio::piped())
        .spawn()
    {
        let wm_match_cmd = ps_child.stdout.take().and_then(|ps_output| {
            Command::new("grep")
                .args(
                    ["-m", "1", "-o", "-F", "-i"].iter().chain(
                        WAYLAND_TILING_WM
                            .iter()
                            .flat_map(|wm_name| [&"-e", wm_name]),
                    ),
                )
                .stdin(Stdio::from(ps_output))
                .output()
                .ok()
                .filter(|out| out.status.success())
        });
        let _ = ps_child.wait();

        if let Some(wm_name_raw) = wm_match_cmd {
            if let Ok(wm_name) = String::from_utf8(wm_name_raw.stdout) {
                if !wm_name.is_empty() {
                    return Some(wm_name);
                }
            }
        }
    }
    None
}

/// Returns the name of the Wayland compositor by reading `/proc/net/unix` to
/// find the process listening on the Wayland socket, avoiding external tool
/// dependencies like `lsof` or `fuser`.
fn get_wayland_compositor_from_socket() -> Option<String> {
    // https://discourse.ubuntu.com/t/environment-variables-for-wayland-hackers/12750
    let xdg_runtime_dir = env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|val| !val.is_empty())?;
    let wayland_display = env::var("WAYLAND_DISPLAY")
        .ok()
        .filter(|val| !val.is_empty())
        .unwrap_or("wayland-0".to_owned());

    // Wayland compositors communicate with their clients using a UNIX socket. This path is the
    // standard location of that socket.
    let wayland_socket_path = path::Path::new(xdg_runtime_dir.as_str()).join(wayland_display);
    let socket_metadata = fs::metadata(&wayland_socket_path).ok()?;

    // Validate that this file is a socket owned by the effective user ID.
    if !socket_metadata.file_type().is_socket()
        || socket_metadata.uid() != nix::unistd::geteuid().as_raw()
    {
        return None;
    }

    let socket_inode = socket_metadata.ino();

    // Scan /proc/<pid>/fd/ for a symlink to the socket inode, then read the
    // compositor name from /proc/<pid>/comm. This avoids external dependencies
    // (lsof, fuser) and runs entirely via the proc filesystem.
    if let Ok(proc_dir) = fs::read_dir("/proc") {
        for entry in proc_dir.flatten() {
            let fd_dir = entry.path().join("fd");
            let fd_entries = match fs::read_dir(&fd_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for fd_entry in fd_entries.flatten() {
                let fd_path = fd_entry.path();
                let fd_meta = match fs::metadata(&fd_path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if fd_meta.ino() == socket_inode {
                    // Found the process holding the socket — read its comm.
                    let comm_path = entry.path().join("comm");
                    if let Ok(comm) = fs::read_to_string(&comm_path) {
                        let name = comm.trim().to_owned();
                        if !name.is_empty() {
                            return Some(name);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Hand-picked tiling wayland compositors. These are the two most starred on GitHub.
const WAYLAND_TILING_WM: &[&str] = &["hyprland", "sway"];

pub(crate) fn is_tiling_window_manager(name: &str) -> bool {
    // List of X11 tiling window managers copied from Chromium repo:
    // https://source.chromium.org/chromium/chromium/src/+/6fa59a48:ui/base/x/x11_util.cc;l=374
    const X11_TILING_WM: &[&str] = &["i3", "ion3", "notion", "ratpoison", "stumpwm"];
    // Dynamic window managers can be configured to function as either tiling or stacking. It is
    // impractical for us to introspect how these are configured, so for now we copy Chrome's
    // approach to assume they are used as tiling.
    const X11_DYNAMIC_WM: &[&str] = &["awesome", "qtile", "xmonad", "wmii"];

    let normalized = name.trim().to_lowercase();
    X11_TILING_WM.contains(&normalized.as_str())
        || X11_DYNAMIC_WM.contains(&normalized.as_str())
        || WAYLAND_TILING_WM.contains(&normalized.as_str())
}

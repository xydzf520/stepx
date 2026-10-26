//! Read geometry only for the scrcpy process owned by this mirror session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MirrorWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub visible: bool,
    pub physical_pixels: bool,
    pub desktop_right: Option<f32>,
}

impl MirrorWindow {
    pub fn sidebar(self, scale: f32) -> [f32; 4] {
        let scale = if self.physical_pixels {
            scale.max(0.5)
        } else {
            1.
        };
        let (x, y, w, h) = (
            self.x / scale,
            self.y / scale,
            self.width / scale,
            self.height / scale,
        );
        let mut left = x + w + 6.;
        if self
            .desktop_right
            .is_some_and(|right| left + 92. > right / scale)
        {
            left = x - 98.;
        }
        [left, y, 92., h.max(500.)]
    }
}

/// OS window-server queries can wait for the compositor. Never run them on the UI thread.
pub struct Watcher {
    latest: std::sync::Arc<std::sync::Mutex<Option<MirrorWindow>>>,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Watcher {
    pub fn new(pid: u32) -> Self {
        let latest = std::sync::Arc::new(std::sync::Mutex::new(None));
        let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (state, stop) = (latest.clone(), stopped.clone());
        let _ = std::thread::Builder::new()
            .name("mirror-window".into())
            .spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let frame = locate(pid);
                    if let Ok(mut value) = state.lock() {
                        *value = frame;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(33));
                }
            });
        Self { latest, stopped }
    }
    pub fn latest(&self) -> Option<MirrorWindow> {
        self.latest.try_lock().ok().and_then(|v| *v)
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
pub fn locate(pid: u32) -> Option<MirrorWindow> {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, MapState},
    };
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = &conn.setup().roots[screen];
    let atom = |name: &[u8]| {
        conn.intern_atom(false, name)
            .ok()?
            .reply()
            .ok()
            .map(|a| a.atom)
    };
    let clients = atom(b"_NET_CLIENT_LIST")?;
    let owner = atom(b"_NET_WM_PID")?;
    let frames = atom(b"_NET_FRAME_EXTENTS")?;
    let state = atom(b"_NET_WM_STATE")?;
    let hidden = atom(b"_NET_WM_STATE_HIDDEN")?;
    let prop = conn
        .get_property(false, root.root, clients, AtomEnum::WINDOW, 0, 4096)
        .ok()?
        .reply()
        .ok()?;
    for win in prop.value32()? {
        let Ok(cookie) = conn.get_property(false, win, owner, AtomEnum::CARDINAL, 0, 1) else {
            continue;
        };
        let Ok(p) = cookie.reply() else {
            continue;
        };
        if p.value32().and_then(|mut values| values.next()) != Some(pid) {
            continue;
        }
        let g = conn.get_geometry(win).ok()?.reply().ok()?;
        let pos = conn
            .translate_coordinates(win, root.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let attributes = conn.get_window_attributes(win).ok()?.reply().ok()?;
        let states = conn
            .get_property(false, win, state, AtomEnum::ATOM, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        let minimized = states
            .value32()
            .is_some_and(|mut atoms| atoms.any(|a| a == hidden));
        let extents = conn
            .get_property(false, win, frames, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?;
        let e: Vec<_> = extents.value32().map(|v| v.collect()).unwrap_or_default();
        let (l, r, t, b) = if e.len() == 4 {
            (e[0], e[1], e[2], e[3])
        } else {
            (0, 0, 0, 0)
        };
        return Some(MirrorWindow {
            x: pos.dst_x as f32 - l as f32,
            y: pos.dst_y as f32 - t as f32,
            width: g.width as f32 + (l + r) as f32,
            height: g.height as f32 + (t + b) as f32,
            visible: attributes.map_state == MapState::VIEWABLE && !minimized,
            physical_pixels: true,
            desktop_right: Some(root.width_in_pixels as f32),
        });
    }
    None
}

#[cfg(target_os = "windows")]
pub fn locate(pid: u32) -> Option<MirrorWindow> {
    use std::ffi::c_void;
    type Hwnd = *mut c_void;
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    struct Search {
        pid: u32,
        result: Option<MirrorWindow>,
    }
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(
            callback: unsafe extern "system" fn(Hwnd, isize) -> i32,
            param: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(window: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowRect(window: Hwnd, rect: *mut Rect) -> i32;
        fn IsWindowVisible(window: Hwnd) -> i32;
        fn IsIconic(window: Hwnd) -> i32;
        fn GetSystemMetrics(index: i32) -> i32;
    }
    unsafe extern "system" fn visit(window: Hwnd, param: isize) -> i32 {
        let search = &mut *(param as *mut Search);
        let mut owner = 0;
        GetWindowThreadProcessId(window, &mut owner);
        if owner != search.pid || IsWindowVisible(window) == 0 {
            return 1;
        }
        let mut r = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(window, &mut r) == 0 {
            return 1;
        }
        search.result = Some(MirrorWindow {
            x: r.left as f32,
            y: r.top as f32,
            width: (r.right - r.left) as f32,
            height: (r.bottom - r.top) as f32,
            visible: IsIconic(window) == 0,
            physical_pixels: true,
            desktop_right: Some((GetSystemMetrics(76) + GetSystemMetrics(78)) as f32),
        });
        0
    }
    let mut search = Search { pid, result: None };
    unsafe {
        EnumWindows(visit, &mut search as *mut Search as isize);
    }
    search.result
}

#[cfg(target_os = "macos")]
pub fn locate(pid: u32) -> Option<MirrorWindow> {
    use std::ffi::{c_char, c_void};
    type Ref = *const c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative: u32) -> Ref;
        static kCGWindowOwnerPID: Ref;
        static kCGWindowBounds: Ref;
        static kCGWindowLayer: Ref;
        static kCGWindowIsOnscreen: Ref;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
        fn CFNumberGetValue(number: Ref, kind: i32, value: *mut c_void) -> bool;
        fn CFBooleanGetValue(value: Ref) -> bool;
        fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
        fn CFRelease(value: Ref);
    }
    unsafe fn number(dict: Ref, key: Ref) -> Option<f64> {
        let n = CFDictionaryGetValue(dict, key);
        let mut out = 0f64;
        if n.is_null() || !CFNumberGetValue(n, 13, &mut out as *mut _ as *mut c_void) {
            None
        } else {
            Some(out)
        }
    }
    unsafe {
        // No screen pixels or window titles are read; filter the owning process.
        let list = CGWindowListCopyWindowInfo(0, 0);
        if list.is_null() {
            return None;
        }
        let mut result = None;
        for i in 0..CFArrayGetCount(list) {
            let row = CFArrayGetValueAtIndex(list, i);
            if number(row, kCGWindowOwnerPID) != Some(pid as f64)
                || number(row, kCGWindowLayer) != Some(0.)
            {
                continue;
            }
            let bounds = CFDictionaryGetValue(row, kCGWindowBounds);
            if bounds.is_null() {
                continue;
            }
            let mut values = Vec::new();
            for name in [b"X\0".as_slice(), b"Y\0", b"Width\0", b"Height\0"] {
                let key = CFStringCreateWithCString(
                    std::ptr::null(),
                    name.as_ptr() as *const c_char,
                    0x08000100,
                );
                let value = number(bounds, key);
                CFRelease(key);
                values.push(value);
            }
            if let [Some(x), Some(y), Some(w), Some(h)] = values.as_slice() {
                let onscreen = CFDictionaryGetValue(row, kCGWindowIsOnscreen);
                result = Some(MirrorWindow {
                    x: *x as f32,
                    y: *y as f32,
                    width: *w as f32,
                    height: *h as f32,
                    visible: !onscreen.is_null() && CFBooleanGetValue(onscreen),
                    physical_pixels: false,
                    desktop_right: None,
                });
                break;
            }
        }
        CFRelease(list);
        result
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn locate(_: u32) -> Option<MirrorWindow> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> MirrorWindow {
        MirrorWindow {
            x: 100.,
            y: 80.,
            width: 360.,
            height: 800.,
            visible: true,
            physical_pixels: true,
            desktop_right: Some(1920.),
        }
    }
    #[test]
    fn follows_move_and_resize() {
        assert_eq!(frame().sidebar(1.), [466., 80., 92., 800.]);
        let mut f = frame();
        f.x = 400.;
        f.width = 500.;
        assert_eq!(f.sidebar(1.), [906., 80., 92., 800.]);
    }
    #[test]
    fn scales_physical_geometry() {
        assert_eq!(frame().sidebar(2.), [236., 40., 92., 500.]);
    }
    #[test]
    fn moves_to_left_at_desktop_edge() {
        let mut f = frame();
        f.x = 1600.;
        assert_eq!(f.sidebar(1.)[0], 1502.);
    }
    #[test]
    fn mac_bounds_are_already_logical() {
        let mut f = frame();
        f.physical_pixels = false;
        assert_eq!(f.sidebar(2.), f.sidebar(1.));
    }
}

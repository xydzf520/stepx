use crate::windowing::winit::app::CustomEvent;
use winit::event_loop::EventLoopProxy;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, MSG,
    WINDOW_EX_STYLE, WNDCLASSW, WNDCLASS_STYLES, WS_POPUP,
};
use windows_core::PCWSTR;

/// Watcher for the hidden listener window thread.
/// Dropping the watcher simply detaches the thread -- the OS cleans up when the
/// process exits, matching the existing pattern in
/// [`super::registry::SystemThemeWatcher`].
#[allow(dead_code)]
pub struct WmSettingChangeWatcher {
    thread: Option<std::thread::JoinHandle<()>>,
}

const WM_SETTINGCHANGE: u32 = 0x001A;

/// Spawns a thread that creates a hidden overlapped (popup) window and runs a
/// `GetMessage` loop.  When `WM_SETTINGCHANGE` arrives with `lParam` pointing
/// to the wide string `"ImmersiveColorSet"` (the signal that the user has
/// toggled dark/light mode), a `CustomEvent::SystemThemeChanged` is sent through
/// the event proxy so the application immediately reflects the new system theme.
pub fn watch_wm_settingchange(event_proxy: EventLoopProxy<CustomEvent>) -> WmSettingChangeWatcher {
    let thread = std::thread::Builder::new()
        .name("WM_SETTINGCHANGE Listener".to_string())
        .spawn(move || unsafe {
            let instance: HINSTANCE = GetModuleHandleW(PCWSTR::null())
                .map(|m| m.into())
                .unwrap_or(HINSTANCE::default());

            let class_name: Vec<u16> = "WarpThemeChangeListener"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let wc = WNDCLASSW {
                style: WNDCLASS_STYLES::default(),
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: std::mem::size_of::<isize>() as i32,
                hInstance: instance,
                hIcon: Default::default(),
                hCursor: Default::default(),
                hbrBackground: Default::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
            };

            if RegisterClassW(&wc as *const WNDCLASSW) == 0 {
                log::warn!("Failed to register WM_SETTINGCHANGE listener window class");
                return;
            }

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            );

            let hwnd = match hwnd {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("Failed to create WM_SETTINGCHANGE listener window: {e:?}");
                    return;
                }
            };

            // Store the event proxy in the window's user-data so the window
            // procedure can reach it.
            let proxy_ptr = Box::into_raw(Box::new(event_proxy)) as *mut _;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, proxy_ptr as isize);

            log::info!("WM_SETTINGCHANGE listener started");

            let mut msg = MSG::default();
            loop {
                let ret = GetMessageW(&mut msg, None, 0, 0);
                if ret.0 <= 0 {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // Clean up the stored proxy object.
            let proxy_ptr =
                GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EventLoopProxy<CustomEvent>;
            if !proxy_ptr.is_null() {
                drop(Box::from_raw(proxy_ptr));
            }
            let _ = DestroyWindow(hwnd);
        });

    match thread {
        Ok(thread) => WmSettingChangeWatcher {
            thread: Some(thread),
        },
        Err(e) => {
            log::warn!("Failed to spawn WM_SETTINGCHANGE listener thread: {e:?}");
            WmSettingChangeWatcher { thread: None }
        }
    }
}

/// Window procedure for the hidden listener window.
///
/// Intercepts `WM_SETTINGCHANGE` and, when `lParam` is a pointer to
/// `"ImmersiveColorSet"`, sends `CustomEvent::SystemThemeChanged` through the
/// stored event proxy.  All other messages (including the original
/// `WM_SETTINGCHANGE`) are forwarded to [`DefWindowProcW`].
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_SETTINGCHANGE {
        // When the user toggles between light / dark mode, Windows broadcasts
        // WM_SETTINGCHANGE with lParam pointing to the wide string
        // "ImmersiveColorSet".
        if let Some(ptr) = lparam_to_ptr(lparam) {
            // The known string: "ImmersiveColorSet\0" (18 code units).
            let target: [u16; 18] = [
                73, 109, 109, 101, 114, 115, 105, 118, 101, 67, 111, 108, 111, 114, 83, 101, 116, 0,
            ];

            let mut matches = true;
            for (i, &expected) in target.iter().enumerate() {
                if *ptr.add(i) != expected {
                    matches = false;
                    break;
                }
            }

            if matches {
                log::info!("Detected system theme change via WM_SETTINGCHANGE");
                let proxy_ptr =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EventLoopProxy<CustomEvent>;
                if !proxy_ptr.is_null() {
                    let _ = (*proxy_ptr).send_event(CustomEvent::SystemThemeChanged);
                }
            }
        }
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Extracts a valid wide-string pointer from an opaque `LPARAM` value.
///
/// Rejects null/integer broadcast sentinels in the first 64 KiB (`<= 0xFFFF`, such as `lParam = 1`
/// from taskbar/DPI/shell broadcasts) and unaligned `u16` addresses so they are never dereferenced.
fn lparam_to_ptr(lparam: LPARAM) -> Option<*const u16> {
    let raw = lparam.0 as usize;
    if raw <= 0xFFFF || raw % core::mem::align_of::<u16>() != 0 {
        None
    } else {
        Some(raw as *const u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lparam_to_ptr_rejects_null_low_integers_and_unaligned_addresses() {
        assert!(lparam_to_ptr(LPARAM(0)).is_none());
        assert!(lparam_to_ptr(LPARAM(1)).is_none());
        assert!(lparam_to_ptr(LPARAM(0xFFFF)).is_none());
        assert!(lparam_to_ptr(LPARAM(0x10001)).is_none());

        let sample: [u16; 2] = [65, 0];
        let addr = sample.as_ptr() as isize;
        assert_eq!(lparam_to_ptr(LPARAM(addr)), Some(sample.as_ptr()));
    }
}

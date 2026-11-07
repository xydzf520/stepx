#!/usr/bin/env python3
"""Apply StepX's narrow X11 toast compatibility fix to its linked UI crate."""
from pathlib import Path
import re

root = Path(__file__).resolve().parent.parent
manifest = root / "desktop/Cargo.toml"
match = re.search(r'^facloud_ui\s*=\s*\{[^\n]*path\s*=\s*"([^"]+)"', manifest.read_text(), re.M)
if not match:
    raise SystemExit("Cannot locate the linked UI crate; review the window compatibility patch")
dependency = match.group(1)
source = (manifest.parent / dependency).resolve() / "src/windowing/winit/window.rs"
before = '''        if tiling_window_manager && window_options.style == WindowStyle::Pin {
            window_attributes = window_attributes.with_x11_window_type(vec![WindowType::Dialog]);
        }'''
after = '''        // X11 ignores with_active(false). Toasts must declare their native type
        // before mapping, or the WM may activate them and interrupt another editor.
        if window_options.style == WindowStyle::PopupToast {
            window_attributes = window_attributes.with_x11_window_type(vec![WindowType::Notification]);
        } else if tiling_window_manager && window_options.style == WindowStyle::Pin {
            window_attributes = window_attributes.with_x11_window_type(vec![WindowType::Dialog]);
        }'''
text = source.read_text()
if after in text:
    print("UI toast compatibility already applied")
elif text.count(before) == 1:
    source.write_text(text.replace(before, after))
    print("Applied X11 nonactivating toast compatibility")
else:
    raise SystemExit("UI window code has changed; review the X11 toast compatibility patch before building")

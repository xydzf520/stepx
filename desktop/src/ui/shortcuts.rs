//! Event-driven global shortcut. No timer, device polling, or keyboard hook.
use super::*;
use facloud_ui::{keymap::Keystroke, TypedActionView};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::str::FromStr;
use stepx_companion::settings::CompactShortcut;

pub(super) fn primary_mod(k: &Keystroke) -> bool {
    #[cfg(target_os = "macos")]
    {
        k.cmd && !k.ctrl && !k.alt
    }
    #[cfg(not(target_os = "macos"))]
    {
        k.ctrl && !k.cmd && !k.alt
    }
}

pub(super) fn word_backward_mod(k: &Keystroke) -> bool {
    #[cfg(target_os = "macos")]
    {
        k.alt && !k.cmd && !k.ctrl && !k.shift
    }
    #[cfg(not(target_os = "macos"))]
    {
        k.ctrl && !k.cmd && !k.alt && !k.shift
    }
}

pub(super) fn line_backward_mod(k: &Keystroke) -> bool {
    #[cfg(target_os = "macos")]
    {
        k.cmd && !k.ctrl && !k.alt && !k.shift
    }
    #[cfg(not(target_os = "macos"))]
    {
        k.ctrl && k.shift && !k.cmd && !k.alt
    }
}

#[derive(Default)]
pub(super) struct Shortcuts {
    manager: Option<GlobalHotKeyManager>,
    active: Option<HotKey>,
    suspended: bool,
    ignore_until: Option<Instant>,
    pub recording: bool,
    pub note: String,
    receiver: Option<async_channel::Receiver<GlobalHotKeyEvent>>,
}
impl Shortcuts {
    fn unregister(manager: &GlobalHotKeyManager, key: HotKey) -> Result<()> {
        manager.unregister(key)?;
        // global-hotkey 0.7 queues X11 UngrabKey without flushing it. The second
        // idempotent unregister performs a keyboard-map round trip first, so the
        // initial ungrab reaches the server before recording or re-binding.
        // Other platforms release synchronously and need only the first call.
        #[cfg(target_os = "linux")]
        manager.unregister(key)?;
        Ok(())
    }
    fn key(value: &CompactShortcut) -> Result<HotKey> {
        anyhow::ensure!(
            value.valid(),
            "请使用 Ctrl/Command + Alt/Shift + 字母、数字或 F1–F12"
        );
        let key = if value.key.len() == 1 {
            format!(
                "{}{}",
                if value.key.as_bytes()[0].is_ascii_digit() {
                    "Digit"
                } else {
                    "Key"
                },
                value.key.to_uppercase()
            )
        } else {
            value.key.to_uppercase()
        };
        let mut mods = Modifiers::empty();
        if value.ctrl {
            mods |= Modifiers::CONTROL;
        }
        if value.alt {
            mods |= Modifiers::ALT;
        }
        if value.cmd {
            mods |= Modifiers::SUPER;
        }
        if value.shift {
            mods |= Modifiers::SHIFT;
        }
        Ok(HotKey::new(Some(mods), Code::from_str(&key)?))
    }
    /// Register the replacement first; a conflict leaves the working binding intact.
    pub fn replace(&mut self, value: &CompactShortcut) -> Result<()> {
        let next = Self::key(value)?;
        let Some(manager) = &self.manager else {
            return Ok(());
        };
        if self.active == Some(next) {
            if self.suspended {
                manager.register(next)?;
                self.suspended = false;
            }
            return Ok(());
        }
        manager.register(next).map_err(|_| {
            anyhow::anyhow!("该快捷键已被占用或系统拒绝注册，请换一个组合；原快捷键保持有效")
        })?;
        if let Some(old) = self.active.filter(|_| !self.suspended) {
            if let Err(e) = Self::unregister(manager, old) {
                let _ = Self::unregister(manager, next);
                anyhow::bail!("无法替换原快捷键：{e}");
            }
        }
        self.active = Some(next);
        self.suspended = false;
        self.note = "程序运行时，可从其他应用直接呼出简洁对话。".into();
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        if self.suspended {
            if let (Some(manager), Some(key)) = (&self.manager, self.active) {
                manager.register(key)?;
            }
            self.suspended = false;
            self.ignore_until = Some(Instant::now() + Duration::from_millis(400));
        }
        Ok(())
    }
    pub fn suspend(&mut self) -> Result<()> {
        if !self.suspended {
            if let (Some(manager), Some(key)) = (&self.manager, self.active) {
                Self::unregister(manager, key)?;
                self.suspended = true;
            }
        }
        Ok(())
    }
    pub fn matches(value: &CompactShortcut, key: &Keystroke) -> bool {
        value.key == key.key.to_lowercase()
            && value.ctrl == key.ctrl
            && value.cmd == key.cmd
            && value.alt == key.alt
            && value.shift == key.shift
            && !key.meta
    }
    pub fn captured(key: &Keystroke) -> CompactShortcut {
        CompactShortcut {
            key: key.key.to_lowercase(),
            ctrl: key.ctrl,
            cmd: key.cmd,
            alt: key.alt,
            shift: key.shift,
        }
    }
}
impl Drop for Shortcuts {
    fn drop(&mut self) {
        if let (Some(manager), Some(key)) = (&self.manager, self.active) {
            let _ = manager.unregister(key);
        }
        if let Some(rx) = &self.receiver {
            rx.close();
        }
    }
}
impl Companion {
    pub(super) fn initialize_shortcut(&mut self, ctx: &mut ViewContext<Self>) {
        if self.demo_mode || self.capture.is_some() {
            if !std::env::args().any(|a| a == "--verify-shell-details") {
                return;
            }
        }
        // The shell explicitly uses X11/XWayland whenever DISPLAY is available.
        // Only a native Wayland launch lacks an X11 shortcut registration channel.
        #[cfg(target_os = "linux")]
        if std::env::var_os("DISPLAY").is_none() {
            self.shortcuts.note =
                "当前 Wayland 会话：快捷键在程序窗口内生效；可从托盘呼出简洁对话。".into();
            return;
        }
        match GlobalHotKeyManager::new() {
            Ok(manager) => self.shortcuts.manager = Some(manager),
            Err(_) => {
                self.shortcuts.note = "系统未开放全局快捷键，仍可在程序窗口内使用。".into();
                return;
            }
        }
        let (tx, rx) = async_channel::bounded(8);
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Released {
                let _ = tx.try_send(event);
            }
        }));
        self.shortcuts.receiver = Some(rx);
        if let Err(e) = self
            .shortcuts
            .replace(&self.settings.value.compact_shortcut)
        {
            self.shortcuts.note = e.to_string();
        }
        self.wait_shortcut(ctx);
    }
    fn wait_shortcut(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(rx) = self.shortcuts.receiver.clone() else {
            return;
        };
        ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
            if let Ok(event) = result {
                if event.state == HotKeyState::Released
                    && me.shortcuts.active.is_some_and(|k| k.id() == event.id)
                    && !me.shortcuts.recording
                    && me
                        .shortcuts
                        .ignore_until
                        .is_none_or(|t| Instant::now() >= t)
                {
                    me.handle_action(&Action::CompactChat, ctx);
                }
                me.wait_shortcut(ctx);
            }
        });
    }
    pub(super) fn summon_compact_chat(&mut self, ctx: &mut ViewContext<Self>) {
        self.settings.hidden = false;
        let id = ctx.window_id();
        ctx.windows().show_window_and_focus_app(id);
        self.set_compact_mode(ShellMode::MiniAi, ctx);
        self.focus = Some(F_AMOO);
        ctx.focus_self();
        ctx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_roundtrip_and_exact_modifiers() {
        let value = CompactShortcut::default();
        assert!(Shortcuts::key(&value).is_ok());
        let mut key = Keystroke {
            key: value.key.clone(),
            ctrl: value.ctrl,
            cmd: value.cmd,
            alt: value.alt,
            shift: value.shift,
            ..Default::default()
        };
        assert!(Shortcuts::matches(&value, &key));
        key.shift = !key.shift;
        assert!(!Shortcuts::matches(&value, &key));
    }
    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated X11 display"]
    fn native_conflict_retains_working_binding() {
        let mut own = Shortcuts::default();
        own.manager = Some(GlobalHotKeyManager::new().unwrap());
        let old = CompactShortcut {
            key: "f9".into(),
            ..Default::default()
        };
        own.replace(&old).unwrap();
        let active = own.active;
        let other = GlobalHotKeyManager::new().unwrap();
        let new = CompactShortcut {
            key: "f10".into(),
            ..Default::default()
        };
        let reserved = Shortcuts::key(&new).unwrap();
        other.register(reserved).unwrap();
        assert!(own.replace(&new).is_err());
        assert_eq!(own.active, active);
        own.suspend().unwrap();
        // Recording must release the binding for other X11 clients immediately.
        other.register(active.unwrap()).unwrap();
        Shortcuts::unregister(&other, active.unwrap()).unwrap();
        own.resume().unwrap();
        assert_eq!(own.active, active);
        Shortcuts::unregister(&other, reserved).unwrap();
        own.replace(&new).unwrap();
        assert_eq!(own.active, Some(reserved));
    }
    #[test]
    fn rejects_typing_and_special_keys() {
        for name in ["escape", "enter", "中", "delete", "f13", ""] {
            assert!(Shortcuts::key(&CompactShortcut {
                key: name.into(),
                ..Default::default()
            })
            .is_err());
        }
        assert!(Shortcuts::key(&CompactShortcut {
            alt: false,
            ..Default::default()
        })
        .is_err());
    }
}

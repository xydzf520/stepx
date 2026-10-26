//! Shared outline icon family. Assets use a 24 px grid and rounded 1.75 px strokes.
//! White SVG masks are required: the renderer uses the red channel as icon alpha.
use super::{widgets::El, Companion};
use facloud_ui::{color::ColorU, elements::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) enum UiIcon {
    Info,
    Calendar,
    Home,
    Phone,
    PhoneOff,
    Smartphone,
    Sms,
    Apps,
    Tools,
    Briefcase,
    Pin,
    Matrix,
    Star,
    GithubStar,
    Bolt,
    Connect,
    Folder,
    File,
    Music,
    Video,
    Image,
    Download,
    Upload,
    Battery,
    Storage,
    Temperature,
    Copy,
    Clipboard,
    Keyboard,
    Speaker,
    Mic,
    Retry,
    Refresh,
    ChevronDown,
    ChevronRight,
    ChevronUp,
    ArrowUp,
    ArrowLeft,
    ExternalLink,
    Plus,
    Minus,
    Close,
    Check,
    Trash,
    Settings,
    Search,
    Package,
    Backspace,
    Camera,
    Lock,
    Shield,
    Monitor,
    Wifi,
    Clock,
    Bell,
    Sun,
    Smile,
    Lightbulb,
    Chat,
    Attach,
    Play,
    Pause,
    Chart,
    Menu,
    Cpu,
    Wave,
    Maximize,
    Restore,
    Contrast,
    Ban,
    Coffee,
    Heart,
    Cat,
    AlertTriangle,
    Flip,
    ArrowDown,
    ArrowRight,
    ChevronLeft,
    Mute,
    Bluetooth,
    Edit,
    ScanText,
    User,
    Sparkles,
    Archive,
    Send,
    CreditCard,
    Key,
    Activity,
    Gift,
}
#[allow(non_upper_case_globals, dead_code)]
impl UiIcon {
    pub(super) const SpeakerMute: Self = Self::Mute;
    pub(super) const SpeakerOff: Self = Self::Mute;
    pub(super) const Wallet: Self = Self::CreditCard;
    pub(super) const Sos: Self = Self::AlertTriangle;

    pub(super) fn path(self) -> &'static str {
        match self {
            Self::Info => "icons/Info.svg",
            Self::Calendar => "icons/Calendar.svg",
            Self::Home => "icons/Home.svg",
            Self::Phone => "icons/Phone.svg",
            Self::PhoneOff => "icons/PhoneOff.svg",
            Self::Smartphone => "icons/Smartphone.svg",
            Self::Sms => "icons/Sms.svg",
            Self::Apps => "icons/Apps.svg",
            Self::Briefcase => "icons/Briefcase.svg",
            Self::Pin => "icons/Pin.svg",
            Self::Tools => "icons/Tools.svg",
            Self::Matrix => "icons/Matrix.svg",
            Self::Star => "icons/Star.svg",
            Self::GithubStar => "icons/GithubStar.svg",
            Self::Bolt => "icons/Bolt.svg",
            Self::Connect => "icons/Connect.svg",
            Self::Folder => "icons/Folder.svg",
            Self::File => "icons/File.svg",
            Self::Music => "icons/Music.svg",
            Self::Video => "icons/Video.svg",
            Self::Image => "icons/Image.svg",
            Self::Download => "icons/Download.svg",
            Self::Upload => "icons/Upload.svg",
            Self::Battery => "icons/Battery.svg",
            Self::Storage => "icons/Storage.svg",
            Self::Temperature => "icons/Temperature.svg",
            Self::Copy => "icons/Copy.svg",
            Self::Clipboard => "icons/Clipboard.svg",
            Self::Keyboard => "icons/Keyboard.svg",
            Self::Speaker => "icons/Speaker.svg",
            Self::Mic => "icons/Mic.svg",
            Self::Retry => "icons/Retry.svg",
            Self::Refresh => "icons/Refresh.svg",
            Self::ChevronDown => "icons/ChevronDown.svg",
            Self::ChevronRight => "icons/ChevronRight.svg",
            Self::ChevronUp => "icons/ChevronUp.svg",
            Self::ArrowUp => "icons/ArrowUp.svg",
            Self::ArrowLeft => "icons/ArrowLeft.svg",
            Self::ExternalLink => "icons/ExternalLink.svg",
            Self::Plus => "icons/Plus.svg",
            Self::Minus => "icons/Minus.svg",
            Self::Close => "icons/Close.svg",
            Self::Check => "icons/Check.svg",
            Self::Trash => "icons/Trash.svg",
            Self::Settings => "icons/Settings.svg",
            Self::Search => "icons/Search.svg",
            Self::Package => "icons/Package.svg",
            Self::Backspace => "icons/Backspace.svg",
            Self::Camera => "icons/Camera.svg",
            Self::Lock => "icons/Lock.svg",
            Self::Shield => "icons/Shield.svg",
            Self::Monitor => "icons/Monitor.svg",
            Self::Wifi => "icons/Wifi.svg",
            Self::Clock => "icons/Clock.svg",
            Self::Bell => "icons/Bell.svg",
            Self::Sun => "icons/Sun.svg",
            Self::Smile => "icons/Smile.svg",
            Self::Lightbulb => "icons/Lightbulb.svg",
            Self::Chat => "icons/Chat.svg",
            Self::Attach => "icons/Attach.svg",
            Self::Play => "icons/Play.svg",
            Self::Pause => "icons/Pause.svg",
            Self::Chart => "icons/Chart.svg",
            Self::Menu => "icons/Menu.svg",
            Self::Cpu => "icons/Cpu.svg",
            Self::Wave => "icons/Wave.svg",
            Self::Maximize => "icons/Maximize.svg",
            Self::Restore => "icons/Restore.svg",
            Self::Contrast => "icons/Contrast.svg",
            Self::Ban => "icons/Ban.svg",
            Self::Coffee => "icons/Coffee.svg",
            Self::Heart => "icons/Heart.svg",
            Self::Cat => "icons/Cat.svg",
            Self::AlertTriangle => "icons/AlertTriangle.svg",
            Self::Flip => "icons/Flip.svg",
            Self::ArrowDown => "icons/ArrowDown.svg",
            Self::ArrowRight => "icons/ArrowRight.svg",
            Self::ChevronLeft => "icons/ChevronLeft.svg",
            Self::Mute => "icons/Mute.svg",
            Self::Bluetooth => "icons/Bluetooth.svg",
            Self::Edit => "icons/Edit.svg",
            Self::ScanText => "icons/ScanText.svg",
            Self::User => "icons/User.svg",
            Self::Sparkles => "icons/Sparkles.svg",
            Self::Archive => "icons/Archive.svg",
            Self::Send => "icons/Send.svg",
            Self::CreditCard => "icons/CreditCard.svg",
            Self::Key => "icons/Key.svg",
            Self::Activity => "icons/Activity.svg",
            Self::Gift => "icons/Gift.svg",
        }
    }
}
pub(super) fn asset(path: &str) -> Option<&'static [u8]> {
    Some(match path {
        "icons/Info.svg" => include_bytes!("../../assets/icons/Info.svg"),
        "icons/Calendar.svg" => include_bytes!("../../assets/icons/Calendar.svg"),
        "icons/Home.svg" => include_bytes!("../../assets/icons/Home.svg"),
        "icons/Phone.svg" => include_bytes!("../../assets/icons/Phone.svg"),
        "icons/PhoneOff.svg" => include_bytes!("../../assets/icons/PhoneOff.svg"),
        "icons/Smartphone.svg" => include_bytes!("../../assets/icons/Smartphone.svg"),
        "icons/Sms.svg" => include_bytes!("../../assets/icons/Sms.svg"),
        "icons/Apps.svg" => include_bytes!("../../assets/icons/Apps.svg"),
        "icons/Briefcase.svg" => include_bytes!("../../assets/icons/Briefcase.svg"),
        "icons/Pin.svg" => include_bytes!("../../assets/icons/Pin.svg"),
        "icons/Tools.svg" => include_bytes!("../../assets/icons/Tools.svg"),
        "icons/Matrix.svg" => include_bytes!("../../assets/icons/Matrix.svg"),
        "icons/Star.svg" => include_bytes!("../../assets/icons/Star.svg"),
        "icons/GithubStar.svg" => include_bytes!("../../assets/icons/GithubStar.svg"),
        "icons/Bolt.svg" => include_bytes!("../../assets/icons/Bolt.svg"),
        "icons/Connect.svg" => include_bytes!("../../assets/icons/Connect.svg"),
        "icons/Folder.svg" => include_bytes!("../../assets/icons/Folder.svg"),
        "icons/File.svg" => include_bytes!("../../assets/icons/File.svg"),
        "icons/Music.svg" => include_bytes!("../../assets/icons/Music.svg"),
        "icons/Video.svg" => include_bytes!("../../assets/icons/Video.svg"),
        "icons/Image.svg" => include_bytes!("../../assets/icons/Image.svg"),
        "icons/Download.svg" => include_bytes!("../../assets/icons/Download.svg"),
        "icons/Upload.svg" => include_bytes!("../../assets/icons/Upload.svg"),
        "icons/Battery.svg" => include_bytes!("../../assets/icons/Battery.svg"),
        "icons/Storage.svg" => include_bytes!("../../assets/icons/Storage.svg"),
        "icons/Temperature.svg" => include_bytes!("../../assets/icons/Temperature.svg"),
        "icons/Copy.svg" => include_bytes!("../../assets/icons/Copy.svg"),
        "icons/Clipboard.svg" => include_bytes!("../../assets/icons/Clipboard.svg"),
        "icons/Keyboard.svg" => include_bytes!("../../assets/icons/Keyboard.svg"),
        "icons/Speaker.svg" => include_bytes!("../../assets/icons/Speaker.svg"),
        "icons/Mic.svg" => include_bytes!("../../assets/icons/Mic.svg"),
        "icons/Retry.svg" => include_bytes!("../../assets/icons/Retry.svg"),
        "icons/Refresh.svg" => include_bytes!("../../assets/icons/Refresh.svg"),
        "icons/ChevronDown.svg" => include_bytes!("../../assets/icons/ChevronDown.svg"),
        "icons/ChevronRight.svg" => include_bytes!("../../assets/icons/ChevronRight.svg"),
        "icons/ChevronUp.svg" => include_bytes!("../../assets/icons/ChevronUp.svg"),
        "icons/ArrowUp.svg" => include_bytes!("../../assets/icons/ArrowUp.svg"),
        "icons/ArrowLeft.svg" => include_bytes!("../../assets/icons/ArrowLeft.svg"),
        "icons/ExternalLink.svg" => include_bytes!("../../assets/icons/ExternalLink.svg"),
        "icons/Plus.svg" => include_bytes!("../../assets/icons/Plus.svg"),
        "icons/Minus.svg" => include_bytes!("../../assets/icons/Minus.svg"),
        "icons/Close.svg" => include_bytes!("../../assets/icons/Close.svg"),
        "icons/Check.svg" => include_bytes!("../../assets/icons/Check.svg"),
        "icons/Trash.svg" => include_bytes!("../../assets/icons/Trash.svg"),
        "icons/Settings.svg" => include_bytes!("../../assets/icons/Settings.svg"),
        "icons/Search.svg" => include_bytes!("../../assets/icons/Search.svg"),
        "icons/Package.svg" => include_bytes!("../../assets/icons/Package.svg"),
        "icons/Backspace.svg" => include_bytes!("../../assets/icons/Backspace.svg"),
        "icons/Camera.svg" => include_bytes!("../../assets/icons/Camera.svg"),
        "icons/Lock.svg" => include_bytes!("../../assets/icons/Lock.svg"),
        "icons/Shield.svg" => include_bytes!("../../assets/icons/Shield.svg"),
        "icons/Monitor.svg" => include_bytes!("../../assets/icons/Monitor.svg"),
        "icons/Wifi.svg" => include_bytes!("../../assets/icons/Wifi.svg"),
        "icons/Clock.svg" => include_bytes!("../../assets/icons/Clock.svg"),
        "icons/Bell.svg" => include_bytes!("../../assets/icons/Bell.svg"),
        "icons/Sun.svg" => include_bytes!("../../assets/icons/Sun.svg"),
        "icons/Smile.svg" => include_bytes!("../../assets/icons/Smile.svg"),
        "icons/Lightbulb.svg" => include_bytes!("../../assets/icons/Lightbulb.svg"),
        "icons/Chat.svg" => include_bytes!("../../assets/icons/Chat.svg"),
        "icons/Attach.svg" => include_bytes!("../../assets/icons/Attach.svg"),
        "icons/Play.svg" => include_bytes!("../../assets/icons/Play.svg"),
        "icons/Pause.svg" => include_bytes!("../../assets/icons/Pause.svg"),
        "icons/Chart.svg" => include_bytes!("../../assets/icons/Chart.svg"),
        "icons/Menu.svg" => include_bytes!("../../assets/icons/Menu.svg"),
        "icons/Cpu.svg" => include_bytes!("../../assets/icons/Cpu.svg"),
        "icons/Wave.svg" => include_bytes!("../../assets/icons/Wave.svg"),
        "icons/Maximize.svg" => include_bytes!("../../assets/icons/Maximize.svg"),
        "icons/Restore.svg" => include_bytes!("../../assets/icons/Restore.svg"),
        "icons/Contrast.svg" => include_bytes!("../../assets/icons/Contrast.svg"),
        "icons/Ban.svg" => include_bytes!("../../assets/icons/Ban.svg"),
        "icons/Coffee.svg" => include_bytes!("../../assets/icons/Coffee.svg"),
        "icons/Heart.svg" => include_bytes!("../../assets/icons/Heart.svg"),
        "icons/Cat.svg" => include_bytes!("../../assets/icons/Cat.svg"),
        "icons/AlertTriangle.svg" => include_bytes!("../../assets/icons/AlertTriangle.svg"),
        "icons/Flip.svg" => include_bytes!("../../assets/icons/Flip.svg"),
        "icons/ArrowDown.svg" => include_bytes!("../../assets/icons/ArrowDown.svg"),
        "icons/ArrowRight.svg" => include_bytes!("../../assets/icons/ArrowRight.svg"),
        "icons/ChevronLeft.svg" => include_bytes!("../../assets/icons/ChevronLeft.svg"),
        "icons/Mute.svg" => include_bytes!("../../assets/icons/Mute.svg"),
        "icons/Bluetooth.svg" => include_bytes!("../../assets/icons/Bluetooth.svg"),
        "icons/Edit.svg" => include_bytes!("../../assets/icons/Edit.svg"),
        "icons/ScanText.svg" => include_bytes!("../../assets/icons/ScanText.svg"),
        "icons/User.svg" => include_bytes!("../../assets/icons/User.svg"),
        "icons/Sparkles.svg" => include_bytes!("../../assets/icons/Sparkles.svg"),
        "icons/Archive.svg" => include_bytes!("../../assets/icons/Archive.svg"),
        "icons/Send.svg" => include_bytes!("../../assets/icons/Send.svg"),
        "icons/CreditCard.svg" => include_bytes!("../../assets/icons/CreditCard.svg"),
        "icons/Key.svg" => include_bytes!("../../assets/icons/Key.svg"),
        "icons/Activity.svg" => include_bytes!("../../assets/icons/Activity.svg"),
        "icons/Gift.svg" => include_bytes!("../../assets/icons/Gift.svg"),
        _ => return None,
    })
}
pub(super) fn draw_icon(icon: UiIcon, size: f32, color: ColorU) -> El {
    ConstrainedBox::new(Icon::new(icon.path(), color).finish())
        .with_width(size)
        .with_height(size)
        .finish()
}
impl Companion {
    pub(super) fn ui_icon(&self, icon: UiIcon, size: f32, color: ColorU) -> El {
        draw_icon(icon, size, color)
    }
}
pub(super) fn symbol(value: &str) -> Option<UiIcon> {
    let trimmed = value.trim_end_matches(['\u{fe0f}', '\u{fe0e}']);
    Some(match trimmed {
        "◈" | "⌂" | "🏠" => UiIcon::Home,
        "❖" => UiIcon::Matrix,
        "✦" => UiIcon::Star,
        "✨" => UiIcon::Sparkles,
        "⚡" => UiIcon::Bolt,
        "✉" => UiIcon::Sms,
        "☎" | "📞" => UiIcon::Phone,
        "📴" => UiIcon::PhoneOff,
        "🚫" => UiIcon::Ban,
        "⊞" | "▣" => UiIcon::Apps,
        "⊟" => UiIcon::Tools,
        "💼" => UiIcon::Briefcase,
        "📌" | "📍" => UiIcon::Pin,
        "⇄" | "⇅" => UiIcon::Connect,
        "📱" => UiIcon::Smartphone,
        "📲" => UiIcon::Send,
        "📁" | "📂" => UiIcon::Folder,
        "📄" | "▤" => UiIcon::File,
        "📝" => UiIcon::ScanText,
        "✎" | "✏" => UiIcon::Edit,
        "🎵" => UiIcon::Music,
        "🎬" | "📹" | "▷" => UiIcon::Video,
        "🖼" => UiIcon::Image,
        "📥" => UiIcon::Download,
        "📤" => UiIcon::Upload,
        "↓" | "⬇" => UiIcon::ArrowDown,
        "⬆" | "↑" => UiIcon::ArrowUp,
        "←" | "⬅" | "◀" => UiIcon::ArrowLeft,
        "➡" => UiIcon::ArrowRight,
        "<" => UiIcon::ChevronLeft,
        "↗" | "↙" => UiIcon::ExternalLink,
        "🔋" => UiIcon::Battery,
        "💾" => UiIcon::Storage,
        "🌡" => UiIcon::Temperature,
        "⧉" => UiIcon::Copy,
        "📋" => UiIcon::Clipboard,
        "⌨" => UiIcon::Keyboard,
        "🔉" | "🔊" => UiIcon::Speaker,
        "🔇" => UiIcon::Mute,
        "🎤" | "🎙" => UiIcon::Mic,
        "↺" | "↶" | "↩" => UiIcon::Retry,
        "↻" | "🔄" => UiIcon::Refresh,
        "▾" => UiIcon::ChevronDown,
        "▴" => UiIcon::ChevronUp,
        "+" | "＋" | "⊕" | "➕" => UiIcon::Plus,
        "−" | "–" | "—" => UiIcon::Minus,
        "✕" | "×" => UiIcon::Close,
        "✓" => UiIcon::Check,
        "🗑" => UiIcon::Trash,
        "ⓘ" | "◉" => UiIcon::Info,
        "⚠" | "🆘" => UiIcon::AlertTriangle,
        "⚙" => UiIcon::Settings,
        "🔍" | "🔎" => UiIcon::Search,
        "📦" => UiIcon::Package,
        "🗜" | "🗂" => UiIcon::Archive,
        "⌫" => UiIcon::Backspace,
        "📷" | "📸" => UiIcon::Camera,
        "🔒" | "🔑" | "🔓" => UiIcon::Lock,
        "🛡" => UiIcon::Shield,
        "💻" | "🖥" => UiIcon::Monitor,
        "📡" | "🌐" | "📶" => UiIcon::Wifi,
        "🔵" => UiIcon::Bluetooth,
        "📅" => UiIcon::Calendar,
        "🕒" | "◷" | "⏰" | "⏳" | "🍅" => UiIcon::Clock,
        "☀" => UiIcon::Sun,
        "🌓" => UiIcon::Contrast,
        "😊" => UiIcon::Smile,
        "💡" => UiIcon::Lightbulb,
        "💬" => UiIcon::Chat,
        "🔔" | "🔕" => UiIcon::Bell,
        "🤖" | "⌘" => UiIcon::Cpu,
        "⤢" | "⛶" => UiIcon::Maximize,
        "📎" => UiIcon::Attach,
        "▶" => UiIcon::Play,
        "⏸" => UiIcon::Pause,
        "📊" | "📈" => UiIcon::Chart,
        "☰" | "≡" => UiIcon::Menu,
        "🌊" => UiIcon::Wave,
        "□" => UiIcon::Monitor,
        "›" | "→" | ">" => UiIcon::ChevronRight,
        "☕" | "🍜" => UiIcon::Coffee,
        "❤" | "♥" | "💊" => UiIcon::Heart,
        "🐱" => UiIcon::Cat,
        "↔" => UiIcon::Flip,
        "👤" => UiIcon::User,
        "💳" => UiIcon::CreditCard,
        "🏃" => UiIcon::Activity,
        "🎂" => UiIcon::Gift,
        _ => return None,
    })
}
/// Only interpret leading UI glyphs (or trailing sort/nav indicators), never symbols embedded in user content.
pub(super) fn label_icon(value: &str) -> Option<(UiIcon, &str)> {
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix("🔍-") {
        return Some((UiIcon::Minus, rest.trim_start()));
    }
    if let Some(rest) = value.strip_prefix("🔍+") {
        return Some((UiIcon::Plus, rest.trim_start()));
    }
    let first_byte = *value.as_bytes().first()?;
    let leading_candidate = !((first_byte < 0x80 && !matches!(first_byte, b'+' | b'<' | b'>'))
        || matches!(first_byte, 0xE4..=0xE9));
    if leading_candidate {
        let first = value.chars().next()?;
        let mut end = first.len_utf8();
        while value[end..].starts_with('\u{fe0f}') || value[end..].starts_with('\u{fe0e}') {
            end += 3;
        }
        if let Some(icon) = symbol(&value[..end]) {
            let rest = &value[end..];
            if rest.is_empty()
                || rest.starts_with(char::is_whitespace)
                || (first as u32 >= 0x2000
                    && !rest.starts_with(|c: char| c.is_ascii_alphanumeric()))
            {
                return Some((icon, rest.trim_start()));
            }
        }
    }
    // Check trailing directional / dismiss indicators on short UI labels (e.g. "降序 ↓", "升序 ↑", "下个月 >", "应用: xxx ×")
    if value.chars().count() <= 24 {
        for (suffix, icon) in [
            (" ↓", UiIcon::ArrowDown),
            (" ↑", UiIcon::ArrowUp),
            (" >", UiIcon::ChevronRight),
            (" ×", UiIcon::Close),
        ] {
            if let Some(prefix) = value.strip_suffix(suffix) {
                if !prefix.trim().is_empty() {
                    return Some((icon, prefix.trim_end()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_and_label_icon_mappings() {
        assert_eq!(symbol("📂"), Some(UiIcon::Folder));
        assert_eq!(
            label_icon("📂 手机文件管理"),
            Some((UiIcon::Folder, "手机文件管理"))
        );

        assert_eq!(symbol("💾"), Some(UiIcon::Storage));
        assert_eq!(
            label_icon("💾 手机存储扩容"),
            Some((UiIcon::Storage, "手机存储扩容"))
        );

        assert_eq!(symbol("🔄"), Some(UiIcon::Refresh));
        assert_eq!(label_icon("🔄 旋转"), Some((UiIcon::Refresh, "旋转")));

        assert_eq!(symbol("⛶"), Some(UiIcon::Maximize));
        assert_eq!(label_icon("⛶ 大图"), Some((UiIcon::Maximize, "大图")));

        assert_eq!(symbol("📅"), Some(UiIcon::Calendar));
        assert_eq!(label_icon("📅"), Some((UiIcon::Calendar, "")));

        assert_eq!(symbol("⏰"), Some(UiIcon::Clock));
        assert_eq!(label_icon("⏰"), Some((UiIcon::Clock, "")));

        assert_eq!(symbol("🔑"), Some(UiIcon::Lock));
        assert_eq!(
            label_icon("🔑 复制验证码"),
            Some((UiIcon::Lock, "复制验证码"))
        );

        assert_eq!(label_icon("🔍-"), Some((UiIcon::Minus, "")));
        assert_eq!(label_icon("🔍+"), Some((UiIcon::Plus, "")));
        assert_eq!(label_icon("+ 新建"), Some((UiIcon::Plus, "新建")));
        assert_eq!(label_icon("普通中文标题"), None);
        assert_eq!(label_icon("Plain ASCII Label"), None);
        assert_eq!(label_icon(""), None);
    }

    #[test]
    fn custom_svg_assets_and_variation_selector_mappings() {
        let custom_icons = [
            UiIcon::Contrast,
            UiIcon::Ban,
            UiIcon::Coffee,
            UiIcon::Heart,
            UiIcon::Cat,
            UiIcon::AlertTriangle,
            UiIcon::Flip,
            UiIcon::ArrowDown,
            UiIcon::ArrowRight,
            UiIcon::ChevronLeft,
            UiIcon::Mute,
            UiIcon::Bluetooth,
            UiIcon::Edit,
            UiIcon::ScanText,
            UiIcon::User,
            UiIcon::Sparkles,
            UiIcon::Archive,
            UiIcon::Send,
            UiIcon::CreditCard,
            UiIcon::Key,
            UiIcon::Activity,
            UiIcon::Gift,
        ];
        for icon in custom_icons {
            let bytes = asset(icon.path()).expect("custom SVG asset must exist");
            let text = std::str::from_utf8(&bytes).expect("SVG must be valid UTF-8");
            assert!(text.contains("<svg"), "invalid SVG for {:?}", icon);
        }

        // Variation Selector-16 (U+FE0F) stripping
        assert_eq!(symbol("🖥️"), Some(UiIcon::Monitor));
        assert_eq!(
            label_icon("🖥️ 投屏窗口"),
            Some((UiIcon::Monitor, "投屏窗口"))
        );
        assert_eq!(symbol("❤️"), Some(UiIcon::Heart));
        assert_eq!(label_icon("❤️ 爱心"), Some((UiIcon::Heart, "爱心")));

        // Newly mapped symbols & no-space CJK prefix
        assert_eq!(symbol("🚫"), Some(UiIcon::Ban));
        assert_eq!(symbol("☕"), Some(UiIcon::Coffee));
        assert_eq!(symbol("🐱"), Some(UiIcon::Cat));
        assert_eq!(symbol("🆘"), Some(UiIcon::AlertTriangle));
        assert_eq!(symbol("🌓"), Some(UiIcon::Contrast));
        assert_eq!(symbol("🔇"), Some(UiIcon::Mute));
        assert_eq!(symbol("🔵"), Some(UiIcon::Bluetooth));
        assert_eq!(symbol("📝"), Some(UiIcon::ScanText));
        assert_eq!(symbol("✨"), Some(UiIcon::Sparkles));
        assert_eq!(label_icon("⚡可秒回"), Some((UiIcon::Bolt, "可秒回")));

        // Trailing sort and filter indicators
        assert_eq!(label_icon("降序 ↓"), Some((UiIcon::ArrowDown, "降序")));
        assert_eq!(label_icon("升序 ↑"), Some((UiIcon::ArrowUp, "升序")));
        assert_eq!(
            label_icon("下个月 >"),
            Some((UiIcon::ChevronRight, "下个月"))
        );
        assert_eq!(
            label_icon("应用: 微信 ×"),
            Some((UiIcon::Close, "应用: 微信"))
        );
    }
}

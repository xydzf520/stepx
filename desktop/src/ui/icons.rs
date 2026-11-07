//! Shared outline icon family. Assets use a 24 px grid and rounded 1.75 px strokes.
//! White SVG masks are required: the renderer uses the red channel as icon alpha.
pub(super) use super::widgets::icon_label;
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
    Usb,
    MoreHorizontal,
    Folder,
    File,
    FilePdf,
    Spreadsheet,
    Presentation,
    Code,

    Music,
    Video,
    Image,
    Download,
    Upload,
    Battery,
    BatteryEmpty,
    BatteryLow,
    BatteryHalf,
    BatteryFull,
    BatteryCharging,
    BatteryPlugged,

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
    Unlock,
    Square,
    Shield,
    Monitor,
    Wifi,
    Clock,
    Bell,
    BellOff,
    Sun,
    CloudRain,
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
    Users,
    MessageCircle,
    Reply,
    CheckAll,
}
#[allow(non_upper_case_globals, dead_code)]
impl UiIcon {
    pub(super) const SpeakerMute: Self = Self::Mute;
    pub(super) const SpeakerOff: Self = Self::Mute;
    pub(super) const Wallet: Self = Self::CreditCard;
    pub(super) const Sos: Self = Self::AlertTriangle;
    pub(super) const FolderOpen: Self = Self::Folder;
    pub(super) const Link: Self = Self::Connect;
    pub(super) const Alert: Self = Self::AlertTriangle;
    pub(super) const ArrowUpRight: Self = Self::ExternalLink;
    pub(super) const Grid: Self = Self::Apps;
    pub(super) const Layers: Self = Self::Archive;
    pub(super) const Database: Self = Self::Storage;
    pub(super) const Quote: Self = Self::Reply;

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
            Self::Usb => "icons/Usb.svg",
            Self::MoreHorizontal => "icons/MoreHorizontal.svg",
            Self::Folder => "icons/Folder.svg",
            Self::File => "icons/File.svg",
            Self::FilePdf => "icons/FilePdf.svg",
            Self::Spreadsheet => "icons/Spreadsheet.svg",
            Self::Presentation => "icons/Presentation.svg",
            Self::Code => "icons/Code.svg",

            Self::Music => "icons/Music.svg",
            Self::Video => "icons/Video.svg",
            Self::Image => "icons/Image.svg",
            Self::Download => "icons/Download.svg",
            Self::Upload => "icons/Upload.svg",
            Self::Battery => "icons/Battery.svg",
            Self::BatteryEmpty => "icons/BatteryEmpty.svg",
            Self::BatteryLow => "icons/BatteryLow.svg",
            Self::BatteryHalf => "icons/BatteryHalf.svg",
            Self::BatteryFull => "icons/BatteryFull.svg",
            Self::BatteryCharging => "icons/BatteryCharging.svg",
            Self::BatteryPlugged => "icons/BatteryPlugged.svg",

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
            Self::Unlock => "icons/Unlock.svg",
            Self::Square => "icons/Square.svg",
            Self::Shield => "icons/Shield.svg",
            Self::Monitor => "icons/Monitor.svg",
            Self::Wifi => "icons/Wifi.svg",
            Self::Clock => "icons/Clock.svg",
            Self::Bell => "icons/Bell.svg",
            Self::BellOff => "icons/BellOff.svg",
            Self::Sun => "icons/Sun.svg",
            Self::CloudRain => "icons/CloudRain.svg",
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
            Self::Users => "icons/Users.svg",
            Self::MessageCircle => "icons/MessageCircle.svg",
            Self::Reply => "icons/Reply.svg",
            Self::CheckAll => "icons/CheckAll.svg",
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
        "icons/Usb.svg" => include_bytes!("../../assets/icons/Usb.svg"),
        "icons/MoreHorizontal.svg" => include_bytes!("../../assets/icons/MoreHorizontal.svg"),
        "icons/Folder.svg" => include_bytes!("../../assets/icons/Folder.svg"),
        "icons/File.svg" => include_bytes!("../../assets/icons/File.svg"),
        "icons/FilePdf.svg" => include_bytes!("../../assets/icons/FilePdf.svg"),
        "icons/Spreadsheet.svg" => include_bytes!("../../assets/icons/Spreadsheet.svg"),
        "icons/Presentation.svg" => include_bytes!("../../assets/icons/Presentation.svg"),
        "icons/Code.svg" => include_bytes!("../../assets/icons/Code.svg"),

        "icons/Music.svg" => include_bytes!("../../assets/icons/Music.svg"),
        "icons/Video.svg" => include_bytes!("../../assets/icons/Video.svg"),
        "icons/Image.svg" => include_bytes!("../../assets/icons/Image.svg"),
        "icons/Download.svg" => include_bytes!("../../assets/icons/Download.svg"),
        "icons/Upload.svg" => include_bytes!("../../assets/icons/Upload.svg"),
        "icons/Battery.svg" => include_bytes!("../../assets/icons/Battery.svg"),
        "icons/BatteryEmpty.svg" => include_bytes!("../../assets/icons/BatteryEmpty.svg"),
        "icons/BatteryLow.svg" => include_bytes!("../../assets/icons/BatteryLow.svg"),
        "icons/BatteryHalf.svg" => include_bytes!("../../assets/icons/BatteryHalf.svg"),
        "icons/BatteryFull.svg" => include_bytes!("../../assets/icons/BatteryFull.svg"),
        "icons/BatteryCharging.svg" => include_bytes!("../../assets/icons/BatteryCharging.svg"),
        "icons/BatteryPlugged.svg" => include_bytes!("../../assets/icons/BatteryPlugged.svg"),

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
        "icons/Unlock.svg" => include_bytes!("../../assets/icons/Unlock.svg"),
        "icons/Square.svg" => include_bytes!("../../assets/icons/Square.svg"),
        "icons/Shield.svg" => include_bytes!("../../assets/icons/Shield.svg"),
        "icons/Monitor.svg" => include_bytes!("../../assets/icons/Monitor.svg"),
        "icons/Wifi.svg" => include_bytes!("../../assets/icons/Wifi.svg"),
        "icons/Clock.svg" => include_bytes!("../../assets/icons/Clock.svg"),
        "icons/Bell.svg" => include_bytes!("../../assets/icons/Bell.svg"),
        "icons/BellOff.svg" => include_bytes!("../../assets/icons/BellOff.svg"),
        "icons/Sun.svg" => include_bytes!("../../assets/icons/Sun.svg"),
        "icons/CloudRain.svg" => include_bytes!("../../assets/icons/CloudRain.svg"),
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
        "icons/Users.svg" => include_bytes!("../../assets/icons/Users.svg"),
        "icons/MessageCircle.svg" => include_bytes!("../../assets/icons/MessageCircle.svg"),
        "icons/Reply.svg" => include_bytes!("../../assets/icons/Reply.svg"),
        "icons/CheckAll.svg" => include_bytes!("../../assets/icons/CheckAll.svg"),
        _ => return None,
    })
}
pub(super) fn draw_icon(icon: UiIcon, size: f32, color: ColorU) -> El {
    ConstrainedBox::new(Icon::new(icon.path(), color).finish())
        .with_width(size)
        .with_height(size)
        .with_min_width(size)
        .with_min_height(size)
        .finish()
}
impl Companion {
    pub(super) fn ui_icon(&self, icon: UiIcon, size: f32, color: ColorU) -> El {
        draw_icon(icon, size, self.p().icon_foreground(color))
    }
}
pub(super) fn symbol(value: &str) -> Option<UiIcon> {
    let trimmed = value.trim_end_matches(['\u{fe0f}', '\u{fe0e}']);
    Some(match trimmed {
        "◈" | "⌂" | "🏠" => UiIcon::Home,
        "❖" => UiIcon::Matrix,
        "✦" => UiIcon::Sparkles,
        "⭐" => UiIcon::Star,
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
        "🎬" | "📹" => UiIcon::Video,
        "🖼" => UiIcon::Image,
        "📥" => UiIcon::Download,
        "📤" => UiIcon::Upload,
        "↓" | "⬇" => UiIcon::ArrowDown,
        "⬆" | "↑" => UiIcon::ArrowUp,
        "←" | "⬅" | "◀" | "◁" | "‹" => UiIcon::ArrowLeft,
        "➡" | "▷" => UiIcon::ArrowRight,
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
        "✓" | "✅" | "☑" => UiIcon::Check,
        "☐" => UiIcon::Square,
        "🗑" => UiIcon::Trash,
        "ⓘ" | "◉" => UiIcon::Info,
        "⚠" | "🆘" | "🚨" => UiIcon::AlertTriangle,
        "⚙" => UiIcon::Settings,
        "🔍" | "🔎" => UiIcon::Search,
        "📦" | "🛍" => UiIcon::Package,
        "🗜" | "🗂" => UiIcon::Archive,
        "⌫" => UiIcon::Backspace,
        "📷" | "📸" => UiIcon::Camera,
        "🔒" | "🔐" => UiIcon::Lock,
        "🔓" => UiIcon::Unlock,
        "🔑" => UiIcon::Key,
        "🛡" => UiIcon::Shield,
        "💻" | "🖥" => UiIcon::Monitor,
        "📡" | "🌐" | "📶" | "G" => UiIcon::Wifi,
        "🔌" => UiIcon::Usb,
        "🔵" => UiIcon::Bluetooth,
        "📅" => UiIcon::Calendar,
        "🕒" | "◷" | "⏰" | "⏳" | "🍅" | "⏱" => UiIcon::Clock,
        "☀" => UiIcon::Sun,
        "🌧" => UiIcon::CloudRain,
        "🌓" => UiIcon::Contrast,
        "😊" => UiIcon::Smile,
        "💡" => UiIcon::Lightbulb,
        "💬" => UiIcon::Chat,
        "🔔" => UiIcon::Bell,
        "🔕" => UiIcon::BellOff,
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
        "👥" => UiIcon::Users,
        "💳" => UiIcon::CreditCard,
        "🏃" => UiIcon::Activity,
        "🎂" => UiIcon::Gift,
        _ => return None,
    })
}
/// Only interpret leading UI glyphs, never bare ASCII symbols or symbols embedded in user content.
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
            let matched = if first.is_ascii() {
                rest.starts_with(' ') && !rest.trim_start().is_empty()
            } else {
                rest.is_empty()
                    || rest.starts_with(char::is_whitespace)
                    || (first as u32 >= 0x2000
                        && !rest.starts_with(|c: char| c.is_ascii_alphanumeric()))
            };
            if matched {
                return Some((icon, rest.trim_start()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entire_functional_svg_family_obeys_the_mask_contract() {
        use facloud_ui::{assets::asset_cache::Asset, image_cache::ImageType};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|x| x.to_str()) != Some("svg") {
                continue;
            }
            let key = format!("icons/{}", path.file_name().unwrap().to_str().unwrap());
            let bytes = asset(&key).unwrap_or_else(|| panic!("SVG not bundled: {key}"));
            assert!(
                ImageType::try_from_bytes(bytes).is_ok(),
                "cannot render {key}"
            );
            let svg = std::str::from_utf8(bytes).unwrap();
            for attr in [
                "viewBox=\"0 0 24 24\"",
                "stroke-width=\"1.75\"",
                "stroke-linecap=\"round\"",
                "stroke-linejoin=\"round\"",
            ] {
                assert!(svg.contains(attr), "{key}: missing {attr}");
            }
            assert!(
                svg.contains("stroke=\"white\"") || svg.contains("stroke=\"#FFFFFF\""),
                "{key}: non-mask stroke"
            );
            count += 1;
        }
        assert!(count >= 100);
    }

    #[test]
    fn state_glyphs_remain_distinct_and_embedded_user_symbols_stay_text() {
        for (glyph, expected) in [
            ("🌧️", UiIcon::CloudRain),
            ("☀️", UiIcon::Sun),
            ("🔕", UiIcon::BellOff),
            ("🔔", UiIcon::Bell),
            ("🔓", UiIcon::Unlock),
            ("🔒", UiIcon::Lock),
            ("🔑", UiIcon::Key),
            ("☐", UiIcon::Square),
            ("☑", UiIcon::Check),
            ("✦", UiIcon::Sparkles),
            ("⭐", UiIcon::Star),
        ] {
            assert_eq!(symbol(glyph), Some(expected));
        }
        assert_eq!(label_icon("☐ 未完成"), Some((UiIcon::Square, "未完成")));
        assert_eq!(label_icon("☑ 已完成"), Some((UiIcon::Check, "已完成")));
        assert_eq!(label_icon("🔓已解锁"), Some((UiIcon::Unlock, "已解锁")));
        assert_eq!(label_icon("⭐ 收藏"), Some((UiIcon::Star, "收藏")));
        for text in ["天气 🌧️", "解锁状态🔓", "我的备注：☐ 未完成", "收藏 ⭐"]
        {
            assert_eq!(label_icon(text), None, "embedded user symbols: {text}");
        }
        for (left, right) in [
            (UiIcon::CloudRain, UiIcon::Sun),
            (UiIcon::BellOff, UiIcon::Bell),
            (UiIcon::Unlock, UiIcon::Lock),
            (UiIcon::Square, UiIcon::Check),
            (UiIcon::Star, UiIcon::Sparkles),
        ] {
            assert_ne!(asset(left.path()), asset(right.path()));
        }
    }

    #[test]
    fn new_state_assets_parse_and_keep_the_shared_white_outline_mask() {
        use facloud_ui::{assets::asset_cache::Asset, image_cache::ImageType};
        for icon in [
            UiIcon::CloudRain,
            UiIcon::BellOff,
            UiIcon::Unlock,
            UiIcon::Square,
            UiIcon::Star,
        ] {
            let bytes = asset(icon.path()).expect("state icon asset must exist");
            assert!(
                ImageType::try_from_bytes(bytes).is_ok(),
                "invalid SVG: {icon:?}"
            );
            let svg = std::str::from_utf8(bytes).unwrap();
            for attribute in [
                "viewBox=\"0 0 24 24\"",
                "fill=\"none\"",
                "stroke=\"white\"",
                "stroke-width=\"1.75\"",
                "stroke-linecap=\"round\"",
                "stroke-linejoin=\"round\"",
            ] {
                assert!(
                    svg.contains(attribute),
                    "outline contract {attribute}: {icon:?}"
                );
            }
        }
    }

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

        assert_eq!(symbol("🔑"), Some(UiIcon::Key));
        assert_eq!(
            label_icon("🔑 复制验证码"),
            Some((UiIcon::Key, "复制验证码"))
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

        // Hollow triangles and bare ASCII symbols / trailing suffixes
        assert_eq!(symbol("◁"), Some(UiIcon::ArrowLeft));
        assert_eq!(symbol("▷"), Some(UiIcon::ArrowRight));
        assert_eq!(label_icon("+"), None);
        assert_eq!(label_icon("+86"), None);
        assert_eq!(label_icon("降序 ↓"), None);
        assert_eq!(label_icon("升序 ↑"), None);
        assert_eq!(label_icon("下个月 >"), None);
        assert_eq!(label_icon("应用: 微信 ×"), None);
    }
}

//! Shared source placeholders. Functional glyphs remain monochrome masks;
//! actual application logos continue to use their original `Image` assets.
use super::{
    icons::UiIcon,
    theme::{rgb, Palette},
};
use facloud_ui::color::ColorU;
use stepx_companion::push::ImChannel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceIconStyle {
    pub icon: UiIcon,
    pub foreground: ColorU,
    pub background: ColorU,
}

pub(super) fn neutral_source_style(icon: UiIcon, p: &Palette) -> SourceIconStyle {
    SourceIconStyle {
        icon,
        foreground: p.text2,
        background: p.card2,
    }
}

/// Content categories share a small, theme-aware palette. These are never
/// applied to navigation or action buttons; colour complements the glyph/label.
pub(super) fn content_style(icon: UiIcon, p: &Palette) -> SourceIconStyle {
    use UiIcon::*;
    let (light, dark) = match icon {
        File | Chat | Sms | MessageCircle | Users | User | Phone | Monitor | Smartphone
        | Calendar | Briefcase => (0x2563a6, 0x86b7f2),
        Spreadsheet | Image | Camera | Heart | Activity => (0x147862, 0x6dd4b5),
        FilePdf => (0xb94b54, 0xf39ba4),
        Sparkles | Apps | Package | Matrix | Video | Music | Mic | Code => (0x7854ac, 0xc2a4ef),
        Folder | Archive | Presentation | Sun | Coffee | Gift => (0x986418, 0xe7bd76),
        Storage | Clock | CloudRain | Tools | Cpu | Shield | Chart => (0x197d91, 0x7acddc),
        _ => return neutral_source_style(icon, p),
    };
    let hex = if p.dark { dark } else { light };
    let foreground = rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    SourceIconStyle {
        icon,
        foreground,
        background: super::theme::lerp_color(p.bg, foreground, if p.dark { 0.14 } else { 0.08 }),
    }
}

/// File identity is based on the file extension, never guessed from its title.
pub(super) fn file_content_icon(name: &str, directory: bool) -> UiIcon {
    if directory {
        return UiIcon::Folder;
    }
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => UiIcon::FilePdf,
        "xlsx" | "xls" | "csv" | "ods" | "tsv" => UiIcon::Spreadsheet,
        "ppt" | "pptx" | "odp" => UiIcon::Presentation,
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "heic" | "heif" | "avif" | "svg" | "bmp" => {
            UiIcon::Image
        }
        "mp4" | "mov" | "mkv" | "webm" | "avi" => UiIcon::Video,
        "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac" | "opus" => UiIcon::Music,
        "zip" | "rar" | "7z" | "gz" | "tar" => UiIcon::Archive,
        "apk" | "aab" => UiIcon::Package,
        "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "html" | "css" | "json" | "yaml" | "toml"
        | "sh" => UiIcon::Code,
        _ => UiIcon::File,
    }
}

pub(super) fn file_content_label(name: &str) -> &'static str {
    match file_content_icon(name, false) {
        UiIcon::FilePdf => "PDF",
        UiIcon::Spreadsheet => "表格",
        UiIcon::Presentation => "演示文稿",
        UiIcon::Image => "图片",
        UiIcon::Video => "视频",
        UiIcon::Music => "音频",
        UiIcon::Archive => "压缩包",
        UiIcon::Package => "安装包",
        UiIcon::Code => "代码",
        _ => match std::path::Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "markdown" => "文档",
            _ => "文件",
        },
    }
}

/// Known source colors identify the application, not delivery/success status.
/// These darker badge shades keep small white outlines readable. They are
/// placeholders rather than modified or invented official brand logos.
pub(super) fn channel_source_style(channel: ImChannel, p: &Palette) -> SourceIconStyle {
    let background = match channel {
        ImChannel::WeChat => rgb(0x07, 0x88, 0x3B),
        ImChannel::WeCom => rgb(0x2B, 0x6C, 0xB0),
        ImChannel::DingTalk => rgb(0x00, 0x6A, 0xD4),
        ImChannel::Feishu => rgb(0x25, 0x63, 0xEB),
        ImChannel::QqTim => rgb(0x03, 0x69, 0xA1),
        ImChannel::Sms => return neutral_source_style(UiIcon::Sms, p),
        ImChannel::All | ImChannel::GlobalIm | ImChannel::SocialDm => {
            return neutral_source_style(UiIcon::Chat, p);
        }
    };
    SourceIconStyle {
        icon: UiIcon::Chat,
        foreground: ColorU::white(),
        background,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        notification_center_state::NotificationVisualKind, pages::home::channel_avatar_spec,
        Companion,
    };

    fn contrast(a: ColorU, b: ColorU) -> f64 {
        let luminance = |c: ColorU| {
            let linear = |value: u8| {
                let value = value as f64 / 255.;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn content_file_types_are_distinct_and_readable_in_both_themes() {
        for dark in [false, true] {
            let p = Palette::for_theme(dark);
            for (name, expected) in [
                ("报告.PDF", UiIcon::FilePdf),
                ("预算.xlsx", UiIcon::Spreadsheet),
                ("方案.pptx", UiIcon::Presentation),
                ("照片.jpg", UiIcon::Image),
                ("视频.mp4", UiIcon::Video),
                ("录音.m4a", UiIcon::Music),
                ("备份.zip", UiIcon::Archive),
            ] {
                assert_eq!(file_content_icon(name, false), expected);
                let style = content_style(expected, &p);
                assert!(
                    contrast(style.foreground, style.background) >= 3.,
                    "{name}, dark={dark}"
                );
                assert!(contrast(style.foreground, p.bg) >= 3.);
            }
            assert_eq!(file_content_icon("PDF 文件夹", true), UiIcon::Folder);
            // Navigation and unbranded sources are unaffected by content category colours.
            assert_eq!(neutral_source_style(UiIcon::Phone, &p).foreground, p.text2);
        }
    }

    #[test]
    fn known_source_badges_match_across_home_compact_and_notifications() {
        for dark in [false, true] {
            let app = Companion::new_for_test();
            let p = Palette::for_theme(dark);
            app.palette.set(p);
            for (package, channel, visual) in [
                (
                    "com.tencent.mm",
                    ImChannel::WeChat,
                    Some(NotificationVisualKind::WeChat),
                ),
                ("com.tencent.wework", ImChannel::WeCom, None),
                (
                    "com.alibaba.android.rimet",
                    ImChannel::DingTalk,
                    Some(NotificationVisualKind::DingTalk),
                ),
                (
                    "com.ss.android.lark",
                    ImChannel::Feishu,
                    Some(NotificationVisualKind::Feishu),
                ),
                ("com.tencent.mobileqq", ImChannel::QqTim, None),
            ] {
                let style = channel_source_style(channel, &p);
                assert_eq!(style.icon, UiIcon::Chat);
                assert!(
                    contrast(style.foreground, style.background) >= 3.,
                    "Unreadable source badge for {package}"
                );
                let (icon, _, bg, fg) = channel_avatar_spec(package, "", &p);
                assert_eq!(
                    (icon, fg, bg),
                    (style.icon, style.foreground, style.background)
                );
                assert_eq!(
                    Companion::im_package_colors(package, &p),
                    (style.foreground, style.background)
                );
                if let Some(visual) = visual {
                    assert_eq!(
                        app.notification_visual_style(visual),
                        (style.icon, style.foreground, style.background)
                    );
                }
            }
        }
    }

    #[test]
    fn unbranded_sms_and_grouped_social_sources_do_not_invent_brand_or_status_colors() {
        for dark in [false, true] {
            let p = Palette::for_theme(dark);
            for package in [
                "",
                "unrecognized.example",
                "org.telegram.messenger",
                "com.xingin.xhs",
                "sms",
            ] {
                let style = channel_source_style(ImChannel::from_package(package), &p);
                assert_eq!((style.foreground, style.background), (p.text2, p.card2));
                assert_eq!(
                    Companion::im_package_colors(package, &p),
                    (p.text2, p.card2)
                );
                let (icon, _, bg, fg) = channel_avatar_spec(package, "来源名称", &p);
                assert_eq!(
                    (icon, fg, bg),
                    (style.icon, style.foreground, style.background)
                );
            }
        }
    }

    #[test]
    fn sync_domain_icons_are_neutral_while_actual_failure_status_remains_distinct() {
        use crate::ui::auto_sync::AutoSyncDomain;
        for dark in [false, true] {
            let app = Companion::new_for_test();
            let p = Palette::for_theme(dark);
            app.palette.set(p);
            for domain in AutoSyncDomain::ALL {
                let (_, fg, bg) = app.auto_sync_domain_visual(domain);
                assert_eq!((fg, bg), (p.text2, p.card2));
                assert_ne!(fg, p.danger);
            }
            let (icon, fg, bg) = app.notification_visual_style(NotificationVisualKind::SystemAlert);
            assert_eq!(
                (icon, fg, bg),
                (UiIcon::AlertTriangle, p.danger, p.danger_soft)
            );
        }
    }
}

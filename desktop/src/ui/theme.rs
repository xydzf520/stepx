//! Design tokens. See `desktop/DESIGN.md` for the rationale behind every value.

use facloud_ui::{
    color::ColorU,
    fonts::{Cache as FontCache, FamilyId},
    platform::SystemTheme,
};

// Spacing scale (multiples of 4).
pub const S1: f32 = 4.;
pub const S2: f32 = 8.;
pub const S3: f32 = 12.;
pub const S4: f32 = 16.;
pub const S5: f32 = 24.;
pub const S6: f32 = 32.;

// Type scale.
pub const TITLE: f32 = 16.;
pub const SUBHEAD: f32 = 14.;
pub const HEADING: f32 = 13.;
pub const BODY: f32 = 12.;
pub const CAPTION: f32 = 12.;
pub const MICRO: f32 = 11.;
pub const METRIC: f32 = 24.;

// Radii.
pub const R_CONTROL: f32 = 8.;
pub const R_PANEL: f32 = 12.;
pub const R_TAG: f32 = 6.;

// Fixed layout sizes.
pub const SIDEBAR_W: f32 = 216.;
pub const LIST_PANE_W: f32 = 296.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
    }

    pub fn is_dark(self, system: SystemTheme) -> bool {
        match self {
            Self::System => matches!(system, SystemTheme::Dark),
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

pub const fn rgb(r: u8, g: u8, b: u8) -> ColorU {
    ColorU { r, g, b, a: 255 }
}

pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> ColorU {
    ColorU { r, g, b, a }
}

pub fn lerp_color(a: ColorU, b: ColorU, t: f32) -> ColorU {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    ColorU {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
        a: ch(a.a, b.a),
    }
}

const fn hex(rgb: u32) -> ColorU {
    ColorU {
        r: ((rgb >> 16) & 0xff) as u8,
        g: ((rgb >> 8) & 0xff) as u8,
        b: (rgb & 0xff) as u8,
        a: 255,
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: ColorU,
    pub workspace: ColorU,
    pub sidebar: ColorU,
    pub surface: ColorU,
    pub surface_border: ColorU,
    pub hover: ColorU,
    pub selected: ColorU,
    pub border: ColorU,
    pub border_strong: ColorU,
    pub text: ColorU,
    pub text2: ColorU,
    pub text3: ColorU,
    pub accent: ColorU,
    pub accent_hover: ColorU,
    pub accent_soft: ColorU,
    pub accent_fg: ColorU,
    pub on_accent: ColorU,
    pub blue: ColorU,
    pub indigo: ColorU,
    pub pink: ColorU,
    pub success: ColorU,
    pub success_text: ColorU,
    pub success_soft: ColorU,
    pub warning: ColorU,
    pub warning_soft: ColorU,
    pub warning_border: ColorU,
    pub danger: ColorU,
    pub danger_soft: ColorU,
    pub tag_bg: ColorU,
}

impl Palette {
    pub fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                dark: true,
                bg: hex(0x18181C),
                workspace: hex(0x0F0F12),
                sidebar: hex(0x141417),
                surface: hex(0x222227),
                surface_border: hex(0x2E2E36),
                hover: hex(0x27272D),
                selected: hex(0x27272A),
                border: hex(0x27272A),
                border_strong: hex(0x3F3F46),
                text: hex(0xF4F4F6),
                text2: hex(0xA1A1AA),
                text3: hex(0x71717A),
                accent: hex(0xF4F4F6),
                accent_hover: hex(0xFFFFFF),
                accent_soft: hex(0x27272A),
                accent_fg: hex(0x18181B),
                on_accent: hex(0x18181B),
                blue: hex(0x3B82F6),
                indigo: hex(0x6366F1),
                pink: hex(0xEC4899),
                success: hex(0x10B981),
                success_text: hex(0x10B981),
                success_soft: ColorU {
                    r: 16,
                    g: 185,
                    b: 129,
                    a: 36,
                },
                warning: hex(0xF59E0B),
                warning_soft: ColorU {
                    r: 245,
                    g: 158,
                    b: 11,
                    a: 26,
                },
                warning_border: ColorU {
                    r: 245,
                    g: 158,
                    b: 11,
                    a: 80,
                },
                danger: hex(0xEF4444),
                danger_soft: ColorU {
                    r: 239,
                    g: 68,
                    b: 68,
                    a: 32,
                },
                tag_bg: hex(0x27272A),
            }
        } else {
            Self {
                dark: false,
                bg: hex(0xFFFFFF),
                workspace: hex(0xFAFAFB),
                sidebar: hex(0xF4F4F6),
                surface: hex(0xF8F8FA),
                surface_border: hex(0xEBEBEF),
                hover: hex(0xF1F1F4),
                selected: hex(0xFFFFFF),
                border: hex(0xE5E5EA),
                border_strong: hex(0xD4D4D8),
                text: hex(0x18181B),
                text2: hex(0x71717A),
                text3: hex(0xA1A1AA),
                accent: hex(0x18181B),
                accent_hover: hex(0x27272A),
                accent_soft: hex(0xF4F4F6),
                accent_fg: hex(0xFFFFFF),
                on_accent: hex(0xFFFFFF),
                blue: hex(0x3B82F6),
                indigo: hex(0x6366F1),
                pink: hex(0xEC4899),
                success: hex(0x10B981),
                success_text: hex(0x059669),
                success_soft: ColorU {
                    r: 16,
                    g: 185,
                    b: 129,
                    a: 28,
                },
                warning: hex(0xF59E0B),
                warning_soft: hex(0xFFFBEB),
                warning_border: ColorU {
                    r: 245,
                    g: 158,
                    b: 11,
                    a: 102,
                },
                danger: hex(0xEF4444),
                danger_soft: hex(0xFEE2E2),
                tag_bg: hex(0xF4F4F5),
            }
        }
    }

    pub fn press_color(&self, base: ColorU) -> ColorU {
        lerp_color(
            base,
            self.border_strong,
            if self.dark { 0.35 } else { 0.42 },
        )
    }
}

fn linux_installed_font_families() -> Option<std::collections::HashSet<String>> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    // Probing via fc-list avoids a NULL-pointer panic inside fontconfig bindings when a
    // requested family is missing (seen with Rust 1.92 debug builds).
    let output = std::process::Command::new("fc-list")
        .args([":", "family"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let set: std::collections::HashSet<String> = text
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    (!set.is_empty()).then_some(set)
}

fn load_first(
    cache: &mut FontCache,
    installed: &Option<std::collections::HashSet<String>>,
    names: &[&str],
) -> Option<FamilyId> {
    names.iter().find_map(|name| {
        installed
            .as_ref()
            .is_none_or(|set| set.contains(*name))
            .then(|| cache.load_system_font(name).ok())
            .flatten()
    })
}

pub fn load_fonts(cache: &mut FontCache) -> (FamilyId, FamilyId) {
    let installed = linux_installed_font_families();
    let ui: &[&str] = if cfg!(target_os = "macos") {
        &[
            "PingFang SC",
            "Hiragino Sans GB",
            ".AppleSystemUIFont",
            "Helvetica Neue",
        ]
    } else if cfg!(windows) {
        &[
            "Microsoft YaHei UI",
            "Microsoft YaHei",
            "Segoe UI",
            "SimHei",
        ]
    } else {
        &[
            "Noto Sans CJK SC",
            "Source Han Sans CN",
            "WenQuanYi Micro Hei",
            "Droid Sans Fallback",
            "Ubuntu Sans",
            "Ubuntu",
            "Cantarell",
            "DejaVu Sans",
        ]
    };
    let ui_font = load_first(cache, &installed, ui)
        .or_else(|| {
            load_first(
                cache,
                &installed,
                &["DejaVu Sans", "Ubuntu", "Arial", "Helvetica", "Segoe UI"],
            )
        })
        .expect("未找到可用的系统字体，请安装中文字体");
    let mono: &[&str] = if cfg!(target_os = "macos") {
        &["SF Mono", "Menlo", "Monaco"]
    } else if cfg!(windows) {
        &["Cascadia Mono", "Consolas"]
    } else {
        &[
            "JetBrains Mono",
            "Noto Sans Mono CJK SC",
            "Noto Sans Mono",
            "Ubuntu Sans Mono",
            "Ubuntu Mono",
            "DejaVu Sans Mono",
        ]
    };
    let mono_font = load_first(cache, &installed, mono).unwrap_or(ui_font);
    (ui_font, mono_font)
}

//! The physical matrix is 17 columns by 23 rows. Twelve corner cells have no LED.
//! Rendering is pure; device playback is bounded, cancellable, and restores settings.
pub use crate::device::is_backscreen_corner;
use crate::{
    adb::{shell_quote, Adb},
    device::Device,
};
use anyhow::{bail, Context, Result};
use chrono::{Local, Timelike};
use image::{imageops, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub const WIDTH: usize = 17;
pub const HEIGHT: usize = 23;
pub const SLOTS: usize = WIDTH * HEIGHT;
pub const LEDS: usize = 379;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Astronaut,
    BlinkCat,
    Heart,
    Meteor,
    Rain,
    Wave,
    Orbit,
    Firework,
    Coffee,
    Equalizer,
    Invader,
    Breath,
    Clock,
    Countdown,
    Battery,
}
impl Preset {
    pub const ALL: [Self; 15] = [
        Self::Astronaut,
        Self::BlinkCat,
        Self::Heart,
        Self::Meteor,
        Self::Rain,
        Self::Wave,
        Self::Orbit,
        Self::Firework,
        Self::Coffee,
        Self::Equalizer,
        Self::Invader,
        Self::Breath,
        Self::Clock,
        Self::Countdown,
        Self::Battery,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Astronaut => "astronaut",
            Self::BlinkCat => "blink_cat",
            Self::Heart => "heart",
            Self::Meteor => "meteor",
            Self::Rain => "rain",
            Self::Wave => "wave",
            Self::Orbit => "orbit",
            Self::Firework => "firework",
            Self::Coffee => "coffee",
            Self::Equalizer => "equalizer",
            Self::Invader => "invader",
            Self::Breath => "breath",
            Self::Clock => "clock",
            Self::Countdown => "countdown",
            Self::Battery => "battery",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Astronaut => "旋转太空人",
            Self::BlinkCat => "眨眼猫咪",
            Self::Heart => "怦然心动",
            Self::Meteor => "流星许愿",
            Self::Rain => "像素雨幕",
            Self::Wave => "潮汐波浪",
            Self::Orbit => "星球环游",
            Self::Firework => "口袋烟花",
            Self::Coffee => "咖啡冒泡",
            Self::Equalizer => "跳动节拍",
            Self::Invader => "太空来客",
            Self::Breath => "呼吸光环",
            Self::Clock => "像素时钟",
            Self::Countdown => "专注倒计时",
            Self::Battery => "手机电量",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Astronaut => "经典漂浮旋转 · 4秒一圈",
            Self::BlinkCat => "眨眼与摇尾的小伙伴",
            Self::Heart => "两拍心跳，渐亮渐暗",
            Self::Meteor => "带着光尾划过夜空",
            Self::Rain => "层次分明的数字雨",
            Self::Wave => "柔和起伏的亮度波",
            Self::Orbit => "小行星环绕旋转",
            Self::Firework => "升空、绽放与消散",
            Self::Coffee => "蒸汽缓缓上升",
            Self::Equalizer => "节奏动画 · 非音频采集",
            Self::Invader => "来自街机的太空朋友",
            Self::Breath => "慢呼吸与扩散光环",
            Self::Clock => "读取手机时间，每分钟更新",
            Self::Countdown => "按播放时长递减至零",
            Self::Battery => "读取手机真实电量",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaybackConfig {
    pub brightness: u8,
    pub fps: u8,
    pub duration_secs: u32,
    pub invert: bool,
}
impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            brightness: 5,
            fps: 6,
            duration_secs: 15,
            invert: false,
        }
    }
}
impl PlaybackConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=8).contains(&self.brightness)
            || !(1..=12).contains(&self.fps)
            || !(1..=3600).contains(&self.duration_secs)
        {
            bail!("亮度需为1–8、帧率1–12、播放时长1–3600秒")
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub enum Source {
    Preset(Preset),
    Image(Vec<u8>),
    App { package: String, preset: Preset },
    SmartCarousel(Vec<SmartDisplaySlide>),
}
#[derive(Clone, Debug)]
pub struct LiveData {
    pub battery_percent: Option<u8>,
    pub hour: u8,
    pub minute: u8,
}
impl Default for LiveData {
    fn default() -> Self {
        let t = Local::now();
        Self {
            battery_percent: None,
            hour: t.hour() as u8,
            minute: t.minute() as u8,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
}
#[derive(Clone, Debug, Default)]
pub struct ImageOptions {
    pub dither: bool,
    pub threshold: Option<u8>,
    pub fit: ImageFit,
    pub rotation_deg: u16,
    pub flip_h: bool,
}

/// Alpha is composited over black and the aspect ratio is preserved before sampling.
pub fn image_frame(path: &Path, options: &ImageOptions) -> Result<Vec<u8>> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let (w, h) = reader.into_dimensions()?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 64_000_000 {
        bail!("图片尺寸为空或超过6400万像素")
    }
    let mut rgba = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
        .context("无法读取图片")?
        .to_rgba8();
    match options.rotation_deg % 360 {
        90 => rgba = imageops::rotate90(&rgba),
        180 => rgba = imageops::rotate180(&rgba),
        270 => rgba = imageops::rotate270(&rgba),
        _ => {}
    }
    if options.flip_h {
        rgba = imageops::flip_horizontal(&rgba);
    }
    let (w, h) = rgba.dimensions();
    let resized = match options.fit {
        ImageFit::Contain => imageops::resize(
            &rgba,
            ((w as f32 * (17.0 / w as f32).min(23.0 / h as f32)).round() as u32).max(1),
            ((h as f32 * (17.0 / w as f32).min(23.0 / h as f32)).round() as u32).max(1),
            imageops::FilterType::Triangle,
        ),
        ImageFit::Cover => {
            let (crop_w, crop_h) = if u64::from(w) * 23 > u64::from(h) * 17 {
                ((u64::from(h) * 17 / 23).max(1) as u32, h)
            } else {
                (w, (u64::from(w) * 23 / 17).max(1) as u32)
            };
            let crop =
                imageops::crop_imm(&rgba, (w - crop_w) / 2, (h - crop_h) / 2, crop_w, crop_h)
                    .to_image();
            imageops::resize(&crop, 17, 23, imageops::FilterType::Triangle)
        }
    };
    let mut canvas = RgbaImage::new(17, 23);
    imageops::overlay(
        &mut canvas,
        &resized,
        (17i64 - resized.width() as i64) / 2,
        (23i64 - resized.height() as i64) / 2,
    );
    let mut luminance = vec![0f32; SLOTS];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let p = canvas.get_pixel(x as u32, y as u32).0;
            luminance[y * WIDTH + x] =
                (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) * p[3] as f32
                    / 255.;
        }
    }
    let mut out = vec![0; SLOTS];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if is_backscreen_corner(x, y) {
                continue;
            }
            let i = y * WIDTH + x;
            let old = luminance[i].clamp(0., 255.);
            let new = if options.dither || options.threshold.is_some() {
                if old >= options.threshold.unwrap_or(128) as f32 {
                    255.
                } else {
                    0.
                }
            } else {
                old
            };
            out[i] = new.round() as u8;
            if options.dither {
                let error = old - new;
                for (dx, dy, factor) in [
                    (1, 0, 7. / 16.),
                    (-1, 1, 3. / 16.),
                    (0, 1, 5. / 16.),
                    (1, 1, 1. / 16.),
                ] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0
                        && nx < 17
                        && ny < 23
                        && !is_backscreen_corner(nx as usize, ny as usize)
                    {
                        luminance[ny as usize * WIDTH + nx as usize] += error * factor;
                    }
                }
            }
        }
    }
    Ok(out)
}

fn put(f: &mut [u8], x: i32, y: i32, v: u8) {
    if (0..17).contains(&x) && (0..23).contains(&y) && !is_backscreen_corner(x as usize, y as usize)
    {
        let i = y as usize * WIDTH + x as usize;
        f[i] = f[i].max(v);
    }
}
fn sprite(f: &mut [u8], rows: &[&str], x: i32, y: i32, v: u8) {
    for (dy, row) in rows.iter().enumerate() {
        for (dx, c) in row.bytes().enumerate() {
            if c != b' ' && c != b'.' {
                put(f, x + dx as i32, y + dy as i32, v)
            }
        }
    }
}
const DIGITS: [[&str; 5]; 10] = [
    ["111", "1 1", "1 1", "1 1", "111"],
    [" 1 ", "11 ", " 1 ", " 1 ", "111"],
    ["111", "  1", "111", "1  ", "111"],
    ["111", "  1", "111", "  1", "111"],
    ["1 1", "1 1", "111", "  1", "  1"],
    ["111", "1  ", "111", "  1", "111"],
    ["111", "1  ", "111", "1 1", "111"],
    ["111", "  1", "  1", " 1 ", " 1 "],
    ["111", "1 1", "111", "1 1", "111"],
    ["111", "1 1", "111", "  1", "111"],
];
fn number(f: &mut [u8], n: u32, x: i32, y: i32, padded: bool) {
    let s = if padded {
        format!("{n:02}")
    } else {
        n.to_string()
    };
    for (i, c) in s.chars().enumerate() {
        if let Some(d) = c.to_digit(10) {
            sprite(f, &DIGITS[d as usize], x + i as i32 * 4, y, 255)
        }
    }
}

/// An original monochrome adaptation of the classic diagonal, body-axis spinning
/// astronaut. Draw the suit in continuous coordinates, then sample each LED at
/// 3×3 subpixels. The visor, chest and backpack change with yaw; the complete
/// image does not spin like a flat card. One full turn takes four seconds.
fn astronaut_frame(seconds: f32) -> Vec<u8> {
    let phase = seconds.rem_euclid(4.) / 4.;
    let theta = phase * std::f32::consts::TAU;
    let (yaw_s, yaw_c) = theta.sin_cos();
    let tilt = -0.28f32;
    let (tilt_s, tilt_c) = tilt.sin_cos();
    let bob = (theta * 2.).sin() * 0.30;
    let mut pixels = vec![0; SLOTS];
    let capsule = |x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32| {
        let dx = bx - ax;
        let dy = by - ay;
        let u = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy).max(0.001)).clamp(0., 1.);
        (x - ax - u * dx).powi(2) + (y - ay - u * dy).powi(2) <= r * r
    };
    let sample = |x: f32, y: f32| -> u8 {
        let mut value = 0;
        // Backpack protrudes on the far side in profile.
        let pack_x = -yaw_s * 2.0;
        if (x - pack_x).abs() < 1.9 && (-1.4..3.4).contains(&y) {
            value = 100;
        }
        // Two legs retain a dark gap in front and overlap naturally in profile.
        for side in [-1f32, 1.] {
            let leg_x = side * 1.2 * yaw_c;
            let depth = side * yaw_s;
            if capsule(x, y, leg_x, 3.2, leg_x + side * 0.40 * yaw_c, 7.3, 0.85) {
                value = if depth > 0. { 235 } else { 160 };
            }
            if capsule(
                x,
                y,
                leg_x + side * 0.40 * yaw_c,
                7.4,
                leg_x + side * 0.85 * yaw_c + 0.15 * yaw_s,
                8.0,
                0.85,
            ) {
                value = if depth > 0. { 255 } else { 185 };
            }
        }
        // Far arm first. Near arm is redrawn after the torso.
        let arm = |side: f32| {
            let shoulder = side * 2.05 * yaw_c + 0.10 * yaw_s;
            let elbow = side * 3.25 * yaw_c + 0.65 * yaw_s;
            let hand = side * 3.50 * yaw_c + 0.95 * yaw_s;
            capsule(x, y, shoulder, -1.55, elbow, 0.5, 0.72)
                || capsule(x, y, elbow, 0.5, hand, 2.05, 0.76)
        };
        let near_side = if yaw_s >= 0. { 1. } else { -1. };
        if arm(-near_side) {
            value = 145;
        }
        // Rounded pressure suit body; the shoulder ring joins the helmet.
        let half = 1.55 + 0.8 * yaw_c.abs();
        if capsule(x, y, 0., -1.55, 0., 3.0, half) {
            value = 220;
        }
        if arm(near_side) {
            value = 250;
        }
        // Belt and chest console give a readable front/back distinction.
        if y > 2.1 && y < 2.65 && x.abs() < half {
            value = 90;
        }
        if yaw_c > 0.15 && y > -1.4 && y < 0.8 && (x - yaw_s * 0.6).abs() < 1.1 * yaw_c {
            value = 55;
            if y > -0.8 && y < -0.3 {
                value = 145;
            }
        }
        if yaw_c < -0.15 && (x + yaw_s * 0.6).abs() < 1.35 * (-yaw_c) && (-1.5..2.5).contains(&y) {
            value = 150;
            if x.abs() < 0.35 || y > 1.9 {
                value = 75;
            }
        }
        // Large round helmet remains legible on a 17-pixel wide display.
        let helmet = x * x / (3.55 * 3.55) + (y + 5.0).powi(2) / (3.45 * 3.45);
        if helmet <= 1. {
            value = 235;
            if helmet > 0.78 {
                value = 180;
            }
            if yaw_c > 0.02 {
                let vx = x - yaw_s * 1.3;
                let visor =
                    vx * vx / (2.85 * yaw_c.max(0.08)).powi(2) + (y + 5.15).powi(2) / (2.45 * 2.45);
                if visor < 1. {
                    value = 14;
                    if vx < -0.85 * yaw_c && y < -5.65 && y > -6.8 {
                        value = 110;
                    }
                }
            } else if (x - yaw_s * 1.4).abs() < 0.30 && (-7.5..-2.8).contains(&y) {
                value = 120;
            }
        }
        value
    };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if is_backscreen_corner(x, y) {
                continue;
            }
            let mut sum = 0u32;
            for dy in [-0.3f32, 0., 0.3] {
                for dx in [-0.3f32, 0., 0.3] {
                    let sx = x as f32 - 8. + dx;
                    let sy = y as f32 - 11. - bob + dy;
                    // Inverse of the fixed screen-space tilt.
                    sum += sample(sx * tilt_c + sy * tilt_s, -sx * tilt_s + sy * tilt_c) as u32;
                }
            }
            pixels[y * WIDTH + x] = (sum / 9) as u8;
        }
    }
    for (index, (x, y)) in [(2, 3), (14, 5), (2, 16), (14, 20)].into_iter().enumerate() {
        let twinkle = 50. + 55. * (theta + index as f32 * 1.6).sin().max(0.);
        put(&mut pixels, x, y, twinkle as u8);
    }
    pixels
}

pub fn frame(
    source: &Source,
    config: &PlaybackConfig,
    elapsed: Duration,
    live: &LiveData,
) -> Vec<u8> {
    if let Source::SmartCarousel(slides) = source {
        if slides.is_empty() {
            return frame(
                &Source::Preset(Preset::Astronaut),
                config,
                elapsed,
                live,
            );
        }
        let elapsed_secs = elapsed.as_secs() as usize;
        let slide_idx = (elapsed_secs / SMART_SLIDE_PROGRESS_CELLS) % slides.len();
        let step_in_slide = elapsed_secs % SMART_SLIDE_PROGRESS_CELLS;
        let remaining_cells = SMART_SLIDE_PROGRESS_CELLS.saturating_sub(step_in_slide).max(1);
        return synthesize_smart_slide_frame(
            &slides[slide_idx],
            remaining_cells,
            config.brightness,
            config.invert,
        );
    }
    let mut f = vec![0u8; SLOTS];
    let t = elapsed.as_secs_f32();
    let p = match source {
        Source::Image(raw) => {
            if raw.len() == SLOTS {
                f.copy_from_slice(raw)
            };
            None
        }
        Source::Preset(p) | Source::App { preset: p, .. } => Some(*p),
        Source::SmartCarousel(_) => None,
    };
    if let Some(p) = p {
        match p {
            Preset::Astronaut => {
                f = astronaut_frame(t);
            }
            Preset::BlinkCat => {
                sprite(
                    &mut f,
                    &["1         1", "11       11", "1 1     1 1", "1  11111  1"],
                    3,
                    3,
                    220,
                );
                let blink = t % 3.5 > 3.1;
                for x in [5, 11] {
                    sprite(
                        &mut f,
                        if blink { &["111"] } else { &["11", "11"] },
                        x - 1,
                        9,
                        255,
                    )
                }
                sprite(
                    &mut f,
                    &["     1     ", "1   1 1   1", " 111   111"],
                    3,
                    12,
                    210,
                );
                let tail = (t * 3.).sin().round() as i32;
                sprite(&mut f, &["11111", "1   1", "11111"], 6, 17, 150);
                put(&mut f, 12 + tail, 18, 220);
                put(&mut f, 13 + tail, 17, 170);
            }
            Preset::Heart => {
                let pulse = 0.45 + 0.55 * ((t * 3.6).sin().max(0.)).powi(2);
                sprite(
                    &mut f,
                    &[
                        " 111   111 ",
                        "11111 11111",
                        "11111111111",
                        "11111111111",
                        " 111111111 ",
                        "  1111111  ",
                        "   11111   ",
                        "    111    ",
                        "     1     ",
                    ],
                    3,
                    7,
                    (pulse * 255.) as u8,
                );
            }
            Preset::Meteor => {
                for k in 0..3 {
                    let phase = (t * 8. + k as f32 * 9.) % 34. - 8.;
                    for tail in 0..7 {
                        put(
                            &mut f,
                            (phase - tail as f32).round() as i32 - k * 3,
                            (phase - tail as f32).round() as i32 + k * 5,
                            (255 - tail * 33) as u8,
                        )
                    }
                }
            }
            Preset::Rain => {
                for x in (1..17).step_by(3) {
                    let y = ((t * (4. + (x % 4) as f32) + x as f32 * 3.) % 30.) as i32;
                    for tail in 0..6 {
                        put(&mut f, x, y - tail, (255 - tail * 40) as u8)
                    }
                }
            }
            Preset::Wave => {
                for x in 0..17 {
                    let y = (11. + (x as f32 * 0.45 + t * 2.).sin() * 6.) as i32;
                    for dy in -1i32..=1 {
                        put(&mut f, x, y + dy, if dy == 0 { 255 } else { 100 })
                    }
                }
            }
            Preset::Orbit => {
                for y in 0..23 {
                    for x in 0..17 {
                        let d = ((x as f32 - 8.).powi(2) + (y as f32 - 11.).powi(2)).sqrt();
                        if (3.2..4.3).contains(&d) {
                            put(&mut f, x, y, 150)
                        }
                    }
                }
                for k in 0..9 {
                    let angle = t * 1.7 - k as f32 * 0.11;
                    put(
                        &mut f,
                        (8. + 7. * angle.cos()).round() as i32,
                        (11. + 9. * angle.sin()).round() as i32,
                        (255 - k * 24) as u8,
                    )
                }
            }
            Preset::Firework => {
                let phase = t % 3.;
                if phase < 0.8 {
                    let y = 22 - (phase * 19.) as i32;
                    put(&mut f, 8, y, 255);
                    put(&mut f, 8, y + 1, 100)
                } else {
                    let radius = (phase - 0.8) * 4.5;
                    for a in 0..12 {
                        let angle = a as f32 * std::f32::consts::TAU / 12.;
                        put(
                            &mut f,
                            (8. + radius * angle.cos()).round() as i32,
                            (10. + radius * angle.sin()).round() as i32,
                            ((1. - (phase - 0.8) / 2.2) * 255.).max(0.) as u8,
                        )
                    }
                }
            }
            Preset::Coffee => {
                sprite(
                    &mut f,
                    &[
                        "111111111  ",
                        "1       111",
                        "1       1 1",
                        "1       111",
                        "1       1  ",
                        " 1111111   ",
                        "           ",
                        "11111111111",
                    ],
                    3,
                    11,
                    230,
                );
                for x in [5, 9] {
                    for y in 3..10 {
                        let drift = ((y as f32 * 0.8 - t * 3.).sin()).round() as i32;
                        put(&mut f, x + drift, y, (90. + (y as f32 * 12.)) as u8)
                    }
                }
            }
            Preset::Equalizer => {
                for k in 0..5 {
                    let h = (4. + ((t * 2.6 + k as f32 * 1.7).sin() + 1.) * 6.) as i32;
                    for y in 21 - h..21 {
                        for dx in 0..2 {
                            put(&mut f, 1 + k * 3 + dx, y, if y < 10 { 255 } else { 170 })
                        }
                    }
                }
            }
            Preset::Invader => {
                let step = (t * 3.).floor() as i32 % 2;
                sprite(
                    &mut f,
                    &[
                        "  1     1  ",
                        "   1   1   ",
                        "  1111111  ",
                        " 11 111 11 ",
                        "11111111111",
                        "1 1111111 1",
                        "1 1     1 1",
                    ],
                    3,
                    6,
                    255,
                );
                sprite(
                    &mut f,
                    if step == 0 {
                        &["  11   11  ", " 11     11 "]
                    } else {
                        &[" 1       1 ", "  11   11  "]
                    },
                    3,
                    13,
                    200,
                );
            }
            Preset::Breath => {
                let radius = 3. + (t * 1.3).sin() * 2.5;
                for y in 0..23 {
                    for x in 0..17 {
                        let d = ((x as f32 - 8.).powi(2) + (y as f32 - 11.).powi(2)).sqrt();
                        let v = ((1. - (d - radius).abs() / 3.).max(0.) * 255.) as u8;
                        put(&mut f, x, y, v)
                    }
                }
            }
            Preset::Clock => {
                number(&mut f, live.hour as u32, 5, 4, true);
                number(&mut f, live.minute as u32, 5, 13, true);
                put(&mut f, 8, 11, if (t as u32) % 2 == 0 { 180 } else { 60 });
            }
            Preset::Countdown => {
                let left = config
                    .duration_secs
                    .saturating_sub(elapsed.as_secs() as u32);
                number(&mut f, left / 60, 5, 4, true);
                number(&mut f, left % 60, 5, 13, true);
                let progress =
                    (left as f32 / config.duration_secs.max(1) as f32 * 15.).ceil() as i32;
                for x in 1..=progress {
                    put(&mut f, x, 21, 180)
                }
            }
            Preset::Battery => {
                sprite(
                    &mut f,
                    &[
                        "   111   ",
                        "111111111",
                        "1       1",
                        "1       1",
                        "1       1",
                        "1       1",
                        "1       1",
                        "1       1",
                        "111111111",
                    ],
                    4,
                    3,
                    200,
                );
                if let Some(percent) = live.battery_percent {
                    for y in (11 - (percent as i32 * 6 / 100))..11 {
                        for x in 6..11 {
                            put(&mut f, x, y, 255)
                        }
                    }
                    let count = percent.to_string().len() as i32;
                    number(
                        &mut f,
                        percent as u32,
                        (17 - (count * 4 - 1)) / 2,
                        15,
                        false,
                    )
                } else {
                    sprite(&mut f, &["111", "  1", " 11", "   ", " 1 "], 7, 15, 150)
                }
            }
        }
    }
    let scale = (config.brightness.clamp(1, 8) as u16 * 32).min(255);
    for y in 0..23 {
        for x in 0..17 {
            let i = y * 17 + x;
            if is_backscreen_corner(x, y) {
                f[i] = 0
            } else {
                let v = if config.invert { 255 - f[i] } else { f[i] };
                f[i] = (v as u16 * scale / 255) as u8
            }
        }
    }
    f
}

#[derive(Clone, Debug)]
pub enum PlaybackEvent {
    Started { message: String },
    Frame { frame: Vec<u8>, elapsed_ms: u64 },
    Finished { cancelled: bool, frames: u64 },
    Failed(String),
}
pub struct Playback {
    pub events: mpsc::Receiver<PlaybackEvent>,
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Playback {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release)
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.cancel();
        let until = Instant::now() + Duration::from_secs(4);
        if let Some(worker) = self.worker.take() {
            while !worker.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(10))
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
// A shell waiting on this stdin restores settings if Desktop exits abruptly.
// Normal completion disarms it only after the Rust path verifies restoration.
struct RestoreGuard {
    child: Child,
    input: Option<ChildStdin>,
}
impl RestoreGuard {
    fn new(adb: &Adb, device: &Device, saved: &Saved) -> Result<Self> {
        let transport = device.resolve_active_transport(adb)?;
        let script=format!("if ! read -r finished; then service call display_ext 5 i32 -1 i32 0 >/dev/null; dumpsys display_ext back_screen brightness manual {} >/dev/null; dumpsys display_ext back_screen brightness auto {} >/dev/null; dumpsys display_ext back_screen basic type {} >/dev/null; dumpsys display_ext back_screen basic display_mode {} >/dev/null; fi",shell_quote(&saved.manual),if saved.automatic=="true"{"on"}else{"off"},shell_quote(&saved.kind),shell_quote(&saved.mode));
        let mut command = Command::new(&adb.path);
        crate::resources::quiet_command(&mut command);
        let mut child = command
            .args(["-s", &transport, "shell", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        Ok(Self { child, input })
    }
    fn disarm(&mut self) {
        if let Some(mut input) = self.input.take() {
            let _ = input.write_all(b"done\n");
        }
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
    }
}
impl Drop for RestoreGuard {
    fn drop(&mut self) {
        self.input.take();
    }
}
fn bs(adb: &Adb, device: &Device, args: &[&str]) -> Result<String> {
    bs_batch(adb, device, &[args])
}
fn bs_batch(adb: &Adb, device: &Device, commands: &[&[&str]]) -> Result<String> {
    if commands.iter().any(|args| native_back_screen_command(args).is_some()) {
        let mut native = crate::native_mcp::Client::connect(adb, device)?;
        let mut output = String::new();
        let mut native_applied = false;
        for args in commands {
            if let Some((method, params)) = native_back_screen_command(args) {
                if native.supports(method) {
                    native.call(method, params)?;
                    native_applied = true;
                    continue;
                }
            }
            output.push_str(&bs_legacy_batch(adb, device, &[args])?);
        }
        if native_applied {
            // Read the resulting state once per batch. Reading all three state
            // sections after every write multiplies wireless round trips and can
            // hold up the next user action without adding verification value.
            output.push_str(&bs_legacy_batch(adb, device, &[&["basic"], &["brightness"], &["special"]])?);
        }
        return Ok(output);
    }
    bs_legacy_batch(adb, device, commands)
}

fn native_back_screen_command(args: &[&str]) -> Option<(&'static str, serde_json::Value)> {
    use serde_json::json;
    Some(match args {
        ["brightness", "auto", value @ ("on" | "off")] => ("set_back_screen_auto_brightness", json!({"enabled":*value == "on"})),
        ["brightness", "manual", value] => ("set_back_screen_brightness", json!({"brightness":value.parse::<u8>().ok()?})),
        ["brightness", "level", value] => ("set_back_screen_brightness", json!({"brightness":(value.parse::<u16>().ok()? * 32).min(255)})),
        ["basic", "display_mode", mode] => ("set_back_screen_display_mode", json!({"mode":mode})),
        ["basic", "type", kind @ ("agent" | "time")] => ("set_back_screen_basic_type", json!({"type":kind})),
        ["wake"] => ("wake_up_back_screen", json!({})),
        ["sleep"] => ("sleep_back_screen", json!({})),
        ["special", "countdown", "cancel"] => ("cancel_back_screen_countdown", json!({})),
        ["special", "countdown", seconds] => ("start_back_screen_countdown", json!({"seconds":seconds.parse::<u8>().ok()?})),
        _ => return None,
    })
}

fn bs_legacy_batch(adb: &Adb, device: &Device, commands: &[&[&str]]) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let command = commands
        .iter()
        .map(|args| {
            let mut a = vec!["dumpsys", "display_ext", "back_screen"];
            a.extend_from_slice(args);
            a.iter()
                .map(|s| shell_quote(s))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" && ");
    let out = adb.run_timeout(
        &["-s", &transport, "shell", &command],
        Duration::from_secs(3),
    )?;
    if out.lines().any(|line| {
        let l = line.trim().to_ascii_lowercase();
        l.starts_with("unknown command")
            || l.starts_with("unknown back_screen command")
            || l.starts_with("invalid")
            || l.starts_with("error:")
            || l.starts_with("exception")
            || l.starts_with("security exception")
            || l.starts_with("java.lang.")
    }) {
        bail!(
            "背屏指令失败：{}",
            out.lines().take(3).collect::<Vec<_>>().join(" ")
        );
    }
    Ok(out)
}
fn token<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix(key))
        .and_then(|s| s.split_whitespace().next())
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    mode: String,
    kind: String,
    manual: String,
    automatic: String,
}
impl Saved {
    fn from_mcp_json(raw: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(raw)
            .context("背屏恢复快照损坏，已保留快照供重试")?;
        // The phone-local worker historically wrote the shorter field names.
        // Accept those snapshots as well, but never default missing data.
        let field = |canonical: &str, legacy: &str| value.get(canonical).or_else(|| value.get(legacy));
        let mode = match field("displayMode", "mode").and_then(serde_json::Value::as_i64) {
            Some(0) => "auto",
            Some(1) => "always",
            Some(2) => "custom",
            _ => bail!("背屏恢复快照缺少有效显示模式，已保留快照"),
        };
        let automatic = match field("autoBrightness", "automatic").and_then(serde_json::Value::as_i64) {
            Some(0) => "false",
            Some(1) => "true",
            _ => bail!("背屏恢复快照缺少有效亮度模式，已保留快照"),
        };
        let manual = field("manualBrightness", "brightness").and_then(serde_json::Value::as_u64)
            .filter(|level| (1..=255).contains(level))
            .context("背屏恢复快照缺少有效亮度，已保留快照")?;
        let kind = field("basicType", "type").and_then(serde_json::Value::as_str)
            .filter(|kind| matches!(*kind, "agent" | "time" | "text"))
            .context("背屏恢复快照缺少有效内容类型，已保留快照")?;
        Ok(Self {
            mode: mode.into(),
            kind: kind.into(),
            manual: manual.to_string(),
            automatic: automatic.into(),
        })
    }

    fn read(adb: &Adb, device: &Device) -> Result<Self> {
        let combined = bs_batch(adb, device, &[&["basic"], &["brightness"]])?;
        let mode = token(&combined, "settingsDisplayMode=").context("固件未返回背屏模式")?;
        let kind = token(&combined, "settingsType=").context("固件未返回背屏内容类型")?;
        let automatic = token(&combined, "auto=").context("固件未返回自动亮度")?;
        let manual = combined
            .lines()
            .find_map(|l| l.split("manual=").nth(1))
            .and_then(|s| s.split_whitespace().next())
            .context("固件未返回手动亮度")?;
        Ok(Self {
            mode: mode.into(),
            kind: kind.into(),
            manual: manual.into(),
            automatic: automatic.into(),
        })
    }
    fn restore(&self, adb: &Adb, device: &Device) -> Result<()> {
        device.verify(adb)?;
        clear_frame(adb, device)?;
        let auto_arg = if self.automatic == "true" {
            "on"
        } else {
            "off"
        };
        bs_batch(
            adb,
            device,
            &[
                &["brightness", "manual", &self.manual],
                &["brightness", "auto", auto_arg],
                &["basic", "type", &self.kind],
                &["basic", "display_mode", &self.mode],
            ],
        )?;
        let now = Self::read(adb, device)?;
        if now.mode != self.mode
            || now.kind != self.kind
            || now.manual != self.manual
            || now.automatic != self.automatic
        {
            bail!("背屏播放已结束，但手机设置恢复校验未通过")
        };
        Ok(())
    }
}

pub fn clear_frame(adb: &Adb, device: &Device) -> Result<()> {
    if crate::native_mcp::try_call(adb, device, "clear_back_screen_frame", serde_json::json!({}))?.is_some() {
        return Ok(());
    }
    let transport = device.resolve_active_transport(adb)?;
    let out = adb.run_timeout(
        &[
            "-s",
            &transport,
            "shell",
            "service call display_ext 5 i32 -1 i32 0",
        ],
        Duration::from_secs(3),
    )?;
    if out.trim() != "Result: Parcel(NULL)" {
        bail!("手机未确认释放背屏帧")
    };
    Ok(())
}
pub fn push_frame(adb: &Adb, device: &Device, pixels: &[u8]) -> Result<()> {
    let mut native = crate::native_mcp::Client::connect(adb, device)?;
    push_frame_with_client(adb, device, pixels, &mut native)
}

fn push_frame_with_client(adb: &Adb, device: &Device, pixels: &[u8], native: &mut crate::native_mcp::Client) -> Result<()> {
    if pixels.len() != SLOTS {
        bail!("背屏帧需391槽")
    };
    if pixels
        .iter()
        .enumerate()
        .any(|(i, p)| is_backscreen_corner(i % 17, i / 17) && *p != 0)
    {
        bail!("不存在的灯珠必须为0")
    };
    if native.supports("set_back_screen_frame") {
        native.call("set_back_screen_frame", serde_json::json!({"frame":pixels}))?;
        return Ok(());
    }
    let transport = device.resolve_active_transport(adb)?;
    use std::fmt::Write as _;
    let mut cmd = String::with_capacity(SLOTS * 8 + 48);
    cmd.push_str("service call display_ext 5 i32 391");
    for p in pixels {
        let _ = write!(cmd, " i32 {p}");
    }
    cmd.push_str(" i32 0");
    let out = adb.run_timeout(
        &["-s", &transport, "shell", &cmd],
        Duration::from_secs(3),
    )?;
    if out.trim() != "Result: Parcel(NULL)" {
        bail!("手机未确认接收背屏帧")
    };
    Ok(())
}

fn live_data(adb: &Adb, device: &Device) -> Result<LiveData> {
    let transport = device.resolve_active_transport(adb)?;
    let out = adb.script(&transport, "date '+%H %M'; dumpsys battery")?;
    let mut t = out.lines().next().unwrap_or("").split_whitespace();
    let hour = t
        .next()
        .and_then(|s| s.parse().ok())
        .context("无法读取手机时间")?;
    let minute = t
        .next()
        .and_then(|s| s.parse().ok())
        .context("无法读取手机时间")?;
    let level = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("level:"))
        .and_then(|s| s.trim().parse::<u16>().ok());
    let scale = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("scale:"))
        .and_then(|s| s.trim().parse::<u16>().ok());
    let battery_percent = level
        .zip(scale)
        .filter(|(_, s)| *s > 0)
        .map(|(l, s)| ((u32::from(l) * 100 / u32::from(s)).min(100)) as u8);
    Ok(LiveData {
        hour,
        minute,
        battery_percent,
    })
}
fn foreground(adb: &Adb, device: &Device) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let out = adb.shell(&transport, &["dumpsys", "activity", "activities"])?;
    for l in out
        .lines()
        .filter(|l| l.contains("mResumedActivity") || l.contains("topResumedActivity"))
    {
        for part in l.split_whitespace() {
            if let Some((pkg, _)) = part.split_once('/') {
                if pkg.contains('.') {
                    return Ok(pkg.into());
                }
            }
        }
    }
    Ok(String::new())
}
fn check_source(adb: &Adb, device: &Device, source: &Source) -> Result<()> {
    match source {
        Source::Image(p) if p.len() != SLOTS => bail!("图片帧需391槽"),
        Source::App { package, .. } => {
            if package.is_empty()
                || !package
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_')
            {
                bail!("应用包名不合法")
            };
            let transport = device.resolve_active_transport(adb)?;
            let result = adb.shell(&transport, &["pm", "path", package])?;
            if !result.starts_with("package:") {
                bail!("手机尚未安装此应用")
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn start(adb: Adb, device: Device, source: Source, config: PlaybackConfig) -> Result<Playback> {
    config.validate()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (tx, events) = mpsc::channel();
    let worker = thread::spawn(move || {
        if worker_cancel.load(Ordering::Acquire) {
            let _ = tx.send(PlaybackEvent::Finished {
                cancelled: true,
                frames: 0,
            });
            return;
        }
        let saved = match (|| {
            let transport = device.resolve_active_transport(&adb)?;
            check_source(&adb, &device, &source)?;
            let before = adb.shell(&transport, &["dumpsys", "display_ext"])?;
            if before.contains("appUsesCustomFrame=true") {
                bail!("手机当前被其他自定义背屏占用；固件无法导出原图，请先结束该背屏内容再播放。")
            };
            Saved::read(&adb, &device)
        })() {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(PlaybackEvent::Failed(e.to_string()));
                return;
            }
        };
        if worker_cancel.load(Ordering::Acquire) {
            let _ = tx.send(PlaybackEvent::Finished {
                cancelled: true,
                frames: 0,
            });
            return;
        }
        let mut guard = match RestoreGuard::new(&adb, &device, &saved) {
            Ok(g) => g,
            Err(e) => {
                let _ = tx.send(PlaybackEvent::Failed(format!("无法建立背屏恢复保护：{e}")));
                return;
            }
        };
        let result = (|| -> Result<(bool, u64)> {
            // Reuse one native session for the animation, rather than rediscover
            // hundreds of tools or create an ADB process for every frame.
            let mut native = crate::native_mcp::Client::connect(&adb, &device)?;
            if worker_cancel.load(Ordering::Acquire) {
                return Ok((true, 0));
            }
            bs_batch(
                &adb,
                &device,
                &[
                    &["brightness", "auto", "off"],
                    &["brightness", "manual", "255"],
                    &["basic", "display_mode", "always"],
                    &["wake"],
                ],
            )?;
            let started = Instant::now();
            let mut frames = 0;
            let needs_live = matches!(
                source,
                Source::Preset(Preset::Clock | Preset::Battery) | Source::App { .. }
            );
            let mut live = if needs_live {
                live_data(&adb, &device)?
            } else {
                LiveData::default()
            };
            let mut sampled = Instant::now();
            let mut first_sample = false;
            let mut active = true;
            let mut last = vec![];
            let mut next = Instant::now();
            let interval = Duration::from_secs_f64(1. / config.fps as f64);
            let mut checked_output = false;
            let mut output_sampled = Instant::now();
            let _ = tx.send(PlaybackEvent::Started {
                message: if matches!(source, Source::App { .. }) {
                    "正在监听手机前台应用；保持USB或无线连接"
                } else {
                    "手机已接收播放配置，正在核对背屏输出"
                }
                .into(),
            });
            loop {
                if worker_cancel.load(Ordering::Acquire) {
                    return Ok((true, frames));
                }
                let elapsed = started.elapsed();
                if elapsed >= Duration::from_secs(config.duration_secs as u64) {
                    if matches!(source, Source::Preset(Preset::Countdown)) {
                        let zero = frame(
                            &source,
                            &config,
                            Duration::from_secs(config.duration_secs as u64),
                            &live,
                        );
                        if worker_cancel.load(Ordering::Acquire) {
                            return Ok((true, frames));
                        }
                        push_frame_with_client(&adb, &device, &zero, &mut native)?;
                        let _ = tx.send(PlaybackEvent::Frame {
                            frame: zero,
                            elapsed_ms: config.duration_secs as u64 * 1000,
                        });
                        thread::sleep(Duration::from_millis(400));
                        frames += 1;
                    }
                    return Ok((false, frames));
                }
                if sampled.elapsed() >= Duration::from_secs(1) || first_sample {
                    first_sample = false;
                    if needs_live {
                        live = live_data(&adb, &device)?;
                        if worker_cancel.load(Ordering::Acquire) {
                            return Ok((true, frames));
                        }
                    }
                    if let Source::App { package, .. } = &source {
                        let now = foreground(&adb, &device)? == *package;
                        if worker_cancel.load(Ordering::Acquire) {
                            return Ok((true, frames));
                        }
                        if active && !now {
                            saved.restore(&adb, &device)?;
                            last.clear();
                            let _ = tx.send(PlaybackEvent::Started {
                                message: "等待目标应用进入前台；已恢复手机原背屏模式与亮度".into(),
                            });
                        } else if !active && now {
                            bs_batch(
                                &adb,
                                &device,
                                &[
                                    &["brightness", "auto", "off"],
                                    &["brightness", "manual", "255"],
                                    &["basic", "display_mode", "always"],
                                    &["wake"],
                                ],
                            )?;
                            checked_output = false;
                        }
                        active = now;
                    }
                    sampled = Instant::now();
                }
                if active {
                    let pixels = frame(&source, &config, elapsed, &live);
                    if pixels != last {
                        if worker_cancel.load(Ordering::Acquire) {
                            return Ok((true, frames));
                        }
                        push_frame_with_client(&adb, &device, &pixels, &mut native)?;
                        frames += 1;
                        last = pixels.clone();
                        let _ = tx.send(PlaybackEvent::Frame {
                            frame: pixels,
                            elapsed_ms: elapsed.as_millis() as u64,
                        });
                    }
                    if !checked_output || output_sampled.elapsed() >= Duration::from_secs(5) {
                        // Keep a user-requested timed presentation awake, while retaining
                        // the firmware's posture, thermal, sleep and battery gates.
                        if checked_output {
                            bs(&adb, &device, &["wake"])?;
                        }
                        let transport = device.resolve_active_transport(&adb)?;
                        let state = adb.shell(&transport, &["dumpsys", "display_ext"])?;
                        if state.contains("policyAllowsOutput=false")
                            || !state.contains("power=on")
                            || !state.contains("appUsesCustomFrame=true")
                            || !state.contains("appliedDriverFlat=391")
                        {
                            let reason = token(&state, "state=").unwrap_or("系统策略");
                            bail!("手机已接收图案，但背屏没有亮起（{reason}）。请将手机背屏朝上，检查勿扰/省电和背屏开关后重试。")
                        };
                        let first_output = !checked_output;
                        checked_output = true;
                        output_sampled = Instant::now();
                        if first_output {
                            let _ = tx.send(PlaybackEvent::Started {
                                message: "手机背屏 HAL 已开启，动画播放中".into(),
                            });
                        }
                    }
                }
                next += interval;
                while Instant::now() < next {
                    if worker_cancel.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(
                        next.saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(20)),
                    );
                }
                if next < Instant::now() {
                    next = Instant::now();
                }
            }
        })();
        let restored = saved.restore(&adb, &device);
        if restored.is_ok() {
            guard.disarm();
        }
        match (result, restored) {
            (Ok((cancelled, frames)), Ok(())) => {
                let _ = tx.send(PlaybackEvent::Finished { cancelled, frames });
            }
            (Err(e), Ok(())) => {
                let _ = tx.send(PlaybackEvent::Failed(e.to_string()));
            }
            (_, Err(e)) => {
                let _ = tx.send(PlaybackEvent::Failed(format!("恢复手机背屏设置失败：{e}")));
            }
        }
    });
    Ok(Playback {
        events,
        cancel,
        worker: Some(worker),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeFeature {
    Notifications,
    Clock,
    Camera,
}
impl NativeFeature {
    pub fn key(self) -> &'static str {
        match self {
            Self::Notifications => "notifications",
            Self::Clock => "clock",
            Self::Camera => "camera",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Notifications => "系统消息联动",
            Self::Clock => "手机原生时钟",
            Self::Camera => "相机联动",
        }
    }
}
pub fn configure_native(adb: &Adb, device: &Device, feature: NativeFeature) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let _ = release_custom_frame_owner(adb, device);
    let state = adb.shell(&transport, &["dumpsys", "display_ext"])?;
    if state.contains("appUsesCustomFrame=true") {
        let _ = clear_frame(adb, device);
    }
    if feature == NativeFeature::Notifications
        && (!state.contains("registered=true userId=")
            || !state.contains("callStateRegistered=true"))
    {
        bail!("当前固件通知/电话监听器未就绪")
    }
    let saved = Saved::read(adb, device)?;
    let camera_before = if feature == NativeFeature::Camera {
        Some(adb.shell(
            &transport,
            &[
                "settings",
                "get",
                "system",
                "stepos_back_screen_camera_link",
            ],
        )?)
    } else {
        None
    };
    let result = (|| -> Result<String> {
        match feature {
            NativeFeature::Notifications => {
                bs(adb, device, &["basic", "type", "agent"])?;
                if !bs(adb, device, &["basic"])?.contains("settingsType=agent") {
                    bail!("消息联动模式回读失败")
                };
                Ok("已切回手机原生消息联动：微信、企微、飞书、钉钉，以及真实来电、未读短信和低电量；亮灯遵循手机姿态、勿扰及省电设置。".into())
            }
            NativeFeature::Clock => {
                bs(adb, device, &["basic", "type", "time"])?;
                if !bs(adb, device, &["basic"])?.contains("settingsType=time") {
                    bail!("时钟设置回读失败")
                };
                Ok(
                    "已启用手机原生时钟；手机按分钟更新，断开电脑仍可使用。亮屏方式沿用手机设置。"
                        .into(),
                )
            }
            NativeFeature::Camera => {
                set_camera_link(adb, device, true)
            }
        }
    })();
    if let Err(error) = result {
        let restore = saved.restore(adb, device);
        let camera_restore = if let Some(previous) = camera_before {
            if previous.trim() == "null" {
                adb.shell(
                    &transport,
                    &[
                        "settings",
                        "delete",
                        "system",
                        "stepos_back_screen_camera_link",
                    ],
                )
            } else {
                adb.shell(
                    &transport,
                    &[
                        "settings",
                        "put",
                        "system",
                        "stepos_back_screen_camera_link",
                        previous.trim(),
                    ],
                )
            }
        } else {
            Ok(String::new())
        };
        if restore.is_err() || camera_restore.is_err() {
            bail!("{error}；恢复原设置失败，请在手机背屏设置中检查")
        }
        return Err(error);
    }
    result
}

pub fn set_camera_link(adb: &Adb, device: &Device, enabled: bool) -> Result<String> {
    if crate::native_mcp::try_call(adb, device, "set_back_screen_camera_link", serde_json::json!({"enabled":enabled}))?.is_some() {
        let state = bs(adb, device, &["special"])?;
        anyhow::ensure!(state.contains(if enabled {"cameraLink=true"} else {"cameraLink=false"}), "相机联动设置回读失败，未重试");
        return Ok(format!("已{}手机原生相机背屏联动（已回读）", if enabled {"开启"} else {"关闭"}));
    }
    let transport = device.resolve_active_transport(adb)?;
    if enabled
        && !adb
            .shell(
                &transport,
                &["pm", "path", "com.stepos.media.camera"],
            )?
            .starts_with("package:")
    {
        bail!("当前手机未安装此固件支持的原生相机")
    }
    let before = adb.shell(
        &transport,
        &[
            "settings",
            "get",
            "system",
            "stepos_back_screen_camera_link",
        ],
    )?;
    let result = (|| -> Result<String> {
        adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "stepos_back_screen_camera_link",
                if enabled { "1" } else { "0" },
            ],
        )?;
        let state = bs(adb, device, &["special"])?;
        if !state.contains(if enabled {
            "cameraLink=true"
        } else {
            "cameraLink=false"
        }) {
            bail!("相机联动开关回读失败")
        }
        Ok(if enabled {
            "已开启手机原生相机背屏联动"
        } else {
            "已关闭手机原生相机背屏联动"
        }
        .into())
    })();
    if result.is_err() {
        let restored = if before.trim() == "null" {
            adb.shell(
                &transport,
                &[
                    "settings",
                    "delete",
                    "system",
                    "stepos_back_screen_camera_link",
                ],
            )
        } else {
            adb.shell(
                &transport,
                &[
                    "settings",
                    "put",
                    "system",
                    "stepos_back_screen_camera_link",
                    before.trim(),
                ],
            )
        };
        if restored.is_err() {
            bail!("相机联动设置失败且未能恢复，请在手机背屏设置检查")
        }
    }
    result
}

pub fn set_display_mode(adb: &Adb, device: &Device, mode: &str) -> Result<String> {
    device.verify(adb)?;
    let mode = match mode {
        "always" => "always",
        "auto" | "auto_on" | "overlay" => "auto",
        "custom" | "scheduled" => "custom",
        _ => bail!("未知背屏显示模式"),
    };
    bs(adb, device, &["basic", "display_mode", mode])?;
    let state = bs(adb, device, &["basic"])?;
    if token(&state, "settingsDisplayMode=") != Some(mode) {
        bail!("显示模式回读失败")
    };
    Ok(format!(
        "手机背屏模式已设为{}",
        match mode {
            "always" => "始终亮屏",
            "custom" => "定时亮屏（沿用手机时间段）",
            _ => "自动亮屏",
        }
    ))
}
pub fn set_brightness(adb: &Adb, device: &Device, level: u8) -> Result<String> {
    device.verify(adb)?;
    if !(1..=8).contains(&level) {
        bail!("亮度级别需1–8")
    };
    let value = ((level as u16 * 32).min(255)).to_string();
    let state = bs_batch(
        adb,
        device,
        &[
            &["brightness", "auto", "off"],
            &["brightness", "manual", &value],
            &["brightness"],
        ],
    )?;
    if !state.contains(&format!("manual={value} ")) {
        bail!("手机亮度回读失败")
    };
    Ok(format!("已设为手动亮度{level}/8"))
}

pub fn set_stay_on_duration(adb: &Adb, device: &Device, seconds: u32) -> Result<String> {
    if !matches!(seconds, 15 | 30 | 60) {
        bail!("背屏亮屏时长支持 15、30 或 60 秒");
    }
    let transport = device.resolve_active_transport(adb)?;
    let val = seconds.to_string();
    adb.shell(
        &transport,
        &[
            "settings",
            "put",
            "system",
            "stepos_back_screen_stay_on_duration",
            &val,
        ],
    )?;
    let _ = bs(adb, device, &["basic", "stay_on_duration", &val]);
    Ok(format!("已将手机背屏亮屏时长设为 {seconds} 秒"))
}

fn set_backscreen_system_flag(
    adb: &Adb,
    device: &Device,
    setting_key: &str,
    enabled: bool,
    enabled_msg: &'static str,
    disabled_msg: &'static str,
) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let val = if enabled { "1" } else { "0" };
    adb.shell(&transport, &["settings", "put", "system", setting_key, val])?;
    Ok(if enabled { enabled_msg } else { disabled_msg }.into())
}

pub fn set_flip_keep_running(adb: &Adb, device: &Device, enabled: bool) -> Result<String> {
    set_backscreen_system_flag(
        adb,
        device,
        "stepos_back_screen_flip_keep_running",
        enabled,
        "已开启翻转手机继续运行（背屏协同保持活跃）",
        "已关闭翻转手机继续运行",
    )
}

pub fn set_incoming_call_effect(adb: &Adb, device: &Device, enabled: bool) -> Result<String> {
    set_backscreen_system_flag(
        adb,
        device,
        "stepos_back_screen_incoming_call_effect",
        enabled,
        "已开启来电背屏灯效提醒",
        "已关闭来电背屏灯效提醒",
    )
}

pub fn set_notification_effect(adb: &Adb, device: &Device, enabled: bool) -> Result<String> {
    set_backscreen_system_flag(
        adb,
        device,
        "stepos_back_screen_notification_effect",
        enabled,
        "已开启通知背屏灯效提醒",
        "已关闭通知背屏灯效提醒",
    )
}

pub fn is_cjk_char(c: char) -> bool {
    ('\u{4e00}'..='\u{9fa5}').contains(&c)
}

/// Checks whether `text` is a valid 1–6 character Chinese signature supported by StepOS 12
/// and the local 17×23 CJK pixel rasterizer.
pub fn is_valid_chinese_signature(text: &str) -> bool {
    let trimmed = text.trim();
    let non_space_count = trimmed.chars().filter(|c| !c.is_whitespace()).count();
    if !(1..=6).contains(&non_space_count) || trimmed.chars().count() > 6 {
        return false;
    }
    let has_cjk = trimmed.chars().any(is_cjk_char);
    let all_allowed = trimmed.chars().all(|c| {
        is_cjk_char(c)
            || c.is_ascii_alphanumeric()
            || matches!(
                c,
                ' ' | '!' | '?' | '-' | ':' | '.' | '！' | '？' | '·' | '，' | '。' | '、' | '~' | '～'
            )
    });
    has_cjk && all_allowed
}

pub fn has_all_cjk_glyphs(text: &str) -> bool {
    let mut any = false;
    for ch in text.trim().chars().filter(|c| !c.is_whitespace()).take(6) {
        any = true;
        if !ch.is_ascii() && cjk_glyph_7x7(ch).is_none() {
            return false;
        }
    }
    any
}

pub fn marquee(
    adb: &Adb,
    device: &Device,
    text: &str,
    scroll: bool,
    invert: bool,
) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let _ = release_custom_frame_owner(adb, device);
    let text = text.trim();
    // Native text supports Chinese. Scrolling/inversion are separate custom
    // rendering features, which this tool does not expose.
    if !scroll && !invert {
        if let Some(receipt) = crate::native_mcp::try_call(adb, device, "set_back_screen_basic_type", serde_json::json!({"type":"text","text":text}))? {
            if text.chars().any(is_cjk_char) {
                // Firmware renders Chinese as a custom frame; it intentionally
                // leaves the persisted ASCII text/type unchanged.
                let state = adb.shell(&transport, &["dumpsys", "display_ext"])?;
                anyhow::ensure!(state.contains("appUsesCustomFrame=true"),
                    "中文点阵请求已提交，但尚未读到自定义帧；未自动改用桌面绘制");
                return Ok(format!("{}；已核对自定义帧，亮屏遵循手机背屏设置", crate::native_mcp::response_note(&receipt)));
            }
            let kind = adb.shell(&transport, &["settings", "get", "system", "stepos_back_screen_basic_type"])?;
            let stored = adb.shell(&transport, &["settings", "get", "system", "stepos_back_screen_basic_text"])?;
            anyhow::ensure!(kind.trim() == "text" && stored.trim() == text.to_uppercase(),
                "背屏文字设置回读不一致；原生请求已提交，未自动改用点阵绘制");
            return Ok(format!("已通过手机原生能力保存并核对背屏文字「{text}」；显示请在手机确认"));
        }
    }
    if is_valid_chinese_signature(text) {
        let pixels = synthesize_chinese_signature_frame(text, invert);
        let mut matrix_csv = String::with_capacity(SLOTS * 2);
        for (i, &v) in pixels.iter().enumerate() {
            if i > 0 {
                matrix_csv.push(',');
            }
            matrix_csv.push(if v > 0 { '1' } else { '0' });
        }
        let _ = adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "stepos_back_screen_signature_text",
                text,
            ],
        );
        let _ = adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "stepos_back_screen_basic_type",
                "text",
            ],
        );
        let _ = adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "stepos_back_screen_basic_text",
                text,
            ],
        );
        let _ = adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "back_screen_custom_matrix_data",
                &matrix_csv,
            ],
        );
        let _ = adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "stepos_back_screen_basic_text_marquee",
                if scroll { "1" } else { "0" },
            ],
        );
        let _ = adb.shell(
            &transport,
            &[
                "settings",
                "put",
                "system",
                "stepos_back_screen_basic_text_invert",
                if invert { "1" } else { "0" },
            ],
        );
        let _ = bs_batch(adb, device, &[&["basic", "type", "text"], &["wake"]]);
        push_frame(adb, device, &pixels)?;
        let _ = bs(adb, device, &["wake"]);
        if has_all_cjk_glyphs(text) {
            return Ok(format!("已将中文签名「{text}」同步到手机背屏（17×23 点阵）"));
        }
        return Ok(format!(
            "已通过系统字库点阵将中文签名「{text}」同步到手机背屏（17×23 点阵）"
        ));
    }
    if text.is_empty()
        || text.len() > 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" !?-:.".contains(&b))
    {
        bail!("背屏原生文字支持 1–6 个汉字签名，或最多 64 个英文字母、数字、空格和 ! ? - : .")
    };
    clear_frame(adb, device)?;
    let _ = adb.shell(
        &transport,
        &[
            "settings",
            "put",
            "system",
            "stepos_back_screen_basic_text",
            text,
        ],
    );
    let _ = adb.shell(
        &transport,
        &[
            "settings",
            "put",
            "system",
            "stepos_back_screen_basic_text_marquee",
            if scroll { "1" } else { "0" },
        ],
    );
    let _ = adb.shell(
        &transport,
        &[
            "settings",
            "put",
            "system",
            "stepos_back_screen_basic_text_invert",
            if invert { "1" } else { "0" },
        ],
    );
    let _ = adb.shell(
        &transport,
        &[
            "settings",
            "put",
            "system",
            "stepos_back_screen_basic_type",
            "text",
        ],
    );
    bs_batch(adb, device, &[&["basic", "type", "text"], &["wake"]])?;
    Ok(format!("已保存手机背屏文字：{text}"))
}

const MCP_REMOTE_DIR: &str = "/data/local/tmp/stepx-companion";
const MCP_STATUS_FILE: &str = "/data/local/tmp/stepx-companion/backscreen-mcp-status.json";
const MCP_ANIM_FILE: &str = "/data/local/tmp/stepx-companion/backscreen-anim.json";
const MCP_JOB_FILE: &str = "/data/local/tmp/stepx-companion/backscreen-anim.job";
const MCP_RESTORE_FILE: &str = "/data/local/tmp/stepx-companion/backscreen-restore.json";

/// Stops any active `--backscreen-loop` or `CompanionService` holding `appFrameOwner`
/// so `uid=2000` (`shell`) can freely drive `IStepOSDisplayManager` (`display_ext`).
pub fn release_custom_frame_owner(adb: &Adb, device: &Device) -> Result<()> {
    let transport = device.resolve_active_transport(adb)?;
    let _ = adb.script(
        &transport,
        &format!(
            "rm -f {job} {anim}; pkill -f 'com.stepx.companion.Bridge --backscreen-loop' 2>/dev/null || true",
            job = shell_quote(MCP_JOB_FILE),
            anim = shell_quote(MCP_ANIM_FILE),
        ),
    );
    if let Ok(dump) = adb.shell(&transport, &["dumpsys", "display_ext"]) {
        if dump.contains("appUsesCustomFrame=true") && !dump.contains("appFrameOwner=uid=2000") {
            let _ = crate::companion_app::stop(adb, device);
        }
    }
    Ok(())
}

fn write_mcp_status_file(
    adb: &Adb,
    transport: &str,
    running: bool,
    mode: &str,
    title: &str,
    brightness: u8,
    fps: u8,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let payload = serde_json::json!({
        "protocol": 2,
        "engine": "phone-backscreen-driver",
        "uid": 2000,
        "running": running,
        "mode": mode,
        "appliedTitle": title,
        "brightness": brightness,
        "fps": fps,
        "pausedReason": "",
        "lastError": "",
        "updatedAtMs": now_ms,
    })
    .to_string();
    let _ = adb.script(
        transport,
        &format!(
            "mkdir -p {dir} && printf '%s\\n' {payload} > {status}",
            dir = shell_quote(MCP_REMOTE_DIR),
            payload = shell_quote(&payload),
            status = shell_quote(MCP_STATUS_FILE),
        ),
    );
}

fn read_mcp_restore_state(adb: &Adb, transport: &str) -> Result<Option<Saved>> {
    let raw = adb.script(transport, &format!(
        "if [ -f {restore} ]; then cat {restore}; else printf '%s' MISSING; fi",
        restore = shell_quote(MCP_RESTORE_FILE),
    )).context("无法读取手机背屏恢复快照")?;
    if raw.trim() == "MISSING" {
        Ok(None)
    } else {
        Saved::from_mcp_json(raw.trim()).map(Some)
    }
}

fn save_mcp_restore_state_if_needed(adb: &Adb, device: &Device, transport: &str) -> Result<()> {
    if read_mcp_restore_state(adb, transport)?.is_some() {
        return Ok(());
    }
    let saved = Saved::read(adb, device)?;
    let mode_int = match saved.mode.as_str() {
        "always" => 1,
        "custom" => 2,
        _ => 0,
    };
    let auto_int = if saved.automatic == "true" { 1 } else { 0 };
    let manual_int = saved.manual.parse::<i32>().unwrap_or(160);
    let payload = serde_json::json!({
        "displayMode": mode_int,
        "autoBrightness": auto_int,
        "manualBrightness": manual_int,
        "basicType": saved.kind,
    })
    .to_string();
    adb.script(
        transport,
        &format!(
            "mkdir -p {dir} && ( [ -f {restore} ] || ( printf '%s\\n' {payload} > {restore}.tmp && mv {restore}.tmp {restore} ) )",
            dir = shell_quote(MCP_REMOTE_DIR),
            restore = shell_quote(MCP_RESTORE_FILE),
            payload = shell_quote(&payload),
        ),
    ).context("无法保存手机背屏恢复快照，尚未修改显示")?;
    anyhow::ensure!(read_mcp_restore_state(adb, transport)?.is_some(),
        "手机未保存背屏恢复快照，尚未修改显示");
    Ok(())
}

/// Historical entry-point name retained for callers. Atomic settings/frames use
/// native MCP when advertised. Persistent animations use the phone-local Binder
/// driver: the catalog exposes no equivalent offline animation scheduler.
pub fn apply_via_firmware_mcp(
    adb: &Adb,
    device: &Device,
    source: &Source,
    config: &PlaybackConfig,
    title: &str,
) -> Result<String> {
    config.validate()?;
    device.verify(adb)?;
    let transport = device.resolve_active_transport(adb)?;
    save_mcp_restore_state_if_needed(adb, device, &transport)?;
    release_custom_frame_owner(adb, device)?;

    let clamped_brightness = config.brightness.clamp(1, 8);
    let level_str = clamped_brightness.to_string();
    let clean_title = if title.trim().is_empty() {
        "自定义点阵"
    } else {
        title.trim()
    };

    match source {
        Source::Preset(Preset::Clock) => {
            let _ = clear_frame(adb, device);
            bs_batch(
                adb,
                device,
                &[
                    &["brightness", "level", &level_str],
                    &["basic", "display_mode", "always"],
                    &["basic", "type", "time"],
                    &["wake"],
                ],
            )?;
            write_mcp_status_file(
                adb,
                &transport,
                true,
                "animation",
                clean_title,
                clamped_brightness,
                1,
            );
            Ok(format!(
                "已应用手机背屏「{clean_title}」· 原生时钟常驻同步"
            ))
        }
        Source::Preset(Preset::Countdown) if config.duration_secs <= 99 => {
            let _ = clear_frame(adb, device);
            let secs_str = config.duration_secs.max(1).to_string();
            bs_batch(
                adb,
                device,
                &[
                    &["brightness", "level", &level_str],
                    &["basic", "display_mode", "always"],
                    &["special", "countdown", &secs_str],
                    &["wake"],
                ],
            )?;
            write_mcp_status_file(
                adb,
                &transport,
                true,
                "animation",
                clean_title,
                clamped_brightness,
                1,
            );
            Ok(format!(
                "已提交手机背屏「{clean_title}」（{} 秒硬件倒计时）",
                config.duration_secs
            ))
        }
        Source::Preset(Preset::Battery) | Source::Image(_) => {
            let live = if matches!(source, Source::Preset(Preset::Battery)) {
                live_data(adb, device).unwrap_or_default()
            } else {
                LiveData::default()
            };
            let pixels = frame(source, config, Duration::ZERO, &live);
            bs_batch(
                adb,
                device,
                &[
                    &["brightness", "auto", "off"],
                    &["brightness", "manual", "255"],
                    &["basic", "display_mode", "always"],
                    &["wake"],
                ],
            )?;
            push_frame(adb, device, &pixels)?;
            let _ = bs(adb, device, &["wake"]);
            write_mcp_status_file(
                adb,
                &transport,
                true,
                "animation",
                clean_title,
                clamped_brightness,
                1,
            );
            Ok(format!(
                "已提交手机背屏「{clean_title}」· 请在手机确认实际显示"
            ))
        }
        _ => {
            crate::bridge::ensure_bridge_jar(adb, device)?;
            let is_smart_carousel = matches!(source, Source::SmartCarousel(slides) if !slides.is_empty());
            let fps = if matches!(source, Source::Preset(Preset::Countdown)) || is_smart_carousel {
                1
            } else {
                config.fps.clamp(1, 12)
            };
            let duration_secs = if matches!(source, Source::Preset(Preset::Countdown)) {
                config.duration_secs.clamp(1, 3600)
            } else if config.duration_secs > 0 {
                config.duration_secs.clamp(5, 60)
            } else {
                30
            };
            let frames: Vec<Vec<u8>> = match source {
                Source::SmartCarousel(slides) if !slides.is_empty() => {
                    synthesize_smart_carousel_frames(slides, clamped_brightness, config.invert)
                }
                _ => {
                    let frame_count: usize = if matches!(source, Source::Preset(Preset::Countdown)) {
                        (duration_secs as usize).clamp(2, 120)
                    } else {
                        24
                    };
                    let live = LiveData::default();
                    let mut out: Vec<Vec<u8>> = Vec::with_capacity(frame_count);
                    for i in 0..frame_count {
                        let elapsed = if matches!(source, Source::Preset(Preset::Countdown)) {
                            let step_secs = (i as u64 * u64::from(duration_secs))
                                / (frame_count.max(1) as u64);
                            Duration::from_secs(step_secs)
                        } else {
                            Duration::from_millis((i as u64 * 1000) / u64::from(fps.max(1)))
                        };
                        out.push(frame(source, config, elapsed, &live));
                    }
                    out
                }
            };

            // Push frame 0 immediately so the backscreen lights up right away
            bs_batch(
                adb,
                device,
                &[
                    &["brightness", "auto", "off"],
                    &["brightness", "manual", "255"],
                    &["basic", "display_mode", "always"],
                    &["wake"],
                ],
            )?;
            push_frame(adb, device, &frames[0])?;
            let _ = bs(adb, device, &["wake"]);

            let job_id = format!(
                "mcp-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let anim_json = serde_json::json!({
                "jobId": job_id,
                "title": clean_title,
                "fps": fps,
                "brightness": clamped_brightness,
                "durationSecs": duration_secs,
                "frames": frames,
            })
            .to_string();

            let mut temp =
                tempfile::NamedTempFile::new().context("无法创建临时背屏动画配置")?;
            std::io::Write::write_all(&mut temp, anim_json.as_bytes())?;
            temp.as_file().sync_all()?;
            adb.script(&transport, &format!("mkdir -p {}", shell_quote(MCP_REMOTE_DIR)))?;
            adb.transfer(
                &transport,
                "push",
                temp.path(),
                std::path::Path::new(MCP_ANIM_FILE),
            )
            .context("无法推送背屏动画配置到手机")?;

            write_mcp_status_file(
                adb,
                &transport,
                false,
                "animation",
                clean_title,
                clamped_brightness,
                fps,
            );
            let prep_cmd = format!(
                "printf '%s\\n' {job} > {job_file}",
                job = shell_quote(&job_id),
                job_file = shell_quote(MCP_JOB_FILE),
            );
            adb.script(&transport, &prep_cmd)?;
            let launch_cmd = format!(
                "CLASSPATH={jar} setsid /system/bin/app_process64 /system/bin com.stepx.companion.Bridge --backscreen-loop {job} </dev/null >/dev/null 2>&1 &",
                jar = shell_quote("/data/local/tmp/stepx-companion/stepx-bridge.jar"),
                job = shell_quote(&job_id),
            );
            adb.script(&transport, &launch_cmd)?;
            // A successful shell launch is only a submission. Require the
            // phone worker's matching job receipt and first completed frame.
            for _ in 0..20 {
                if let Ok(raw) = adb.shell(&transport, &["cat", MCP_STATUS_FILE]) {
                    if let Ok(status) = serde_json::from_str::<serde_json::Value>(raw.trim()) {
                        if status["jobId"] == job_id {
                            if status["running"] == true && status["frameCount"].as_u64().unwrap_or(0) > 0 {
                                return Ok(format!("手机已确认开始播放「{clean_title}」· 本地点阵驱动，请确认实际显示"));
                            }
                            if status["running"] == false {
                                bail!("手机点阵任务未运行：{}；未自动重启", status["lastError"].as_str().filter(|s| !s.is_empty()).unwrap_or("任务已停止或受手机策略限制"));
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
            bail!("播放请求已提交，尚未收到手机首帧回执；请检查手机任务状态，未自动重启")
        }
    }
}

/// Stops the phone-local driver or CompanionService output and restores
/// the phone's saved display mode, brightness, and basic type.
pub fn stop_via_firmware_mcp(adb: &Adb, device: &Device) -> Result<String> {
    device.verify(adb)?;
    let transport = device.resolve_active_transport(adb)?;
    release_custom_frame_owner(adb, device)?;
    let _ = bs(adb, device, &["special", "countdown", "cancel"]);
    clear_frame(adb, device)?;

    // Do not invent default settings when no snapshot exists. A failed read,
    // invalid snapshot, or failed restore must remain retryable on the phone.
    let saved = read_mcp_restore_state(adb, &transport)?;
    if let Some(saved) = &saved {
        saved.restore(adb, device)
            .context("背屏已停止，但原设置尚未恢复；恢复快照已保留，请重试")?;
    }
    adb.script(
        &transport,
        &format!(
            "rm -f {restore} {status} {job} {anim}",
            restore = shell_quote(MCP_RESTORE_FILE),
            status = shell_quote(MCP_STATUS_FILE),
            job = shell_quote(MCP_JOB_FILE),
            anim = shell_quote(MCP_ANIM_FILE),
        ),
    ).context("背屏已停止，但清理任务回执失败；可以重试")?;
    Ok(if saved.is_some() {
        "已停止手机自定义点阵，并核对恢复手机原背屏设置。"
    } else {
        "已停止手机自定义点阵，保留手机当前背屏设置。"
    }.into())
}

pub fn start_pomodoro_countdown(
    adb: &Adb,
    device: &Device,
    duration_secs: u32,
    brightness: u8,
) -> Result<String> {
    let clamped_brightness = brightness.clamp(1, 8);
    let clamped_duration = duration_secs.clamp(60, 3600);
    let cfg = PlaybackConfig {
        brightness: clamped_brightness,
        fps: 1,
        duration_secs: clamped_duration,
        invert: false,
    };
    let msg = apply_via_firmware_mcp(
        adb,
        device,
        &Source::Preset(Preset::Countdown),
        &cfg,
        "专注倒计时",
    )?;
    Ok(format!(
        "已启动 {} 分钟背屏专注番茄钟 · {}",
        clamped_duration / 60,
        msg
    ))
}

pub fn stop_pomodoro_countdown(adb: &Adb, device: &Device) -> Result<String> {
    let _ = stop_via_firmware_mcp(adb, device)?;
    Ok("已结束背屏专注番茄钟，保留原背屏设置".into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmartMessageIconKind {
    #[default]
    GeneralMessage,
    WeChat,
    Feishu,
    DingTalk,
    WeWork,
    SmsOrOtp,
    PhoneCall,
    Delivery,
}

impl SmartMessageIconKind {
    pub const ALL: [Self; 8] = [
        Self::GeneralMessage,
        Self::WeChat,
        Self::Feishu,
        Self::DingTalk,
        Self::WeWork,
        Self::SmsOrOtp,
        Self::PhoneCall,
        Self::Delivery,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::GeneralMessage => "general",
            Self::WeChat => "wechat",
            Self::Feishu => "feishu",
            Self::DingTalk => "dingtalk",
            Self::WeWork => "wework",
            Self::SmsOrOtp => "sms_otp",
            Self::PhoneCall => "phone_call",
            Self::Delivery => "delivery",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::GeneralMessage => "新消息",
            Self::WeChat => "微信",
            Self::Feishu => "飞书",
            Self::DingTalk => "钉钉",
            Self::WeWork => "企业微信",
            Self::SmsOrOtp => "短信/验证码",
            Self::PhoneCall => "未接来电",
            Self::Delivery => "外卖/快递",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::GeneralMessage => "通用新消息信封像素图标（1条显示图标，多条显示图标+数量）",
            Self::WeChat => "微信气泡像素图标（1条显示专属图标，多条显示图标+数量）",
            Self::Feishu => "飞书飞鸟像素图标（1条显示专属图标，多条显示图标+数量）",
            Self::DingTalk => "钉钉闪电像素图标（1条显示专属图标，多条显示图标+数量）",
            Self::WeWork => "企业微信像素图标（1条显示专属图标，多条显示图标+数量）",
            Self::SmsOrOtp => "短信与验证码像素图标（1条显示专属图标，多条显示图标+数量）",
            Self::PhoneCall => "未接来电听筒像素图标（1条显示专属图标，多条显示图标+数量）",
            Self::Delivery => "外卖与快递包裹像素图标（1条显示专属图标，多条显示图标+数量）",
        }
    }

    pub fn classify_from_source(source: &str, title: &str) -> Self {
        let s = format!("{} {}", source.to_ascii_lowercase(), title.to_ascii_lowercase());
        if s.contains("企业微信") || s.contains("wework") || s.contains("wxwork") {
            Self::WeWork
        } else if s.contains("微信") || s.contains("wechat") || s.contains("tencent.mm") {
            Self::WeChat
        } else if s.contains("飞书") || s.contains("lark") || s.contains("feishu") {
            Self::Feishu
        } else if s.contains("钉钉") || s.contains("dingtalk") || s.contains("rimet") {
            Self::DingTalk
        } else if s.contains("验证码") || s.contains("短信") || s.contains("sms") || s.contains("otp") {
            Self::SmsOrOtp
        } else if s.contains("来电") || s.contains("电话") || s.contains("未接") {
            Self::PhoneCall
        } else if s.contains("快递")
            || s.contains("外卖")
            || s.contains("取件")
            || s.contains("叮咚")
            || s.contains("美团")
            || s.contains("顺丰")
            || s.contains("菜鸟")
        {
            Self::Delivery
        } else {
            Self::GeneralMessage
        }
    }

    /// Priority rank (lower number = higher priority):
    /// WeChat (1) > Feishu (2) > DingTalk (3) > WeWork (4) > SmsOrOtp (5) > Delivery (6) > PhoneCall (7) > GeneralMessage (8).
    pub fn priority_rank(self) -> u8 {
        match self {
            Self::WeChat => 1,
            Self::Feishu => 2,
            Self::DingTalk => 3,
            Self::WeWork => 4,
            Self::SmsOrOtp => 5,
            Self::Delivery => 6,
            Self::PhoneCall => 7,
            Self::GeneralMessage => 8,
        }
    }

    pub fn is_high_priority(self) -> bool {
        !matches!(self, Self::GeneralMessage)
    }

    #[allow(dead_code)]
    /// 13×11 pixel sprite used when `count == 1` (single new message: large centered icon).
    fn single_sprite(self) -> [&'static str; 11] {
        match self {
            Self::WeChat => [
                "  1111111    ",
                " 111111111   ",
                "111 111 111  ",
                "11111111111  ",
                " 111111111111",
                "  11111 11 11",
                " 11  11111111",
                "     11111111",
                "      111111 ",
                "          11 ",
                "           1 ",
            ],
            Self::Feishu => [
                "        111  ",
                "      11111  ",
                " 11  111111  ",
                " 111111111   ",
                "  11111111111",
                "   111111111 ",
                "  11111111   ",
                " 1111  111   ",
                " 11     111  ",
                "         11  ",
                "          1  ",
            ],
            Self::DingTalk => [
                "     111111  ",
                "    111111   ",
                "   111111    ",
                "  1111111111 ",
                "  111111111  ",
                "     11111   ",
                "    11111    ",
                "   11111     ",
                "  1111       ",
                "  111        ",
                "  1          ",
            ],
            Self::WeWork => [
                "  111111111  ",
                " 11111111111 ",
                "111 11111 111",
                "111  111  111",
                "1111111111111",
                "111 11111 111",
                " 11111111111 ",
                "  111111111  ",
                "     111     ",
                "    11111    ",
                "             ",
            ],
            Self::SmsOrOtp => [
                "             ",
                "1111111111111",
                "11         11",
                "1 1       1 1",
                "1  1     1  1",
                "1   1   1   1",
                "1    111    1",
                "1           1",
                "1111111111111",
                "             ",
                "             ",
            ],
            Self::PhoneCall => [
                "  1111       ",
                " 111111      ",
                " 11111       ",
                "  1111       ",
                "   1111      ",
                "    1111     ",
                "     1111    ",
                "      111111 ",
                "       111111",
                "        1111 ",
                "             ",
            ],
            Self::Delivery => [
                "     111     ",
                "   1111111   ",
                " 11111111111 ",
                "11111 1 11111",
                "11111 1 11111",
                "1111111111111",
                "111   1   111",
                "1111111111111",
                "1111111111111",
                " 11111111111 ",
                "             ",
            ],
            Self::GeneralMessage => [
                "     111     ",
                "    11111    ",
                "   1111111   ",
                "  111111111  ",
                "  111 1 111  ",
                "  111 1 111  ",
                "  111111111  ",
                " 11111111111 ",
                "1111111111111",
                "     111     ",
                "      1      ",
            ],
        }
    }

    /// 11×8 compact sprite used in the top half (`y = 2..9`) when `count >= 2` (icon + numeric count).
    fn compact_sprite(self) -> [&'static str; 8] {
        match self {
            Self::WeChat => [
                " 1111111   ",
                "11 111 11  ",
                "111111111  ",
                " 1111111111",
                " 1  11 11 1",
                "    1111111",
                "     11111 ",
                "        11 ",
            ],
            Self::Feishu => [
                "      111  ",
                " 11  1111  ",
                " 1111111   ",
                "  111111111",
                "   1111111 ",
                " 1111 111  ",
                " 11    11  ",
                "        1  ",
            ],
            Self::DingTalk => [
                "    11111  ",
                "   11111   ",
                "  11111111 ",
                "  1111111  ",
                "    1111   ",
                "   1111    ",
                "  111      ",
                "  1        ",
            ],
            Self::WeWork => [
                " 111111111 ",
                "11 11111 11",
                "11  111  11",
                "11111111111",
                "11 11111 11",
                " 111111111 ",
                "    111    ",
                "   11111   ",
            ],
            Self::SmsOrOtp => [
                "11111111111",
                "11       11",
                "1 1     1 1",
                "1  1   1  1",
                "1   111   1",
                "1         1",
                "11111111111",
                "           ",
            ],
            Self::PhoneCall => [
                " 1111      ",
                "11111      ",
                " 1111      ",
                "  1111     ",
                "   1111    ",
                "    111111 ",
                "     111111",
                "      1111 ",
            ],
            Self::Delivery => [
                "    111    ",
                "  1111111  ",
                "1111 1 1111",
                "1111 1 1111",
                "11111111111",
                "11   1   11",
                "11111111111",
                " 111111111 ",
            ],
            Self::GeneralMessage => [
                "    111    ",
                "   11111   ",
                "  111 111  ",
                "  111 111  ",
                "  1111111  ",
                " 111111111 ",
                "    111    ",
                "     1     ",
            ],
        }
    }
}

/// Number of LED cells in the bottom countdown progress bar (`x = 3..=12`, `y = 21`, height = 1 cell).
/// Starts full (`10` cells) and decreases by `1` cell per second (`fps = 1`); when finished,
/// the carousel advances to the next slide.
pub const SMART_SLIDE_PROGRESS_CELLS: usize = 10;

/// When active notifications exceed 12, high-priority notifications (SMS OTP, WeChat, Feishu,
/// DingTalk, WeWork, Delivery, PhoneCall) take precedence and low-priority general notifications
/// are filtered out so the backscreen focuses on important messages.
pub const MAX_SMART_PRIORITY_NOTIFICATIONS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmartDisplaySlide {
    AppNotification {
        kind: SmartMessageIconKind,
        count: u32,
        label: String,
    },
    OtpCode {
        code: String,
        label: String,
    },
}

impl SmartDisplaySlide {
    pub fn priority_rank(&self) -> u8 {
        match self {
            Self::OtpCode { .. } => 0,
            Self::AppNotification { kind, .. } => kind.priority_rank(),
        }
    }

    pub fn short_badge(&self) -> String {
        match self {
            Self::AppNotification { kind, count, .. } => {
                let c = (*count).max(1);
                let cnt_str = if c > 99 {
                    "99+".to_string()
                } else {
                    c.to_string()
                };
                format!("{} · {}条", kind.label(), cnt_str)
            }
            Self::OtpCode { code, .. } => format!("验证码 {code}"),
        }
    }

    pub fn display_label(&self) -> &str {
        match self {
            Self::AppNotification { label, .. } | Self::OtpCode { label, .. } => label.as_str(),
        }
    }
}

/// Builds prioritized `SmartDisplaySlide`s from real active notification candidates
/// `(kind, title, optional_otp_code)`.
/// - When total notifications `> MAX_SMART_PRIORITY_NOTIFICATIONS` (`12`), sorts by priority
///   (`验证码` > `微信` > `飞书` > `钉钉` > `企业微信` > `短信` > `外卖/快递` > `未接来电` > `新消息`)
///   and keeps the top 12 high-priority notifications (dropping low-priority `GeneralMessage` noise).
/// - Returns slides sorted in strict priority order.
pub fn build_prioritized_smart_slides(
    candidates: &[(SmartMessageIconKind, String, Option<String>)],
) -> Vec<SmartDisplaySlide> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut indexed: Vec<(usize, SmartMessageIconKind, &str, Option<&str>)> = candidates
        .iter()
        .enumerate()
        .map(|(idx, (k, title, otp))| (idx, *k, title.as_str(), otp.as_deref()))
        .collect();

    // Sort by (is_otp DESC, priority_rank ASC, original_index ASC)
    indexed.sort_by_key(|(idx, kind, _, otp)| {
        let rank = if otp.is_some_and(|c| !c.trim().is_empty()) {
            0
        } else {
            kind.priority_rank()
        };
        (rank, *idx)
    });

    // If total notifications > 12, cap to top 12 by priority (and drop GeneralMessage if high-priority items exist)
    let has_high_priority = indexed
        .iter()
        .any(|(_, k, _, otp)| otp.is_some() || k.is_high_priority());
    let filtered: Vec<(usize, SmartMessageIconKind, &str, Option<&str>)> =
        if indexed.len() > MAX_SMART_PRIORITY_NOTIFICATIONS {
            indexed
                .into_iter()
                .filter(|(_, k, _, otp)| {
                    if has_high_priority {
                        otp.is_some() || k.is_high_priority()
                    } else {
                        true
                    }
                })
                .take(MAX_SMART_PRIORITY_NOTIFICATIONS)
                .collect()
        } else {
            indexed
        };

    let mut slides: Vec<SmartDisplaySlide> = Vec::new();
    let mut seen_otp: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut group_order: Vec<SmartMessageIconKind> = Vec::new();
    let mut group_counts: std::collections::HashMap<SmartMessageIconKind, (u32, String)> =
        std::collections::HashMap::new();

    for (_, kind, title, otp) in filtered {
        if let Some(code) = otp.map(str::trim).filter(|c| !c.is_empty()) {
            if seen_otp.insert(code.to_string()) {
                slides.push(SmartDisplaySlide::OtpCode {
                    code: code.to_string(),
                    label: format!("验证码 {code}"),
                });
            }
            continue;
        }
        let entry = group_counts.entry(kind).or_insert_with(|| {
            group_order.push(kind);
            (0, title.to_string())
        });
        entry.0 = entry.0.saturating_add(1);
    }

    group_order.sort_by_key(|k| k.priority_rank());
    for kind in group_order {
        if let Some((count, sample_title)) = group_counts.remove(&kind) {
            let c = count.max(1);
            let label = if sample_title.trim().is_empty() {
                format!("{} · {}条", kind.label(), c)
            } else {
                format!("{} · {}条 ({})", kind.label(), c, sample_title.trim())
            };
            slides.push(SmartDisplaySlide::AppNotification {
                kind,
                count: c,
                label,
            });
        }
    }

    slides.sort_by_key(|s| s.priority_rank());
    slides
}

fn draw_bottom_progress_bar(f: &mut [u8], remaining_cells: usize) {
    let lit = remaining_cells.clamp(0, SMART_SLIDE_PROGRESS_CELLS);
    for i in 0..SMART_SLIDE_PROGRESS_CELLS {
        let x = 3 + i as i32;
        if i < lit {
            put(f, x, 21, 255);
        } else {
            put(f, x, 21, 38);
        }
    }
}

/// Synthesizes a 17×23 (391-slot) frame for a single `SmartDisplaySlide` at a given
/// countdown progress step (`remaining_cells` in `1..=10`, decreasing 1 cell per second):
/// - `SmartDisplaySlide::AppNotification { kind, count, .. }`:
///   - Top (`y = 1..=8`): `11×8` app icon (`微信`, `飞书`, `钉钉`, `企业微信`, `短信`, `未接来电`, `外卖/快递`, `新消息`)
///   - Middle (`y = 11..=15`): pixel message count (`1`..`99+`) — shown even when `count == 1`
///   - Bottom (`y = 21`, `x = 3..=12`, height = 1 cell): 10-cell countdown progress bar (`remaining_cells` lit)
/// - `SmartDisplaySlide::OtpCode { code, .. }`:
///   - Top (`y = 1..=2`): verification key indicator bar
///   - Middle (`y = 4..=15`): 4–6 verification code digits (`3×5` pixel font)
///   - Bottom (`y = 21`, `x = 3..=12`, height = 1 cell): 10-cell countdown progress bar (`remaining_cells` lit)
pub fn synthesize_smart_slide_frame(
    slide: &SmartDisplaySlide,
    remaining_cells: usize,
    brightness: u8,
    invert: bool,
) -> Vec<u8> {
    let mut f = vec![0u8; SLOTS];
    match slide {
        SmartDisplaySlide::AppNotification { kind, count, .. } => {
            let effective_count = (*count).max(1);
            // Upper area: 11x8 app icon at x=3..=13, y=1..=8
            sprite(&mut f, &kind.compact_sprite(), 3, 1, 245);
            // Middle area: numeric message count at y=11..=15 (including 1!)
            if effective_count <= 9 {
                number(&mut f, effective_count, 7, 11, false);
            } else if effective_count <= 99 {
                number(&mut f, effective_count, 5, 11, true);
            } else {
                number(&mut f, 99, 3, 11, true);
                put(&mut f, 12, 12, 255);
                put(&mut f, 11, 13, 255);
                put(&mut f, 12, 13, 255);
                put(&mut f, 13, 13, 255);
                put(&mut f, 12, 14, 255);
            }
        }
        SmartDisplaySlide::OtpCode { code, .. } => {
            // Top verification indicator bar at y=1..=2
            for x in 5..=11 {
                put(&mut f, x, 1, 165);
            }
            put(&mut f, 8, 2, 120);
            let digits: Vec<u32> = code
                .chars()
                .filter_map(|c| c.to_digit(10))
                .take(6)
                .collect();
            if digits.is_empty() {
                number(&mut f, 8888, 1, 8, false);
            } else if digits.len() <= 4 {
                let total_w = digits.len() as i32 * 4 - 1;
                let start_x = (17 - total_w) / 2;
                for (i, &d) in digits.iter().enumerate() {
                    sprite(&mut f, &DIGITS[d as usize], start_x + i as i32 * 4, 8, 255);
                }
            } else {
                // Upper 3 digits at y=4..=8, x=3, 7, 11
                for (i, &d) in digits.iter().take(3).enumerate() {
                    sprite(&mut f, &DIGITS[d as usize], 3 + i as i32 * 4, 4, 255);
                }
                // Lower remaining digits at y=11..=15
                let rem = &digits[3..];
                let total_w = rem.len() as i32 * 4 - 1;
                let start_x = (17 - total_w) / 2;
                for (i, &d) in rem.iter().enumerate() {
                    sprite(&mut f, &DIGITS[d as usize], start_x + i as i32 * 4, 11, 255);
                }
            }
        }
    }

    draw_bottom_progress_bar(&mut f, remaining_cells);

    let scale = (brightness.clamp(1, 8) as u16 * 32).min(255);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = y * WIDTH + x;
            if is_backscreen_corner(x, y) {
                f[i] = 0;
            } else {
                let v = if invert { 255 - f[i] } else { f[i] };
                f[i] = (v as u16 * scale / 255) as u8;
            }
        }
    }
    f
}

/// Synthesizes all frames for a multi-slide 1-FPS Smart Display carousel.
/// - Each slide produces `SMART_SLIDE_PROGRESS_CELLS` (`10`) frames where the bottom progress bar
///   starts full (`10` cells) and decreases by `1` cell per second (`10 -> 9 -> ... -> 1`),
///   then immediately transitions to the next slide.
/// - When `slides` is empty, returns 24 frames of the normal animation (`Preset::Astronaut`).
pub fn synthesize_smart_carousel_frames(
    slides: &[SmartDisplaySlide],
    brightness: u8,
    invert: bool,
) -> Vec<Vec<u8>> {
    if slides.is_empty() {
        let cfg = PlaybackConfig {
            brightness: brightness.clamp(1, 8),
            fps: 6,
            duration_secs: 15,
            invert,
        };
        let live = LiveData::default();
        return (0..24)
            .map(|i| {
                frame(
                    &Source::Preset(Preset::Astronaut),
                    &cfg,
                    Duration::from_millis((i as u64 * 1000) / 6),
                    &live,
                )
            })
            .collect();
    }
    let max_slides = 180 / SMART_SLIDE_PROGRESS_CELLS;
    let effective_slides = &slides[..slides.len().min(max_slides)];
    let mut frames = Vec::with_capacity(effective_slides.len() * SMART_SLIDE_PROGRESS_CELLS);
    for slide in effective_slides {
        for step in 0..SMART_SLIDE_PROGRESS_CELLS {
            let remaining = SMART_SLIDE_PROGRESS_CELLS - step;
            frames.push(synthesize_smart_slide_frame(
                slide,
                remaining,
                brightness,
                invert,
            ));
        }
    }
    frames
}

/// Synthesizes a 17×23 (391-slot) smart notification frame with full bottom progress bar:
/// always renders the app icon (`y = 1..=8`), numeric message count (`y = 11..=15`, including `1`),
/// and full 10-cell bottom countdown progress bar (`y = 20..=21`).
pub fn synthesize_smart_notification_frame(
    kind: SmartMessageIconKind,
    count: u32,
    brightness: u8,
    invert: bool,
) -> Vec<u8> {
    let slide = SmartDisplaySlide::AppNotification {
        kind,
        count: count.max(1),
        label: kind.label().into(),
    };
    synthesize_smart_slide_frame(&slide, SMART_SLIDE_PROGRESS_CELLS, brightness, invert)
}

/// Synchronizes the user's autonomous Smart BackScreen configuration (`enabled`,
/// `auto_notify_flash`, `brightness`, `invert`) to `com.stepx.companion.app` on the phone
/// so `PushNotificationService` + `SmartBackScreenController` can drive the rear LED matrix
/// on-device without desktop ADB frame streaming.
pub fn sync_phone_autonomous_smart_backscreen(
    adb: &Adb,
    device: &Device,
    enabled: bool,
    auto_notify_flash: bool,
    brightness: u8,
    invert: bool,
) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let transport = device.resolve_active_transport(adb)?;
    let clamped_brightness = brightness.clamp(1, 8);
    let req = serde_json::json!({
        "op": "configure_smart_backscreen",
        "enabled": enabled,
        "auto_notify_flash": auto_notify_flash,
        "autoNotifyFlash": auto_notify_flash,
        "brightness": clamped_brightness,
        "invert": invert,
    });
    let encoded = STANDARD.encode(serde_json::to_vec(&req)?);
    let cmd = format!(
        "content call --user 0 --uri content://com.stepx.companion.app.push --method request --arg {}",
        shell_quote(&encoded),
    );
    adb.script(&transport, &cmd)?;
    Ok(())
}

/// Pushes a dynamic Smart Display carousel (or restores native standby when `slides` is empty)
/// to the physical rear LED matrix through the phone backscreen output path.
pub fn push_smart_carousel_to_phone(
    adb: &Adb,
    device: &Device,
    slides: &[SmartDisplaySlide],
    brightness: u8,
    invert: bool,
) -> Result<String> {
    if slides.is_empty() {
        let _ = stop_via_firmware_mcp(adb, device)?;
        return Ok("已归还手机原生背屏智能待机 (旋转太空人)".into());
    }
    let title = if slides.len() == 1 {
        match &slides[0] {
            SmartDisplaySlide::AppNotification { kind, count, .. } => {
                format!("智能显示 · {} ({}条新消息)", kind.label(), (*count).max(1))
            }
            SmartDisplaySlide::OtpCode { code, .. } => {
                format!("智能显示 · 验证码 {code}")
            }
        }
    } else {
        let badges: Vec<String> = slides.iter().take(3).map(|s| s.short_badge()).collect();
        format!(
            "智能显示 · 轮播{}项 ({})",
            slides.len(),
            badges.join(" / ")
        )
    };
    let cfg = PlaybackConfig {
        brightness: brightness.clamp(1, 8),
        fps: 1,
        duration_secs: (slides.len() as u32 * SMART_SLIDE_PROGRESS_CELLS as u32).clamp(10, 20),
        invert,
    };
    apply_via_firmware_mcp(
        adb,
        device,
        &Source::SmartCarousel(slides.to_vec()),
        &cfg,
        &title,
    )
}

/// Pushes a smart notification icon + numeric count + 1-cell/sec bottom countdown progress bar
/// directly to the physical rear LED matrix through the phone backscreen output path.
pub fn push_smart_notification_to_phone(
    adb: &Adb,
    device: &Device,
    kind: SmartMessageIconKind,
    count: u32,
    brightness: u8,
    invert: bool,
) -> Result<String> {
    let effective_count = count.max(1);
    let slide = SmartDisplaySlide::AppNotification {
        kind,
        count: effective_count,
        label: format!("{} · {}条", kind.label(), effective_count),
    };
    push_smart_carousel_to_phone(adb, device, &[slide], brightness, invert)
}

/// Renders a 1–6 character Chinese signature (or mixed short CJK/ASCII) onto a 17×23 pixel frame.
pub fn synthesize_chinese_signature_frame(text: &str, invert: bool) -> Vec<u8> {
    let mut f = vec![0u8; SLOTS];
    let mut chars = ['\0'; 6];
    let mut char_len = 0usize;
    for ch in text.trim().chars().filter(|c| !c.is_whitespace()).take(6) {
        chars[char_len] = ch;
        char_len += 1;
    }
    match char_len {
        0 => {}
        1 => {
            for x in 4..=12 {
                put(&mut f, x, 4, 120);
                put(&mut f, x, 18, 120);
            }
            draw_cjk_glyph_7x7(&mut f, chars[0], 5, 8, 255);
        }
        2 => {
            draw_cjk_glyph_7x7(&mut f, chars[0], 5, 3, 255);
            put(&mut f, 8, 11, 120);
            draw_cjk_glyph_7x7(&mut f, chars[1], 5, 13, 255);
        }
        3 => {
            draw_cjk_glyph_7x7(&mut f, chars[0], 1, 3, 255);
            draw_cjk_glyph_7x7(&mut f, chars[1], 9, 3, 255);
            for x in 3..=13 {
                put(&mut f, x, 11, 100);
            }
            draw_cjk_glyph_7x7(&mut f, chars[2], 5, 13, 255);
        }
        4 => {
            for x in 4..=12 {
                put(&mut f, x, 1, 110);
                put(&mut f, x, 21, 110);
            }
            draw_cjk_glyph_7x7(&mut f, chars[0], 1, 3, 255);
            draw_cjk_glyph_7x7(&mut f, chars[1], 9, 3, 255);
            draw_cjk_glyph_7x7(&mut f, chars[2], 1, 13, 255);
            draw_cjk_glyph_7x7(&mut f, chars[3], 9, 13, 255);
        }
        5 => {
            draw_cjk_glyph_7x7(&mut f, chars[0], 1, 1, 255);
            draw_cjk_glyph_7x7(&mut f, chars[1], 9, 1, 255);
            draw_cjk_glyph_7x7(&mut f, chars[2], 1, 8, 255);
            draw_cjk_glyph_7x7(&mut f, chars[3], 9, 8, 255);
            draw_cjk_glyph_7x7(&mut f, chars[4], 5, 15, 255);
        }
        _ => {
            draw_cjk_glyph_7x7(&mut f, chars[0], 1, 1, 255);
            draw_cjk_glyph_7x7(&mut f, chars[1], 9, 1, 255);
            draw_cjk_glyph_7x7(&mut f, chars[2], 1, 8, 255);
            draw_cjk_glyph_7x7(&mut f, chars[3], 9, 8, 255);
            draw_cjk_glyph_7x7(&mut f, chars[4], 1, 15, 255);
            draw_cjk_glyph_7x7(&mut f, chars[5], 9, 15, 255);
        }
    }
    if invert {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let idx = y * WIDTH + x;
                if is_backscreen_corner(x, y) {
                    f[idx] = 0;
                } else {
                    f[idx] = 255u8.saturating_sub(f[idx]);
                }
            }
        }
    }
    f
}

struct SystemCjkBitmapFont {
    file: std::sync::Mutex<std::fs::File>,
    cmap_groups: Vec<(u32, u32, u32)>,
    eblc_strike0: Vec<u8>,
    subtable_ranges: Vec<(u16, u16, usize)>,
    ebdt_offset: u64,
}

impl SystemCjkBitmapFont {
    fn load() -> Option<Self> {
        use std::io::{Read, Seek, SeekFrom};
        const CANDIDATES: &[&str] = &[
            "/usr/share/fonts/truetype/arphic/uming.ttc",
            "/usr/share/fonts/arphic-uming/uming.ttc",
        ];
        let path = CANDIDATES
            .iter()
            .map(std::path::Path::new)
            .find(|p| p.is_file())?;
        let mut file = std::fs::File::open(path).ok()?;
        let mut hdr = [0u8; 16];
        file.read_exact(&mut hdr).ok()?;
        let sfnt_off = if &hdr[0..4] == b"ttcf" {
            u32::from_be_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]) as u64
        } else {
            0u64
        };
        file.seek(SeekFrom::Start(sfnt_off)).ok()?;
        let mut sfnt_hdr = [0u8; 12];
        file.read_exact(&mut sfnt_hdr).ok()?;
        let num_tables = u16::from_be_bytes([sfnt_hdr[4], sfnt_hdr[5]]) as usize;
        let mut dir = vec![0u8; num_tables.min(64) * 16];
        file.read_exact(&mut dir).ok()?;
        let mut cmap_off = None;
        let mut eblc_off = None;
        let mut ebdt_off = None;
        for chunk in dir.chunks_exact(16) {
            let tag = &chunk[0..4];
            let toff = u32::from_be_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]) as u64;
            match tag {
                b"cmap" => cmap_off = Some(toff),
                b"EBLC" => eblc_off = Some(toff),
                b"EBDT" => ebdt_off = Some(toff),
                _ => {}
            }
        }
        let (cmap_off, eblc_off, ebdt_offset) = (cmap_off?, eblc_off?, ebdt_off?);

        // Parse cmap format 12
        file.seek(SeekFrom::Start(cmap_off)).ok()?;
        let mut cmap_hdr = [0u8; 4];
        file.read_exact(&mut cmap_hdr).ok()?;
        let num_cmap = u16::from_be_bytes([cmap_hdr[2], cmap_hdr[3]]) as usize;
        let mut cmap_recs = vec![0u8; num_cmap.min(32) * 8];
        file.read_exact(&mut cmap_recs).ok()?;
        let mut f12_off = None;
        for rec in cmap_recs.chunks_exact(8) {
            let sub_off = u32::from_be_bytes([rec[4], rec[5], rec[6], rec[7]]) as u64;
            file.seek(SeekFrom::Start(cmap_off + sub_off)).ok()?;
            let mut fmt_buf = [0u8; 2];
            if file.read_exact(&mut fmt_buf).is_ok()
                && u16::from_be_bytes(fmt_buf) == 12
            {
                f12_off = Some(cmap_off + sub_off);
                break;
            }
        }
        let f12_off = f12_off?;
        file.seek(SeekFrom::Start(f12_off + 12)).ok()?;
        let mut ng_buf = [0u8; 4];
        file.read_exact(&mut ng_buf).ok()?;
        let num_groups = u32::from_be_bytes(ng_buf) as usize;
        if num_groups == 0 || num_groups > 20_000 {
            return None;
        }
        let mut groups_raw = vec![0u8; num_groups * 12];
        file.read_exact(&mut groups_raw).ok()?;
        let mut cmap_groups = Vec::with_capacity(num_groups);
        for g in groups_raw.chunks_exact(12) {
            let sc = u32::from_be_bytes([g[0], g[1], g[2], g[3]]);
            let ec = u32::from_be_bytes([g[4], g[5], g[6], g[7]]);
            let sg = u32::from_be_bytes([g[8], g[9], g[10], g[11]]);
            cmap_groups.push((sc, ec, sg));
        }

        // Parse EBLC strike 0
        file.seek(SeekFrom::Start(eblc_off + 8)).ok()?;
        let mut s0_hdr = [0u8; 16];
        file.read_exact(&mut s0_hdr).ok()?;
        let idx_sub_off = u32::from_be_bytes([s0_hdr[0], s0_hdr[1], s0_hdr[2], s0_hdr[3]]) as u64;
        let idx_sub_size = u32::from_be_bytes([s0_hdr[4], s0_hdr[5], s0_hdr[6], s0_hdr[7]]) as usize;
        let num_subtables = u32::from_be_bytes([s0_hdr[8], s0_hdr[9], s0_hdr[10], s0_hdr[11]]) as usize;
        if idx_sub_size < num_subtables * 8 || idx_sub_size > 512_000 {
            return None;
        }
        file.seek(SeekFrom::Start(eblc_off + idx_sub_off)).ok()?;
        let mut eblc_strike0 = vec![0u8; idx_sub_size];
        file.read_exact(&mut eblc_strike0).ok()?;

        let mut subtable_ranges = Vec::with_capacity(num_subtables);
        for chunk in eblc_strike0[..num_subtables * 8].chunks_exact(8) {
            let fg = u16::from_be_bytes([chunk[0], chunk[1]]);
            let lg = u16::from_be_bytes([chunk[2], chunk[3]]);
            let add_off = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
            subtable_ranges.push((fg, lg, add_off));
        }
        subtable_ranges.sort_by_key(|&(fg, _, _)| fg);

        Some(Self {
            file: std::sync::Mutex::new(file),
            cmap_groups,
            eblc_strike0,
            subtable_ranges,
            ebdt_offset,
        })
    }

    fn glyph_id(&self, ch: char) -> Option<u16> {
        let cp = ch as u32;
        let idx = self.cmap_groups.partition_point(|&(sc, _, _)| sc <= cp);
        if idx == 0 {
            return None;
        }
        let (sc, ec, sg) = self.cmap_groups[idx - 1];
        if cp <= ec {
            u16::try_from(sg + (cp - sc)).ok().filter(|&gid| gid > 0)
        } else {
            None
        }
    }

    fn read_at(&self, buf: &mut [u8], offset: u64) -> Option<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt as _;
            let file = self.file.lock().ok()?;
            file.read_exact_at(buf, offset).ok()
        }
        #[cfg(not(unix))]
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = self.file.lock().ok()?;
            file.seek(SeekFrom::Start(offset)).ok()?;
            file.read_exact(buf).ok()
        }
    }

    fn glyph_7x7(&self, ch: char) -> Option<[[bool; 7]; 7]> {
        let gid = self.glyph_id(ch)?;
        let range_idx = self.subtable_ranges.partition_point(|&(fg, _, _)| fg <= gid);
        if range_idx == 0 {
            return None;
        }
        let (fg, lg, add_off) = self.subtable_ranges[range_idx - 1];
        if gid > lg || add_off + 8 > self.eblc_strike0.len() {
            return None;
        }
        let sub = &self.eblc_strike0[add_off..];
        let idx_fmt = u16::from_be_bytes([sub[0], sub[1]]);
        let img_fmt = u16::from_be_bytes([sub[2], sub[3]]);
        let img_data_off = u32::from_be_bytes([sub[4], sub[5], sub[6], sub[7]]) as u64;
        let glyph_index = (gid - fg) as usize;

        let mut stack_buf = [0u8; 72];
        let (h, w, raw_bits): (usize, usize, &[u8]) = if idx_fmt == 2 && img_fmt == 5 {
            if sub.len() < 14 {
                return None;
            }
            let img_size = u32::from_be_bytes([sub[8], sub[9], sub[10], sub[11]]) as usize;
            let h = sub[12] as usize;
            let w = sub[13] as usize;
            if h == 0 || w == 0 || img_size == 0 || img_size > 64 {
                return None;
            }
            let b_off = self.ebdt_offset + img_data_off + (glyph_index * img_size) as u64;
            self.read_at(&mut stack_buf[..img_size], b_off)?;
            (h, w, &stack_buf[..img_size])
        } else if idx_fmt == 1 && img_fmt == 7 {
            let off_pos = 8 + glyph_index * 4;
            if sub.len() < off_pos + 8 {
                return None;
            }
            let s0 = u32::from_be_bytes([
                sub[off_pos],
                sub[off_pos + 1],
                sub[off_pos + 2],
                sub[off_pos + 3],
            ]) as u64;
            let s1 = u32::from_be_bytes([
                sub[off_pos + 4],
                sub[off_pos + 5],
                sub[off_pos + 6],
                sub[off_pos + 7],
            ]) as u64;
            if s1 <= s0 + 8 || (s1 - s0) > 72 {
                return None;
            }
            let len = (s1 - s0) as usize;
            let b_off = self.ebdt_offset + img_data_off + s0;
            self.read_at(&mut stack_buf[..len], b_off)?;
            let h = stack_buf[0] as usize;
            let w = stack_buf[1] as usize;
            if h == 0 || w == 0 {
                return None;
            }
            (h, w, &stack_buf[8..len])
        } else {
            return None;
        };

        if raw_bits.len() * 8 < h * w {
            return None;
        }
        let bit_at = |r: usize, c: usize| -> bool {
            let bit_idx = r * w + c;
            let b = raw_bits[bit_idx / 8];
            ((b >> (7 - (bit_idx % 8))) & 1) != 0
        };
        let mut r_min = 0;
        let mut r_max = h - 1;
        while r_min < r_max && !(0..w).any(|c| bit_at(r_min, c)) {
            r_min += 1;
        }
        while r_max > r_min && !(0..w).any(|c| bit_at(r_max, c)) {
            r_max -= 1;
        }
        let mut c_min = 0;
        let mut c_max = w - 1;
        while c_min < c_max && !(0..h).any(|r| bit_at(r, c_min)) {
            c_min += 1;
        }
        while c_max > c_min && !(0..h).any(|r| bit_at(r, c_max)) {
            c_max -= 1;
        }
        let bh = (r_max - r_min + 1).max(1);
        let bw = (c_max - c_min + 1).max(1);
        let mut out = [[false; 7]; 7];
        for (ry, row) in out.iter_mut().enumerate() {
            let sy = r_min + (ry * (bh - 1) + 3) / 6;
            for (rx, cell) in row.iter_mut().enumerate() {
                let sx = c_min + (rx * (bw - 1) + 3) / 6;
                *cell = bit_at(sy.min(h - 1), sx.min(w - 1));
            }
        }
        Some(out)
    }
}

fn system_font_cjk_glyph_7x7(ch: char) -> Option<[[bool; 7]; 7]> {
    static FONT: std::sync::OnceLock<Option<SystemCjkBitmapFont>> = std::sync::OnceLock::new();
    static GLYPH_CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<char, Option<[[bool; 7]; 7]>>>,
    > = std::sync::OnceLock::new();

    let cache = GLYPH_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::with_capacity(256)));
    if let Ok(guard) = cache.lock() {
        if let Some(&cached) = guard.get(&ch) {
            return cached;
        }
    }
    let glyph = FONT
        .get_or_init(SystemCjkBitmapFont::load)
        .as_ref()
        .and_then(|font| font.glyph_7x7(ch));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(ch, glyph);
    }
    glyph
}

fn draw_cjk_glyph_7x7(f: &mut [u8], ch: char, x: i32, y: i32, v: u8) {
    if let Some(rows) = cjk_glyph_7x7(ch) {
        sprite(f, &rows, x, y, v);
        return;
    }
    if ch.is_ascii() {
        let g = tiny_glyph_3x5(ch.to_ascii_uppercase());
        sprite(f, &g, x + 2, y + 1, v);
        return;
    }
    if let Some(grid) = system_font_cjk_glyph_7x7(ch) {
        for (dy, row) in grid.iter().enumerate() {
            for (dx, &lit) in row.iter().enumerate() {
                if lit {
                    put(f, x + dx as i32, y + dy as i32, v);
                }
            }
        }
        return;
    }
    // Fallback if no system CJK bitmap font is present: generate a distinct stroke skeleton per codepoint.
    let mut h = (ch as u32).wrapping_mul(0x9E37_79B9);
    h ^= h >> 13;
    h = h.wrapping_mul(0x85EB_CA6B);
    for dy in 0..7i32 {
        for dx in 0..7i32 {
            let sym_x = dx.min(6 - dx) as u32;
            let bit = (h >> ((dy as u32 * 4 + sym_x) % 24)) & 1;
            let is_stroke = (dy == 0 && (1..=5).contains(&dx))
                || (dy == 3 && (1..=5).contains(&dx))
                || (dx == 3 && (1..=6).contains(&dy))
                || bit != 0;
            if is_stroke {
                put(f, x + dx, y + dy, v);
            }
        }
    }
}

fn cjk_glyph_7x7(ch: char) -> Option<[&'static str; 7]> {
    Some(match ch {
        '开' => [
            "#######",
            " #   # ",
            " #   # ",
            "#######",
            " #   # ",
            " #   # ",
            "#    # ",
        ],
        '会' => [
            "   #   ",
            "  # #  ",
            " #   # ",
            "#######",
            " ##### ",
            " #   # ",
            "#######",
        ],
        '勿' => [
            " ##### ",
            "#  # # ",
            "  # # #",
            " # # # ",
            "# #  # ",
            "   # # ",
            "    ## ",
        ],
        '扰' => [
            " #  #  ",
            "### ###",
            " #  #  ",
            "##  # #",
            " # #  #",
            " # #  #",
            "## # ##",
        ],
        '议' => [
            "#    # ",
            " # #  #",
            "#  # # ",
            "#   #  ",
            "#  # # ",
            "# #   #",
            "##    #",
        ],
        '中' => [
            "   #   ",
            "#######",
            "#  #  #",
            "#  #  #",
            "#######",
            "   #   ",
            "   #   ",
        ],
        '专' => [
            "   #   ",
            "#######",
            "  #    ",
            "#######",
            "    #  ",
            " # #   ",
            "  #### ",
        ],
        '注' => [
            "#   #  ",
            " # ####",
            "#   #  ",
            " #  ###",
            "#   #  ",
            "#   #  ",
            "# #####",
        ],
        '写' => [
            "#######",
            "#     #",
            " ##### ",
            "  #    ",
            " ######",
            "     # ",
            "#####  ",
        ],
        '代' => [
            " #  # #",
            "##  #  ",
            " # ####",
            " #  #  ",
            " #  #  ",
            " #   # ",
            " #    #",
        ],
        '码' => [
            "### ###",
            " #  # #",
            "#   ###",
            "###   #",
            "# # ###",
            "###   #",
            "#  ####",
        ],
        '工' => [
            "#######",
            "   #   ",
            "   #   ",
            "   #   ",
            "   #   ",
            "   #   ",
            "#######",
        ],
        '作' => [
            " #  #  ",
            "## ####",
            " # # # ",
            " # ### ",
            " # # # ",
            " # ### ",
            " # #   ",
        ],
        '去' => [
            "   #   ",
            " ##### ",
            "   #   ",
            "#######",
            "  #    ",
            " #   # ",
            "#######",
        ],
        '吃' => [
            "### #  ",
            "# # ###",
            "###  # ",
            "    #  ",
            "  ###  ",
            " #   # ",
            "  #### ",
        ],
        '饭' => [
            "#  ####",
            "## #   ",
            "#  ### ",
            "# # # #",
            "###  # ",
            "# # # #",
            "#  #  #",
        ],
        '干' => [
            " ##### ",
            "   #   ",
            "   #   ",
            "#######",
            "   #   ",
            "   #   ",
            "   #   ",
        ],
        '午' => [
            " #     ",
            " ##### ",
            "#  #   ",
            "#######",
            "   #   ",
            "   #   ",
            "   #   ",
        ],
        '餐' => [
            "# # ###",
            "### # #",
            " #  ###",
            " ##### ",
            " #   # ",
            " ##### ",
            "#  #  #",
        ],
        '休' => [
            " #  #  ",
            "##  #  ",
            " # ####",
            " #  #  ",
            " # ### ",
            " # # # ",
            " ##   #",
        ],
        '息' => [
            "  ###  ",
            " # # # ",
            "  ###  ",
            " # # # ",
            "  ###  ",
            "#  #  #",
            " ##### ",
        ],
        '一' => [
            "       ",
            "       ",
            "       ",
            "#######",
            "       ",
            "       ",
            "       ",
        ],
        '下' => [
            "#######",
            "   #   ",
            "   # # ",
            "   #  #",
            "   #   ",
            "   #   ",
            "   #   ",
        ],
        '小' => [
            "   #   ",
            "   #   ",
            " # # # ",
            " # # # ",
            "#  #  #",
            "   #   ",
            "  ##   ",
        ],
        '憩' => [
            "### ###",
            " #  # #",
            "### ###",
            " ##### ",
            "  ###  ",
            "#  #  #",
            " ##### ",
        ],
        '摸' => [
            " # ####",
            "### # #",
            " # ####",
            "## #  #",
            " # ####",
            " #  #  ",
            "#  #  #",
        ],
        '鱼' => [
            "  ##   ",
            " #  #  ",
            " ##### ",
            " # # # ",
            " ##### ",
            "   #   ",
            "#######",
        ],
        '马' => [
            "#####  ",
            "    #  ",
            "#####  ",
            "#   #  ",
            "###### ",
            " ##### ",
            "    ## ",
        ],
        '上' => [
            "   #   ",
            "   #   ",
            "   ### ",
            "   #   ",
            "   #   ",
            "   #   ",
            "#######",
        ],
        '回' => [
            "#######",
            "#     #",
            "# ### #",
            "# # # #",
            "# ### #",
            "#     #",
            "#######",
        ],
        '来' => [
            "   #   ",
            " ##### ",
            " # # # ",
            "#######",
            "  ###  ",
            " # # # ",
            "#  #  #",
        ],
        '稍' => [
            "##  #  ",
            " # # # ",
            "### #  ",
            " # ####",
            "# # # #",
            " # ####",
            " # #  #",
        ],
        '等' => [
            "# # # #",
            "##  ## ",
            " ##### ",
            "#######",
            "   #   ",
            " ##### ",
            "  # #  ",
        ],
        '请' => [
            "#   #  ",
            " # ####",
            "#   #  ",
            " # ####",
            "#  #  #",
            "#  ####",
            "## #  #",
        ],
        '打' => [
            " # ####",
            "###  # ",
            " #   # ",
            "##   # ",
            " #   # ",
            " #   # ",
            "##  ## ",
        ],
        '静' => [
            "### #  ",
            " # ####",
            "### # #",
            "### ###",
            "# #  # ",
            "###  # ",
            "# # ## ",
        ],
        '音' => [
            "   #   ",
            " ##### ",
            " #   # ",
            "#######",
            " ##### ",
            " #   # ",
            " ##### ",
        ],
        '已' => [
            "###### ",
            "     # ",
            "     # ",
            "###### ",
            "#      ",
            "#    # ",
            "###### ",
        ],
        '班' => [
            "### ###",
            " # # # ",
            "### ###",
            " # # # ",
            " # # # ",
            "### ###",
            "  #    ",
        ],
        '放' => [
            " #  #  ",
            "### ###",
            " #  # #",
            "### # #",
            "# #  # ",
            "# # # #",
            "#  #  #",
        ],
        '假' => [
            " # ####",
            "## #  #",
            " # ####",
            " # #   ",
            " # ####",
            " #  # #",
            " # #  #",
        ],
        '充' => [
            "   #   ",
            "#######",
            "  # #  ",
            " ##### ",
            "  # #  ",
            " #  # #",
            "#    ##",
        ],
        '电' => [
            "   #   ",
            " ##### ",
            " # # # ",
            " ##### ",
            " # # # ",
            " ##### ",
            "   ####",
        ],
        '满' => [
            "#  ####",
            " #  ## ",
            "# #####",
            " # # # ",
            "# # # #",
            "# # # #",
            "# #   #",
        ],
        '顺' => [
            "# # ###",
            "# #  # ",
            "# # ###",
            "# # # #",
            "# # # #",
            "# #  # ",
            "#  #  #",
        ],
        '丰' => [
            "   #   ",
            " ##### ",
            "   #   ",
            " ##### ",
            "   #   ",
            "#######",
            "   #   ",
        ],
        '取' => [
            "####   ",
            "# # ###",
            "### # #",
            "# #  # ",
            "###  # ",
            "  # # #",
            "   #  #",
        ],
        '件' => [
            " #  #  ",
            "## # # ",
            " # ####",
            " #  #  ",
            " # ####",
            " #  #  ",
            " #  #  ",
        ],
        '快' => [
            " #  #  ",
            "# # ###",
            "##  # #",
            " # ####",
            " #  #  ",
            " # # # ",
            " # #  #",
        ],
        '递' => [
            "#   # #",
            " # ####",
            "#   # #",
            " # ####",
            "#   #  ",
            " #  #  ",
            "# #####",
        ],
        '好' => [
            " # ####",
            " #   # ",
            "###  # ",
            "# # ###",
            " ##  # ",
            " #   # ",
            "#   ## ",
        ],
        '早' => [
            " ##### ",
            " #   # ",
            " ##### ",
            "   #   ",
            "#######",
            "   #   ",
            "   #   ",
        ],
        '安' => [
            "   #   ",
            "#######",
            "#  #  #",
            " ##### ",
            "  # #  ",
            "  ###  ",
            "#    ##",
        ],
        '晚' => [
            "##  ## ",
            "# # # #",
            "## ####",
            "# # # #",
            "##  ###",
            "   # # ",
            "  #  ##",
        ],
        '加' => [
            " #  ###",
            "### # #",
            " # # # ",
            " # # # ",
            " # # # ",
            "# # ###",
            "# #    ",
        ],
        '油' => [
            "#   #  ",
            " # ####",
            "# # # #",
            " # ####",
            "# # # #",
            "#  ####",
            "#      ",
        ],
        '爱' => [
            " ##### ",
            " # # # ",
            "#######",
            "  #    ",
            " ##### ",
            "  # #  ",
            " #   # ",
        ],
        '心' => [
            "   #   ",
            " #   # ",
            "#  #   ",
            "#  #  #",
            "#     #",
            " ##### ",
            "       ",
        ],
        '猫' => [
            "#  # # ",
            "## ####",
            " #  #  ",
            "## ####",
            " # # # ",
            "#  ####",
            "   #  #",
        ],
        '忙' => [
            " #  #  ",
            "# # #  ",
            "## ####",
            " # #   ",
            " # #   ",
            " # #   ",
            " # ####",
        ],
        '冥' => [
            "#######",
            "#     #",
            " ##### ",
            " #   # ",
            " ##### ",
            "  # #  ",
            " #   # ",
        ],
        '想' => [
            " #  ###",
            "### # #",
            " #  ###",
            "# # ###",
            "   #   ",
            "#  #  #",
            " ##### ",
        ],
        '呼' => [
            "### # #",
            "# #  # ",
            "### ###",
            "   # # ",
            "  #####",
            "    #  ",
            "   ##  ",
        ],
        '吸' => [
            "### ###",
            "# # # #",
            "### # #",
            "   ## #",
            "   # # ",
            "  #  # ",
            " #    #",
        ],
        '出' => [
            "   #   ",
            "#  #  #",
            "#######",
            "   #   ",
            "#  #  #",
            "#######",
            "#     #",
        ],
        '门' => [
            "# #####",
            "      #",
            "#     #",
            "#     #",
            "#     #",
            "#     #",
            "#    ##",
        ],
        '带' => [
            " # # # ",
            "#######",
            "   #   ",
            "#######",
            "#  #  #",
            "# ### #",
            "   #   ",
        ],
        '伞' => [
            "   #   ",
            "  # #  ",
            " #   # ",
            "#######",
            " # # # ",
            "   #   ",
            "   #   ",
        ],
        '雨' => [
            "#######",
            "   #   ",
            "#######",
            "#  #  #",
            "# # # #",
            "#  #  #",
            "#  # ##",
        ],
        '天' => [
            "#######",
            "   #   ",
            "   #   ",
            "#######",
            "  # #  ",
            " #   # ",
            "#     #",
        ],
        '气' => [
            " #     ",
            "###### ",
            "#      ",
            " ##### ",
            "     # ",
            "     # ",
            "    ## ",
        ],
        '步' => [
            "   #   ",
            " # ### ",
            " # #   ",
            "#######",
            "  # #  ",
            " # #   ",
            "#   ## ",
        ],
        '高' => [
            "   #   ",
            "#######",
            " ##### ",
            " #   # ",
            "#######",
            "# ### #",
            "# ### #",
        ],
        '升' => [
            " ## #  ",
            "#   #  ",
            " #  #  ",
            "#######",
            " #  #  ",
            " #  #  ",
            "#   #  ",
        ],
        '路' => [
            "###  # ",
            "# # ###",
            "### # #",
            " #   # ",
            "### ###",
            "# # # #",
            "### ###",
        ],
        '风' => [
            "###### ",
            "#    # ",
            "# #  # ",
            "#  # # ",
            "# #  # ",
            "#    # ",
            "#    ##",
        ],
        '平' => [
            "#######",
            " #   # ",
            "  # #  ",
            "#######",
            "   #   ",
            "   #   ",
            "   #   ",
        ],
        '今' => [
            "   #   ",
            "  # #  ",
            " #   # ",
            "#  #  #",
            " ##### ",
            "    #  ",
            "  ##   ",
        ],
        '日' => [
            " ##### ",
            " #   # ",
            " #   # ",
            " ##### ",
            " #   # ",
            " #   # ",
            " ##### ",
        ],
        '明' => [
            "### ###",
            "# # # #",
            "### ###",
            "# # # #",
            "### ###",
            "    # #",
            "   #  #",
        ],
        '起' => [
            " #  ###",
            "###   #",
            " #  ###",
            "##  #  ",
            " # # ##",
            " ##    ",
            "#  ####",
        ],
        '床' => [
            "   #   ",
            "#######",
            "#  #   ",
            "# #####",
            "#  ### ",
            "# # # #",
            "#   #  ",
        ],
        '闹' => [
            "# #####",
            "   #  #",
            "# #####",
            "#  #  #",
            "# ### #",
            "#  #  #",
            "#    ##",
        ],
        '钟' => [
            " #  #  ",
            "### ###",
            " #  # #",
            "### ###",
            " #   # ",
            " #   # ",
            "##   # ",
        ],
        '行' => [
            " # ####",
            "#    # ",
            " ## ###",
            " #   # ",
            " #   # ",
            " #   # ",
            " #  ## ",
        ],
        '车' => [
            "  #    ",
            "#######",
            " # #   ",
            "###### ",
            "   #   ",
            "#######",
            "   #   ",
        ],
        '自' => [
            "   #   ",
            " ##### ",
            " #   # ",
            " ##### ",
            " #   # ",
            " ##### ",
            " #   # ",
        ],
        '驾' => [
            "# # ###",
            "### # #",
            " #  ###",
            " ##### ",
            "    #  ",
            " ##### ",
            "   ### ",
        ],
        '旅' => [
            " #  #  ",
            "### ###",
            " # #   ",
            "## ### ",
            "# # #  ",
            "# # # #",
            "#  ## #",
        ],
        '游' => [
            "#  # # ",
            " # ####",
            "#  # # ",
            " # # ##",
            "#  # # ",
            "# #  # ",
            "#   ## ",
        ],
        '周' => [
            "#######",
            "#  #  #",
            "# ### #",
            "#  #  #",
            "# ### #",
            "# # # #",
            "# ### #",
        ],
        '末' => [
            "   #   ",
            "#######",
            "   #   ",
            " ##### ",
            "  ###  ",
            " # # # ",
            "#  #  #",
        ],
        '乐' => [
            " ##### ",
            " #     ",
            "###### ",
            "   #   ",
            " # # # ",
            "#  #  #",
            "  ##   ",
        ],
        '笑' => [
            "# # # #",
            "##  ## ",
            " ##### ",
            "#######",
            "  # #  ",
            " #   # ",
            "#     #",
        ],
        '水' => [
            "   #   ",
            "   #  #",
            "## # # ",
            "  ###  ",
            " # # # ",
            "#  #  #",
            "  ##   ",
        ],
        '茶' => [
            " # # # ",
            "#######",
            "  # #  ",
            " #   # ",
            " ##### ",
            " # # # ",
            "   #   ",
        ],
        '咖' => [
            "### # #",
            "# # ###",
            "### # #",
            "   # # ",
            "  # ## ",
            "  #    ",
            " #     ",
        ],
        '啡' => [
            "### # #",
            "# # ###",
            "### # #",
            "   ####",
            "    # #",
            "   ####",
            "    # #",
        ],
        '待' => [
            " #  #  ",
            "#  ### ",
            " ## #  ",
            " # ####",
            " #   # ",
            " # # # ",
            " #  ## ",
        ],
        '办' => [
            "   #   ",
            " ##### ",
            "#  #  #",
            " # # # ",
            "  # #  ",
            " #  #  ",
            "#  ##  ",
        ],
        '提' => [
            " #  ###",
            "### # #",
            " #  ###",
            "## ####",
            " #  # #",
            " # # # ",
            "# #  ##",
        ],
        '醒' => [
            "### ###",
            "# # # #",
            "### ###",
            "# #  # ",
            "### ###",
            "# #  # ",
            "### ###",
        ],
        _ => return None,
    })
}

/// Instant (<1ms) deterministic 17x23 pixel badge & text synthesizer for human prompts.
pub fn synthesize_prompt_frame(prompt: &str) -> (String, Vec<u8>) {
    let p = prompt.trim();
    let lower = p.to_lowercase();
    let cfg = PlaybackConfig {
        brightness: 8,
        ..Default::default()
    };
    if is_valid_chinese_signature(p) {
        let f = synthesize_chinese_signature_frame(p, false);
        return (format!("中文签名 · {p}"), f);
    }
    if lower.contains("开会勿扰") {
        return (
            "中文签名 · 开会勿扰".into(),
            synthesize_chinese_signature_frame("开会勿扰", false),
        );
    }
    if lower.contains("请勿打扰") {
        return (
            "中文签名 · 请勿打扰".into(),
            synthesize_chinese_signature_frame("请勿打扰", false),
        );
    }
    if lower.contains("马上回来") || lower.contains("稍等") {
        let sig = if lower.contains("稍等") {
            "稍等"
        } else {
            "马上回来"
        };
        return (
            format!("中文签名 · {sig}"),
            synthesize_chinese_signature_frame(sig, false),
        );
    }
    if lower.contains("去吃饭") || lower.contains("干饭") || lower.contains("午餐") {
        let sig = if lower.contains("干饭") {
            "干饭中"
        } else if lower.contains("午餐") {
            "午餐"
        } else {
            "去吃饭"
        };
        return (
            format!("中文签名 · {sig}"),
            synthesize_chinese_signature_frame(sig, false),
        );
    }
    if lower.contains("休息一下") || lower.contains("小憩") || lower.contains("摸鱼") {
        let sig = if lower.contains("摸鱼") {
            "摸鱼"
        } else if lower.contains("小憩") {
            "小憩"
        } else {
            "休息一下"
        };
        return (
            format!("中文签名 · {sig}"),
            synthesize_chinese_signature_frame(sig, false),
        );
    }
    if lower.contains("已下班") || lower.contains("下班") || lower.contains("放假") {
        let sig = if lower.contains("放假") {
            "放假"
        } else {
            "已下班"
        };
        return (
            format!("中文签名 · {sig}"),
            synthesize_chinese_signature_frame(sig, false),
        );
    }
    if lower.contains("专注中") || lower.contains("写代码") || lower.contains("工作") {
        let sig = if lower.contains("写代码") {
            "写代码"
        } else if lower.contains("工作") {
            "工作"
        } else {
            "专注中"
        };
        return (
            format!("中文签名 · {sig}"),
            synthesize_chinese_signature_frame(sig, false),
        );
    }
    if lower.contains("顺丰") || lower.contains("快递") {
        let sig = if lower.contains("顺丰") {
            "顺丰"
        } else {
            "快递"
        };
        return (
            format!("中文签名 · {sig}"),
            synthesize_chinese_signature_frame(sig, false),
        );
    }
    if lower.contains("会议")
        || lower.contains("勿扰")
        || lower.contains("dnd")
        || lower.contains("静音")
    {
        let mut f = vec![0u8; SLOTS];
        // Draw DND / No-disturb circle with horizontal bar + "勿扰" 7x7 glyphs
        sprite(
            &mut f,
            &[
                "  #####  ",
                " ##   ## ",
                "##     ##",
                "## ### ##",
                "## ### ##",
                "##     ##",
                " ##   ## ",
                "  #####  ",
            ],
            4,
            2,
            255,
        );
        draw_cjk_glyph_7x7(&mut f, '勿', 1, 13, 240);
        draw_cjk_glyph_7x7(&mut f, '扰', 9, 13, 240);
        return ("会议中勿扰".into(), f);
    }
    if lower.contains("sos") || lower.contains("求助") || lower.contains("紧急") {
        let mut f = vec![0u8; SLOTS];
        sprite(
            &mut f,
            &[
                " ###  ###  ### ",
                "#    #   # #   ",
                " ### #   #  ###",
                "   # #   #    #",
                "###   ###  ### ",
            ],
            1,
            5,
            255,
        );
        sprite(
            &mut f,
            &[
                "  #    #    #  ",
                "  #    #    #  ",
                "  #    #    #  ",
                "               ",
                "  #    #    #  ",
            ],
            1,
            13,
            255,
        );
        return ("SOS 紧急灯牌".into(), f);
    }
    if lower.contains("爱心")
        || lower.contains("心")
        || lower.contains("heart")
        || lower.contains("love")
    {
        let f = frame(
            &Source::Preset(Preset::Heart),
            &cfg,
            Duration::from_millis(200),
            &LiveData::default(),
        );
        return ("心动光斑".into(), f);
    }
    if lower.contains("猫") || lower.contains("宠物") || lower.contains("cat") {
        let mut f = vec![0u8; SLOTS];
        sprite(
            &mut f,
            &[
                " #         # ",
                " ##       ## ",
                " ###     ### ",
                " ########### ",
                " #  ## ##  # ",
                " ########### ",
                " #   ###   # ",
                "  #########  ",
                "    #####    ",
                "   #######   ",
                "  ## ### ##  ",
            ],
            2,
            6,
            255,
        );
        return ("像素猫咪".into(), f);
    }
    if lower.contains("闪电")
        || lower.contains("能量")
        || lower.contains("充电")
        || lower.contains("电池")
    {
        let f = frame(
            &Source::Preset(Preset::Battery),
            &cfg,
            Duration::from_millis(200),
            &LiveData::default(),
        );
        return ("赛博能量闪电".into(), f);
    }
    if lower.contains("时间") || lower.contains("时钟") || lower.contains("clock") {
        let f = frame(
            &Source::Preset(Preset::Clock),
            &cfg,
            Duration::from_millis(200),
            &LiveData::default(),
        );
        return ("像素数字时钟".into(), f);
    }
    if lower.contains("笑")
        || lower.contains("开心")
        || lower.contains("休息")
        || lower.contains("smile")
    {
        let mut f = vec![0u8; SLOTS];
        sprite(
            &mut f,
            &[
                "   #######   ",
                " ##       ## ",
                "#  ##   ##  #",
                "#  ##   ##  #",
                "#           #",
                "#  #     #  #",
                "#   #####   #",
                " ##       ## ",
                "   #######   ",
            ],
            2,
            7,
            255,
        );
        return ("微笑休息灯牌".into(), f);
    }
    // Extract digits/letters (e.g. pickup code like "5-2-108" or "4210" or custom short text)
    let ascii: Vec<char> = p
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if !ascii.is_empty() {
        let mut f = vec![0u8; SLOTS];
        // Render up to 3 rows of 4 chars (3x5 font)
        for (idx, ch) in ascii.iter().take(12).enumerate() {
            let row = (idx / 4) as i32;
            let col = (idx % 4) as i32;
            let glyph = tiny_glyph_3x5(*ch);
            sprite(&mut f, &glyph, 1 + col * 4, 3 + row * 6, 255);
        }
        return (format!("点阵灯牌 · {}", p), f);
    }
    let f = frame(
        &Source::Preset(Preset::Meteor),
        &cfg,
        Duration::from_millis(300),
        &LiveData::default(),
    );
    (format!("点阵方案 · {}", p), f)
}

fn tiny_glyph_3x5(ch: char) -> [&'static str; 5] {
    match ch {
        '0' | 'O' => ["###", "# #", "# #", "# #", "###"],
        '1' | 'I' => [" # ", "## ", " # ", " # ", "###"],
        '2' | 'Z' => ["###", "  #", "###", "#  ", "###"],
        '3' => ["###", "  #", "###", "  #", "###"],
        '4' => ["# #", "# #", "###", "  #", "  #"],
        '5' | 'S' => ["###", "#  ", "###", "  #", "###"],
        '6' => ["###", "#  ", "###", "# #", "###"],
        '7' => ["###", "  #", " # ", " # ", " # "],
        '8' | 'B' => ["###", "# #", "###", "# #", "###"],
        '9' => ["###", "# #", "###", "  #", "###"],
        '-' => ["   ", "   ", "###", "   ", "   "],
        'A' => [" # ", "# #", "###", "# #", "# #"],
        'C' => ["###", "#  ", "#  ", "#  ", "###"],
        'D' => ["## ", "# #", "# #", "# #", "## "],
        'E' => ["###", "#  ", "###", "#  ", "###"],
        'F' => ["###", "#  ", "###", "#  ", "#  "],
        'G' => ["###", "#  ", "# #", "# #", "###"],
        'H' => ["# #", "# #", "###", "# #", "# #"],
        'K' => ["# #", "## ", "#  ", "## ", "# #"],
        'L' => ["#  ", "#  ", "#  ", "#  ", "###"],
        'M' => ["# #", "###", "###", "# #", "# #"],
        'N' => ["###", "# #", "# #", "# #", "# #"],
        'P' => ["###", "# #", "###", "#  ", "#  "],
        'R' => ["###", "# #", "## ", "# #", "# #"],
        'T' => ["###", " # ", " # ", " # ", " # "],
        'U' | 'V' => ["# #", "# #", "# #", "# #", "###"],
        'W' => ["# #", "# #", "###", "###", "# #"],
        'X' => ["# #", "# #", " # ", "# #", "# #"],
        'Y' => ["# #", "# #", " # ", " # ", " # "],
        _ => ["###", "# #", " # ", "   ", " # "],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_restore_snapshot_preserves_user_display_and_brightness() {
        let saved = Saved::from_mcp_json(r#"{"displayMode":2,"autoBrightness":0,"manualBrightness":77,"basicType":"text"}"#).unwrap();
        assert_eq!(saved, Saved {
            mode: "custom".into(), kind: "text".into(),
            manual: "77".into(), automatic: "false".into(),
        });
        assert_eq!(saved, Saved::from_mcp_json(r#"{"mode":2,"automatic":0,"brightness":77,"type":"text","rotate":0}"#).unwrap());
    }

    #[test]
    fn mcp_restore_snapshot_never_substitutes_defaults_for_invalid_data() {
        for raw in [
            "", "{}", "null", "not json",
            r#"{"displayMode":9,"autoBrightness":1,"manualBrightness":160,"basicType":"agent"}"#,
            r#"{"displayMode":0,"autoBrightness":2,"manualBrightness":160,"basicType":"agent"}"#,
            r#"{"displayMode":0,"autoBrightness":1,"manualBrightness":0,"basicType":"agent"}"#,
            r#"{"displayMode":0,"autoBrightness":1,"manualBrightness":256,"basicType":"agent"}"#,
            r#"{"displayMode":0,"autoBrightness":1,"manualBrightness":160,"basicType":"unknown"}"#,
        ] {
            assert!(Saved::from_mcp_json(raw).is_err(), "invalid snapshot accepted: {raw}");
        }
    }

    #[test]
    fn all_presets_are_masked_and_animated() {
        let cfg = PlaybackConfig::default();
        let live = LiveData {
            battery_percent: Some(73),
            hour: 12,
            minute: 34,
        };
        for p in Preset::ALL {
            let a = frame(&Source::Preset(p), &cfg, Duration::ZERO, &live);
            assert_eq!(a.len(), SLOTS);
            assert!(a.iter().any(|v| *v > 0));
            for (i, v) in a.iter().enumerate() {
                if is_backscreen_corner(i % 17, i / 17) {
                    assert_eq!(*v, 0)
                }
            }
            if !matches!(p, Preset::Clock | Preset::Battery | Preset::Countdown) {
                assert!(
                    (1..20).any(|n| frame(
                        &Source::Preset(p),
                        &cfg,
                        Duration::from_millis(n * 150),
                        &live
                    ) != a),
                    "{} must animate",
                    p.key()
                )
            }
        }
        assert_eq!(
            (0..SLOTS)
                .filter(|i| !is_backscreen_corner(i % 17, i / 17))
                .count(),
            LEDS
        );
    }
    #[test]
    fn image_respects_alpha_and_aspect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        let img = RgbaImage::from_pixel(100, 20, image::Rgba([255, 255, 255, 0]));
        img.save(&path).unwrap();
        assert!(image_frame(&path, &ImageOptions::default())
            .unwrap()
            .iter()
            .all(|p| *p == 0));
        let img = RgbaImage::from_pixel(100, 20, image::Rgba([255, 255, 255, 255]));
        img.save(&path).unwrap();
        let f = image_frame(&path, &ImageOptions::default()).unwrap();
        assert_eq!(f[8], 0);
        assert_eq!(f[11 * 17 + 8], 255);
    }
    #[test]
    fn astronaut_is_first_loops_without_clipping_and_has_real_yaw() {
        assert_eq!(Preset::ALL[0], Preset::Astronaut);
        let config = PlaybackConfig {
            brightness: 8,
            ..Default::default()
        };
        let draw = |ms| {
            frame(
                &Source::Preset(Preset::Astronaut),
                &config,
                Duration::from_millis(ms),
                &LiveData::default(),
            )
        };
        let phases: std::collections::HashSet<_> = (0..24).map(|i| draw(i * 166)).collect();
        assert!(phases.len() >= 20, "rotation needs distinct phases");
        assert_eq!(draw(0), draw(4000));
        for f in phases {
            assert!(f.iter().filter(|v| **v > 30).count() > 60);
            assert!(f
                .iter()
                .enumerate()
                .all(|(i, v)| !is_backscreen_corner(i % 17, i / 17) || *v == 0));
        }
        assert_ne!(
            draw(0),
            draw(2000),
            "front visor must differ from back helmet"
        );
    }
    #[test]
    fn image_header_and_extreme_cover_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wrong.png");
        let img = image::RgbImage::from_pixel(1, 10000, image::Rgb([240, 240, 240]));
        img.save_with_format(&path, image::ImageFormat::Jpeg)
            .unwrap();
        let f = image_frame(
            &path,
            &ImageOptions {
                fit: ImageFit::Cover,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(f.len(), 391);
        assert!(f[11 * 17 + 8] > 200);
    }
    #[test]
    fn countdown_finishes_and_battery_changes() {
        let cfg = PlaybackConfig {
            duration_secs: 60,
            ..Default::default()
        };
        let a = frame(
            &Source::Preset(Preset::Countdown),
            &cfg,
            Duration::ZERO,
            &LiveData::default(),
        );
        let b = frame(
            &Source::Preset(Preset::Countdown),
            &cfg,
            Duration::from_secs(60),
            &LiveData::default(),
        );
        assert_ne!(a, b);
        let a = frame(
            &Source::Preset(Preset::Battery),
            &cfg,
            Duration::ZERO,
            &LiveData {
                battery_percent: Some(11),
                ..Default::default()
            },
        );
        let b = frame(
            &Source::Preset(Preset::Battery),
            &cfg,
            Duration::ZERO,
            &LiveData {
                battery_percent: Some(88),
                ..Default::default()
            },
        );
        assert_ne!(a, b);
    }

    #[test]
    fn chinese_signature_validation_checks_1_to_6_cjk_chars() {
        assert!(!is_valid_chinese_signature(""));
        assert!(!is_valid_chinese_signature("   "));
        assert!(!is_valid_chinese_signature("STEPX AI"));
        assert!(is_valid_chinese_signature("好"));
        assert!(is_valid_chinese_signature("摸鱼"));
        assert!(is_valid_chinese_signature("专注中"));
        assert!(is_valid_chinese_signature("开会勿扰"));
        assert!(is_valid_chinese_signature("马上回来！"));
        assert!(is_valid_chinese_signature("一路顺风平安"));
        assert!(!is_valid_chinese_signature("超过六个汉字的签名不能通过"));
    }

    #[test]
    fn synthesize_prompt_frame_and_chinese_signatures_respect_391_slots_and_corners() {
        let presets = [
            "开会勿扰",
            "会议中",
            "专注中",
            "写代码",
            "工作",
            "去吃饭",
            "干饭中",
            "午餐",
            "休息一下",
            "小憩",
            "摸鱼",
            "马上回来",
            "稍等",
            "请勿打扰",
            "静音",
            "已下班",
            "放假",
            "充电中",
            "满电",
            "顺丰",
            "取件",
            "快递",
            "好",
            "早安",
            "步步高升",
            "一路顺风平安",
            "冥想呼吸",
            "出门带伞",
        ];
        for text in presets {
            assert!(
                has_all_cjk_glyphs(text),
                "{text} must have full 7x7 glyph coverage"
            );
            let (title, f) = synthesize_prompt_frame(text);
            assert!(!title.is_empty(), "{text} title must not be empty");
            assert_eq!(f.len(), SLOTS, "{text} must have 391 slots");
            assert!(
                f.iter().filter(|&&v| v > 0).count() >= 10,
                "{text} must light up LEDs"
            );
            for (i, &v) in f.iter().enumerate() {
                if is_backscreen_corner(i % WIDTH, i / WIDTH) {
                    assert_eq!(v, 0, "{text} corner cell {i} must be 0");
                }
            }
            let inv = synthesize_chinese_signature_frame(text, true);
            assert_eq!(inv.len(), SLOTS);
            for (i, &v) in inv.iter().enumerate() {
                if is_backscreen_corner(i % WIDTH, i / WIDTH) {
                    assert_eq!(v, 0, "{text} inverted corner cell {i} must be 0");
                }
            }
        }
        assert!(!has_all_cjk_glyphs("饕餮盛宴"));
        assert_ne!(
            synthesize_chinese_signature_frame("冥想呼吸", false),
            synthesize_chinese_signature_frame("出门带伞", false)
        );
        let f_taotie = synthesize_chinese_signature_frame("饕餮盛宴", false);
        let f_longfeng = synthesize_chinese_signature_frame("龙凤呈祥", false);
        let f_xile = synthesize_chinese_signature_frame("喜乐安康", false);
        assert_ne!(
            f_taotie, f_longfeng,
            "unlisted CJK phrases must render distinct glyphs rather than identical placeholder boxes"
        );
        assert_ne!(f_taotie, f_xile);
        assert_ne!(f_longfeng, f_xile);
        for f in [&f_taotie, &f_longfeng, &f_xile] {
            assert_eq!(f.len(), SLOTS);
            assert!(f.iter().filter(|&&v| v > 0).count() >= 25);
            for (i, &v) in f.iter().enumerate() {
                if is_backscreen_corner(i % WIDTH, i / WIDTH) {
                    assert_eq!(v, 0);
                }
            }
        }
    }

    #[test]
    fn glyph_cache_accelerates_repeated_cjk_frame_synthesis() {
        let phrases = ["饕餮盛宴", "龙凤呈祥", "喜乐安康", "鸿蒙智行", "阿维塔", "步步高升"];
        let iterations = 200;

        // Warm up GLYPH_CACHE
        for p in phrases {
            std::hint::black_box(synthesize_chinese_signature_frame(p, false));
        }

        let t_cached_glyph = Instant::now();
        for _ in 0..iterations {
            for p in phrases {
                for ch in p.chars() {
                    std::hint::black_box(system_font_cjk_glyph_7x7(ch));
                }
            }
        }
        let cached_glyph_us = t_cached_glyph.elapsed().as_micros() as f64 / iterations as f64;

        let t_synth = Instant::now();
        for _ in 0..iterations {
            for p in phrases {
                std::hint::black_box(synthesize_chinese_signature_frame(p, false));
            }
        }
        let synth_batch_us = t_synth.elapsed().as_micros() as f64 / iterations as f64;
        let synth_per_frame_us = synth_batch_us / phrases.len() as f64;

        if let Some(font) = SystemCjkBitmapFont::load() {
            let t_uncached = Instant::now();
            for _ in 0..iterations {
                for p in phrases {
                    for ch in p.chars() {
                        std::hint::black_box(font.glyph_7x7(ch));
                    }
                }
            }
            let uncached_us = t_uncached.elapsed().as_micros() as f64 / iterations as f64;
            eprintln!(
                "[BACKSCREEN BENCHMARK] 6-phrase CJK glyph lookup: uncached={uncached_us:.2}us -> cached={cached_glyph_us:.2}us; full frame synthesis={synth_per_frame_us:.2}us/frame"
            );
            assert!(cached_glyph_us < uncached_us);
        } else {
            eprintln!(
                "[BACKSCREEN BENCHMARK] 6-phrase CJK glyph lookup (fallback cached): cached={cached_glyph_us:.2}us; full frame synthesis={synth_per_frame_us:.2}us/frame"
            );
        }
    }

    static LIVE_BACKSCREEN_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[ignore = "mutates a physical phone; use the explicit verify_native_mcp example for routing checks"]
    fn live_firmware_mcp_backscreen_apply_and_inspect_roundtrip() {
        let _guard = LIVE_BACKSCREEN_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Ok(adb) = crate::adb::Adb::discover() else {
            return;
        };
        let Ok(raw_devices) = adb.run_timeout(&["devices", "-l"], Duration::from_secs(2)) else {
            return;
        };
        let transports = crate::device::parse_devices(&raw_devices);
        let Some(device) = transports
            .iter()
            .filter(|t| t.state == "device")
            .find_map(|t| crate::device::Device::inspect(&adb, &t.id).ok())
        else {
            return;
        };
        let cfg = PlaybackConfig::default();
        let clock_msg =
            apply_via_firmware_mcp(&adb, &device, &Source::Preset(Preset::Clock), &cfg, "像素时钟")
                .expect("clock apply via firmware MCP must succeed");
        assert!(clock_msg.contains("手机背屏"));

        let astro_msg = apply_via_firmware_mcp(
            &adb,
            &device,
            &Source::Preset(Preset::Astronaut),
            &cfg,
            "旋转太空人",
        )
        .expect("astronaut apply via firmware MCP must succeed");
        assert!(astro_msg.contains("手机已确认开始播放"));

        std::thread::sleep(Duration::from_millis(600));
        let dump = adb
            .shell(&device.transport, &["dumpsys", "display_ext"])
            .expect("dumpsys display_ext must succeed");
        assert!(dump.contains("appUsesCustomFrame=true"), "dump: {dump}");
        assert!(dump.contains("appFrameOwner=uid=2000"), "dump: {dump}");
        assert!(dump.contains("appliedDriverFlat=391"), "dump: {dump}");
        assert!(dump.contains("power=on"), "dump: {dump}");

        let status = crate::companion_app::inspect(&adb, &device)
            .expect("companion inspect must succeed with firmware MCP status");
        assert!(status.running);
        assert_eq!(status.applied_title, "旋转太空人");
        assert!(!status.message.contains("请将背屏朝向自己"), "status: {status:?}");
        assert!(status.message.contains("旋转太空人"), "status: {status:?}");
        let _ = stop_via_firmware_mcp(&adb, &device);
    }

    #[test]
    fn smart_notification_frames_render_single_icon_and_multi_count_badges() {
        for kind in SmartMessageIconKind::ALL {
            let mut prev_count_slice = Vec::new();
            for count in [1u32, 2, 3, 5, 12, 120] {
                let fm = synthesize_smart_notification_frame(kind, count, 8, false);
                assert_eq!(fm.len(), SLOTS);
                // Corners must be masked
                assert_eq!(fm[0], 0);
                assert_eq!(fm[WIDTH - 1], 0);
                assert_eq!(fm[(HEIGHT - 1) * WIDTH], 0);
                assert_eq!(fm[SLOTS - 1], 0);

                // Upper app icon in y=1..=8 AND middle numeric count in y=11..=15 must both be lit (even for count == 1)
                let top_icon_lit = fm[1 * WIDTH..=8 * WIDTH + (WIDTH - 1)]
                    .iter()
                    .filter(|&&v| v > 0)
                    .count();
                let count_badge_lit = fm[11 * WIDTH..=15 * WIDTH + (WIDTH - 1)]
                    .iter()
                    .filter(|&&v| v > 0)
                    .count();
                assert!(
                    top_icon_lit >= 10,
                    "smart notification frame ({kind:?}, count={count}) must light upper app icon"
                );
                assert!(
                    count_badge_lit >= 6,
                    "smart notification frame ({kind:?}, count={count}) must light numeric count"
                );
                let count_slice = fm[11 * WIDTH..16 * WIDTH].to_vec();
                assert_ne!(
                    count_slice, prev_count_slice,
                    "numeric count must change across counts (count={count})"
                );
                prev_count_slice = count_slice;

                // Bottom countdown progress bar at y=21 (strictly 1 cell high: y=20 and y=22 are 0), x=3..=12 must start full (10 bright cells)
                let bright_progress_cells = (3..=12)
                    .filter(|&x| fm[21 * WIDTH + x] >= 200)
                    .count();
                assert_eq!(
                    bright_progress_cells, SMART_SLIDE_PROGRESS_CELLS,
                    "full progress bar must have all 10 cells lit on row 21"
                );
                assert!(
                    (0..WIDTH).all(|x| fm[20 * WIDTH + x] == 0 && fm[22 * WIDTH + x] == 0),
                    "bottom progress bar must be strictly 1 cell high (rows 20 and 22 must be 0)"
                );
            }
        }

        // Verify 1-cell-per-second progress countdown on both AppNotification and OtpCode slides
        let otp_slide = SmartDisplaySlide::OtpCode {
            code: "167188".into(),
            label: "验证码 167188".into(),
        };
        for remaining in (1..=SMART_SLIDE_PROGRESS_CELLS).rev() {
            let f_otp = synthesize_smart_slide_frame(&otp_slide, remaining, 8, false);
            let bright_cells = (3..=12)
                .filter(|&x| f_otp[21 * WIDTH + x] >= 200)
                .count();
            assert_eq!(
                bright_cells, remaining,
                "OTP slide at remaining={remaining} must have exactly {remaining} bright progress cells"
            );
            assert!(
                (0..WIDTH).all(|x| f_otp[20 * WIDTH + x] == 0 && f_otp[22 * WIDTH + x] == 0),
                "OTP bottom progress bar must be strictly 1 cell high"
            );
            let digits_lit = f_otp[4 * WIDTH..=15 * WIDTH + (WIDTH - 1)]
                .iter()
                .filter(|&&v| v >= 200)
                .count();
            assert!(digits_lit >= 30, "6-digit OTP must render all digits clearly");
        }

        // Verify multi-slide carousel transitions and empty-carousel fallback to Astronaut
        let slides = vec![
            SmartDisplaySlide::OtpCode {
                code: "167188".into(),
                label: "验证码 167188".into(),
            },
            SmartDisplaySlide::AppNotification {
                kind: SmartMessageIconKind::WeChat,
                count: 3,
                label: "微信 · 3条".into(),
            },
        ];
        let carousel = synthesize_smart_carousel_frames(&slides, 8, false);
        assert_eq!(carousel.len(), 2 * SMART_SLIDE_PROGRESS_CELLS);
        // Frame 9 (end of slide 0) has 1 bright cell; Frame 10 (start of slide 1) resets to 10 bright cells!
        let end_slide_0_bright = (3..=12).filter(|&x| carousel[9][21 * WIDTH + x] >= 200).count();
        let start_slide_1_bright = (3..=12).filter(|&x| carousel[10][21 * WIDTH + x] >= 200).count();
        assert_eq!(end_slide_0_bright, 1);
        assert_eq!(start_slide_1_bright, 10);

        let idle_frames = synthesize_smart_carousel_frames(&[], 8, false);
        assert_eq!(idle_frames.len(), 24);

        // Verify priority filtering & ordering when total notifications > 12
        let mut over_12_candidates = vec![
            (SmartMessageIconKind::GeneralMessage, "系统推广1".to_string(), None),
            (SmartMessageIconKind::GeneralMessage, "系统推广2".to_string(), None),
            (SmartMessageIconKind::GeneralMessage, "系统推广3".to_string(), None),
            (SmartMessageIconKind::GeneralMessage, "系统推广4".to_string(), None),
            (SmartMessageIconKind::GeneralMessage, "系统推广5".to_string(), None),
            (SmartMessageIconKind::PhoneCall, "未接来电 · 张三".to_string(), None),
            (SmartMessageIconKind::Delivery, "顺丰速运 · 已送达".to_string(), None),
            (SmartMessageIconKind::DingTalk, "钉钉 · 审批".to_string(), None),
            (SmartMessageIconKind::DingTalk, "钉钉 · 日报".to_string(), None),
            (SmartMessageIconKind::Feishu, "飞书 · 会议".to_string(), None),
            (SmartMessageIconKind::Feishu, "飞书 · 文档".to_string(), None),
            (SmartMessageIconKind::WeChat, "微信 · 群聊1".to_string(), None),
            (SmartMessageIconKind::WeChat, "微信 · 群聊2".to_string(), None),
            (SmartMessageIconKind::WeChat, "微信 · 群聊3".to_string(), None),
            (SmartMessageIconKind::SmsOrOtp, "验证码 839201".to_string(), Some("839201".to_string())),
        ];
        assert_eq!(over_12_candidates.len(), 15);
        let prioritized = build_prioritized_smart_slides(&over_12_candidates);
        // Must drop GeneralMessage when > 12 notifications exist and sort by priority:
        // OtpCode (0) -> WeChat (1) -> Feishu (2) -> DingTalk (3) -> Delivery (6) -> PhoneCall (7)
        assert_eq!(prioritized.len(), 6);
        assert!(matches!(&prioritized[0], SmartDisplaySlide::OtpCode { code, .. } if code == "839201"));
        assert!(matches!(&prioritized[1], SmartDisplaySlide::AppNotification { kind: SmartMessageIconKind::WeChat, count: 3, .. }));
        assert!(matches!(&prioritized[2], SmartDisplaySlide::AppNotification { kind: SmartMessageIconKind::Feishu, count: 2, .. }));
        assert!(matches!(&prioritized[3], SmartDisplaySlide::AppNotification { kind: SmartMessageIconKind::DingTalk, count: 2, .. }));
        assert!(matches!(&prioritized[4], SmartDisplaySlide::AppNotification { kind: SmartMessageIconKind::Delivery, count: 1, .. }));
        assert!(matches!(&prioritized[5], SmartDisplaySlide::AppNotification { kind: SmartMessageIconKind::PhoneCall, count: 1, .. }));
        over_12_candidates.clear();
        assert!(build_prioritized_smart_slides(&over_12_candidates).is_empty());
    }

    #[test]
    #[ignore = "mutates a physical phone; requires deliberate live test invocation"]
    fn live_smart_notification_single_and_multi_count_e2e() {
        let _guard = LIVE_BACKSCREEN_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Ok(adb) = crate::adb::Adb::discover() else {
            return;
        };
        let Ok(raw_devices) = adb.run_timeout(&["devices", "-l"], Duration::from_secs(2)) else {
            return;
        };
        let transports = crate::device::parse_devices(&raw_devices);
        let Some(device) = transports
            .iter()
            .filter(|t| t.state == "device")
            .find_map(|t| crate::device::Device::inspect(&adb, &t.id).ok())
        else {
            return;
        };

        // 1. Verify single notification (1条 -> 应用图标 + 消息数量 1 + 底部10格进度条一秒减少一格)
        let msg1 = push_smart_notification_to_phone(
            &adb,
            &device,
            SmartMessageIconKind::WeChat,
            1,
            8,
            false,
        )
        .expect("push 1 message icon+count+progress to phone must succeed");
        assert!(msg1.contains("智能显示 · 微信 (1条新消息)"), "msg1: {msg1}");
        let status1 = crate::companion_app::inspect(&adb, &device).expect("inspect after 1 msg");
        assert!(status1.running);
        assert_eq!(status1.applied_title, "智能显示 · 微信 (1条新消息)");

        // 2. Verify multi-notification + OTP carousel (微信12条 -> 验证码167188 -> 外卖/快递2条 -> 未接来电2条)
        let carousel_slides = vec![
            SmartDisplaySlide::AppNotification {
                kind: SmartMessageIconKind::WeChat,
                count: 12,
                label: "微信 · 12条".into(),
            },
            SmartDisplaySlide::OtpCode {
                code: "167188".into(),
                label: "验证码 167188".into(),
            },
            SmartDisplaySlide::AppNotification {
                kind: SmartMessageIconKind::Delivery,
                count: 2,
                label: "外卖/快递 · 2条".into(),
            },
            SmartDisplaySlide::AppNotification {
                kind: SmartMessageIconKind::PhoneCall,
                count: 2,
                label: "未接来电 · 2条".into(),
            },
        ];
        let msg_multi = push_smart_carousel_to_phone(&adb, &device, &carousel_slides, 8, false)
            .expect("push multi-slide carousel to phone must succeed");
        assert!(msg_multi.contains("智能显示 · 轮播4项"), "msg_multi: {msg_multi}");
        std::thread::sleep(Duration::from_millis(2200));
        let dump = adb
            .shell(&device.transport, &["dumpsys", "display_ext"])
            .expect("dumpsys display_ext");
        assert!(dump.contains("appUsesCustomFrame=true"));
        assert!(dump.contains("appliedDriverFlat=391"));
        assert!(dump.contains("power=on"));
        let status_multi =
            crate::companion_app::inspect(&adb, &device).expect("inspect after multi carousel");
        assert!(status_multi.running);
        assert!(status_multi.applied_title.contains("智能显示 · 轮播4项"));
        let _ = stop_via_firmware_mcp(&adb, &device);
    }
}

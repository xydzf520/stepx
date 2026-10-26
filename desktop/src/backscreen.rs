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
        let script=format!("if ! read -r finished; then service call display_ext 5 i32 -1 i32 0 >/dev/null; dumpsys display_ext back_screen brightness manual {} >/dev/null; dumpsys display_ext back_screen brightness auto {} >/dev/null; dumpsys display_ext back_screen basic type {} >/dev/null; dumpsys display_ext back_screen basic display_mode {} >/dev/null; fi",shell_quote(&saved.manual),if saved.automatic=="true"{"on"}else{"off"},shell_quote(&saved.kind),shell_quote(&saved.mode));
        let mut command = Command::new(&adb.path);
        crate::resources::quiet_command(&mut command);
        let mut child = command
            .args(["-s", &device.transport, "shell", &script])
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
        &["-s", &device.transport, "shell", &command],
        Duration::from_secs(3),
    )?;
    if out.lines().any(|line| {
        let l = line.trim().to_ascii_lowercase();
        l.starts_with("unknown command")
            || l.starts_with("invalid")
            || l.starts_with("error:")
            || l.starts_with("exception")
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
#[derive(Clone, Debug)]
struct Saved {
    mode: String,
    kind: String,
    manual: String,
    automatic: String,
}
impl Saved {
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
    let out = adb.run_timeout(
        &[
            "-s",
            &device.transport,
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
    let mut cmd = String::from("service call display_ext 5 i32 391");
    for p in pixels {
        cmd.push_str(&format!(" i32 {p}"))
    }
    cmd.push_str(" i32 0");
    let out = adb.run_timeout(
        &["-s", &device.transport, "shell", &cmd],
        Duration::from_secs(3),
    )?;
    if out.trim() != "Result: Parcel(NULL)" {
        bail!("手机未确认接收背屏帧")
    };
    Ok(())
}

fn live_data(adb: &Adb, device: &Device) -> Result<LiveData> {
    let out = adb.script(&device.transport, "date '+%H %M'; dumpsys battery")?;
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
    let out = adb.shell(&device.transport, &["dumpsys", "activity", "activities"])?;
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
            let result = adb.shell(&device.transport, &["pm", "path", package])?;
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
            device.verify(&adb)?;
            check_source(&adb, &device, &source)?;
            let before = adb.shell(&device.transport, &["dumpsys", "display_ext"])?;
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
                        push_frame(&adb, &device, &zero)?;
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
                        push_frame(&adb, &device, &pixels)?;
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
                        let state = adb.shell(&device.transport, &["dumpsys", "display_ext"])?;
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
    device.verify(adb)?;
    let state = adb.shell(&device.transport, &["dumpsys", "display_ext"])?;
    if state.contains("appUsesCustomFrame=true") {
        bail!("手机当前被自定义背屏占用，请先结束当前播放，再启用原生联动")
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
            &device.transport,
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
                let camera = adb.shell(
                    &device.transport,
                    &["pm", "path", "com.stepos.media.camera"],
                )?;
                if !camera.starts_with("package:") {
                    bail!("当前手机未安装此固件支持的原生相机")
                }
                adb.shell(
                    &device.transport,
                    &[
                        "settings",
                        "put",
                        "system",
                        "stepos_back_screen_camera_link",
                        "1",
                    ],
                )?;
                let enabled = bs(adb, device, &["special"])?;
                if !enabled.contains("cameraLink=true") {
                    bail!("相机联动设置回读失败")
                }
                Ok("已开启手机原生相机背屏联动；手机已确认 cameraLink 开关生效。".into())
            }
        }
    })();
    if let Err(error) = result {
        let restore = saved.restore(adb, device);
        let camera_restore = if let Some(previous) = camera_before {
            if previous.trim() == "null" {
                adb.shell(
                    &device.transport,
                    &[
                        "settings",
                        "delete",
                        "system",
                        "stepos_back_screen_camera_link",
                    ],
                )
            } else {
                adb.shell(
                    &device.transport,
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
    device.verify(adb)?;
    if enabled
        && !adb
            .shell(
                &device.transport,
                &["pm", "path", "com.stepos.media.camera"],
            )?
            .starts_with("package:")
    {
        bail!("当前手机未安装此固件支持的原生相机")
    }
    let before = adb.shell(
        &device.transport,
        &[
            "settings",
            "get",
            "system",
            "stepos_back_screen_camera_link",
        ],
    )?;
    let result = (|| -> Result<String> {
        adb.shell(
            &device.transport,
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
                &device.transport,
                &[
                    "settings",
                    "delete",
                    "system",
                    "stepos_back_screen_camera_link",
                ],
            )
        } else {
            adb.shell(
                &device.transport,
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
pub fn marquee(
    adb: &Adb,
    device: &Device,
    text: &str,
    scroll: bool,
    invert: bool,
) -> Result<String> {
    device.verify(adb)?;
    let text = text.trim();
    if text.is_empty()
        || text.len() > 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" !?-".contains(&b))
    {
        bail!("手机原生字库支持最多64个英文字母、数字、空格和 ! ? -；中文请使用 Amoo 生成像素图")
    };
    clear_frame(adb, device)?;
    bs_batch(
        adb,
        device,
        &[
            &["basic", "text", text],
            &["basic", "marquee", if scroll { "on" } else { "off" }],
            &["basic", "invert", if invert { "on" } else { "off" }],
            &["basic", "type", "text"],
        ],
    )?;
    Ok(format!("已保存手机背屏文字：{text}"))
}

pub fn start_pomodoro_countdown(
    adb: &Adb,
    device: &Device,
    duration_secs: u32,
    brightness: u8,
) -> Result<String> {
    let clamped_brightness = brightness.clamp(1, 8);
    let clamped_duration = duration_secs.clamp(60, 3600);
    if let Ok(message) = crate::companion_app::apply_preset(
        adb,
        device,
        Preset::Countdown.key(),
        clamped_brightness,
        clamped_duration,
    ) {
        return Ok(format!(
            "已启动 {} 分钟背屏专注番茄钟 · {}",
            clamped_duration / 60,
            message
        ));
    }
    device.verify(adb)?;
    let cfg = PlaybackConfig {
        brightness: clamped_brightness,
        fps: 6,
        duration_secs: clamped_duration,
        invert: false,
    };
    let initial = frame(
        &Source::Preset(Preset::Countdown),
        &cfg,
        Duration::ZERO,
        &LiveData::default(),
    );
    let _ = bs(adb, device, &["wake"]);
    let _ = push_frame(adb, device, &initial);
    Ok(format!(
        "已开启 {} 分钟背屏专注番茄钟（手机背屏已同步倒计时画面）",
        clamped_duration / 60
    ))
}

pub fn stop_pomodoro_countdown(adb: &Adb, device: &Device) -> Result<String> {
    let _ = crate::companion_app::stop(adb, device);
    let _ = clear_frame(adb, device);
    Ok("已结束背屏专注番茄钟，恢复手机默认背屏显示".into())
}

/// Instant (<1ms) deterministic 17x23 pixel badge & text synthesizer for human prompts.
pub fn synthesize_prompt_frame(prompt: &str) -> (String, Vec<u8>) {
    let p = prompt.trim();
    let lower = p.to_lowercase();
    let cfg = PlaybackConfig {
        brightness: 8,
        ..Default::default()
    };
    if lower.contains("会议")
        || lower.contains("勿扰")
        || lower.contains("dnd")
        || lower.contains("静音")
    {
        let mut f = vec![0u8; SLOTS];
        // Draw DND / No-disturb circle with horizontal bar + "DND" 3x5 text
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
            3,
            255,
        );
        sprite(
            &mut f,
            &[
                "##. #.# ##.",
                "#.# ### #.#",
                "#.# ### #.#",
                "#.# #.# #.#",
                "##. #.# ##.",
            ],
            3,
            14,
            230,
        );
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
}

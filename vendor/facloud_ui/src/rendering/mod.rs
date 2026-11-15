pub(crate) mod atlas;
pub(crate) mod glyph_cache;
#[cfg(wgpu)]
pub mod wgpu;

pub use facloud_ui_core::rendering::*;
use facloud_ui_core::scene::Dash;

pub(crate) use glyph_cache::{GlyphCache, GlyphRasterBoundsFn, RasterizeGlyphFn};

/// Cache for the result of calling [`is_low_power_gpu_available`], as the
/// check can be expensive.
static LOW_POWER_GPU_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Returns `true` if a low power GPU is available for rendering. Typically, this is true for
/// machines with two GPUs -- a dedicated discrete high-performance GPU and a lower power
/// integrated GPU.
pub fn is_low_power_gpu_available() -> bool {
    *LOW_POWER_GPU_AVAILABLE.get_or_init(|| {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "macos")] {
                crate::platform::mac::is_low_power_gpu_available()
            } else if #[cfg(wgpu)] {
                facloud_ui_core::r#async::block_on(wgpu::is_low_power_gpu_available())
            } else {
                false
            }
        }
    })
}

/// Returns the gap length between each dash to ensure that the stroke begins and ends with a full dash,
/// minimizing deviation from the target gap length.
// adapted from Blink dashed border rendering code:
// https://source.chromium.org/chromium/chromium/src/+/refs/heads/main:third_party/blink/renderer/platform/graphics/stroke_data.cc;l=130-147;drc=51e1b713f6da38219910bf8fb93a81262340bf97
pub(crate) fn get_best_dash_gap(
    stroke_length: f32,
    Dash {
        dash_length,
        gap_length,
        force_consistent_gap_length,
    }: Dash,
) -> f32 {
    if force_consistent_gap_length {
        return gap_length;
    }

    // If no space for two dashes and a gap between, return gap length 0 (solid border)
    if stroke_length < 2. * dash_length + gap_length {
        return 0.;
    }

    let min_num_dashes = (stroke_length / (dash_length + gap_length)).floor();
    let max_num_dashes = min_num_dashes + 1.;
    let min_num_gaps = min_num_dashes - 1.;
    let max_num_gaps = max_num_dashes - 1.;
    let min_gap = (stroke_length - min_num_dashes * dash_length) / min_num_gaps;
    let max_gap = (stroke_length - max_num_dashes * dash_length) / max_num_gaps;
    if max_gap <= 0. || ((min_gap - gap_length).abs() < (max_gap - gap_length).abs()) {
        min_gap
    } else {
        max_gap
    }
}

const TRANSPARENT_PIXEL_RGBA: [u8; 4] = [0, 0, 0, 0];

/// Validates RGBA8 texture dimensions and downsamples oversized images so GPU backends
/// (Metal / WGPU) never abort on `0`-sized or `> max_dim` textures (e.g. stitched long
/// mobile screenshots such as `1080x20000`).
///
/// - Degenerate `width == 0 || height == 0 || max_dim == 0` (or truncated pixel buffers)
///   clamp safely to a `1x1` transparent RGBA pixel.
/// - In-bounds textures borrow `rgba` with zero allocation.
/// - Oversized textures (`width > max_dim || height > max_dim`) are box-downsampled while
///   preserving aspect ratio so both dimensions fit within `1..=max_dim`.
#[cfg_attr(not(any(target_os = "macos", wgpu)), allow(dead_code))]
pub(crate) fn prepare_rgba_texture_data(
    rgba: &[u8],
    width: u32,
    height: u32,
    max_dim: u32,
) -> (u32, u32, std::borrow::Cow<'_, [u8]>) {
    use std::borrow::Cow;

    if width == 0 || height == 0 || max_dim == 0 {
        return (1, 1, Cow::Borrowed(&TRANSPARENT_PIXEL_RGBA));
    }

    let Some(expected_len) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
    else {
        return (1, 1, Cow::Borrowed(&TRANSPARENT_PIXEL_RGBA));
    };
    if rgba.len() < expected_len {
        return (1, 1, Cow::Borrowed(&TRANSPARENT_PIXEL_RGBA));
    }

    if width <= max_dim && height <= max_dim {
        return (width, height, Cow::Borrowed(&rgba[..expected_len]));
    }

    let scale = f64::min(max_dim as f64 / width as f64, max_dim as f64 / height as f64);
    let dst_w = ((width as f64 * scale).round() as u32).clamp(1, max_dim);
    let dst_h = ((height as f64 * scale).round() as u32).clamp(1, max_dim);

    let src_w = width as usize;
    let src_h = height as usize;
    let out_w = dst_w as usize;
    let out_h = dst_h as usize;

    let mut out = vec![0u8; out_w * out_h * 4];
    for dy in 0..out_h {
        let y0 = (dy * src_h) / out_h;
        let y1 = (((dy + 1) * src_h).div_ceil(out_h)).max(y0 + 1).min(src_h);
        for dx in 0..out_w {
            let x0 = (dx * src_w) / out_w;
            let x1 = (((dx + 1) * src_w).div_ceil(out_w)).max(x0 + 1).min(src_w);

            let mut r_acc = 0u64;
            let mut g_acc = 0u64;
            let mut b_acc = 0u64;
            let mut a_acc = 0u64;
            let mut count = 0u64;

            for sy in y0..y1 {
                let row_base = sy * src_w * 4;
                for sx in x0..x1 {
                    let idx = row_base + sx * 4;
                    r_acc += u64::from(rgba[idx]);
                    g_acc += u64::from(rgba[idx + 1]);
                    b_acc += u64::from(rgba[idx + 2]);
                    a_acc += u64::from(rgba[idx + 3]);
                    count += 1;
                }
            }

            let out_idx = (dy * out_w + dx) * 4;
            if count > 0 {
                out[out_idx] = ((r_acc + count / 2) / count) as u8;
                out[out_idx + 1] = ((g_acc + count / 2) / count) as u8;
                out[out_idx + 2] = ((b_acc + count / 2) / count) as u8;
                out[out_idx + 3] = ((a_acc + count / 2) / count) as u8;
            }
        }
    }

    (dst_w, dst_h, Cow::Owned(out))
}

#[cfg(test)]
mod texture_prep_tests {
    use super::prepare_rgba_texture_data;

    #[test]
    fn clamps_zero_dimensions_to_transparent_1x1() {
        let (w, h, bytes) = prepare_rgba_texture_data(&[], 0, 100, 8192);
        assert_eq!((w, h), (1, 1));
        assert_eq!(&*bytes, &[0, 0, 0, 0]);

        let (w, h, bytes) = prepare_rgba_texture_data(&[], 100, 0, 8192);
        assert_eq!((w, h), (1, 1));
        assert_eq!(&*bytes, &[0, 0, 0, 0]);
    }

    #[test]
    fn downsamples_oversized_long_screenshot_within_max_dim() {
        let src_w = 108u32;
        let src_h = 2000u32;
        let src = vec![200u8; (src_w * src_h * 4) as usize];
        let (dst_w, dst_h, bytes) = prepare_rgba_texture_data(&src, src_w, src_h, 512);
        assert!(dst_w >= 1 && dst_w <= 512);
        assert_eq!(dst_h, 512);
        assert_eq!(bytes.len(), (dst_w * dst_h * 4) as usize);
        assert_eq!(&bytes[..4], &[200, 200, 200, 200]);
    }
}

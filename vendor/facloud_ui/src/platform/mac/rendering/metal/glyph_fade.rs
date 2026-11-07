use facloud_ui_core::scene::GlyphFade;

// Match the WGPU glyph shader contract: positions are device pixels, while
// bloom progress and the no-fade/bloom sentinels are independent of DPI.
pub(super) fn glyph_fade_uniforms(fade: Option<GlyphFade>, scale_factor: f32) -> (f32, f32) {
    match fade {
        None => (0.0, -1.0),
        Some(GlyphFade::Horizontal { start, end }) => (start * scale_factor, end * scale_factor),
        Some(GlyphFade::Clip {
            fade_start,
            fade_end,
        }) => (fade_start * scale_factor, fade_end * scale_factor),
        Some(GlyphFade::Bloom { progress }) => (progress.clamp(0.0, 1.0), -2.0),
    }
}

#[cfg(test)]
mod glyph_fade_tests {
    use super::{glyph_fade_uniforms, GlyphFade};

    #[test]
    fn no_fade_stays_visible_on_retina_displays() {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            // Scaling -1 by Retina DPI would select bloom mode at progress 0,
            // making ordinary text fully transparent in the Metal shader.
            assert_eq!(glyph_fade_uniforms(None, scale), (0.0, -1.0));
        }
    }

    #[test]
    fn horizontal_and_clip_fades_use_device_pixel_coordinates() {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            for (start, end) in [(12.0, 24.0), (24.0, 12.0)] {
                let expected = (start * scale, end * scale);
                assert_eq!(
                    glyph_fade_uniforms(Some(GlyphFade::Horizontal { start, end }), scale),
                    expected
                );
                assert_eq!(
                    glyph_fade_uniforms(
                        Some(GlyphFade::Clip {
                            fade_start: start,
                            fade_end: end
                        }),
                        scale
                    ),
                    expected
                );
            }
        }
    }

    #[test]
    fn bloom_progress_and_mode_are_not_scaled_by_dpi() {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            for progress in [0.0, 0.25, 0.5, 1.0] {
                assert_eq!(
                    glyph_fade_uniforms(Some(GlyphFade::Bloom { progress }), scale),
                    (progress, -2.0)
                );
            }
        }
    }

    #[test]
    fn bloom_progress_is_bounded() {
        for (progress, expected) in [(-1.0, 0.0), (2.0, 1.0)] {
            assert_eq!(
                glyph_fade_uniforms(Some(GlyphFade::Bloom { progress }), 2.0),
                (expected, -2.0)
            );
        }
    }
}


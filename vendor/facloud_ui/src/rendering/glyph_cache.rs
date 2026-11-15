use crate::fonts::{canvas, RasterizedGlyph};
use crate::rendering::atlas::{self, AllocatedRegion, TextureId};
use crate::{fonts::SubpixelAlignment, rendering, scene::GlyphKey};
use anyhow::Result;
use ordered_float::OrderedFloat;
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use std::collections::HashMap;

const ATLAS_SIZE: usize = 1024;

/// Callback to create a texture at a given size.
type CreateTextureCallback<'a, T> = dyn Fn(usize) -> T + 'a;

/// Callback to insert [`RasterizedGlyph`] at a region identified by [`AllocatedRegion`] into a
/// texture, `T`.
type InsertIntoTextureCallback<'a, T> = dyn Fn(AllocatedRegion, &RasterizedGlyph, &mut T) + 'a;

/// Callback to compute the bounds of a glyph when rasterized.
pub(crate) type GlyphRasterBoundsFn<'a> =
    dyn Fn(GlyphKey, Vector2F, &rendering::GlyphConfig) -> Result<RectI> + 'a;

/// Callback to rasterize a glyph.
pub(crate) type RasterizeGlyphFn<'a> = dyn Fn(
        GlyphKey,
        Vector2F,
        SubpixelAlignment,
        &rendering::GlyphConfig,
        canvas::RasterFormat,
    ) -> Result<RasterizedGlyph>
    + 'a;

/// A cache that caches glyphs in a texture atlas.  
pub struct GlyphCache<Texture> {
    textures: Vec<Texture>,
    cache: HashMap<GlyphCacheKey, GlyphTextureOffset>,
    glyph_config: rendering::GlyphConfig,
    atlas_manager: atlas::Manager,
}

#[derive(Hash, PartialEq, Eq)]
struct GlyphCacheKey {
    glyph_key: GlyphKey,
    scale_factor: OrderedFloat<f32>,
    subpixel_alignment: SubpixelAlignment,
}

impl GlyphCacheKey {
    fn new(glyph_key: GlyphKey, scale_factor: f32, subpixel_alignment: SubpixelAlignment) -> Self {
        GlyphCacheKey {
            glyph_key,
            scale_factor: scale_factor.into(),
            subpixel_alignment,
        }
    }
}

/// A glyph within a texture atlas.
#[derive(Copy, Debug, Clone)]
pub(crate) struct GlyphTextureOffset {
    pub texture_id: TextureId,
    pub allocated_region: AllocatedRegion,
    pub raster_bounds: RectF,
    pub is_emoji: bool,
}

impl<Texture> GlyphCache<Texture> {
    /// Maximum number of 1024x1024 RGBA8 glyph atlas pages (~96 MiB VRAM) retained across frames
    /// before clearing cold glyph pages at a frame boundary so visible glyphs re-rasterize cleanly.
    pub(crate) const MAX_GLYPH_ATLAS_PAGES: usize = 24;

    pub(crate) fn new(glyph_config: rendering::GlyphConfig) -> Self {
        GlyphCache {
            textures: Vec::new(),
            cache: HashMap::new(),
            glyph_config,
            atlas_manager: atlas::Manager::new(ATLAS_SIZE),
        }
    }

    pub(crate) fn update_config(&mut self, glyph_config: &rendering::GlyphConfig) {
        // If the glyph rendering configuration has changed, blow away the cache
        // and replace ourself with a new one.
        if *glyph_config != self.glyph_config {
            *self = GlyphCache::new(*glyph_config);
        }
    }

    /// Clears cached glyph entries and extra atlas textures at a frame boundary (before any
    /// `TextureId`s are queued for the frame) when the atlas page count exceeds
    /// [`Self::MAX_GLYPH_ATLAS_PAGES`].
    pub(crate) fn trim_if_over_high_water_mark(&mut self) -> bool {
        if self.textures.len() > Self::MAX_GLYPH_ATLAS_PAGES {
            log::info!(
                "GlyphCache exceeded high-water mark ({} > {} atlas pages); resetting cold glyph atlases",
                self.textures.len(),
                Self::MAX_GLYPH_ATLAS_PAGES,
            );
            self.textures.clear();
            self.textures.shrink_to(Self::MAX_GLYPH_ATLAS_PAGES);
            self.cache.clear();
            self.cache.shrink_to(4096);
            self.atlas_manager.reset();
            true
        } else {
            false
        }
    }

    /// Returns the texture identified by [`TextureId`].
    pub(crate) fn texture(&self, texture_id: &TextureId) -> Option<&Texture> {
        self.textures.get(texture_id.as_usize())
    }

    /// Returns a [`GlyphTextureOffset`] identified by [`GlyphKey`]. If the [`GlyphKey`] has not
    /// been previously cached, the glyph is rasterized and inserted into the texture via the
    /// `insert_into_texture` callback. If a new texture needs to be created (since a previous
    /// texture is now fill), the `create_texture` callback is called to construct a new texture
    /// atlas.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn get(
        &mut self,
        glyph_key: GlyphKey,
        scale_factor: f32,
        subpixel_alignment: SubpixelAlignment,
        create_texture: &CreateTextureCallback<'_, Texture>,
        insert_into_texture: &InsertIntoTextureCallback<'_, Texture>,
        raster_bounds_fn: &GlyphRasterBoundsFn<'_>,
        rasterize_glyph_fn: &RasterizeGlyphFn<'_>,
    ) -> Result<Option<GlyphTextureOffset>> {
        let cache_key = GlyphCacheKey::new(glyph_key, scale_factor, subpixel_alignment);

        match self.cache.get(&cache_key) {
            None => {
                let bounds =
                    raster_bounds_fn(glyph_key, Vector2F::splat(scale_factor), &self.glyph_config)?;

                if bounds.size().x() <= 0 || bounds.size().y() <= 0 {
                    return Ok(None);
                }

                let rasterized_glyph = rasterize_glyph_fn(
                    glyph_key,
                    Vector2F::splat(scale_factor),
                    subpixel_alignment,
                    &self.glyph_config,
                    crate::fonts::canvas::RasterFormat::Rgba32,
                )?;

                if rasterized_glyph.canvas.size.x() <= 0 || rasterized_glyph.canvas.size.y() <= 0 {
                    return Ok(None);
                }

                let texture_offset = self.atlas_manager.insert(rasterized_glyph.canvas.size)?;
                let idx = texture_offset.texture_id.as_usize();
                if idx >= self.textures.len() {
                    self.textures
                        .resize_with(idx + 1, || create_texture(ATLAS_SIZE));
                }
                let texture = &mut self.textures[idx];
                insert_into_texture(texture_offset.allocated_region, &rasterized_glyph, texture);

                let glyph_texture_offset = GlyphTextureOffset {
                    texture_id: texture_offset.texture_id,
                    raster_bounds: bounds.to_f32(),
                    is_emoji: rasterized_glyph.is_emoji,
                    allocated_region: texture_offset.allocated_region,
                };

                self.cache.insert(cache_key, glyph_texture_offset);
                Ok(Some(glyph_texture_offset))
            }
            Some(gto) => Ok(Some(*gto)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::{canvas::Canvas, canvas::RasterFormat, FontId};
    use pathfinder_geometry::vector::vec2i;

    #[test]
    fn glyph_cache_trims_when_exceeding_high_water_mark_and_rerasterizes_cleanly() {
        let mut cache: GlyphCache<usize> = GlyphCache::new(rendering::GlyphConfig::default());
        // Each 500x500 glyph takes an entire 1024x1024 row/page after 4 glyphs (2x2 per 1024x1024 page).
        // Inserting (MAX_GLYPH_ATLAS_PAGES * 4) + 1 glyphs allocates MAX_GLYPH_ATLAS_PAGES + 1 pages.
        let total_glyphs = (GlyphCache::<usize>::MAX_GLYPH_ATLAS_PAGES * 4) + 1;
        for id in 0..total_glyphs {
            let key = GlyphKey {
                font_id: FontId(0),
                glyph_id: id as u32,
                font_size: 32.0.into(),
            };
            let gto = cache
                .get(
                    key,
                    1.0,
                    SubpixelAlignment::new(pathfinder_geometry::vector::Vector2F::zero()),
                    &|size| size,
                    &|_region, _glyph, _tex| {},
                    &|_k, _s, _cfg| Ok(RectI::new(vec2i(0, 0), vec2i(500, 500))),
                    &|_k, _s, _align, _cfg, format| {
                        Ok(RasterizedGlyph {
                            canvas: Canvas {
                                pixels: vec![255; 500 * 500 * 4],
                                size: vec2i(500, 500),
                                row_stride: 500 * 4,
                                format,
                            },
                            is_emoji: false,
                        })
                    },
                )
                .unwrap()
                .unwrap();
            if id == total_glyphs - 1 {
                assert_eq!(
                    gto.texture_id.as_usize(),
                    GlyphCache::<usize>::MAX_GLYPH_ATLAS_PAGES
                );
            }
        }

        assert_eq!(
            cache.textures.len(),
            GlyphCache::<usize>::MAX_GLYPH_ATLAS_PAGES + 1
        );
        assert!(cache.trim_if_over_high_water_mark());
        assert_eq!(cache.textures.len(), 0);
        assert!(!cache.trim_if_over_high_water_mark());

        // Visible glyphs on the new frame re-rasterize cleanly into TextureId(0).
        let first_key = GlyphKey {
            font_id: FontId(0),
            glyph_id: 1,
            font_size: 32.0.into(),
        };
        let rerasterized = cache
            .get(
                first_key,
                1.0,
                SubpixelAlignment::new(pathfinder_geometry::vector::Vector2F::zero()),
                &|size| size,
                &|_region, _glyph, _tex| {},
                &|_k, _s, _cfg| Ok(RectI::new(vec2i(0, 0), vec2i(24, 24))),
                &|_k, _s, _align, _cfg, _format| {
                    Ok(RasterizedGlyph {
                        canvas: Canvas {
                            pixels: vec![255; 24 * 24 * 4],
                            size: vec2i(24, 24),
                            row_stride: 24 * 4,
                            format: RasterFormat::Rgba32,
                        },
                        is_emoji: false,
                    })
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(rerasterized.texture_id.as_usize(), 0);
        assert_eq!(cache.textures.len(), 1);
    }
}

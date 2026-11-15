use super::*;

#[test]
fn test_subpixel_alignment_computation() {
    {
        // Default case - a non-fractional offset should have an alignment
        // value of zero.
        let pos = vec2f(0., 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // 0.1 rounds down to 0 (not up to 0.33 -> 1)
        let pos = vec2f(0.1, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // y-position doesn't affect computation
        let pos = vec2f(0.1, 0.33);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // 0.2 rounds up to 0.33 -> 1
        let pos = vec2f(0.2, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 1);
    }
    {
        // 0.66 doesn't round, and converts to 2
        let pos = vec2f(0.66, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 2);
    }
    {
        // 0.9 rounds up to 1.0 -> 0
        let pos = vec2f(0.9, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // Negative coordinates use Euclidean remainder (-0.34 -> 0.66 -> 2, -0.8 -> 0.2 -> 1)
        assert_eq!(SubpixelAlignment::new(vec2f(-0.34, 0.)).0, 2);
        assert_eq!(SubpixelAlignment::new(vec2f(-0.8, 0.)).0, 1);
    }
}

#[test]
fn glyph_for_char_treats_notdef_glyph_as_missing() {
    let cache = Cache::new(Box::new(crate::platform::test::FontDB::new()));

    assert_eq!(cache.glyph_for_char(FontId(0), '中', true), None);
}

#[test]
fn glyph_for_char_uses_fallback_font_for_chinese_character() {
    let cache = Cache::new(Box::new(CjkFallbackFontDB::new()));

    assert_eq!(
        cache.glyph_for_char(FontId(0), '中', true),
        Some((42, FontId(1)))
    );
}

#[test]
fn glyph_for_char_retries_fallback_after_direct_miss() {
    let cache = Cache::new(Box::new(CjkFallbackFontDB::new()));

    assert_eq!(cache.glyph_for_char(FontId(0), '中', false), None);
    assert_eq!(
        cache.glyph_for_char(FontId(0), '中', true),
        Some((42, FontId(1)))
    );
}

struct CjkFallbackFontDB {
    base: crate::platform::test::FontDB,
}

impl CjkFallbackFontDB {
    fn new() -> Self {
        Self {
            base: crate::platform::test::FontDB::new(),
        }
    }
}

impl crate::platform::FontDB for CjkFallbackFontDB {
    fn load_from_bytes(&mut self, name: &str, bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        self.base.load_from_bytes(name, bytes)
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, font_family: &str) -> Result<FamilyId> {
        self.base.load_from_system(font_family)
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn crate::platform::LoadedSystemFonts>> {
        self.base.load_all_system_fonts()
    }

    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        loaded_system_fonts: Box<dyn crate::platform::LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, FontInfo)> {
        self.base.process_loaded_system_fonts(loaded_system_fonts)
    }

    fn family_id_for_name(&self, name: &str) -> Option<FamilyId> {
        self.base.family_id_for_name(name)
    }

    fn load_family_name_from_id(&self, id: FamilyId) -> Option<String> {
        self.base.load_family_name_from_id(id)
    }

    fn select_font(&self, family_id: FamilyId, properties: Properties) -> FontId {
        self.base.select_font(family_id, properties)
    }

    fn fallback_fonts(&self, character: char, font_id: FontId) -> Vec<FontId> {
        match (character, font_id) {
            ('中', FontId(0)) => vec![FontId(1)],
            _ => Vec::new(),
        }
    }

    fn font_metrics(&self, font_id: FontId) -> Metrics {
        self.base.font_metrics(font_id)
    }

    fn glyph_advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Vector2I> {
        self.base.glyph_advance(font_id, glyph_id)
    }

    fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &crate::rendering::GlyphConfig,
    ) -> Result<RectI> {
        self.base
            .glyph_raster_bounds(font_id, size, glyph_id, scale, glyph_config)
    }

    fn glyph_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<RectI> {
        self.base.glyph_typographic_bounds(font_id, glyph_id)
    }

    fn rasterize_glyph(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &crate::rendering::GlyphConfig,
        format: canvas::RasterFormat,
    ) -> Result<RasterizedGlyph> {
        self.base.rasterize_glyph(
            font_id,
            size,
            glyph_id,
            scale,
            subpixel_alignment,
            glyph_config,
            format,
        )
    }

    fn glyph_for_char(&self, font_id: FontId, character: char) -> Option<GlyphId> {
        match (font_id, character) {
            (FontId(0), '中') => Some(0),
            (FontId(1), '中') => Some(42),
            _ => self.base.glyph_for_char(font_id, character),
        }
    }

    fn text_layout_system(&self) -> &dyn crate::platform::TextLayoutSystem {
        self.base.text_layout_system()
    }
}

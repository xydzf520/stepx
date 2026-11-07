pub mod canvas;
mod external_fallback;
mod metrics;
mod text_layout_system;

pub use crate::text_layout::GlyphBloomSchedule;
pub use text_layout_system::TextLayoutSystem;

use std::hash::Hash;

use crate::{platform, rendering, scene::GlyphKey, SingletonEntity};
use anyhow::{Error, Result};
use dashmap::{
    mapref::{entry::Entry, one::Ref},
    DashMap,
};

use enum_iterator::Sequence;
use markdown_parser::weight::CustomWeight;
use ordered_float::OrderedFloat;
use pathfinder_geometry::vector::Vector2I;
use pathfinder_geometry::{
    rect::{RectF, RectI},
    vector::{vec2f, Vector2F},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Sequence, Serialize, Deserialize)]
#[cfg_attr(feature = "schema_gen", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema_gen",
    schemars(
        description = "Font weight for terminal text.",
        rename_all = "snake_case"
    )
)]
#[cfg_attr(feature = "settings_value", derive(settings_value::SettingsValue))]
pub enum Weight {
    Thin,
    ExtraLight,
    Light,
    #[default]
    Normal,
    /// Dark-mode retinal-irradiation-compensated Medium (`460` vs standard `500`).
    /// Prevents light-on-dark strokes from blooming and appearing overly bold.
    MediumOpticalDark,
    Medium,
    /// Dark-mode retinal-irradiation-compensated Semibold (`560` vs standard `600`).
    SemiboldOpticalDark,
    Semibold,
    Bold,
    ExtraBold,
    Black,
}

impl Weight {
    #[allow(non_upper_case_globals)]
    pub const Regular: Self = Self::Normal;

    pub fn is_normal(&self) -> bool {
        matches!(self, Self::Normal)
    }

    pub const fn to_css_weight(self) -> u16 {
        match self {
            Weight::Thin => 100,
            Weight::ExtraLight => 200,
            Weight::Light => 300,
            Weight::Normal => 400,
            Weight::MediumOpticalDark => 460,
            Weight::Medium => 500,
            Weight::SemiboldOpticalDark => 560,
            Weight::Semibold => 600,
            Weight::Bold => 700,
            Weight::ExtraBold => 800,
            Weight::Black => 900,
        }
    }

    /// Returns the dark-mode optical-irradiation-compensated weight equivalent:
    /// `Medium (500) -> MediumOpticalDark (460)`, `Semibold (600) -> SemiboldOpticalDark (560)`.
    pub const fn optical_dark(self) -> Self {
        match self {
            Weight::Medium => Weight::MediumOpticalDark,
            Weight::Semibold => Weight::SemiboldOpticalDark,
            other => other,
        }
    }

    /// Returns `true` if this weight is an optical dark-mode compensated weight
    /// (`MediumOpticalDark` or `SemiboldOpticalDark`).
    pub const fn is_optical_dark_compensated(self) -> bool {
        matches!(
            self,
            Weight::MediumOpticalDark | Weight::SemiboldOpticalDark
        )
    }

    pub fn from_custom_weight(custom_weight: Option<CustomWeight>) -> Weight {
        if let Some(custom_weight) = custom_weight {
            match custom_weight {
                CustomWeight::Thin => Weight::Thin,
                CustomWeight::ExtraLight => Weight::ExtraLight,
                CustomWeight::Light => Weight::Light,
                CustomWeight::Medium => Weight::Medium,
                CustomWeight::Semibold => Weight::Semibold,
                CustomWeight::Bold => Weight::Bold,
                CustomWeight::ExtraBold => Weight::ExtraBold,
                CustomWeight::Black => Weight::Black,
            }
        } else {
            Weight::Normal
        }
    }

    pub fn to_custom_weight(&self) -> Option<CustomWeight> {
        match self {
            Weight::Thin => Some(CustomWeight::Thin),
            Weight::ExtraLight => Some(CustomWeight::ExtraLight),
            Weight::Light => Some(CustomWeight::Light),
            Weight::Normal => None,
            Weight::MediumOpticalDark | Weight::Medium => Some(CustomWeight::Medium),
            Weight::SemiboldOpticalDark | Weight::Semibold => Some(CustomWeight::Semibold),
            Weight::Bold => Some(CustomWeight::Bold),
            Weight::ExtraBold => Some(CustomWeight::ExtraBold),
            Weight::Black => Some(CustomWeight::Black),
        }
    }

    pub fn matches_custom_weight(&self, custom_weight: Option<CustomWeight>) -> bool {
        match custom_weight {
            Some(custom_weight) => custom_weight.is_weight(*self),
            None => self.is_normal(),
        }
    }
}

impl std::fmt::Display for Weight {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Weight::Thin => write!(f, "Thin"),
            Weight::ExtraLight => write!(f, "ExtraLight"),
            Weight::Light => write!(f, "Light"),
            Weight::Normal => write!(f, "Normal"),
            Weight::MediumOpticalDark => write!(f, "MediumOpticalDark"),
            Weight::Medium => write!(f, "Medium"),
            Weight::SemiboldOpticalDark => write!(f, "SemiboldOpticalDark"),
            Weight::Semibold => write!(f, "Semibold"),
            Weight::Bold => write!(f, "Bold"),
            Weight::ExtraBold => write!(f, "ExtraBold"),
            Weight::Black => write!(f, "Black"),
        }
    }
}

pub trait CustomWeightConversion {
    fn is_weight(&self, weight: Weight) -> bool;
}

impl CustomWeightConversion for CustomWeight {
    fn is_weight(&self, weight: Weight) -> bool {
        matches!(
            (self, weight),
            (CustomWeight::Thin, Weight::Thin)
                | (CustomWeight::ExtraLight, Weight::ExtraLight)
                | (CustomWeight::Light, Weight::Light)
                | (
                    CustomWeight::Medium,
                    Weight::Medium | Weight::MediumOpticalDark
                )
                | (
                    CustomWeight::Semibold,
                    Weight::Semibold | Weight::SemiboldOpticalDark
                )
                | (CustomWeight::Bold, Weight::Bold)
                | (CustomWeight::ExtraBold, Weight::ExtraBold)
                | (CustomWeight::Black, Weight::Black)
        )
    }
}

#[cfg(not(target_family = "wasm"))]
use {futures_util::future::BoxFuture, futures_util::FutureExt};

pub(crate) use external_fallback::{FontBytes, RequestedFallbackFontSource};

pub use external_fallback::{ExternalFontFamily, FallbackFontEvent, FallbackFontModel};
pub use metrics::Metrics;

pub type GlyphId = u32;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct FamilyId(pub usize);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct FontId(pub usize);

type FontFamilyName = &'static str;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Properties {
    pub style: Style,
    pub weight: Weight,
}

pub struct RasterizedGlyph {
    pub canvas: canvas::Canvas,
    pub is_emoji: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Style {
    #[default]
    Normal,
    Italic,
}

/// A structure to represent the subpixel alignment of a given glyph.
///
/// The reason this only stores a single value - the _horizontal_ subpixel
/// alignment - is that we vertically snap glyphs to pixel boundaries.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct SubpixelAlignment(u8);

impl SubpixelAlignment {
    /// The number of subdivisions to slice a pixel into.
    const STEPS: u8 = 3;

    /// Quantizes the horizontal component of the provided position to the
    /// nearest subpixel position, where `Self::STEPS` is the number of possible
    /// subpixel positions.
    ///
    /// This is semantically a modulus operation - a value such as 0.95 will be
    /// rounded up-and-around to 0.0.
    pub fn new(glyph_position: Vector2F) -> Self {
        // Scale the floating point range [0, 1) to [0, Self::STEPS).
        let scaled_pos = glyph_position.x().fract() * Self::STEPS as f32;
        // Round the scaled value to the nearest integer in range 0..Self::STEPS
        // and convert to an integer.  We use modulus to make sure we don't
        // exceed the range.
        let alignment = scaled_pos.round() as u8 % Self::STEPS;
        Self(alignment)
    }

    /// Converts a `SubpixelAlignment` to a vector representing the horizontal
    /// offset the given alignment within its pixel.
    pub fn to_offset(&self) -> Vector2F {
        vec2f(self.0 as f32 / Self::STEPS as f32, 0.)
    }
}

#[derive(Debug, Clone)]
pub struct FontInfo {
    /// The family name of the font, which is displayed to users.
    pub family_name: String,
    /// A list of all Apple font names for fonts in this family.
    #[cfg(target_os = "macos")]
    pub font_names: Vec<String>,
    pub is_monospace: bool,
}

type RasterBoundsKey = (GlyphKey, (OrderedFloat<f32>, OrderedFloat<f32>));

pub struct Cache {
    selections: DashMap<(FamilyId, Properties), FontId>,
    /// Note that the properties stored in this map might not exactly match the
    /// font. The font represents a "best match" in the font family given these
    /// properties.
    font_properties: DashMap<FontId, Properties>,
    platform: Box<dyn platform::FontDB>,
    font_metrics: DashMap<FontId, Metrics>,
    glyphs_by_char: DashMap<(FontId, char), Option<(GlyphId, FontId)>>, // Also caching font id here for possible fallback fonts.
    glyph_advances: DashMap<(FontId, GlyphId), Result<Vector2I, Error>>,
    glyph_typographic_bounds: DashMap<(FontId, GlyphId), Result<RectI, Error>>,
    raster_bounds: DashMap<RasterBoundsKey, Result<RectI, Error>>,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    available_system_fonts: Option<Vec<(Option<FamilyId>, FontInfo)>>,
    font_fallback_cache: FontFallbackCache,
}

#[derive(Default)]
struct FontFallbackCache {
    loaded_fallback_families: DashMap<FontFamilyName, FamilyId>,
    requested_fallback_families: DashMap<ExternalFontFamily, Vec<RequestedFallbackFontSource>>,
    fallback_font_fn: Option<Box<dyn Fn(char) -> Option<ExternalFontFamily> + Send + Sync>>,
}

impl Properties {
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Compensates font weight for dark-mode retinal irradiation (`500 -> 460`, `600 -> 560`)
    /// when `is_dark` is `true`.
    pub fn with_optical_dark_weight(mut self, is_dark: bool) -> Self {
        if is_dark {
            self.weight = self.weight.optical_dark();
        }
        self
    }
}

impl Cache {
    pub fn new(font_db: Box<dyn platform::FontDB>) -> Self {
        Self {
            platform: font_db,
            selections: Default::default(),
            font_properties: Default::default(),
            font_metrics: Default::default(),
            glyphs_by_char: Default::default(),
            glyph_advances: Default::default(),
            glyph_typographic_bounds: Default::default(),
            raster_bounds: Default::default(),
            available_system_fonts: Default::default(),
            font_fallback_cache: Default::default(),
        }
    }

    /// Returns the [`TextLayoutSystem`], which can be used to layout text either on the main thread
    /// or in the background.
    pub fn text_layout_system(&self) -> TextLayoutSystem<'_> {
        TextLayoutSystem {
            platform: self.font_db().text_layout_system(),
            cache: &self.font_fallback_cache,
        }
    }

    // TODO(alokedesai): Better consolidate the caching logic between the FontCache and the
    // TextLayoutCache so we don't need to leak the platform-specific implementation of the
    // FontDB outside of this struct.
    pub(super) fn font_db(&self) -> &dyn platform::FontDB {
        self.platform.as_ref()
    }

    /// Returns all of the system fonts on the user's current machine. If already cached, the
    /// current set of system fonts are immediately returned. If not cached, a future is returned
    /// that returns all of the system fonts when awaited.
    /// NOTE it is up to the caller to cache the result of the future via a call to
    /// [`Self::set_system_fonts`].
    #[cfg(not(target_family = "wasm"))]
    pub fn all_system_fonts(
        &self,
        ctx: &mut crate::ModelContext<Self>,
    ) -> BoxFuture<'static, Vec<(Option<FamilyId>, FontInfo)>> {
        if let Some(fonts) = self.available_system_fonts.as_ref() {
            futures::future::ready(fonts.clone()).boxed()
        } else {
            log::info!("Computing available system fonts");
            let (tx, rx) = futures::channel::oneshot::channel();
            ctx.spawn(
                self.platform.load_all_system_fonts(),
                |me, loaded_system_fonts, _ctx| {
                    let system_fonts = me.platform.process_loaded_system_fonts(loaded_system_fonts);
                    me.available_system_fonts = Some(system_fonts.clone());
                    let _ = tx.send(system_fonts);
                },
            );
            rx.map(Result::unwrap_or_default).boxed()
        }
    }

    pub fn load_family_name_from_id(&self, id: FamilyId) -> Option<String> {
        self.platform.load_family_name_from_id(id)
    }

    pub fn load_family_from_bytes(&mut self, name: &str, bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        self.platform.load_from_bytes(name, bytes)
    }

    #[cfg(not(target_family = "wasm"))]
    /// Returns the family ID for a given font, loading it into memory if it's
    /// not already known to the cache.
    pub fn get_or_load_system_font(&mut self, font_family: &str) -> Result<FamilyId> {
        match self.family_id_for_name(font_family) {
            Some(id) => {
                if let Some(available_system_fonts) = self.available_system_fonts.as_mut() {
                    if let Some(entry) =
                        available_system_fonts.iter_mut().find(|(family_id, data)| {
                            data.family_name == font_family && family_id.is_none()
                        })
                    {
                        entry.0 = Some(id);
                    }
                }
                Ok(id)
            }
            None => self.load_system_font(font_family),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn load_system_font(&mut self, font_family: &str) -> Result<FamilyId> {
        self.platform.load_from_system(font_family)
    }

    pub fn select_font(&self, family: FamilyId, properties: Properties) -> FontId {
        match self.selections.entry((family, properties)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let font = self.platform.select_font(family, properties);
                self.font_properties.insert(font, properties);
                *entry.insert(font)
            }
        }
    }

    /// Returns the requested [`Properties`] (style and weight) associated with `font`, if known.
    pub fn font_properties(&self, font: FontId) -> Option<Properties> {
        self.font_properties.get(&font).map(|entry| *entry)
    }

    pub fn line_height(&self, font_size: f32, line_height_ratio: f32) -> f32 {
        line_height_ratio * font_size
    }

    pub fn ascent(&self, font: FontId, point_size: f32) -> f32 {
        let metrics = self.metrics(font);
        metrics.ascent as f32 * metrics.font_scale(point_size)
    }

    pub fn descent(&self, font: FontId, point_size: f32) -> f32 {
        let metrics = self.metrics(font);
        metrics.descent as f32 * metrics.font_scale(point_size)
    }

    /// Returns the "leading" - the gap between two lines - for this font at the
    /// given size.
    pub fn leading(&self, font: FontId, point_size: f32) -> f32 {
        let metrics = self.metrics(font);
        metrics.line_gap as f32 * metrics.font_scale(point_size)
    }

    pub fn glyph_advance(&self, font: FontId, point_size: f32, glyph: GlyphId) -> Result<Vector2F> {
        let advance = match self.glyph_advances.entry((font, glyph)) {
            Entry::Occupied(entry) => entry.into_ref(),
            Entry::Vacant(entry) => entry.insert(self.platform.glyph_advance(font, glyph)),
        };
        match advance.value() {
            Ok(advance) => Ok(advance.to_f32() * self.metrics(font).font_scale(point_size)),
            Err(error) => Err(Error::msg(error.to_string())),
        }
    }

    pub fn glyph_typographic_bounds(
        &self,
        font: FontId,
        point_size: f32,
        glyph: GlyphId,
    ) -> Result<RectF> {
        let bounds = match self.glyph_typographic_bounds.entry((font, glyph)) {
            Entry::Occupied(entry) => entry.into_ref(),
            Entry::Vacant(entry) => {
                entry.insert(self.platform.glyph_typographic_bounds(font, glyph))
            }
        };
        match bounds.value() {
            Ok(bounds) => Ok(bounds.to_f32() * self.metrics(font).font_scale(point_size)),
            Err(error) => Err(Error::msg(error.to_string())),
        }
    }

    pub fn glyph_raster_bounds(
        &self,
        glyph_key: GlyphKey,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> Result<RectI> {
        let entry = self
            .raster_bounds
            .entry((glyph_key, (scale.x().into(), scale.y().into())));
        let bounds = match entry {
            Entry::Occupied(entry) => entry.into_ref(),
            Entry::Vacant(entry) => entry.insert(self.platform.glyph_raster_bounds(
                glyph_key.font_id,
                glyph_key.font_size.into(),
                glyph_key.glyph_id,
                scale,
                glyph_config,
            )),
        };
        match bounds.value() {
            Ok(bounds) => Ok(*bounds),
            Err(error) => Err(Error::msg(error.to_string())),
        }
    }

    pub fn rasterized_glyph(
        &self,
        glyph_key: GlyphKey,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: canvas::RasterFormat,
    ) -> Result<RasterizedGlyph> {
        self.platform.rasterize_glyph(
            glyph_key.font_id,
            glyph_key.font_size.into(),
            glyph_key.glyph_id,
            scale,
            subpixel_alignment,
            glyph_config,
            format,
        )
    }

    /// Checks for a matching glyph in the system fallback fonts.
    fn system_font_fallback(&self, ch: char, font: FontId) -> Option<(GlyphId, FontId)> {
        self.platform
            .fallback_fonts(ch, font)
            .into_iter()
            .find_map(|font| self.glyph_for_char(font, ch, false))
    }

    // Returns the `GlyphId` for a given a character and font. Optionally returns
    // the font ID of the font the character would be rendered with, which could be
    // a fallback font if the font does not contain the glyph
    pub fn glyph_for_char(
        &self,
        font: FontId,
        char: char,
        include_fallback_fonts: bool,
    ) -> Option<(GlyphId, FontId)> {
        if let Some(glyph_and_font) = self.glyphs_by_char.get(&(font, char)) {
            return *glyph_and_font;
        }

        if let Some(glyph_id) = self
            .platform
            .glyph_for_char(font, char)
            .filter(|glyph_id| *glyph_id != 0)
        {
            let glyph_and_font = Some((glyph_id, font));
            self.glyphs_by_char.insert((font, char), glyph_and_font);
            return glyph_and_font;
        }

        if include_fallback_fonts {
            self.font_fallback_cache.request_fallback_font_for_char(
                char,
                RequestedFallbackFontSource::GlyphForChar((font, char)),
            );

            let fallback_glyph_and_font = self
                .app_font_fallback(char, font)
                .or(self.system_font_fallback(char, font));

            if fallback_glyph_and_font.is_some() {
                self.glyphs_by_char
                    .insert((font, char), fallback_glyph_and_font);
            }
            return fallback_glyph_and_font;
        }

        None
    }

    fn metrics(&self, font: FontId) -> Ref<'_, FontId, Metrics> {
        match self.font_metrics.entry(font) {
            Entry::Occupied(entry) => entry.into_ref().downgrade(),
            Entry::Vacant(entry) => entry.insert(self.platform.font_metrics(font)).downgrade(),
        }
    }

    pub fn family_id_for_name(&self, name: &str) -> Option<FamilyId> {
        self.platform.family_id_for_name(name)
    }

    pub fn em_width(&self, font_family: FamilyId, font_size: f32) -> f32 {
        let font_id = self.select_font(font_family, Default::default());
        let Some((glyph_id, glyph_font_id)) = self
            .glyph_for_char(font_id, 'm', false)
            .or_else(|| self.glyph_for_char(font_id, 'm', true))
        else {
            log::warn!("could not find an 'm' glyph when measuring em width; using font size");
            return font_size;
        };
        match self.glyph_typographic_bounds(glyph_font_id, font_size, glyph_id) {
            Ok(bounds) => bounds.width(),
            Err(error) => {
                log::warn!("could not measure 'm' glyph bounds for em width: {error}");
                font_size
            }
        }
    }

    pub(crate) fn remove_glyphs_by_char_entry(&mut self, key: (FontId, char)) {
        self.glyphs_by_char.remove(&key);
    }
}

impl crate::Entity for Cache {
    type Event = ();
}

impl SingletonEntity for Cache {}

trait MetricsExt {
    fn font_scale(&self, point_size: f32) -> f32;
}

impl MetricsExt for Metrics {
    fn font_scale(&self, point_size: f32) -> f32 {
        point_size / self.units_per_em as f32
    }
}

#[cfg(test)]
#[path = "fonts_test.rs"]
mod tests;

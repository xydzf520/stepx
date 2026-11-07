mod font_handle;
mod str_index_map;
#[cfg(not(feature = "fontkit-rasterizer"))]
mod swash_rasterizer;
mod text_layout;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;
use facloud_ui_core::fonts::{Style, Weight};
#[cfg(target_os = "windows")]
use windows::loader;

use std::any::Any;
use std::collections::HashMap;
use std::ops::{DerefMut, Range};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use bimap::BiMap;
use pathfinder_geometry::{
    rect::{RectF, RectI},
    vector::Vector2F,
};
use resvg::usvg::fontdb;
use resvg::usvg::fontdb::Query;
use vec1::Vec1;

use cosmic_text::{
    Align, Attrs, AttrsList, BidiParagraphs, LayoutGlyph, LayoutLine, ShapeLine, Shaping, Wrap,
};
use dashmap::{mapref::entry::Entry, DashMap};
use fontdb::Source;
use itertools::Itertools;
use parking_lot::RwLock;
use pathfinder_geometry::vector::{vec2f, vec2i, Vector2I};

use self::font_handle::{FontData, FontHandle};
use self::str_index_map::StrIndexMap;
use self::text_layout::{RunBuilder, TextStylesMap};
use crate::fonts::Metrics;
use crate::platform::{self};
use crate::text_layout::{CaretPosition, TextAlignment};
use crate::{
    fonts::{
        canvas::RasterFormat, FamilyId, FontId, GlyphId, Properties, RasterizedGlyph,
        SubpixelAlignment,
    },
    platform::LineStyle,
    rendering::GlyphConfig,
    text_layout::{ClipConfig, Line, StyleAndFont, TextFrame},
};

struct FontFamily {
    name: String,
    fonts: Vec<FontHandle>,
}

#[cfg(target_os = "linux")]
mod loader {
    use super::*;
    use crate::windowing::winit::fonts::linux::{Error, FontconfigLoader};
    use anyhow::Result;

    pub fn load_all_system_fonts() -> LoadedSystemFonts {
        let manager = match FontconfigLoader::new() {
            Ok(x) => x,
            Err(e) => {
                log::error!("Failed to load system fonts: {e:?}");
                return LoadedSystemFonts(vec![]);
            }
        };
        let handles = match manager.get_all_families() {
            Ok(x) => x,
            Err(e) => {
                log::error!("Failed to load system fonts: {e:?}");
                return LoadedSystemFonts(vec![]);
            }
        };

        let mut families = vec![];
        for handle in handles.into_iter() {
            let name = handle.name().to_string();

            // Note: this call will do a validation check when we load
            match handle.into_info_and_family() {
                Ok((info, family)) => families.push((info, family)),
                Err(e) => {
                    // Making sure this is just a debug message, since this is not necessarily
                    // a bug if this fails to load (ex: could be an unsupported font format)
                    log::debug!("Failed to load system fonts for family {name}: {e:?}")
                }
            };
        }
        log::info!("Loaded {} font families", families.len());
        LoadedSystemFonts(families)
    }

    pub fn load_system_font(font_family: &str) -> Result<FontFamily> {
        let manager = FontconfigLoader::new()?;
        let handle = manager.get_family(font_family)?;
        Ok(handle.into_family()?)
    }

    pub fn fallback_fonts(
        family_name: &str,
        properties: Properties,
    ) -> Result<Vec<FontHandle>, Error> {
        let loader = FontconfigLoader::new()?;
        loader.fallback_fonts(family_name, properties)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod loader {
    use super::*;
    #[cfg(not(target_family = "wasm"))]
    use crate::fonts::FontInfo;

    #[cfg(not(target_family = "wasm"))]
    pub fn load_system_font(font_family: &str) -> Result<FontFamily> {
        log::warn!("have not yet implemented loading system fonts: {font_family}");
        anyhow::bail!("loading system fonts not implemented on this platform")
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn load_all_system_fonts() -> Vec<(FontInfo, FontFamily)> {
        vec![]
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn fallback_fonts(family_name: &str, _properties: Properties) -> Result<Vec<FontHandle>> {
        log::warn!("have not yet implemented fallback fonts: {family_name}");
        Ok(vec![])
    }

    #[cfg(target_family = "wasm")]
    pub fn fallback_fonts(_family_name: &str, _properties: Properties) -> Result<Vec<FontHandle>> {
        Ok(vec![])
    }

    // On WASM, font fallback is intentionally empty. Browser fonts are
    // handled by the hosting page, not the Rust binary.
}

// We use font-kit's family handle to load fonts that come with Warp as
// these binaries are already in memory and won't increase our memory load.
fn load_font_family_from_bytes(name: &str, font_bytes: Vec<Vec<u8>>) -> Result<FontFamily> {
    use owned_ttf_parser::OwnedFace;

    let fonts = font_bytes
        .into_iter()
        .map(|font| {
            // We use index 0 here since the each set of bytes are assumed to be a single font
            // face.
            let face = OwnedFace::from_vec(font, 0)?;
            let handle = FontHandle::from(face);
            Ok(handle)
        })
        .collect::<Result<_>>()?;

    Ok(FontFamily {
        fonts,
        name: name.to_string(),
    })
}

/// Enum indicating whether font validation should enforce that the font supports the english language.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[derive(Copy, Clone)]
enum ValidateFontSupportsEn {
    Yes,
    No,
}

struct Family {
    name: String,
    font_ids: Vec1<FontId>,
}

fn next_family_id() -> FamilyId {
    static FAMILY_ID: AtomicUsize = AtomicUsize::new(0);
    let next = FAMILY_ID.fetch_add(1, Ordering::Relaxed);
    FamilyId(next)
}

fn next_font_id() -> FontId {
    static FONT_ID: AtomicUsize = AtomicUsize::new(0);
    let next = FONT_ID.fetch_add(1, Ordering::Relaxed);
    FontId(next)
}

/// Identifier of a system font we've loaded into the database.
#[derive(PartialEq, Eq, Hash, Debug, Clone)]
struct FontKey {
    /// The path to the font on the user's filesystem.
    path: PathBuf,
    /// The index within the font file that was loaded.
    index: u32,
}

pub struct TextLayoutSystem {
    families: HashMap<FamilyId, Family>,
    /// The internal font database that stores all of our loaded fonts. Since internally,
    /// `cosmic_text` caches font selection, all of its functions to layout text are `&mut`.
    /// However, the `FontDB` trait for text layout is _immutable_ and cannot be easily changed to
    /// mutable. Therefore, we use an [`RwLock`] for internal mutability. `FontDB` is Sync and Send,
    /// so we can't use a `RefCell`.
    font_store: RwLock<cosmic_text::FontSystem>,
    font_id_map: RwLock<BiMap<FontId, fontdb::ID>>,
    font_selections: DashMap<(FamilyId, Properties), FontId>,
    loaded_fonts: DashMap<FontKey, FontId>,
    #[cfg(feature = "fontkit-rasterizer")]
    /// The set of loaded fonts since the last time we tried to rasterize. There's an unfortunate
    /// dependency where we need to load fonts within the TextLayoutSystem since we load fallback
    /// fonts while doing text layout. This means we need cache the fonts we loaded so we can read
    /// it out at raster time and load any necessary fonts.
    loaded_font_ids_since_last_raster: RwLock<Vec<FontId>>,
    #[cfg(not(any(target_os = "windows", target_family = "wasm")))]
    fallback_fonts: DashMap<FontId, Vec<FontId>>,
}

pub struct FontDB {
    text_layout_system: TextLayoutSystem,
    #[cfg(feature = "fontkit-rasterizer")]
    font_kit_rasterizer: crate::fonts::font_kit::Rasterizer,
    #[cfg(not(feature = "fontkit-rasterizer"))]
    swash_cache: RwLock<cosmic_text::SwashCache>,
}

impl FontDB {
    pub fn new() -> Self {
        Self {
            text_layout_system: TextLayoutSystem::new(),
            #[cfg(feature = "fontkit-rasterizer")]
            font_kit_rasterizer: crate::fonts::font_kit::Rasterizer::new(),
            #[cfg(not(feature = "fontkit-rasterizer"))]
            swash_cache: RwLock::new(cosmic_text::SwashCache::new()),
        }
    }

    /// Inserts the given font family into the DB, returning a [`FamilyId`]
    /// for the inserted family.
    fn insert_font_family(&mut self, font_family: FontFamily) -> Result<FamilyId> {
        let mut font_ids = Vec::with_capacity(font_family.fonts.len());
        for font in font_family.fonts {
            let font_id = self.text_layout_system.insert_font(font)?;

            #[cfg(feature = "fontkit-rasterizer")]
            self.load_font_kit_font(font_id)?;

            font_ids.push(font_id);
        }

        let font_ids = Vec1::try_from_vec(font_ids)?;

        let family_id = next_family_id();
        self.text_layout_system.families.insert(
            family_id,
            Family {
                name: font_family.name,
                font_ids,
            },
        );
        Ok(family_id)
    }

    #[cfg(feature = "fontkit-rasterizer")]
    fn load_font_kit_font(&self, font_id: FontId) -> Result<()> {
        let font_kit_font = self
            .text_layout_system
            .try_read_face_source(font_id, |source, index| {
                match source {
                    Source::Binary(bytes) => {
                        // There's no easy way to convert from an `Arc<dyn AsRef<[u8]>` to a
                        // `Vec<u8>`, which means we need to clone here. This is unfortunate
                        // since internally the font source is actually represented as a `Vec`,
                        // but there's no way of downcasting back to the concrete type.
                        // TODO(alokedesai): Make refactors to fontdb and/or font-kit to make
                        // this conversion more performant.
                        font_kit::loader::Loader::from_bytes(
                            Arc::new(bytes.as_ref().as_ref().to_vec()),
                            index,
                        )
                    }
                    Source::File(path) => font_kit::loader::Loader::from_path(path, index),
                    Source::SharedFile(path, _) => font_kit::loader::Loader::from_path(path, index),
                }
            })
            .ok_or_else(|| anyhow!("Unable to find font with given font ID"))?;

        // We have to use an `Arc` with a non send / sync type since
        // the font kit rasterizer is actually send + sync on Mac, which results
        // in it being wrapped in an Arc.
        #[allow(clippy::arc_with_non_send_sync)]
        self.font_kit_rasterizer
            .insert(font_id, Arc::new(font_kit_font?));
        Ok(())
    }
}

impl Default for TextLayoutSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextLayoutSystem {
    pub fn new() -> Self {
        Self {
            families: Default::default(),
            font_store: RwLock::new(cosmic_text::FontSystem::new_with_locale_and_db(
                // Locale is needed for font fallback. For now, we hardcode this to "zh-CN" to
                // prioritize Chinese font fallback.
                "zh-CN".into(),
                Default::default(),
            )),
            font_id_map: Default::default(),
            font_selections: Default::default(),
            loaded_fonts: Default::default(),
            #[cfg(not(any(target_os = "windows", target_family = "wasm")))]
            fallback_fonts: Default::default(),
            #[cfg(feature = "fontkit-rasterizer")]
            loaded_font_ids_since_last_raster: Default::default(),
        }
    }

    /// Helper function to read a Font Face ([`owned_ttf_parser::Face`]) identified by a [`FontId`].
    ///
    /// ## Panics
    /// This function can panic under a few conditions:
    /// 1) If there is no font identified by [`FontId`] stored in the [`TextLayoutSystem`].
    /// 2) The underlying [`fontdb::Database`] does not hold a font identified by [`ID`]. We only
    ///    store an [`ID`] in the `font_id_map` upon inserting it in the [`fontdb::Database`].
    /// 3) The font can't be parsed by [`ttf_parser`]. We validate fonts can be parsed by
    ///    `owned_ttf_parser` when we initially load a font.
    fn read_font_face<T, F: FnOnce(owned_ttf_parser::Face) -> T>(
        &self,
        font_id: FontId,
        ttf_font_callback: F,
    ) -> T {
        self.read_font_face_data(font_id, |data, index| {
            // SAFETY: We've already validated that we can parse the font when loading it.
            let font_face =
                owned_ttf_parser::Face::parse(data, index).expect("Must be able to parse font");
            ttf_font_callback(font_face)
        })
    }

    /// Safe version of [`Self::read_font_face`]. Instead of panicking, will return None
    fn try_read_font_face<T, F: FnOnce(owned_ttf_parser::Face) -> T>(
        &self,
        font_id: FontId,
        ttf_font_callback: F,
    ) -> Option<T> {
        let result = self.try_read_font_face_data(font_id, |data, index| {
            let Ok(font_face) = owned_ttf_parser::Face::parse(data, index) else {
                return None;
            };
            Some(ttf_font_callback(font_face))
        });
        match result {
            // Flatten out the Option<Option<T>> into an Option<T>,
            // to keep the API in line with try_read_font_face_data
            None => None,
            Some(None) => None,
            Some(Some(x)) => Some(x),
        }
    }

    /// Helper function to read the font data of a font identified by a [`FontId`].
    ///
    /// ## Panics
    /// This function can panic under a few conditions:
    /// 1) If there is no font identified by [`FontId`] stored in the [`TextLayoutSystem`].
    /// 2) The underlying [`fontdb::Database`] does not hold a font identified by [`ID`]. We only
    ///    store an [`ID`] in the `font_id_map` upon inserting it in the [`fontdb::Database`].
    fn read_font_face_data<T, F: FnOnce(&[u8], u32) -> T>(
        &self,
        font_id: FontId,
        font_data_callback: F,
    ) -> T {
        match self.try_read_font_face_data(font_id, font_data_callback) {
            Some(x) => x,
            None => {
                match self.font_id_map.read().get_by_left(&font_id) {
                    Some(internal_font_id) => {
                        let internal_font_id = *internal_font_id;
                        let panic_font_data =
                            self.read_font_face_panic_data(font_id, Some(internal_font_id));
                        let panic_message = match self.font_store.read().db().face_source(internal_font_id)
                    {
                        None => {
                            format!("does not have an internal font source for id {internal_font_id}")
                        }
                        Some((source, idx)) => format!(
                            "was unable to load font source ({source:?}, {idx}) for id {internal_font_id}"
                        )
                    };
                        log::warn!(
                            "Tried to load font data {panic_font_data}, but {panic_message}"
                        );
                    }
                    None => {
                        let panic_font_data = self.read_font_face_panic_data(font_id, None);
                        log::warn!("Tried to load font data {panic_font_data}, but no corresponding internal id exists");
                    }
                };
                panic!("Tried to load font data. No font source");
            }
        }
    }

    /// Safe version of [`Self::read_font_face_data`]. Instead of panicking, will return None
    fn try_read_font_face_data<T, F: FnOnce(&[u8], u32) -> T>(
        &self,
        font_id: FontId,
        font_data_callback: F,
    ) -> Option<T> {
        let internal_font_id = *self.font_id_map.read().get_by_left(&font_id)?;
        self.font_store
            .read()
            .db()
            .with_face_data(internal_font_id, |font_data, index| {
                font_data_callback(font_data, index)
            })
    }

    /// Returns the [`Source`] and corresponding font index of a font identified by
    /// [`FontId`].
    #[allow(dead_code)]
    fn try_read_face_source<T, F: FnOnce(Source, u32) -> T>(
        &self,
        font_id: FontId,
        face_source_callback: F,
    ) -> Option<T> {
        let internal_font_id = *self.font_id_map.read().get_by_left(&font_id)?;
        let font_store = self.font_store.read();

        let (source, index) = font_store.db().face_source(internal_font_id)?;
        Some(face_source_callback(source, index))
    }

    fn read_font_face_panic_data(
        &self,
        font_id: FontId,
        internal_id: Option<fontdb::ID>,
    ) -> String {
        let internal_id = internal_id
            .map(|id| format!("{id}"))
            .unwrap_or("None".to_string());
        let family_name = self
            .families
            .values()
            .find(|family| family.font_ids.contains(&font_id))
            .map(|f| f.name.as_str())
            .unwrap_or("Unavailable");
        let font_loaded = self.loaded_fonts.iter().any(|z| z.value() == &font_id);
        format!(
            "Font(id={font_id:?}, internal={internal_id}, family={family_name}, loaded={font_loaded}"
        )
    }

    #[cfg(feature = "fontkit-rasterizer")]
    fn take_fonts_loaded_since_last_raster(&self) -> Vec<FontId> {
        let mut new_vec = vec![];
        let mut loaded_fonts = self.loaded_font_ids_since_last_raster.write();

        std::mem::swap(loaded_fonts.as_mut(), &mut new_vec);
        new_vec
    }

    /// Inserts the font identified by the given [`FontHandle`] into the DB, returning the [`FontId`] identified by that
    /// font.
    /// If the font has already been loaded, the same [`FontId`] from the initial time the font was loaded is returned.
    fn insert_font(&self, font_handle: FontHandle) -> Result<FontId> {
        let (source, index) = match font_handle.into_data() {
            FontData::Bytes(face) => (fontdb::Source::Binary(Arc::new(face.into_vec())), 0),
            FontData::Path { path, index, .. } => (fontdb::Source::File(path.clone()), index),
        };

        if let Source::File(path) = &source {
            // The font we're trying to load has already been loaded into the DB.
            if let Some(id) = self.loaded_fonts.get(&FontKey {
                path: path.clone(),
                index,
            }) {
                return Ok(*id);
            }
        }

        // TODO(alokedesai): Consider using FontDB's `make_shared_font_data` here. FontDB creates a temporary memory
        // mapped file every time data is read via a call to `with_face_data`. When rendering text this can
        // repeatedly cause allocation of a large amount of virtual address space, especially when trying to load
        // large fonts. See https://github.com/RazrFalcon/fontdb/issues/18 for more details.
        let fontdb_ids = self.font_store.write().db_mut().load_font_source(source);

        if fontdb_ids.is_empty() {
            bail!("Loaded a font source that corresponds to 0 font faces")
        }

        let mut font_id_to_return = None;

        for id in fontdb_ids {
            let font_store = self.font_store.read();
            let Some(face_info) = font_store.db().face(id) else {
                continue;
            };

            // Update our FontID map for every font that was loaded into the internal fontDB. cosmic_text may return a
            // `fontdb::ID` for a glyph based on the state of the fontDB, so we need to make sure we have a matching
            // FontID or else we will panic.
            let font_id = next_font_id();
            self.font_id_map.write().insert(font_id, id);
            #[cfg(feature = "fontkit-rasterizer")]
            self.loaded_font_ids_since_last_raster.write().push(font_id);

            if let Source::File(path) = &face_info.source {
                self.loaded_fonts.insert(
                    FontKey {
                        path: path.clone(),
                        index: face_info.index,
                    },
                    font_id,
                );
            }

            if face_info.index == index {
                font_id_to_return = Some(font_id);
            }
        }

        font_id_to_return
            .ok_or_else(|| anyhow!("Requested index for the font handle was unable to be loaded"))
    }

    /// Loads all of the fallback fonts for the `font_id` with a given `family_name` and set of `properties`.
    /// Noops if fallback fonts for the font are already loaded.
    #[cfg(not(any(target_os = "windows", target_family = "wasm")))]
    fn load_fallback_fonts(&self, font_id: FontId, family_name: &str, properties: Properties) {
        if self.fallback_fonts.contains_key(&font_id) {
            return;
        }

        if let Ok(fallback_fonts) = loader::fallback_fonts(family_name, properties) {
            let fallback_fonts = fallback_fonts
                .into_iter()
                .filter_map(|font| self.insert_font(font).ok())
                .collect_vec();

            self.fallback_fonts.insert(font_id, fallback_fonts);
        }
    }

    /// On Windows, this is a no-op. We retrieve fallback fonts dynamically as needed.
    #[cfg(target_os = "windows")]
    fn load_fallback_fonts(&self, _font_id: FontId, _family_name: &str, _properties: Properties) {}

    /// Font fallback is intentionally empty on WASM — browser fonts are controlled
    /// by the hosting HTML page.
    #[cfg(target_family = "wasm")]
    fn load_fallback_fonts(&self, _font_id: FontId, _family_name: &str, _properties: Properties) {}

    #[allow(clippy::too_many_arguments)]
    fn create_text_frame(
        &self,
        layout_lines: impl Iterator<Item = (LayoutLine, bool)>,
        line_style: LineStyle,
        text_styles_map: &TextStylesMap,
        max_height: f32,
        alignment: TextAlignment,
        str_index_map: &StrIndexMap,
        text: &str,
    ) -> TextFrame {
        let (_, upper_bound) = layout_lines.size_hint();
        let mut lines = match upper_bound {
            None => vec![],
            Some(size) => Vec::with_capacity(size),
        };

        let mut max_line_width: f32 = 0.;
        let mut total_height = 0.;
        let mut line_glyph_start_index: usize = 0;

        let mut layout_lines = layout_lines.peekable();
        while let Some((line, has_trailing_newline)) = layout_lines.next() {
            max_line_width = max_line_width.max(line.w);
            let is_last_line = layout_lines.peek().is_none();
            let line = self.create_line(
                line,
                line_style,
                text_styles_map,
                is_last_line.then_some(ClipConfig::default()),
                str_index_map,
                text,
                line_glyph_start_index,
                has_trailing_newline,
            );
            total_height += line.height();

            // We add 1 to the last caret position here to skip the newline character
            // when advancing to the next paragraph. Wrapped lines within the same
            // paragraph (`has_trailing_newline == false`) already have `glyph.start`
            // byte offsets relative to the start of the paragraph.
            if has_trailing_newline {
                line_glyph_start_index = line
                    .caret_positions
                    .last()
                    .map(|position| position.last_offset)
                    .unwrap_or(line_glyph_start_index)
                    + 1;
            }

            // TODO(alokedesai): Properly clip multi-line text using the same strategy we use on mac.
            // See https://github.com/warpdotdev/warp-internal/blob/91dfe429074c6129a6b5c1c57c55c1daf6d274a9/ui/src/platform/mac/text_layout.rs#L318-L359.
            if total_height > max_height {
                break;
            }
            lines.push(line);
        }

        match Vec1::try_from_vec(lines) {
            Ok(lines) => TextFrame::new(lines, max_line_width, alignment),
            Err(_) => TextFrame::empty(line_style.font_size, line_style.line_height_ratio),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create_line(
        &self,
        layout_line: LayoutLine,
        line_style: LineStyle,
        text_styles_map: &TextStylesMap,
        clip_config: Option<ClipConfig>,
        str_index_map: &StrIndexMap,
        text: &str,
        line_glyph_start_index: usize,
        has_trailing_newline: bool,
    ) -> Line {
        let Some(first_glyph) = layout_line.glyphs.first() else {
            return Line::empty(
                line_style.font_size,
                line_style.line_height_ratio,
                line_glyph_start_index,
            );
        };
        let width = layout_line.w;

        let initial_font_id = *self
            .font_id_map
            .read()
            .get_by_right(&first_glyph.font_id)
            .expect("Font must exist in map");

        let mut run_builder = RunBuilder::new(text_styles_map, initial_font_id, str_index_map);
        let mut caret_positions = vec![];
        let mut chars_with_missing_glyphs = vec![];
        let mut last_glyph_offset: usize = 0;
        let line_byte_start_index = str_index_map
            .byte_index(line_glyph_start_index)
            .unwrap_or(0);

        run_builder.reserve_capacity(layout_line.glyphs.len());
        for glyph in layout_line.glyphs {
            // TODO(daprahamian): when we have time, we should investigate pulling
            // caret position data out of font gdef table. Right now, this impl does
            // not work for certain ligatures
            if glyph.w > 0.0 {
                let start_offset = str_index_map
                    .char_index(line_byte_start_index + glyph.start)
                    .unwrap_or(0);
                let last_offset = str_index_map
                    .char_index(line_byte_start_index + glyph.end)
                    .unwrap_or_else(|| str_index_map.num_chars())
                    .saturating_sub(1);
                caret_positions.push(CaretPosition {
                    position_in_line: glyph.x,
                    start_offset,
                    last_offset,
                });
                last_glyph_offset = last_offset;
            }

            // A glyph_id of 0 implies that no glyph was found for this character.
            if glyph.glyph_id == 0 {
                if let Some(ch) = Self::char_for_glyph(&glyph, text) {
                    chars_with_missing_glyphs.push(ch);
                }
            }

            run_builder.push_glyph(glyph, |id| {
                *self
                    .font_id_map
                    .read()
                    .get_by_right(id)
                    .expect("Font must exist in map")
            });
        }

        if has_trailing_newline {
            caret_positions.push(CaretPosition {
                position_in_line: layout_line.w,
                start_offset: last_glyph_offset + 1,
                last_offset: last_glyph_offset + 1,
            });
        }

        Line {
            width,
            // TODO(vorporeal): See if we need to compute this (and if so, how to).
            trailing_whitespace_width: 0.0,
            runs: run_builder.build(),
            font_size: line_style.font_size,
            line_height_ratio: line_style.line_height_ratio,
            baseline_ratio: line_style.baseline_ratio,
            ascent: layout_line.max_ascent,
            descent: layout_line.max_descent,
            clip_config,
            caret_positions,
            chars_with_missing_glyphs,
        }
    }

    fn char_for_glyph(glyph: &LayoutGlyph, text: &str) -> Option<char> {
        if !text.is_char_boundary(glyph.start) {
            log::warn!("Expected glyph start to be a char boundary");
            return None;
        }

        text[glyph.start..].chars().next()
    }

    /// Produces an [`AttrsList`] to layout text given a list of `style_runs` and the `text` the
    /// runs correspond to.
    fn build_attrs_list(
        &self,
        text: &str,
        style_runs: &[(Range<usize>, StyleAndFont)],
        text_styles_map: &mut TextStylesMap,
        str_index_map: &StrIndexMap,
    ) -> AttrsList {
        let attrs = Attrs::new();
        let mut attrs_list = AttrsList::new(attrs);

        for (range, style_and_font) in style_runs {
            let start_byte_index = str_index_map.byte_index(range.start).unwrap_or(text.len());
            let end_byte_index = str_index_map.byte_index(range.end).unwrap_or(text.len());

            // Perform font selection using font-db before passing the font to cosmic text. Since it
            // does not use the CSS3 font selection algorithm, it will panic if we pass a font
            // weight or style that isn't available for the font. See https://github.com/pop-os/cosmic-text/issues/58.
            let selected_font =
                self.select_font(style_and_font.font_family, style_and_font.properties);
            let id = *self
                .font_id_map
                .read()
                .get_by_left(&selected_font)
                .expect("Selected font must exist in font_id_map");

            let font_store = self.font_store.read();
            let face = match font_store.db().face(id) {
                None => continue,
                Some(face) => face,
            };

            let Some((family, _)) = face.families.first() else {
                continue;
            };

            let mut span_style = style_and_font.style;
            if style_and_font
                .properties
                .weight
                .is_optical_dark_compensated()
            {
                span_style.optical_dark_compensated = true;
            }
            let style_index = text_styles_map.insert(span_style);

            attrs_list.add_span(
                start_byte_index..end_byte_index,
                Attrs {
                    color_opt: None,
                    family: cosmic_text::Family::Name(family),
                    stretch: Default::default(),
                    style: face.style,
                    weight: face.weight,
                    metadata: style_index,
                    cache_key_flags: cosmic_text::CacheKeyFlags::empty(),
                    metrics_opt: None,
                },
            );
        }
        attrs_list
    }
}

#[cfg_attr(target_family = "wasm", expect(dead_code))]
struct LoadedSystemFonts(Vec<(crate::fonts::FontInfo, FontFamily)>);

impl platform::LoadedSystemFonts for LoadedSystemFonts {
    fn as_any(self: Box<Self>) -> Box<dyn Any> {
        self as Box<dyn Any>
    }
}

impl platform::FontDB for FontDB {
    fn load_from_bytes(&mut self, name: &str, bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        let family = load_font_family_from_bytes(name, bytes)?;
        self.insert_font_family(family)
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, font_family: &str) -> Result<FamilyId> {
        let family = loader::load_system_font(font_family)?;
        self.insert_font_family(family)
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn platform::LoadedSystemFonts>> {
        self.text_layout_system.load_all_system_fonts()
    }

    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        loaded_system_fonts: Box<dyn platform::LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, crate::fonts::FontInfo)> {
        let loaded_system_fonts: Box<LoadedSystemFonts> = loaded_system_fonts
            .as_any()
            .downcast()
            .expect("should not fail to downcast to concrete type");

        loaded_system_fonts
            .0
            .into_iter()
            .map(|(font_info, family)| {
                let family_id = self.insert_font_family(family).ok();
                (family_id, font_info)
            })
            .collect_vec()
    }

    fn family_id_for_name(&self, name: &str) -> Option<FamilyId> {
        self.text_layout_system.family_id_for_name(name)
    }

    fn load_family_name_from_id(&self, id: FamilyId) -> Option<String> {
        self.text_layout_system.load_family_name_from_id(id)
    }

    fn select_font(&self, family_id: FamilyId, properties: Properties) -> FontId {
        self.text_layout_system.select_font(family_id, properties)
    }

    fn fallback_fonts(&self, character: char, font_id: FontId) -> Vec<FontId> {
        self.text_layout_system.fallback_fonts(character, font_id)
    }

    fn font_metrics(&self, font_id: FontId) -> Metrics {
        self.text_layout_system.font_metrics(font_id)
    }

    fn glyph_advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Vector2I> {
        self.text_layout_system.glyph_advance(font_id, glyph_id)
    }

    fn glyph_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<RectI> {
        self.text_layout_system
            .glyph_typographic_bounds(font_id, glyph_id)
    }

    #[cfg(feature = "fontkit-rasterizer")]
    fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &GlyphConfig,
    ) -> Result<RectI> {
        let fonts = self
            .text_layout_system
            .take_fonts_loaded_since_last_raster();
        for font in fonts {
            self.load_font_kit_font(font)?
        }
        self.font_kit_rasterizer
            .glyph_raster_bounds(font_id, size, glyph_id, scale, glyph_config)
    }

    #[cfg(not(feature = "fontkit-rasterizer"))]
    fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &GlyphConfig,
    ) -> Result<RectI> {
        Self::glyph_raster_bounds(self, font_id, size, glyph_id, scale, glyph_config)
    }

    #[cfg(feature = "fontkit-rasterizer")]
    fn rasterize_glyph(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &GlyphConfig,
        format: RasterFormat,
    ) -> Result<RasterizedGlyph> {
        let fonts = self
            .text_layout_system
            .take_fonts_loaded_since_last_raster();
        for font in fonts {
            self.load_font_kit_font(font)?
        }
        self.font_kit_rasterizer.rasterize_glyph(
            font_id,
            size,
            glyph_id,
            scale,
            subpixel_alignment,
            glyph_config,
            format,
        )
    }

    #[cfg(not(feature = "fontkit-rasterizer"))]
    fn rasterize_glyph(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &GlyphConfig,
        format: RasterFormat,
    ) -> Result<RasterizedGlyph> {
        Self::rasterize_glyph(
            self,
            font_id,
            size,
            glyph_id,
            scale,
            subpixel_alignment,
            glyph_config,
            format,
        )
    }

    fn glyph_for_char(&self, font_id: FontId, char: char) -> Option<GlyphId> {
        self.text_layout_system.glyph_for_char(font_id, char)
    }

    fn text_layout_system(&self) -> &dyn platform::TextLayoutSystem {
        &self.text_layout_system
    }
}

impl platform::TextLayoutSystem for TextLayoutSystem {
    fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        clip_config: ClipConfig,
    ) -> Line {
        // cosmic_text panics if we pass text that is multiple paragraphs. Since we are laying out
        // a line (which is by definition _not_ wrapped), combine all of the paragraphs together
        // with a zero-width space (U+200B). We do this to ensure that all of the text renders on a
        // single line, while also ensuring the style runs (which previously included the separator)
        // are still respected properly.
        let text = BidiParagraphs::new(text).join("\u{200B}");

        let mut text_styles_map = TextStylesMap::new();
        let str_index_map = StrIndexMap::new(&text);
        let attrs_list = self.build_attrs_list(
            text.as_str(),
            style_runs,
            &mut text_styles_map,
            &str_index_map,
        );

        let tab_width = line_style.fixed_width_tab_size.unwrap_or(4).into();
        let shape_line = ShapeLine::new(
            self.font_store.write().deref_mut(),
            text.as_str(),
            &attrs_list,
            Shaping::Advanced,
            tab_width,
        );

        // Layout the line, passing `Wrap::None` here since we want to render all of the text on a
        // single line.
        let layout = shape_line.layout(
            line_style.font_size,
            Some(max_width),
            Wrap::None,
            Some(Align::Left),
            None,
            None,
        );

        // Since we passed `Wrap::None`, cosmic text will only produce a single laid out line.
        debug_assert_eq!(
            layout.len(),
            1,
            "Expected a single laid out line but there were {} lines instead",
            layout.len()
        );
        let first_line = match layout.into_iter().next() {
            Some(line) => line,
            None => return Line::empty(line_style.font_size, line_style.line_height_ratio, 0),
        };

        self.create_line(
            first_line,
            line_style,
            &text_styles_map,
            Some(clip_config),
            &str_index_map,
            &text,
            0,
            false,
        )
    }
    fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        let mut text_styles_map = TextStylesMap::new();
        let str_index_map = StrIndexMap::new(text);
        let mut attrs_list =
            self.build_attrs_list(text, style_runs, &mut text_styles_map, &str_index_map);
        let mut font_store = self.font_store.write();

        let tab_width = line_style.fixed_width_tab_size.unwrap_or(4).into();
        let mut num_bytes_seen = 0;
        let layouts = BidiParagraphs::new(text).flat_map(|paragraph| {
            let following_paragraph_text = &text[num_bytes_seen + paragraph.len()..];
            let mut char_indices = following_paragraph_text.char_indices();

            // Determine the byte length of the separator. The `BidiParagraphs` iterator does not
            // coalesce multiple paragraph separators so we know there is at most one separator.
            let (separator_byte_length, has_trailing_separator) = match char_indices.next() {
                Some((start_byte_index, _char)) => {
                    // Determine the end byte index of the character by checking the start character
                    // index of the following character. If the current character is the last
                    // character, the end byte index is the total length of the string.
                    let end_byte_index = char_indices
                        .next()
                        .map(|(index, _)| index)
                        .unwrap_or(following_paragraph_text.len());

                    (end_byte_index - start_byte_index, true)
                }
                None => (0, false),
            };
            num_bytes_seen += paragraph.len() + separator_byte_length;

            // Remove the attributes from the attrs list that correspond to this specific paragraph.
            // We also remove the paragraph's separator--this won't impact styling (since the length
            // of the text is smaller than that of the attribute list) but ensures all of the styles
            // related to this line of text are correctly removed.
            let mut current_attrs_list =
                attrs_list.split_off(paragraph.len() + separator_byte_length);
            std::mem::swap(&mut current_attrs_list, &mut attrs_list);

            let shape_line = ShapeLine::new(
                font_store.deref_mut(),
                paragraph,
                &current_attrs_list,
                Shaping::Advanced,
                tab_width,
            );

            let mut layout_lines = shape_line.layout(
                line_style.font_size,
                Some(max_width),
                Wrap::WordOrGlyph,
                Some(Align::Left),
                first_line_head_indent,
                None,
            );
            rebalance_orphan_last_line(paragraph, &mut layout_lines, max_width, line_style.font_size);
            let layout_line_count = layout_lines.len();
            layout_lines
                .into_iter()
                .enumerate()
                .map(|(line_idx, line)| {
                    // Each ShapeLine may get wrapped to multiple LayoutLines.
                    // We should only add the trailing separator to the last LayoutLine.
                    let is_last = line_idx == layout_line_count.saturating_sub(1);
                    let include_trailing_separator = is_last && has_trailing_separator;
                    (line, include_trailing_separator)
                })
                .collect_vec()
        });

        self.create_text_frame(
            layouts,
            line_style,
            &text_styles_map,
            max_height,
            alignment,
            &str_index_map,
            text,
        )
    }
}

impl TextLayoutSystem {
    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn platform::LoadedSystemFonts>> {
        use futures::FutureExt as _;

        async { Box::new(loader::load_all_system_fonts()) as Box<dyn platform::LoadedSystemFonts> }
            .boxed()
    }

    fn load_family_name_from_id(&self, id: FamilyId) -> Option<String> {
        self.families.get(&id).map(|family| family.name.to_owned())
    }

    fn select_font(&self, family_id: FamilyId, properties: Properties) -> FontId {
        match self.font_selections.entry((family_id, properties)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let family = self
                    .families
                    .get(&family_id)
                    .expect("Font family must exist");

                let weight =
                    fontdb::Weight(font_weight_to_cosmic_text(properties.weight).0);
                let style = match properties.style {
                    Style::Normal => fontdb::Style::Normal,
                    Style::Italic => fontdb::Style::Italic,
                };
                let best_match = self.font_store.read().db().query(&Query {
                    families: &[fontdb::Family::Name(family.name.as_str())],
                    weight,
                    stretch: Default::default(),
                    style,
                });

                let best_match =
                    best_match.and_then(|id| self.font_id_map.read().get_by_right(&id).copied());
                let best_match = best_match.unwrap_or(*family.font_ids.first());

                self.load_fallback_fonts(best_match, family.name.as_str(), properties);

                *entry.insert(best_match)
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_family = "wasm")))]
    fn fallback_fonts(&self, _ch: char, font_id: FontId) -> Vec<FontId> {
        self.fallback_fonts
            .get(&font_id)
            .map(|fallbacks| fallbacks.clone())
            .unwrap_or_default()
    }

    /// On WASM, font fallback is intentionally empty — browser fonts are
    /// handled by the hosting page, not the Rust binary.
    #[cfg(target_family = "wasm")]
    fn fallback_fonts(&self, _ch: char, _font_id: FontId) -> Vec<FontId> {
        vec![]
    }

    #[cfg(target_os = "windows")]
    fn fallback_fonts(&self, character: char, font_id: FontId) -> Vec<FontId> {
        self.get_fallback_fonts_for_character(character, font_id)
            .map_err(|err| {
                log::warn!("Unable to fetch fallback fonts for character {character:?}: {err:?}");
                err
            })
            .unwrap_or_default()
    }

    fn font_metrics(&self, font_id: FontId) -> crate::fonts::Metrics {
        self.read_font_face(font_id, |font_face| crate::fonts::Metrics {
            units_per_em: font_face.units_per_em().into(),
            ascent: font_face.ascender(),
            descent: font_face.descender(),
            line_gap: font_face.line_gap(),
        })
    }

    fn glyph_advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Vector2I> {
        self.read_font_face(font_id, |font_face| {
            let glyph_id = owned_ttf_parser::GlyphId(glyph_id as u16);

            let horizontal_advance = font_face.glyph_hor_advance(glyph_id).unwrap_or(0);
            let vertical_advance = font_face.glyph_ver_advance(glyph_id).unwrap_or(0);

            Ok(vec2i(horizontal_advance.into(), vertical_advance.into()))
        })
    }

    fn glyph_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<RectI> {
        self.read_font_face(font_id, |font_face| {
            let ttf_parser_glyph_id = owned_ttf_parser::GlyphId::from_glyph_id(glyph_id);

            let raster_image = font_face.glyph_raster_image(ttf_parser_glyph_id, u16::MAX);
            if let Some(raster_image) = raster_image {
                // The raster image's bounds is in pixels. Determine the scale factor to convert the raster image's
                // bounds back to font units.
                let scale = if raster_image.pixels_per_em != 0 {
                    font_face.units_per_em() as f32 / raster_image.pixels_per_em as f32
                } else {
                    1.0
                };

                let bounding_box = RectF::new(
                    vec2f(raster_image.x as f32, raster_image.y as f32),
                    vec2f(raster_image.width as f32, raster_image.height as f32),
                );

                return Ok((bounding_box * scale).to_i32());
            }
            let bounding_box = font_face
                .glyph_bounding_box(ttf_parser_glyph_id)
                .ok_or_else(|| anyhow!("No bounding box for glyph id {glyph_id:?}"))?;

            Ok(bounding_box.to_recti())
        })
    }

    fn glyph_for_char(&self, font_id: FontId, char: char) -> Option<GlyphId> {
        self.try_read_font_face(font_id, |font_face| {
            font_face.glyph_index(char).map(GlyphIdExt::to_glyph_id)
        })?
    }

    fn family_id_for_name(&self, name: &str) -> Option<FamilyId> {
        self.families
            .iter()
            .find_map(|(family_id, family)| (family.name == name).then_some(*family_id))
    }
}

/// Helper extension trait to convert to a [`RectI`].
trait ToRectI {
    fn to_recti(self) -> RectI;
}

impl ToRectI for owned_ttf_parser::Rect {
    fn to_recti(self) -> RectI {
        RectI::new(
            vec2i(self.x_min.into(), self.y_min.into()),
            vec2i(self.width().into(), self.height().into()),
        )
    }
}

/// Helper extension trait to convert to a [`GlyphId`]
trait GlyphIdExt {
    fn to_glyph_id(self) -> GlyphId;

    fn from_glyph_id(glyph_id: GlyphId) -> Self;
}

impl GlyphIdExt for owned_ttf_parser::GlyphId {
    fn to_glyph_id(self) -> GlyphId {
        self.0.into()
    }

    fn from_glyph_id(glyph_id: GlyphId) -> Self {
        Self(glyph_id as u16)
    }
}

fn font_weight_to_cosmic_text(weight: Weight) -> cosmic_text::Weight {
    match weight {
        Weight::Thin => cosmic_text::Weight::THIN,
        Weight::ExtraLight => cosmic_text::Weight::EXTRA_LIGHT,
        Weight::Light => cosmic_text::Weight::LIGHT,
        Weight::Normal => cosmic_text::Weight::NORMAL,
        Weight::MediumOpticalDark => cosmic_text::Weight(460),
        Weight::Medium => cosmic_text::Weight::MEDIUM,
        Weight::SemiboldOpticalDark => cosmic_text::Weight(560),
        Weight::Semibold => cosmic_text::Weight::SEMIBOLD,
        Weight::Bold => cosmic_text::Weight::BOLD,
        Weight::ExtraBold => cosmic_text::Weight::EXTRA_BOLD,
        Weight::Black => cosmic_text::Weight::BLACK,
    }
}

/// Prevents "orphan" last lines (e.g., a single CJK character + period `"了。"`, or a single
/// short 1–3 letter English word) by pulling 1 word or 1 CJK character from the end of the
/// penultimate line onto the start of the last line whenever space permits.
fn rebalance_orphan_last_line(
    paragraph: &str,
    layout_lines: &mut [LayoutLine],
    max_width: f32,
    font_size: f32,
) {
    if layout_lines.len() < 2 || !max_width.is_finite() || max_width < font_size * 6.0 {
        return;
    }

    let n = layout_lines.len();
    let (head, tail) = layout_lines.split_at_mut(n - 1);
    let prev = &mut head[n - 2];
    let last = &mut tail[0];

    if last.glyphs.is_empty() || prev.glyphs.len() < 3 {
        return;
    }

    // Extract the substring of the last line to inspect its characters.
    let last_start_byte = last.glyphs.first().map(|g| g.start).unwrap_or(0);
    let last_end_byte = last.glyphs.last().map(|g| g.end).unwrap_or(0);
    let last_slice = paragraph.get(last_start_byte..last_end_byte).unwrap_or("");

    let is_cjk_or_punct = |c: char| -> bool {
        matches!(
            c as u32,
            0x3000..=0x303F
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xF900..=0xFAFF
                | 0xFF00..=0xFFEF
        )
    };
    let is_closing_punct = |c: char| -> bool {
        matches!(
            c,
            '。' | '，'
                | '、'
                | '！'
                | '？'
                | '；'
                | '：'
                | '”'
                | '’'
                | '）'
                | '】'
                | '》'
                | '.'
                | ','
                | '!'
                | '?'
                | ';'
                | ':'
                | ')'
                | ']'
        )
    };

    let chars: Vec<char> = last_slice.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return;
    }

    let non_punct_count = chars.iter().filter(|&&c| !is_closing_punct(c)).count();
    let has_cjk = chars.iter().any(|&c| is_cjk_or_punct(c));

    // Orphan criteria:
    // - CJK: at most 1 non-punctuation character (e.g. "了", "了。", "吗？")
    // - Latin/ASCII: single short word of <= 4 visible characters OR width < font_size * 2.2
    let is_orphan = if has_cjk {
        non_punct_count <= 1 && chars.len() <= 3
    } else {
        let word_count = last_slice.split_whitespace().count();
        word_count <= 1 && (non_punct_count <= 3 || last.w < font_size * 2.2)
    };

    if !is_orphan {
        return;
    }

    // Determine how many trailing glyphs to pull from `prev`:
    // - If `prev` ends with a CJK glyph, pull 1 glyph (1 CJK character).
    // - If `prev` ends with Latin/ASCII, pull the last word (back to the last whitespace in `prev`,
    //   up to 8 glyphs, leaving at least 2 glyphs on `prev`).
    let prev_len = prev.glyphs.len();
    let last_prev_glyph = &prev.glyphs[prev_len - 1];
    let last_prev_str = paragraph
        .get(last_prev_glyph.start..last_prev_glyph.end)
        .unwrap_or("");
    let prev_ends_cjk = last_prev_str.chars().any(is_cjk_or_punct);

    let pull_count = if prev_ends_cjk {
        1usize
    } else {
        // Walk backwards from the end of `prev.glyphs`, skipping any trailing whitespace glyphs,
        // then collecting the last word's glyphs.
        let mut idx = prev_len;
        while idx > 0 {
            let g = &prev.glyphs[idx - 1];
            let s = paragraph.get(g.start..g.end).unwrap_or("");
            if s.chars().all(|c| c.is_whitespace()) {
                idx -= 1;
            } else {
                break;
            }
        }
        let word_end = idx;
        while idx > 0 {
            let g = &prev.glyphs[idx - 1];
            let s = paragraph.get(g.start..g.end).unwrap_or("");
            if s.chars().any(|c| c.is_whitespace()) || is_cjk_or_punct(s.chars().next().unwrap_or(' '))
            {
                break;
            }
            idx -= 1;
        }
        let word_glyphs = word_end.saturating_sub(idx);
        if word_glyphs == 0 || word_glyphs > 10 || idx < 2 {
            return;
        }
        // Pull from `idx` to `prev_len` (includes any trailing space cosmic-text left on `prev`).
        prev_len - idx
    };

    if pull_count == 0 || prev_len <= pull_count + 1 {
        return;
    }

    let split_at = prev_len - pull_count;
    let pulled_width: f32 = prev.glyphs[split_at..].iter().map(|g| g.w).sum();
    // Add a small space allowance when pulling a Latin word onto a Latin orphan line
    let space_gap = if !prev_ends_cjk && !has_cjk {
        font_size * 0.28
    } else {
        0.0
    };

    if last.w + pulled_width + space_gap > max_width {
        return;
    }

    let mut pulled: Vec<LayoutGlyph> = prev.glyphs.drain(split_at..).collect();
    // Strip any trailing whitespace glyph from `prev` and from `pulled`
    while let Some(g) = prev.glyphs.last() {
        let s = paragraph.get(g.start..g.end).unwrap_or("");
        if s.chars().all(|c| c.is_whitespace()) {
            prev.glyphs.pop();
        } else {
            break;
        }
    }
    while let Some(g) = pulled.last() {
        let s = paragraph.get(g.start..g.end).unwrap_or("");
        if s.chars().all(|c| c.is_whitespace()) {
            pulled.pop();
        } else {
            break;
        }
    }

    if pulled.is_empty() {
        return;
    }

    // Recompute x positions for `pulled` at the start of `last`, then shift existing `last.glyphs` right.
    let mut cursor_x = 0.0f32;
    for g in &mut pulled {
        g.x = cursor_x;
        cursor_x += g.w;
    }
    cursor_x += space_gap;

    for g in &mut last.glyphs {
        g.x += cursor_x;
    }

    pulled.append(&mut last.glyphs);
    last.glyphs = pulled;

    prev.w = prev
        .glyphs
        .last()
        .map(|g| g.x + g.w)
        .unwrap_or(0.0);
    last.w = last
        .glyphs
        .last()
        .map(|g| g.x + g.w)
        .unwrap_or(0.0);
}

#[cfg(test)]
#[path = "text_layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
mod micro_craft_tests {
    use super::*;
    use crate::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
    use crate::fonts::{collect_glyph_indices, init_fonts};
    use crate::platform::FontDB as _;
    use crate::text_layout::{GlyphBloomSchedule, TextStyle, DEFAULT_TOP_BOTTOM_RATIO};

    fn template_glyph() -> LayoutGlyph {
        let (font_db, roboto) = init_fonts();
        let text = "A";
        let style_runs = [(
            0..1,
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )];
        let mut text_styles_map = TextStylesMap::new();
        let str_index_map = StrIndexMap::new(text);
        let attrs_list = font_db.text_layout_system.build_attrs_list(
            text,
            &style_runs,
            &mut text_styles_map,
            &str_index_map,
        );
        let mut font_store = font_db.text_layout_system.font_store.write();
        let shape_line = ShapeLine::new(
            font_store.deref_mut(),
            text,
            &attrs_list,
            Shaping::Advanced,
            4,
        );
        let lines = shape_line.layout(16.0, None, Wrap::None, None, None, None);
        lines[0].glyphs[0].clone()
    }

    fn make_mock_glyph(template: &LayoutGlyph, start: usize, end: usize, x: f32, w: f32) -> LayoutGlyph {
        let mut g = template.clone();
        g.start = start;
        g.end = end;
        g.x = x;
        g.w = w;
        g
    }

    fn build_layout_line_for_range(
        template: &LayoutGlyph,
        paragraph: &str,
        char_range: Range<usize>,
        advance: f32,
    ) -> LayoutLine {
        let char_indices: Vec<(usize, char)> = paragraph.char_indices().collect();
        let mut glyphs = Vec::new();
        let mut x = 0.0f32;
        for idx in char_range {
            let (start, ch) = char_indices[idx];
            let end = start + ch.len_utf8();
            let w = if ch.is_ascii() { advance * 0.55 } else { advance };
            glyphs.push(make_mock_glyph(template, start, end, x, w));
            x += w;
        }
        LayoutLine {
            w: x,
            max_ascent: 12.0,
            max_descent: 4.0,
            glyphs,
            line_height_opt: None,
        }
    }

    fn line_text<'a>(paragraph: &'a str, line: &LayoutLine) -> &'a str {
        let start = line.glyphs.first().map(|g| g.start).unwrap_or(0);
        let end = line.glyphs.last().map(|g| g.end).unwrap_or(0);
        &paragraph[start..end]
    }

    #[test]
    fn test_orphan_rebalance_cjk_single_char() {
        let tmpl = template_glyph();
        let paragraph = "这是一段用于测试孤字预防的中文最后一个字";
        let total_chars = paragraph.chars().count();
        // Line 0 has all chars except the last one ("字"), Line 1 has only "字".
        let mut lines = vec![
            build_layout_line_for_range(&tmpl, paragraph, 0..(total_chars - 1), 16.0),
            build_layout_line_for_range(&tmpl, paragraph, (total_chars - 1)..total_chars, 16.0),
        ];
        rebalance_orphan_last_line(paragraph, &mut lines, 320.0, 16.0);
        assert_eq!(line_text(paragraph, &lines[1]), "个字");
        assert!(line_text(paragraph, &lines[0]).ends_with("最后一"));
    }

    #[test]
    fn test_orphan_rebalance_cjk_char_with_period() {
        let tmpl = template_glyph();
        let paragraph = "整个流式排版和渲染微工艺已经全部完成了。";
        let total_chars = paragraph.chars().count();
        // Line 0 ends before "了。", Line 1 has orphan "了。".
        let mut lines = vec![
            build_layout_line_for_range(&tmpl, paragraph, 0..(total_chars - 2), 16.0),
            build_layout_line_for_range(&tmpl, paragraph, (total_chars - 2)..total_chars, 16.0),
        ];
        rebalance_orphan_last_line(paragraph, &mut lines, 320.0, 16.0);
        assert_eq!(line_text(paragraph, &lines[1]), "成了。");
        assert!(line_text(paragraph, &lines[0]).ends_with("完"));
    }

    #[test]
    fn test_orphan_rebalance_english_short_word() {
        let tmpl = template_glyph();
        let paragraph = "Claude Desktop builds a remarkably smooth UI";
        // Split before "UI": Line 0 = "Claude Desktop builds a remarkably smooth ", Line 1 = "UI"
        let split_char = paragraph.chars().count() - 2;
        let mut lines = vec![
            build_layout_line_for_range(&tmpl, paragraph, 0..split_char, 16.0),
            build_layout_line_for_range(
                &tmpl,
                paragraph,
                split_char..paragraph.chars().count(),
                16.0,
            ),
        ];
        rebalance_orphan_last_line(paragraph, &mut lines, 400.0, 16.0);
        assert_eq!(line_text(paragraph, &lines[1]), "smooth UI");
        assert_eq!(
            line_text(paragraph, &lines[0]),
            "Claude Desktop builds a remarkably"
        );

        // Also verify end-to-end via layout_text with bundled Roboto font:
        let (font_db, roboto) = init_fonts();
        let line_style = LineStyle {
            font_size: 16.0,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        };
        let style_runs = [(
            0..paragraph.chars().count(),
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )];
        // Measure full single-line width so we can pick a max_width that would normally orphan "UI".
        let full_line = font_db.text_layout_system().layout_line(
            paragraph,
            line_style,
            &style_runs,
            f32::MAX,
            ClipConfig::default(),
        );
        let narrow_width = full_line.width - 4.0;
        let frame = font_db.text_layout_system().layout_text(
            paragraph,
            line_style,
            &style_runs,
            narrow_width,
            f32::MAX,
            TextAlignment::Left,
            None,
        );
        let indices = collect_glyph_indices(&frame);
        assert_eq!(indices.len(), 2);
        // Last line must have more than just the 2 glyphs of "UI" (it pulled "smooth" too).
        assert!(indices[1].len() >= 8, "expected 'smooth UI' on last line, got {:?}", indices[1]);
    }

    #[test]
    fn test_optical_dark_weights_and_bloom_schedule() {
        assert_eq!(Weight::Medium.optical_dark(), Weight::MediumOpticalDark);
        assert_eq!(Weight::Semibold.optical_dark(), Weight::SemiboldOpticalDark);
        assert_eq!(Weight::MediumOpticalDark.to_css_weight(), 460);
        assert_eq!(Weight::SemiboldOpticalDark.to_css_weight(), 560);
        assert_eq!(
            font_weight_to_cosmic_text(Weight::MediumOpticalDark),
            cosmic_text::Weight(460)
        );
        assert_eq!(
            font_weight_to_cosmic_text(Weight::SemiboldOpticalDark),
            cosmic_text::Weight(560)
        );

        let props = Properties::default()
            .weight(Weight::Medium)
            .with_optical_dark_weight(true);
        assert!(props.weight.is_optical_dark_compensated());

        let schedule = GlyphBloomSchedule {
            reveal_start_char: 10,
            elapsed_ms: 45.0,
            stagger_ms: 6.0,
            duration_ms: 90.0,
        };
        assert_eq!(schedule.progress_for_char(5), 1.0);
        assert!((schedule.progress_for_char(10) - 0.5).abs() < 1e-5);
        assert_eq!(schedule.progress_for_char(20), 0.0);
    }
}

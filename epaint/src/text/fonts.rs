#[cfg(feature = "system_fonts")]
use font_kit::family_handle::FamilyHandle;
use std::collections::BTreeMap;
#[cfg(feature = "system_fonts")]
use std::fs;
use std::sync::Arc;

use crate::{
    mutex::{Mutex, MutexGuard},
    text::{
        font::{FontImpl, FontImplManager},
        Galley, LayoutJob,
    },
    TextureAtlas,
};
use emath::NumExt as _;

// ----------------------------------------------------------------------------

/// How to select a sized font.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FontId {
    /// Height in points.
    pub size: f32,

    /// What font family to use.
    pub font_type: FontType,
    // TODO(emilk): weight (bold), italics, …
}

impl Default for FontId {
    #[inline]
    fn default() -> Self {
        Self {
            size: 14.0,
            font_type: FontType::Proportional,
        }
    }
}

impl FontId {
    #[inline]
    pub const fn new(size: f32, font_type: FontType) -> Self {
        Self { size, font_type }
    }

    #[inline]
    pub const fn proportional(size: f32) -> Self {
        Self::new(size, FontType::Proportional)
    }

    #[inline]
    pub const fn monospace(size: f32) -> Self {
        Self::new(size, FontType::Monospace)
    }
}

#[allow(clippy::derive_hash_xor_eq)]
impl std::hash::Hash for FontId {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let Self { size, font_type } = self;
        crate::f32_hash(state, *size);
        font_type.hash(state);
    }
}

// ----------------------------------------------------------------------------

/// Font of unknown size.
///
/// Which style of font: [`Monospace`][`FontFamily::Monospace`], [`Proportional`][`FontFamily::Proportional`],
/// or by user-chosen name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum FontType {
    /// A font where some characters are wider than other (e.g. 'w' is wider than 'i').
    ///
    /// Proportional fonts are easier to read and should be the preferred choice in most situations.
    Proportional,

    /// A font where each character is the same width (`w` is the same width as `i`).
    ///
    /// Useful for code snippets, or when you need to align numbers or text.
    Monospace,

    /// One of the names in [`FontDefinitions::families`].
    ///
    /// ```
    /// # use epaint::FontFamily;
    /// // User-chosen names:
    /// FontFamily::Name("arial".into());
    /// FontFamily::Name("serif".into());
    /// ```
    Name(Arc<str>),
}

impl FontType {
    pub fn name(&self) -> String {
        match self {
            Self::Monospace => "Monospace".to_string(),
            Self::Proportional => "Proportional".to_string(),
            Self::Name(name) => (*name).to_string(),
        }
    }
}

impl Default for FontType {
    #[inline]
    fn default() -> Self {
        FontType::Proportional
    }
}

impl std::fmt::Display for FontType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Monospace => "Monospace".fmt(f),
            Self::Proportional => "Proportional".fmt(f),
            Self::Name(name) => (*name).fmt(f),
        }
    }
}

// ----------------------------------------------------------------------------

/// A `.ttf` or `.otf` file and a font face index.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FontData {
    /// The content of a `.ttf` or `.otf` file.
    pub font: std::borrow::Cow<'static, [u8]>,

    /// Which font face in the file to use.
    /// When in doubt, use `0`.
    pub index: u32,

    /// Extra scale and vertical tweak to apply to all text of this font.
    pub tweak: FontTweak,
}

impl FontData {
    pub fn from_static(font: &'static [u8]) -> Self {
        Self {
            font: std::borrow::Cow::Borrowed(font),
            index: 0,
            tweak: Default::default(),
        }
    }

    pub fn from_owned(font: Vec<u8>) -> Self {
        Self {
            font: std::borrow::Cow::Owned(font),
            index: 0,
            tweak: Default::default(),
        }
    }

    pub fn tweak(self, tweak: FontTweak) -> Self {
        Self { tweak, ..self }
    }
}

// ----------------------------------------------------------------------------

/// Extra scale and vertical tweak to apply to all text of a certain font.
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FontTweak {
    /// Scale the font by this much.
    ///
    /// Default: `1.0` (no scaling).
    pub scale: f32,

    /// Shift font downwards by this fraction of the font size (in points).
    ///
    /// A positive value shifts the text downwards.
    /// A negative value shifts it upwards.
    ///
    /// Example value: `-0.2`.
    pub y_offset_factor: f32,

    /// Shift font downwards by this amount of logical points.
    ///
    /// Example value: `2.0`.
    pub y_offset: f32,
}

impl Default for FontTweak {
    fn default() -> Self {
        Self {
            scale: 1.0,
            y_offset_factor: -0.2, // makes the default fonts look more centered in buttons and such
            y_offset: 0.0,
        }
    }
}

// ----------------------------------------------------------------------------

fn ab_glyph_font_from_font_data(name: &str, data: &FontData) -> ab_glyph::FontArc {
    match &data.font {
        std::borrow::Cow::Borrowed(bytes) => {
            ab_glyph::FontRef::try_from_slice_and_index(bytes, data.index)
                .map(ab_glyph::FontArc::from)
        }
        std::borrow::Cow::Owned(bytes) => {
            ab_glyph::FontVec::try_from_vec_and_index(bytes.clone(), data.index)
                .map(ab_glyph::FontArc::from)
        }
    }
    .unwrap_or_else(|err| panic!("Error parsing {:?} TTF/OTF font file: {}", name, err))
}

/// Describes the font data and the sizes to use.
///
/// Often you would start with [`FontDefinitions::default()`] and then add/change the contents.
///
/// This is how you install your own custom fonts:
/// ```
/// # use {epaint::text::{FontDefinitions, FontFamily, FontData}};
/// # struct FakeEguiCtx {};
/// # impl FakeEguiCtx { fn set_fonts(&self, _: FontDefinitions) {} }
/// # let egui_ctx = FakeEguiCtx {};
/// let mut fonts = FontDefinitions::default();
///
/// // Install my own font (maybe supporting non-latin characters):
/// fonts.font_data.insert("my_font".to_owned(),
///    FontData::from_static(include_bytes!("../../fonts/Ubuntu-Light.ttf"))); // .ttf and .otf supported
///
/// // Put my font first (highest priority):
/// fonts.families.get_mut(&FontFamily::Proportional).unwrap()
///     .insert(0, "my_font".to_owned());
///
/// // Put my font as last fallback for monospace:
/// fonts.families.get_mut(&FontFamily::Monospace).unwrap()
///     .push("my_font".to_owned());
///
/// egui_ctx.set_fonts(fonts);
/// ```
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FontDefinitions {
    /// List of font names and their definitions.
    ///
    /// `epaint` has built-in-default for these, but you can override them if you like.
    pub font_data_map: BTreeMap<String, FontData>,

    /// Which fonts (names) to use for each [`FontFamily`].
    ///
    /// The list should be a list of keys into [`Self::font_data`].
    /// When looking for a character glyph `epaint` will start with
    /// the first font and then move to the second, and so on.
    /// So the first font is the primary, and then comes a list of fallbacks in order of priority.
    pub type_fonts: BTreeMap<FontType, Vec<String>>,
}

impl FontDefinitions {
    #[cfg(feature = "system_fonts")]
    pub fn query_fonts_for_character(c: char) -> Option<FamilyHandle> {
        use font_kit::source::SystemSource;
        use skia_safe::{FontMgr, FontStyle};

        let source = SystemSource::new();
        let font_mgr = FontMgr::new();

        if let Some(typeface) =
            font_mgr.match_family_style_character("", FontStyle::normal(), &[], c as i32)
        {
            let family_name = typeface.family_name();
            if let Ok(fonts) = source.select_family_by_name(&family_name) {
                return Some(fonts);
            }
        }
        None
    }
}

impl Default for FontDefinitions {
    fn default() -> Self {
        #[allow(unused)]
        let mut font_data_map: BTreeMap<String, FontData> = BTreeMap::new();

        let mut type_fonts = BTreeMap::new();

        #[cfg(feature = "default_fonts")]
        {
            font_data_map.insert(
                "Hack".to_owned(),
                FontData::from_static(include_bytes!("../../fonts/Hack-Regular.ttf")),
            );
            font_data_map.insert(
                "Ubuntu-Light".to_owned(),
                FontData::from_static(include_bytes!("../../fonts/Ubuntu-Light.ttf")),
            );

            // Some good looking emojis. Use as first priority:
            font_data_map.insert(
                "NotoEmoji-Regular".to_owned(),
                FontData::from_static(include_bytes!("../../fonts/NotoEmoji-Regular.ttf")),
            );

            // Bigger emojis, and more. <http://jslegers.github.io/emoji-icon-font/>:
            font_data_map.insert(
                "emoji-icon-font".to_owned(),
                FontData::from_static(include_bytes!("../../fonts/emoji-icon-font.ttf")).tweak(
                    FontTweak {
                        scale: 0.8,            // make it smaller
                        y_offset_factor: 0.07, // move it down slightly
                        y_offset: 0.0,
                    },
                ),
            );

            type_fonts.insert(
                FontType::Monospace,
                vec![
                    "Hack".to_owned(),
                    "Ubuntu-Light".to_owned(), // fallback for √ etc
                    "NotoEmoji-Regular".to_owned(),
                    "emoji-icon-font".to_owned(),
                ],
            );
            type_fonts.insert(
                FontType::Proportional,
                vec![
                    "Ubuntu-Light".to_owned(),
                    "NotoEmoji-Regular".to_owned(),
                    "emoji-icon-font".to_owned(),
                ],
            );
        }

        #[cfg(not(feature = "default_fonts"))]
        {
            families.insert(FontType::Monospace, vec![]);
            families.insert(FontType::Proportional, vec![]);
        }

        Self {
            font_data_map,
            type_fonts,
        }
    }
}

// ----------------------------------------------------------------------------

/// The collection of fonts used by `epaint`.
///
/// Required in order to paint text. Create one and reuse. Cheap to clone.
///
/// Each [`Fonts`] comes with a font atlas textures that needs to be used when painting.
///
/// If you are using `egui`, use `egui::Context::set_fonts` and `egui::Context::fonts`.
///
/// You need to call [`Self::begin_frame`] and [`Self::font_image_delta`] once every frame.
pub struct FontPaintManager(Arc<Mutex<FontManagerAndGallyCache>>);

impl FontPaintManager {
    /// Create a new [`Fonts`] for text layout.
    /// This call is expensive, so only create one [`Fonts`] and then reuse it.
    ///
    /// * `pixels_per_point`: how many physical pixels per logical "point".
    /// * `max_texture_side`: largest supported texture size (one side).
    pub fn new(
        pixels_per_point: f32,
        max_texture_side: usize,
        definitions: FontDefinitions,
    ) -> Self {
        let fonts_and_cache = FontManagerAndGallyCache {
            font_manager: FontsManager::new(pixels_per_point, max_texture_side, definitions),
            galley_cache: Default::default(),
        };
        Self(Arc::new(Mutex::new(fonts_and_cache)))
    }

    /// Call at the start of each frame with the latest known
    /// `pixels_per_point` and `max_texture_side`.
    ///
    /// Call after painting the previous frame, but before using [`Fonts`] for the new frame.
    ///
    /// This function will react to changes in `pixels_per_point` and `max_texture_side`,
    /// as well as notice when the font atlas is getting full, and handle that.
    pub fn begin_frame(&self, pixels_per_point: f32, max_texture_side: usize) {
        let mut fonts_and_cache = self.0.lock();

        let pixels_per_point_changed =
            (fonts_and_cache.font_manager.pixels_per_point - pixels_per_point).abs() > 1e-3;
        let max_texture_side_changed =
            fonts_and_cache.font_manager.max_texture_side != max_texture_side;
        let font_atlas_almost_full = fonts_and_cache.font_manager.atlas.lock().fill_ratio() > 0.8;
        let needs_recreate =
            pixels_per_point_changed || max_texture_side_changed || font_atlas_almost_full;

        if needs_recreate {
            let definitions = fonts_and_cache.font_manager.definitions.clone();

            *fonts_and_cache = FontManagerAndGallyCache {
                font_manager: FontsManager::new(pixels_per_point, max_texture_side, definitions),
                galley_cache: Default::default(),
            };
        }

        fonts_and_cache.galley_cache.flush_cache();
    }

    /// Call at the end of each frame (before painting) to get the change to the font texture since last call.
    pub fn font_image_delta(&self) -> Option<crate::ImageDelta> {
        self.lock().font_manager.atlas.lock().take_delta()
    }

    /// Access the underlying [`FontsAndCache`].
    #[doc(hidden)]
    #[inline]
    pub fn lock(&self) -> MutexGuard<'_, FontManagerAndGallyCache> {
        self.0.lock()
    }

    #[inline]
    pub fn pixels_per_point(&self) -> f32 {
        self.lock().font_manager.pixels_per_point
    }

    #[inline]
    pub fn max_texture_side(&self) -> usize {
        self.lock().font_manager.max_texture_side
    }

    /// The font atlas.
    /// Pass this to [`crate::Tessellator`].
    pub fn texture_atlas(&self) -> Arc<Mutex<TextureAtlas>> {
        self.lock().font_manager.atlas.clone()
    }

    /// Current size of the font image.
    /// Pass this to [`crate::Tessellator`].
    pub fn font_image_size(&self) -> [usize; 2] {
        self.lock().font_manager.atlas.lock().size()
    }

    /// Width of this character in points.
    #[inline]
    pub fn glyph_width(&self, font_id: &FontId, c: char) -> f32 {
        self.lock().font_manager.glyph_width(font_id, c)
    }

    /// Height of one row of text in points
    #[inline]
    pub fn row_height(&self, font_id: &FontId) -> f32 {
        self.lock().font_manager.row_height(font_id)
    }

    /// List of all known font families.
    pub fn families(&self) -> Vec<FontType> {
        self.lock()
            .font_manager
            .definitions
            .type_fonts
            .keys()
            .cloned()
            .collect()
    }

    /// Layout some text.
    ///
    /// This is the most advanced layout function.
    /// See also [`Self::layout`], [`Self::layout_no_wrap`] and
    /// [`Self::layout_delayed_color`].
    ///
    /// The implementation uses memoization so repeated calls are cheap.
    #[inline]
    pub fn layout_job(&self, job: LayoutJob) -> Arc<Galley> {
        self.lock().layout_job(job)
    }

    pub fn num_galleys_in_cache(&self) -> usize {
        self.lock().galley_cache.num_galleys_in_cache()
    }

    /// How full is the font atlas?
    ///
    /// This increases as new fonts and/or glyphs are used,
    /// but can also decrease in a call to [`Self::begin_frame`].
    pub fn font_atlas_fill_ratio(&self) -> f32 {
        self.lock().font_manager.atlas.lock().fill_ratio()
    }

    /// Will wrap text at the given width and line break at `\n`.
    ///
    /// The implementation uses memoization so repeated calls are cheap.
    pub fn layout(
        &self,
        text: String,
        font_id: FontId,
        color: crate::Color32,
        wrap_width: f32,
    ) -> Arc<Galley> {
        let job = LayoutJob::simple(text, font_id, color, wrap_width);
        self.layout_job(job)
    }

    /// Will line break at `\n`.
    ///
    /// The implementation uses memoization so repeated calls are cheap.
    pub fn layout_no_wrap(
        &self,
        text: String,
        font_id: FontId,
        color: crate::Color32,
    ) -> Arc<Galley> {
        let job = LayoutJob::simple(text, font_id, color, f32::INFINITY);
        self.layout_job(job)
    }

    /// Like [`Self::layout`], made for when you want to pick a color for the text later.
    ///
    /// The implementation uses memoization so repeated calls are cheap.
    pub fn layout_delayed_color(
        &self,
        text: String,
        font_id: FontId,
        wrap_width: f32,
    ) -> Arc<Galley> {
        self.layout_job(LayoutJob::simple(
            text,
            font_id,
            crate::Color32::TEMPORARY_COLOR,
            wrap_width,
        ))
    }
}

// ----------------------------------------------------------------------------

pub struct FontManagerAndGallyCache {
    pub font_manager: FontsManager,
    galley_cache: GalleyCache,
}

impl FontManagerAndGallyCache {
    fn layout_job(&mut self, job: LayoutJob) -> Arc<Galley> {
        self.galley_cache.layout(&mut self.font_manager, job)
    }
}

// ----------------------------------------------------------------------------

/// The collection of fonts used by `epaint`.
///
/// Required in order to paint text.
pub struct FontsManager {
    pixels_per_point: f32,
    max_texture_side: usize,
    definitions: FontDefinitions,
    atlas: Arc<Mutex<TextureAtlas>>,
    fonts_impl_cache: FontsImplCache,
    font_impl_manager_map: ahash::AHashMap<(u32, FontType), FontImplManager>,
}

impl FontsManager {
    /// Create a new [`FontsImpl`] for text layout.
    /// This call is expensive, so only create one [`FontsImpl`] and then reuse it.
    pub fn new(
        pixels_per_point: f32,
        max_texture_side: usize,
        definitions: FontDefinitions,
    ) -> Self {
        assert!(
            0.0 < pixels_per_point && pixels_per_point < 100.0,
            "pixels_per_point out of range: {}",
            pixels_per_point
        );

        let texture_width = max_texture_side.at_most(8 * 1024);
        let initial_height = 64;
        let atlas = TextureAtlas::new([texture_width, initial_height]);

        let atlas = Arc::new(Mutex::new(atlas));

        let font_impl_cache =
            FontsImplCache::new(atlas.clone(), pixels_per_point, &definitions.font_data_map);

        Self {
            pixels_per_point,
            max_texture_side,
            definitions,
            atlas,
            fonts_impl_cache: font_impl_cache,
            font_impl_manager_map: Default::default(),
        }
    }

    #[inline(always)]
    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    #[inline]
    pub fn definitions(&self) -> &FontDefinitions {
        &self.definitions
    }

    pub fn definitions_mut(&mut self) -> &mut FontDefinitions {
        &mut self.definitions
    }

    /// Get the right font implementation from size and [`FontFamily`].
    pub fn font(&mut self, font_id: &FontId) -> &mut FontImplManager {
        let FontId { size, font_type } = font_id;
        let scale_in_pixels = self.fonts_impl_cache.scale_as_pixels(*size);

        self.font_impl_manager_map
            .entry((scale_in_pixels, font_type.clone()))
            .or_insert_with(|| {
                let fonts = &self.definitions.type_fonts.get(font_type);

                let fonts = fonts.unwrap_or_else(|| {
                    panic!("FontType::{:?} is not bound to any fonts", font_type)
                });

                println!("font_type:{:?} fonts:{:?}", font_type, fonts);

                let fonts: Vec<Arc<FontImpl>> = fonts
                    .iter()
                    .map(|font_name| self.fonts_impl_cache.font_impl(scale_in_pixels, font_name))
                    .collect();

                FontImplManager::new(fonts)
            })
    }

    /// Width of this character in points.
    fn glyph_width(&mut self, font_id: &FontId, c: char) -> f32 {
        self.font(font_id).glyph_width(c)
    }

    /// Height of one row of text. In points
    fn row_height(&mut self, font_id: &FontId) -> f32 {
        self.font(font_id).row_height()
    }

    #[cfg(feature = "system_fonts")]
    pub fn ensure_correct_fonts_for_text(&mut self, text: &str, main_font_id: &FontId) {
        use font_kit::handle::Handle;
        let FontId { size, font_type: _ } = main_font_id;
        let scale_in_pixels = self.fonts_impl_cache.scale_as_pixels(*size);

        let mut font_impl_manager = self.font(main_font_id);
        for c in text.chars() {
            if font_impl_manager.has_glyph_info_and_cache(c) {
                continue;
            }
            if let Some(fonts) = FontDefinitions::query_fonts_for_character(c) {
                for font in fonts.fonts() {
                    if let Handle::Path {
                        path,
                        font_index: _,
                    } = font
                    {
                        if let Ok(buf) = fs::read(path) {
                            let new_font_name =
                                path.file_name().unwrap().to_str().unwrap().to_string();
                            // update FontData
                            let font_data = self
                                .definitions
                                .font_data_map
                                .entry(new_font_name.clone())
                                .or_insert_with(|| FontData::from_owned(buf));

                            self.definitions
                                .type_fonts
                                .entry(FontType::Monospace)
                                .or_default()
                                .push(new_font_name.clone());
                            self.definitions
                                .type_fonts
                                .entry(FontType::Proportional)
                                .or_default()
                                .push(new_font_name.clone());
                            // update fonts_impl_cache
                            let ab_glyph = ab_glyph_font_from_font_data(&new_font_name, font_data);
                            let tweak = font_data.tweak;
                            self.fonts_impl_cache
                                .ab_glyph_fonts
                                .insert(new_font_name.clone(), (tweak, ab_glyph));
                            // update fonts_impl_cache
                            let new_font_impl = self
                                .fonts_impl_cache
                                .font_impl(scale_in_pixels, &new_font_name);
                            font_impl_manager = self.font(main_font_id);
                            font_impl_manager.push_font_impl(new_font_impl);
                        }
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------

struct CachedGalley {
    /// When it was last used
    last_used: u32,
    galley: Arc<Galley>,
}

#[derive(Default)]
struct GalleyCache {
    /// Frame counter used to do garbage collection on the cache
    generation: u32,
    cache: nohash_hasher::IntMap<u64, CachedGalley>,
}

impl GalleyCache {
    fn layout(&mut self, fonts: &mut FontsManager, job: LayoutJob) -> Arc<Galley> {
        let hash = crate::util::hash(&job); // TODO: even faster hasher?

        match self.cache.entry(hash) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                let cached = entry.into_mut();
                cached.last_used = self.generation;
                cached.galley.clone()
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let galley = super::layout(fonts, job.into());
                let galley = Arc::new(galley);
                entry.insert(CachedGalley {
                    last_used: self.generation,
                    galley: galley.clone(),
                });
                galley
            }
        }
    }

    pub fn num_galleys_in_cache(&self) -> usize {
        self.cache.len()
    }

    /// Must be called once per frame to clear the [`Galley`] cache.
    pub fn flush_cache(&mut self) {
        let current_generation = self.generation;
        self.cache.retain(|_key, cached| {
            cached.last_used == current_generation // only keep those that were used this frame
        });
        self.generation = self.generation.wrapping_add(1);
    }
}

// ----------------------------------------------------------------------------

struct FontsImplCache {
    atlas: Arc<Mutex<TextureAtlas>>,
    pixels_per_point: f32,
    ab_glyph_fonts: BTreeMap<String, (FontTweak, ab_glyph::FontArc)>,

    /// Map font pixel sizes and names to the cached [`FontImpl`].
    cache: ahash::AHashMap<(u32, String), Arc<FontImpl>>,
}

impl FontsImplCache {
    pub fn new(
        atlas: Arc<Mutex<TextureAtlas>>,
        pixels_per_point: f32,
        font_data: &BTreeMap<String, FontData>,
    ) -> Self {
        let ab_glyph_fonts = font_data
            .iter()
            .map(|(name, font_data)| {
                let tweak = font_data.tweak;
                let ab_glyph = ab_glyph_font_from_font_data(name, font_data);
                (name.clone(), (tweak, ab_glyph))
            })
            .collect();

        Self {
            atlas,
            pixels_per_point,
            ab_glyph_fonts,
            cache: Default::default(),
        }
    }

    #[inline]
    pub fn scale_as_pixels(&self, scale_in_points: f32) -> u32 {
        let scale_in_pixels = self.pixels_per_point * scale_in_points;

        // Round to an even number of physical pixels to get even kerning.
        // See https://github.com/emilk/egui/issues/382
        scale_in_pixels.round() as u32
    }

    pub fn font_impl(&mut self, scale_in_pixels: u32, font_name: &str) -> Arc<FontImpl> {
        let (tweak, ab_glyph_font) = self
            .ab_glyph_fonts
            .get(font_name)
            .unwrap_or_else(|| panic!("No font data found for {:?}", font_name))
            .clone();

        let scale_in_pixels = (scale_in_pixels as f32 * tweak.scale).round() as u32;

        let y_offset_points = {
            let scale_in_points = scale_in_pixels as f32 / self.pixels_per_point;
            scale_in_points * tweak.y_offset_factor
        } + tweak.y_offset;

        self.cache
            .entry((scale_in_pixels, font_name.to_owned()))
            .or_insert_with(|| {
                Arc::new(FontImpl::new(
                    self.atlas.clone(),
                    self.pixels_per_point,
                    font_name.to_owned(),
                    ab_glyph_font,
                    scale_in_pixels,
                    y_offset_points,
                ))
            })
            .clone()
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Text shaping for TrueType and OpenType fonts.
//!
//! Shaping turns a line of text into glyphs with positions, applying kerning, ligatures, mark
//! placement, and the scripts' own rules. It uses `harfrust`, a Rust port of HarfBuzz, the same
//! engine as egui's text layout, and reads outlines with `skrifa`.

use crate::error::{Error, Result};
use crate::font::{Font, Glyph, OutlineSink};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlineGlyphCollection};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

/// The bytes of a font file, shared between the loaded font and its shaper.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// One glyph chosen by the shaper, in the units of [`Glyph`] strokes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    /// The glyph's index in the font. Zero is the font's "not found" glyph.
    pub id: u32,
    /// The byte offset in the shaped text of the first character this glyph represents.
    pub cluster: usize,
    /// How far to move along the line after this glyph, including kerning.
    pub x_advance: f64,
    /// Offset of the glyph from the pen, to the right.
    pub x_offset: f64,
    /// Offset of the glyph from the baseline, upward; used by combining marks.
    pub y_offset: f64,
}

/// The optional features of shaping that can be turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeFeatures {
    /// Adjust the space between pairs of letters, as `AV` is set closer than `AH`.
    pub kerning: bool,
    /// Replace letter sequences with the font's ligatures, as `fi` with one glyph.
    pub ligatures: bool,
}

impl Default for ShapeFeatures {
    fn default() -> Self {
        Self {
            kerning: true,
            ligatures: true,
        }
    }
}

impl ShapeFeatures {
    /// The features to switch off, as HarfBuzz settings.
    fn disabled(self) -> Vec<harfrust::Feature> {
        let tags: &[&[u8; 4]] = match (self.kerning, self.ligatures) {
            (true, true) => &[],
            (false, true) => &[b"kern"],
            (true, false) => &[b"liga", b"clig"],
            (false, false) => &[b"kern", b"liga", b"clig"],
        };
        tags.iter()
            .map(|tag| harfrust::Feature::new(harfrust::Tag::new(tag), 0, ..))
            .collect()
    }
}

/// Reads glyph outlines by index.
pub(crate) struct OutlineReader<'a> {
    outlines: OutlineGlyphCollection<'a>,
    tol: f64,
    factor: f64,
}

impl<'a> OutlineReader<'a> {
    /// A reader that scales so that a capital `A` is nine units tall, and flattens curves to
    /// within `seg_arc` degrees.
    pub(crate) fn new(face: &FontRef<'a>, seg_arc: f64) -> Self {
        let mut reader = Self {
            outlines: face.outline_glyphs(),
            tol: seg_arc.max(0.01).to_radians(),
            factor: 1.0,
        };
        let probe = face
            .charmap()
            .map('A')
            .and_then(|id| reader.read(id))
            .and_then(|glyph| {
                let y_max = glyph.y_max();
                (y_max > 0.0).then_some(y_max)
            });
        let units_per_em = face
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
        let cap_height = probe.unwrap_or_else(|| f64::from(units_per_em) * 0.7);
        reader.factor = 9.0 / cap_height;
        reader
    }

    /// The same scaling as a reader built earlier, for the same font.
    fn with_factor(face: &FontRef<'a>, tol: f64, factor: f64) -> Self {
        Self {
            outlines: face.outline_glyphs(),
            tol,
            factor,
        }
    }

    /// The factor from font units to strokes units.
    pub(crate) fn factor(&self) -> f64 {
        self.factor
    }

    /// The tolerance, in radians, the curves are flattened to.
    pub(crate) fn tolerance(&self) -> f64 {
        self.tol
    }

    /// The scaled strokes of glyph `id`, or `None` if it has no outline, as a space.
    ///
    /// Contours are normalized so outer loops run clockwise.
    pub(crate) fn read(&self, id: GlyphId) -> Option<Glyph> {
        let mut sink = OutlineSink::new(self.tol);
        self.outlines
            .get(id)?
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                &mut sink,
            )
            .ok()?;
        let mut strokes = sink.strokes;
        if strokes.is_empty() {
            return None;
        }
        Font::normalize_winding(&mut strokes);
        for s in &mut strokes {
            s.x1 *= self.factor;
            s.y1 *= self.factor;
            s.x2 *= self.factor;
            s.y2 *= self.factor;
        }
        Some(Glyph { strokes })
    }
}

/// Shapes text with one face of a font and reads the outlines of the glyphs chosen.
#[derive(Clone)]
pub struct Shaper {
    inner: Arc<ShaperInner>,
}

struct ShaperInner {
    data: FontData,
    face: u32,
    tol: f64,
    factor: f64,
    shaper_data: harfrust::ShaperData,
    glyphs: Mutex<HashMap<u32, Arc<Glyph>>>,
}

impl fmt::Debug for Shaper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shaper")
            .field("face", &self.inner.face)
            .finish_non_exhaustive()
    }
}

/// Two shapers are equal if they are the same one, which is when they read the same font.
impl PartialEq for Shaper {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Shaper {
    /// A shaper for face `face` of the font in `data`, using `reader`'s scaling.
    pub(crate) fn new(data: FontData, face: u32, reader: &OutlineReader<'_>) -> Result<Self> {
        let font = Self::font_ref(&data, face)?;
        Ok(Self {
            inner: Arc::new(ShaperInner {
                shaper_data: harfrust::ShaperData::new(&font),
                tol: reader.tolerance(),
                factor: reader.factor(),
                data,
                face,
                glyphs: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub(crate) fn font_ref(data: &FontData, face: u32) -> Result<FontRef<'_>> {
        let bytes: &[u8] = (**data).as_ref();
        FontRef::from_index(bytes, face)
            .map_err(|e| Error::Font(format!("unable to parse font: {e}")))
    }

    /// Shapes one line of text, with the direction and script guessed from the text.
    ///
    /// Glyphs come in the order they are drawn from left to right. The line must not contain
    /// line breaks. `features` turns off kerning or ligatures.
    pub fn shape(&self, text: &str, features: ShapeFeatures) -> Vec<ShapedGlyph> {
        let Ok(font) = Self::font_ref(&self.inner.data, self.inner.face) else {
            return Vec::new();
        };
        let shaper = self.inner.shaper_data.shaper(&font).build();
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let disabled = features.disabled();
        let shaped = shaper.shape(buffer, harfrust::ShapeOptions::new().features(&disabled));
        let k = self.inner.factor;
        shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, pos)| ShapedGlyph {
                id: info.glyph_id,
                cluster: info.cluster as usize,
                x_advance: f64::from(pos.x_advance) * k,
                x_offset: f64::from(pos.x_offset) * k,
                y_offset: f64::from(pos.y_offset) * k,
            })
            .collect()
    }

    /// The strokes of glyph `id`; empty for a glyph without an outline.
    pub fn glyph(&self, id: u32) -> Arc<Glyph> {
        let cached = {
            let glyphs = self
                .inner
                .glyphs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            glyphs.get(&id).cloned()
        };
        if let Some(glyph) = cached {
            return glyph;
        }
        let read = Self::font_ref(&self.inner.data, self.inner.face)
            .ok()
            .and_then(|font| {
                OutlineReader::with_factor(&font, self.inner.tol, self.inner.factor)
                    .read(GlyphId::new(id))
            });
        let glyph = Arc::new(read.unwrap_or_default());
        self.inner
            .glyphs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, glyph.clone());
        glyph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    fn ubuntu() -> Font {
        Font::from_outline_font(
            epaint_default_fonts::UBUNTU_LIGHT,
            0,
            5.0,
            &Font::codes_in(1..=0xFF),
        )
        .unwrap()
    }

    fn advance_of(shaper: &Shaper, text: &str) -> f64 {
        shaper
            .shape(text, ShapeFeatures::default())
            .iter()
            .map(|g| g.x_advance)
            .sum()
    }

    #[test]
    fn kerning_tightens_a_pair() {
        let font = ubuntu();
        let shaper = font.shaper.as_ref().unwrap();
        let together = advance_of(shaper, "AV");
        let apart = advance_of(shaper, "A") + advance_of(shaper, "V");
        assert!(together < apart - 0.05, "{together} vs {apart}");
    }

    #[test]
    fn one_glyph_per_letter_and_clusters_are_byte_offsets() {
        let font = ubuntu();
        let shaped = font
            .shaper
            .as_ref()
            .unwrap()
            .shape("aé b", ShapeFeatures::default());
        let clusters: Vec<usize> = shaped.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, [0, 1, 3, 4]);
        assert!(shaped.iter().all(|g| g.id != 0));
    }

    #[test]
    fn a_character_the_font_lacks_shapes_to_the_not_found_glyph() {
        let font = ubuntu();
        let shaped = font
            .shaper
            .as_ref()
            .unwrap()
            .shape("\u{4e00}", ShapeFeatures::default());
        assert_eq!(shaped[0].id, 0);
    }

    #[test]
    fn glyph_outlines_are_read_by_index_and_cached() {
        let font = ubuntu();
        let shaper = font.shaper.as_ref().unwrap();
        let id = shaper.shape("H", ShapeFeatures::default())[0].id;
        let glyph = shaper.glyph(id);
        assert!(!glyph.strokes.is_empty());
        assert!(Arc::ptr_eq(&glyph, &shaper.glyph(id)));
        assert_eq!(*glyph, font.glyph('H').unwrap().clone());
        assert!(
            shaper
                .glyph(shaper.shape(" ", ShapeFeatures::default())[0].id)
                .strokes
                .is_empty()
        );
    }

    #[test]
    fn kerning_can_be_turned_off() {
        let font = ubuntu();
        let shaper = font.shaper.as_ref().unwrap();
        let off = ShapeFeatures {
            kerning: false,
            ..ShapeFeatures::default()
        };
        let kerned = advance_of(shaper, "AV");
        let unkerned: f64 = shaper.shape("AV", off).iter().map(|g| g.x_advance).sum();
        let apart = advance_of(shaper, "A") + advance_of(shaper, "V");
        assert!(unkerned > kerned + 0.05, "{unkerned} vs {kerned}");
        assert_abs_diff_eq!(unkerned, apart, epsilon = 1e-9);
    }

    #[test]
    fn ligatures_can_be_turned_off() {
        let font = ubuntu();
        let shaper = font.shaper.as_ref().unwrap();
        let off = ShapeFeatures {
            ligatures: false,
            ..ShapeFeatures::default()
        };
        assert_eq!(shaper.shape("fi", off).len(), 2);
        assert_eq!(shaper.shape("fi", ShapeFeatures::default()).len(), 1);
    }
}

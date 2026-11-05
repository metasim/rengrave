// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2011-2026 Scorch
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
//
// Ported from `parse` of F-Engrave 1.78 (CXF fonts), which credits cxf2cnc.py by Ben Lipkowitz. The
// TrueType code is independent of the GPL-2.0-only ttf2cxf_stream helper; see COPYRIGHT.md.

//! Stroke fonts: the CXF file format and TrueType/OpenType outlines.
//!
//! Every input the engine accepts, including traced images and DXF drawings, is reduced to a
//! [`Font`] whose glyphs are lists of straight strokes.

use crate::error::{Error, Result};
use crate::unicode_blocks::UnicodeBlock;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::Path;

/// A straight stroke in font units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    /// Start X.
    pub x1: f64,
    /// Start Y.
    pub y1: f64,
    /// End X.
    pub x2: f64,
    /// End Y.
    pub y2: f64,
}

impl Stroke {
    /// Creates a stroke from its endpoints.
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self { x1, y1, x2, y2 }
    }
}

/// The strokes that draw one character.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Glyph {
    /// Strokes in cutting order.
    pub strokes: Vec<Stroke>,
}

impl Glyph {
    /// Largest X coordinate, or zero for an empty glyph.
    pub fn x_max(&self) -> f64 {
        self.fold(f64::max, |s| s.x1.max(s.x2))
    }

    /// Largest Y coordinate, or zero for an empty glyph.
    pub fn y_max(&self) -> f64 {
        self.fold(f64::max, |s| s.y1.max(s.y2))
    }

    /// Smallest Y coordinate, or zero for an empty glyph.
    pub fn y_min(&self) -> f64 {
        self.fold(f64::min, |s| s.y1.min(s.y2))
    }

    fn fold(&self, pick: fn(f64, f64) -> f64, key: fn(&Stroke) -> f64) -> f64 {
        self.strokes.iter().map(key).reduce(pick).unwrap_or(0.0)
    }
}

/// The Unicode blocks a font has characters in; see [`Font::coverage`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontCoverage {
    /// The number of characters in Basic Latin and Latin-1 Supplement together.
    pub latin: usize,
    /// Every block with at least one character, in code point order, with the character count.
    pub blocks: Vec<(&'static UnicodeBlock, usize)>,
    /// Every code point the font maps to a glyph, in ascending order.
    codes: Vec<u32>,
}

impl FontCoverage {
    /// The characters of the font with code points in `range`, as text. Control characters, which
    /// cannot be shown or pasted usefully, are left out.
    pub fn chars_in(&self, range: RangeInclusive<u32>) -> String {
        let start = self.codes.partition_point(|&c| c < *range.start());
        let end = self.codes.partition_point(|&c| c <= *range.end());
        self.codes[start..end]
            .iter()
            .filter_map(|&c| char::from_u32(c))
            .filter(|c| !c.is_control())
            .collect()
    }
}

/// A character set mapping Unicode code points to glyphs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Font {
    /// Glyphs by code point, in ascending order. Only the characters that were asked for are
    /// loaded.
    pub glyphs: BTreeMap<u32, Glyph>,
    /// Every code point the font maps to a glyph, whether loaded or not, in ascending order.
    pub available: Vec<u32>,
}

impl Font {
    /// Returns `true` if the font contains no characters.
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Which Unicode blocks the font has characters in.
    pub fn coverage(&self) -> FontCoverage {
        let mut blocks: Vec<(&'static UnicodeBlock, usize)> = Vec::new();
        for &code in &self.available {
            let Some(block) = UnicodeBlock::containing(code) else {
                continue;
            };
            match blocks.last_mut() {
                Some((last, count)) if std::ptr::eq(*last, block) => *count += 1,
                _ => blocks.push((block, 1)),
            }
        }
        FontCoverage {
            latin: self.count_in(0..=0xFF),
            blocks,
            codes: self.available.clone(),
        }
    }

    /// The number of characters the font has among the code points in `range`.
    pub fn count_in(&self, range: RangeInclusive<u32>) -> usize {
        let start = self.available.partition_point(|&c| c < *range.start());
        let end = self.available.partition_point(|&c| c <= *range.end());
        end - start
    }

    /// Looks up the glyph for `ch`.
    pub fn glyph(&self, ch: char) -> Option<&Glyph> {
        self.glyphs.get(&(ch as u32))
    }

    /// Parses QCAD/LibreCAD `.cxf` font text.
    ///
    /// Arcs are approximated by a stroke every `seg_arc` degrees. Malformed stroke lines are
    /// skipped so that one bad record does not lose the font.
    pub fn parse_cxf(text: &str, seg_arc: f64) -> Self {
        let seg_arc = seg_arc.max(0.01);
        let mut font = Self::default();
        let mut key: Option<u32> = None;
        let mut strokes: Vec<Stroke> = Vec::new();

        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            if let Some(header) = Font::cxf_header(line) {
                if let Some(k) = key {
                    font.glyphs.insert(
                        k,
                        Glyph {
                            strokes: std::mem::take(&mut strokes),
                        },
                    );
                }
                strokes.clear();
                key = Font::cxf_key(header);
            } else if let Some(rest) = line.strip_prefix("L ") {
                if let [x1, y1, x2, y2] = Font::parse_floats(rest)[..] {
                    strokes.push(Stroke::new(x1, y1, x2, y2));
                }
            } else if let Some(rest) = line.strip_prefix("A ")
                && let [xc, yc, r, start, end] = Font::parse_floats(rest)[..]
            {
                Font::push_cxf_arc(&mut strokes, xc, yc, r, start, end, seg_arc);
            }
        }
        if let Some(k) = key {
            font.glyphs.insert(k, Glyph { strokes });
        }
        font.available = font.glyphs.keys().copied().collect();
        font
    }

    /// Reads the code points in `wanted` from face `face` of a TrueType or OpenType font in memory.
    ///
    /// Glyphs are scaled so that a capital `A` is nine units tall and curves are flattened to
    /// within `seg_arc` degrees. Contours are normalized so outer loops run clockwise. Code
    /// points the font does not contain, or that have no outline (a space), are skipped. `face`
    /// selects a font within a collection (`.ttc`) and is zero for a single font.
    pub fn from_outline_font(data: &[u8], face: u32, seg_arc: f64, wanted: &[u32]) -> Result<Self> {
        let face = FontRef::from_index(data, face)
            .map_err(|e| Error::Font(format!("unable to parse font: {e}")))?;
        let tol = seg_arc.max(0.01).to_radians();
        let charmap = face.charmap();
        let outlines = face.outline_glyphs();
        let draw = |id: GlyphId| -> Option<Vec<Stroke>> {
            let mut sink = OutlineSink::new(tol);
            outlines
                .get(id)?
                .draw(
                    DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                    &mut sink,
                )
                .ok()?;
            (!sink.strokes.is_empty()).then_some(sink.strokes)
        };

        let probe = charmap.map('A').and_then(draw).and_then(|strokes| {
            let y_max = strokes
                .iter()
                .map(|s| s.y1.max(s.y2))
                .fold(f64::NEG_INFINITY, f64::max);
            (y_max > 0.0).then_some(y_max)
        });
        let units_per_em = face
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
        let cap_height = probe.unwrap_or_else(|| f64::from(units_per_em) * 0.7);
        let factor = 9.0 / cap_height;

        let mut available: Vec<u32> = charmap
            .mappings()
            .map(|(code, _)| code)
            .filter(|&code| code != 0)
            .collect();
        available.sort_unstable();
        available.dedup();
        let mut font = Self {
            available,
            ..Self::default()
        };
        for &code in wanted {
            let Some(mut strokes) = charmap.map(code).and_then(draw) else {
                continue;
            };
            Font::normalize_winding(&mut strokes);
            for s in &mut strokes {
                s.x1 *= factor;
                s.y1 *= factor;
                s.x2 *= factor;
                s.y2 *= factor;
            }
            font.glyphs.insert(code, Glyph { strokes });
        }
        if font.is_empty() {
            return Err(Error::Font(
                "font has no outlines for the requested characters".into(),
            ));
        }
        Ok(font)
    }

    /// The code points in `range`, without the null character.
    pub fn codes_in(range: RangeInclusive<u32>) -> Vec<u32> {
        range.filter(|&c| c != 0).collect()
    }

    /// The code points to load by default: Latin-1 and every character of `text`.
    ///
    /// Loading only what the text needs keeps large fonts, such as CJK fonts with tens of
    /// thousands of glyphs, cheap to use.
    pub fn codes_for_text(text: &str) -> Vec<u32> {
        let mut codes: Vec<u32> = (1..256).chain(text.chars().map(u32::from)).collect();
        codes.sort_unstable();
        codes.dedup();
        codes
    }

    /// Loads a font from a `.cxf`, `.ttf`, `.otf`, or `.ttc` file.
    ///
    /// `face` selects a font within a collection and is ignored for other formats. `wanted` is
    /// ignored for `.cxf`, which is always read whole.
    pub fn load_file(path: &Path, face: u32, seg_arc: f64, wanted: &[u32]) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        match extension_upper(path).as_str() {
            "CXF" => Ok(Self::parse_cxf(&String::from_utf8_lossy(&bytes), seg_arc)),
            "TTF" | "OTF" | "TTC" | "OTC" => Self::from_outline_font(&bytes, face, seg_arc, wanted),
            other => Err(Error::Font(format!(
                "unsupported font file type {other:?}: {}",
                path.display()
            ))),
        }
    }

    /// Wraps a single drawing (a traced image or DXF) as a one-character font.
    ///
    /// The drawing is addressed by [`IMAGE_GLYPH`].
    pub fn from_drawing(strokes: Vec<Stroke>) -> Self {
        let mut font = Self::default();
        font.glyphs.insert(IMAGE_GLYPH as u32, Glyph { strokes });
        font.available = vec![IMAGE_GLYPH as u32];
        font
    }
}

/// The character under which drawings are stored by [`Font::from_drawing`].
pub const IMAGE_GLYPH: char = 'F';

/// The file's extension in upper case, or an empty string if it has none.
pub fn extension_upper(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default()
}

/// Collects a glyph outline as strokes, flattening curves adaptively.
struct OutlineSink {
    strokes: Vec<Stroke>,
    tol: f64,
    start: (f64, f64),
    cur: (f64, f64),
}

impl OutlineSink {
    fn new(tol: f64) -> Self {
        Self {
            strokes: Vec::new(),
            tol,
            start: (0.0, 0.0),
            cur: (0.0, 0.0),
        }
    }

    /// Appends strokes approximating a curve parameterized over `0..=1`.
    ///
    /// Each span is split in half until the two halves' chords turn by no more than half of the
    /// tolerance, which on a circular arc corresponds to the span's angle meeting the tolerance.
    /// The curve is first divided into a few spans, so that an inflection at a midpoint cannot
    /// make a curved span look straight.
    fn flatten(&mut self, eval: impl Fn(f64) -> (f64, f64)) {
        const INITIAL_SPANS: u32 = 4;
        const MAX_DEPTH: u32 = 12;
        let turn_limit = self.tol / 2.0;
        let mut pending: Vec<(f64, f64, u32)> = (0..INITIAL_SPANS)
            .rev()
            .map(|k| {
                let n = f64::from(INITIAL_SPANS);
                (f64::from(k) / n, f64::from(k + 1) / n, 0)
            })
            .collect();
        while let Some((lo, hi, depth)) = pending.pop() {
            let (a, c) = (eval(lo), eval(hi));
            let mid = (lo + hi) / 2.0;
            let b = eval(mid);
            let (ab, bc) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
            let turn = (ab.0 * bc.1 - ab.1 * bc.0)
                .atan2(ab.0 * bc.0 + ab.1 * bc.1)
                .abs();
            if turn > turn_limit && depth < MAX_DEPTH {
                pending.push((mid, hi, depth + 1));
                pending.push((lo, mid, depth + 1));
            } else {
                self.strokes.push(Stroke::new(a.0, a.1, c.0, c.1));
            }
        }
    }
}

impl OutlinePen for OutlineSink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = (f64::from(x), f64::from(y));
        self.cur = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = (f64::from(x), f64::from(y));
        self.strokes
            .push(Stroke::new(self.cur.0, self.cur.1, to.0, to.1));
        self.cur = to;
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (p0, c, to) = (
            self.cur,
            (f64::from(cx), f64::from(cy)),
            (f64::from(x), f64::from(y)),
        );
        self.flatten(|t| {
            let u = 1.0 - t;
            (
                u * u * p0.0 + 2.0 * t * u * c.0 + t * t * to.0,
                u * u * p0.1 + 2.0 * t * u * c.1 + t * t * to.1,
            )
        });
        self.cur = to;
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let p0 = self.cur;
        let c1 = (f64::from(c1x), f64::from(c1y));
        let c2 = (f64::from(c2x), f64::from(c2y));
        let to = (f64::from(x), f64::from(y));
        self.flatten(|t| {
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (
                a * p0.0 + b * c1.0 + c * c2.0 + d * to.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * to.1,
            )
        });
        self.cur = to;
    }

    fn close(&mut self) {
        if self.cur != self.start {
            self.strokes.push(Stroke::new(
                self.cur.0,
                self.cur.1,
                self.start.0,
                self.start.1,
            ));
        }
        self.cur = self.start;
    }
}

impl Font {
    /// Returns the text between the brackets of a `[x]` glyph header line.
    fn cxf_header(line: &str) -> Option<&str> {
        let inner = line.strip_prefix('[')?;
        let end = (0..inner.len()).rev().find(|&i| {
            inner.as_bytes()[i] == b']'
                && inner[i + 1..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })?;
        Some(&inner[..end])
    }

    /// Decodes a glyph header to a code point: a literal character, `0041`, or `#0041`.
    fn cxf_key(header: &str) -> Option<u32> {
        let mut chars = header.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Some(c as u32);
        }
        let hex = if header.chars().count() == 5 {
            header.get(header.char_indices().nth(1)?.0..)?
        } else {
            header
        };
        if hex.len() == 4 {
            u32::from_str_radix(hex, 16).ok()
        } else {
            None
        }
    }

    fn parse_floats(text: &str) -> Vec<f64> {
        text.split(',')
            .map(|t| t.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
            .collect::<Option<_>>()
            .unwrap_or_default()
    }

    fn push_cxf_arc(
        strokes: &mut Vec<Stroke>,
        xc: f64,
        yc: f64,
        r: f64,
        mut start: f64,
        end: f64,
        seg_arc: f64,
    ) {
        // CXF arcs are counter-clockwise; an end before the start wraps through zero.
        if end < start {
            start -= 360.0;
        }
        let segs = ((end - start) / seg_arc) as usize + 1;
        let incr = (end - start) / segs as f64;
        let mut x0 = start.to_radians().cos() * r + xc;
        let mut y0 = start.to_radians().sin() * r + yc;
        let mut angle = start;
        for _ in 0..segs {
            angle += incr;
            let x1 = angle.to_radians().cos() * r + xc;
            let y1 = angle.to_radians().sin() * r + yc;
            strokes.push(Stroke::new(x0, y0, x1, y1));
            x0 = x1;
            y0 = y1;
        }
    }

    /// Reverses a glyph whose outer contours run counter-clockwise (the CFF convention).
    fn normalize_winding(strokes: &mut [Stroke]) {
        let doubled_area: f64 = strokes.iter().map(|s| s.x1 * s.y2 - s.x2 * s.y1).sum();
        if doubled_area > 0.0 {
            strokes.reverse();
            for s in strokes.iter_mut() {
                std::mem::swap(&mut s.x1, &mut s.x2);
                std::mem::swap(&mut s.y1, &mut s.y2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    const CXF: &str = "# comment\n\n[A] 3\nL 0,0,1,9\nL 1,9,2,0\n\n[#0042]\nL 0,0,0,9\n\n[0043] C\nL 0,1,0,2 \nbad line\nL 1,x,2,3\n\n[toolong]\nL 9,9,9,9\n\n[D]\nL 5,5,6,6\n";

    #[test]
    fn coverage_lists_the_characters_of_a_range() {
        let font = Font::from_outline_font(
            epaint_default_fonts::HACK_REGULAR,
            0,
            5.0,
            &Font::codes_in(1..=0xFF),
        )
        .unwrap();
        let coverage = font.coverage();
        let basic = coverage.chars_in(0..=0x7F);
        assert!(basic.starts_with(' '), "{basic:?}");
        assert!(basic.contains('A') && !basic.contains('\u{e9}'));
        assert!(!basic.contains('\u{7f}'), "no control characters");
        assert!(coverage.chars_in(0xE9..=0xE9) == "\u{e9}");
        assert!(coverage.chars_in(0x4E00..=0x4E10).is_empty());
    }

    #[test]
    fn cxf_header_forms() {
        let font = Font::parse_cxf(CXF, 5.0);
        assert_eq!(font.glyph('A').unwrap().strokes.len(), 2);
        assert_eq!(font.glyph('B').unwrap().strokes.len(), 1);
        assert_eq!(font.glyph('C').unwrap().strokes.len(), 1);
        assert_eq!(font.glyphs.len(), 4, "invalid header must be dropped");
    }

    #[test]
    fn cxf_skips_non_finite_coordinates() {
        let font = Font::parse_cxf("[A]\nL 0,0,nan,1\nL 0,inf,1,1\nL 0,0,1,1\n", 5.0);
        assert_eq!(font.glyph('A').unwrap().strokes.len(), 1);
    }

    #[test]
    fn cxf_keeps_stroke_on_final_line() {
        let font = Font::parse_cxf("[A]\nL 0,0,1,1", 5.0);
        assert_eq!(font.glyph('A').unwrap().strokes.len(), 1);
    }

    #[test]
    fn cxf_arc_is_flattened_on_circle() {
        let font = Font::parse_cxf("[O]\nA 1,2,3,0,360\n", 10.0);
        let strokes = &font.glyph('O').unwrap().strokes;
        assert_eq!(strokes.len(), 37);
        for s in strokes {
            assert_abs_diff_eq!((s.x1 - 1.0).hypot(s.y1 - 2.0), 3.0, epsilon = 1e-9);
            assert_abs_diff_eq!((s.x2 - 1.0).hypot(s.y2 - 2.0), 3.0, epsilon = 1e-9);
        }
        for pair in strokes.windows(2) {
            assert_abs_diff_eq!(pair[0].x2, pair[1].x1, epsilon = 1e-12);
            assert_abs_diff_eq!(pair[0].y2, pair[1].y1, epsilon = 1e-12);
        }
    }

    #[test]
    fn cxf_arc_wraps_through_zero() {
        let font = Font::parse_cxf("[O]\nA 0,0,1,270,90\n", 180.0);
        let s = &font.glyph('O').unwrap().strokes;
        assert_abs_diff_eq!(s[0].x1, 0.0, epsilon = 1e-9);
        assert_abs_diff_eq!(s[0].y1, -1.0, epsilon = 1e-9);
        assert_abs_diff_eq!(s[s.len() - 1].y2, 1.0, epsilon = 1e-9);
        assert!(
            s.iter().all(|p| p.x1 >= -1e-9 && p.x2 >= -1e-9),
            "must pass through +X"
        );
    }

    #[test]
    fn glyph_extents() {
        let g = Glyph {
            strokes: vec![
                Stroke::new(0.0, -1.0, 2.0, 3.0),
                Stroke::new(5.0, 0.0, 1.0, 1.0),
            ],
        };
        assert_eq!(g.x_max(), 5.0);
        assert_eq!(g.y_max(), 3.0);
        assert_eq!(g.y_min(), -1.0);
        let empty = Glyph::default();
        assert_eq!(
            (empty.x_max(), empty.y_max(), empty.y_min()),
            (0.0, 0.0, 0.0)
        );
    }

    fn signed_area(strokes: &[Stroke]) -> f64 {
        strokes
            .iter()
            .map(|s| s.x1 * s.y2 - s.x2 * s.y1)
            .sum::<f64>()
            / 2.0
    }

    /// Loads one of the typefaces that egui ships, which tests use as fixtures.
    fn fixture(data: &'static [u8], seg_arc: f64, wanted: &[u32]) -> Font {
        Font::from_outline_font(data, 0, seg_arc, wanted).unwrap()
    }

    fn latin1() -> Vec<u32> {
        Font::codes_for_text("")
    }

    #[test]
    fn outline_fonts_scale_capital_to_nine_units() {
        for data in [
            epaint_default_fonts::UBUNTU_LIGHT,
            epaint_default_fonts::HACK_REGULAR,
        ] {
            let font = fixture(data, 5.0, &latin1());
            let a = font.glyph('A').unwrap();
            assert_abs_diff_eq!(a.y_max(), 9.0, epsilon = 1e-9);
            assert!(font.glyph('z').is_some());
        }
    }

    #[test]
    fn outline_contours_close_and_wind_clockwise() {
        let font = fixture(epaint_default_fonts::UBUNTU_LIGHT, 5.0, &latin1());
        for ch in ['A', 'O', 'g', '8'] {
            let strokes = &font.glyph(ch).unwrap().strokes;
            assert!(
                signed_area(strokes) < 0.0,
                "{ch} should have clockwise outer loops"
            );
            // Every contour closes: each stroke end is the start of some stroke.
            for s in strokes {
                assert!(
                    strokes
                        .iter()
                        .any(|o| (o.x1 - s.x2).abs() < 1e-9 && (o.y1 - s.y2).abs() < 1e-9)
                );
            }
        }
    }

    #[test]
    fn normalization_reverses_counter_clockwise_contours() {
        let mut square = vec![
            Stroke::new(0.0, 0.0, 1.0, 0.0),
            Stroke::new(1.0, 0.0, 1.0, 1.0),
            Stroke::new(1.0, 1.0, 0.0, 1.0),
            Stroke::new(0.0, 1.0, 0.0, 0.0),
        ];
        assert!(signed_area(&square) > 0.0);
        Font::normalize_winding(&mut square);
        assert!(signed_area(&square) < 0.0);
        assert_eq!(square[0], Stroke::new(0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn flattening_stays_within_the_angle_tolerance_on_a_circle() {
        let mut sink = OutlineSink::new(10f64.to_radians());
        sink.flatten(|t| {
            let a = t * std::f64::consts::TAU;
            (5.0 * a.cos(), 5.0 * a.sin())
        });
        assert!(sink.strokes.len() >= 36, "{}", sink.strokes.len());
        for w in sink.strokes.windows(2) {
            assert_eq!((w[0].x2, w[0].y2), (w[1].x1, w[1].y1), "strokes connect");
            let a = (w[0].y2 - w[0].y1).atan2(w[0].x2 - w[0].x1);
            let b = (w[1].y2 - w[1].y1).atan2(w[1].x2 - w[1].x1);
            let turn = (b - a).sin().atan2((b - a).cos()).abs();
            assert!(turn <= 10.5f64.to_radians(), "{}", turn.to_degrees());
        }
        let (f, l) = (sink.strokes[0], sink.strokes[sink.strokes.len() - 1]);
        assert!((f.x1 - l.x2).abs() < 1e-9 && (f.y1 - l.y2).abs() < 1e-9);
    }

    #[test]
    fn flattening_a_straight_curve_stays_minimal() {
        let mut sink = OutlineSink::new(5f64.to_radians());
        sink.flatten(|t| (t, 2.0 * t));
        assert_eq!(sink.strokes.len(), 4);
    }

    #[test]
    fn finer_tolerance_gives_more_strokes() {
        let ubuntu = epaint_default_fonts::UBUNTU_LIGHT;
        let coarse = fixture(ubuntu, 45.0, &latin1());
        let fine = fixture(ubuntu, 2.0, &latin1());
        let count = |f: &Font| f.glyph('O').unwrap().strokes.len();
        assert!(count(&fine) > count(&coarse));
    }

    #[test]
    fn only_wanted_code_points_are_loaded() {
        let ubuntu = epaint_default_fonts::UBUNTU_LIGHT;
        let few = fixture(ubuntu, 30.0, &Font::codes_for_text(""));
        assert_eq!(few.glyphs.len(), 189);
        let more = fixture(ubuntu, 30.0, &Font::codes_for_text("A\u{3b1}\u{3b2}"));
        assert_eq!(more.glyphs.len(), few.glyphs.len() + 2);
        assert!(more.glyph('\u{3b1}').is_some());
    }

    #[test]
    fn codes_for_text_are_sorted_and_unique() {
        let codes = Font::codes_for_text("zz\u{3b1}a");
        assert!(codes.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(codes.first(), Some(&1));
        assert_eq!(codes.last(), Some(&0x3b1));
    }

    #[test]
    fn invalid_font_data_is_an_error() {
        assert!(Font::from_outline_font(b"not a font", 0, 5.0, &[65]).is_err());
    }
}

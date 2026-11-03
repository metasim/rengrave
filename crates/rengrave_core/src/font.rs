// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2011-2026 Scorch
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
//
// Ported from `parse` of F-Engrave 1.78 (CXF fonts), which credits cxf2cnc.py by Ben Lipkowitz. The
// TrueType code is independent of the GPL-2.0-only ttf2cxf_stream helper; see COPYRIGHT.md.

//! Stroke fonts: the CXF file format, TrueType/OpenType outlines, and embedded typefaces.
//!
//! Every input the engine accepts, including traced images and DXF drawings, is reduced to a
//! [`Font`] whose glyphs are lists of straight strokes.

use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// A character set mapping Unicode code points to glyphs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Font {
    /// Glyphs by code point, in ascending order.
    pub glyphs: BTreeMap<u32, Glyph>,
}

/// A typeface compiled into the program, available without any font files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFont {
    /// Ubuntu Light, a proportional sans-serif.
    UbuntuLight,
    /// Hack, a monospaced sans-serif.
    Hack,
}

impl BuiltinFont {
    /// All embedded typefaces.
    pub const ALL: [Self; 2] = [Self::UbuntuLight, Self::Hack];

    /// Display name, also the identifier accepted by [`crate::Settings::font_file`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::UbuntuLight => "Ubuntu Light",
            Self::Hack => "Hack",
        }
    }

    /// Finds an embedded typeface by its [`BuiltinFont::name`], ignoring case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name.trim()))
    }

    const fn data(self) -> &'static [u8] {
        match self {
            Self::UbuntuLight => epaint_default_fonts::UBUNTU_LIGHT,
            Self::Hack => epaint_default_fonts::HACK_REGULAR,
        }
    }

    /// Loads the typeface, flattening curves to within `seg_arc` degrees.
    pub fn load(self, seg_arc: f64, extended_chars: bool) -> Result<Font> {
        Font::from_outline_font(self.data(), seg_arc, extended_chars)
    }
}

impl Font {
    /// Returns `true` if the font contains no characters.
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
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
        font
    }

    /// Reads a TrueType or OpenType font from memory.
    ///
    /// Glyphs are scaled so that a capital `A` is nine units tall and curves are flattened to
    /// within `seg_arc` degrees. Latin-1 characters are loaded unless `extended_chars` asks for
    /// every code point in the font. Contours are normalized so outer loops run clockwise.
    pub fn from_outline_font(data: &[u8], seg_arc: f64, extended_chars: bool) -> Result<Self> {
        let face = ttf_parser::Face::parse(data, 0)
            .map_err(|e| Error::Font(format!("unable to parse font: {e}")))?;
        let tol = seg_arc.max(0.01).to_radians();

        let probe = face.glyph_index('A').and_then(|id| {
            let mut sink = OutlineSink::new(tol);
            face.outline_glyph(id, &mut sink)?;
            let y_max = sink
                .strokes
                .iter()
                .map(|s| s.y1.max(s.y2))
                .fold(f64::NEG_INFINITY, f64::max);
            (y_max > 0.0).then_some(y_max)
        });
        let cap_height = probe.unwrap_or_else(|| f64::from(face.units_per_em()) * 0.7);
        let factor = 9.0 / cap_height;

        let mut font = Self::default();
        let mut chars: Vec<u32> = Vec::new();
        if extended_chars {
            if let Some(cmap) = face.tables().cmap {
                for sub in cmap.subtables.into_iter().filter(|s| s.is_unicode()) {
                    sub.codepoints(|c| chars.push(c));
                }
            }
            chars.sort_unstable();
            chars.dedup();
        } else {
            chars.extend(1..256);
        }
        for code in chars {
            let Some(id) = char::from_u32(code).and_then(|c| face.glyph_index(c)) else {
                continue;
            };
            let mut sink = OutlineSink::new(tol);
            if face.outline_glyph(id, &mut sink).is_none() {
                continue;
            }
            let mut strokes = sink.strokes;
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
            return Err(Error::Font("font contains no outline glyphs".into()));
        }
        Ok(font)
    }

    /// Loads a font from a `.cxf`, `.ttf`, or `.otf` file.
    pub fn load_file(path: &Path, seg_arc: f64, extended_chars: bool) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        match extension_upper(path).as_str() {
            "CXF" => Ok(Self::parse_cxf(&String::from_utf8_lossy(&bytes), seg_arc)),
            "TTF" | "OTF" => Self::from_outline_font(&bytes, seg_arc, extended_chars),
            other => Err(Error::Font(format!(
                "unsupported font file type {other:?}: {}",
                path.display()
            ))),
        }
    }

    /// Lists the `.cxf`, `.ttf`, and `.otf` files directly inside `dir`, sorted by name.
    pub fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
        let entries = std::fs::read_dir(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| matches!(extension_upper(p).as_str(), "CXF" | "TTF" | "OTF"))
            .collect();
        files.sort();
        Ok(files)
    }

    /// Wraps a single drawing (a traced image or DXF) as a one-character font.
    ///
    /// The drawing is addressed by [`IMAGE_GLYPH`].
    pub fn from_drawing(strokes: Vec<Stroke>) -> Self {
        let mut font = Self::default();
        font.glyphs.insert(IMAGE_GLYPH as u32, Glyph { strokes });
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

impl ttf_parser::OutlineBuilder for OutlineSink {
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

    #[test]
    fn builtin_fonts_scale_capital_to_nine_units() {
        for builtin in BuiltinFont::ALL {
            let font = builtin.load(5.0, false).unwrap();
            let a = font.glyph('A').unwrap();
            assert_abs_diff_eq!(a.y_max(), 9.0, epsilon = 1e-9);
            assert!(font.glyph('z').is_some());
        }
    }

    #[test]
    fn outline_contours_close_and_wind_clockwise() {
        let font = BuiltinFont::UbuntuLight.load(5.0, false).unwrap();
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
        let coarse = BuiltinFont::UbuntuLight.load(45.0, false).unwrap();
        let fine = BuiltinFont::UbuntuLight.load(2.0, false).unwrap();
        let count = |f: &Font| f.glyph('O').unwrap().strokes.len();
        assert!(count(&fine) > count(&coarse));
    }

    #[test]
    fn extended_loading_adds_code_points() {
        let latin = BuiltinFont::UbuntuLight.load(30.0, false).unwrap();
        let all = BuiltinFont::UbuntuLight.load(30.0, true).unwrap();
        assert!(all.glyphs.len() > latin.glyphs.len());
    }

    #[test]
    fn invalid_font_data_is_an_error() {
        assert!(Font::from_outline_font(b"not a font", 5.0, false).is_err());
    }

    #[test]
    fn builtin_lookup_ignores_case() {
        assert_eq!(BuiltinFont::from_name(" hack "), Some(BuiltinFont::Hack));
        assert_eq!(BuiltinFont::from_name("nope"), None);
    }
}

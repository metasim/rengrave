//! Lays out text or a drawing as positioned outline segments.

use crate::error::{Error, Result};
use crate::font::{Font, IMAGE_GLYPH};
use crate::geom::{Bounds, Point, Segment, ZERO};
use crate::settings::{CutType, HeightCalc, InputType, Justify, Origin, Settings};

/// The positioned outline of a job, ready for toolpath generation.
#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    /// Outline segments in machine coordinates.
    pub segments: Vec<Segment>,
    /// Extents of the design in machine coordinates, including any box.
    pub bounds: Bounds,
    /// Radius of the circle added around circular text; zero if none.
    pub circle_radius: f64,
    /// Center of that circle, in machine coordinates.
    pub circle_center: Point,
    /// Characters in the text that the font does not contain.
    pub missing_chars: Vec<char>,
}

/// Extents of the glyphs that determine scale.
struct LineMetrics {
    height: f64,
    depth: f64,
}

impl Design {
    /// Maps `(x, y)` onto a circle of the given radius, then rotates by `angle` degrees.
    ///
    /// Returns the mapped point and the angle along the circle in radians.
    fn wrap_rotate(x: f64, y: f64, angle: f64, radius: f64) -> (f64, f64, f64) {
        let (xx, yy, alpha) = if radius != 0.0 {
            let alpha = x / radius;
            (
                (radius + y) * alpha.sin(),
                (radius + y) * alpha.cos(),
                alpha,
            )
        } else {
            (x, y, 0.0)
        };
        let rad = (xx * xx + yy * yy).sqrt();
        let theta = yy.atan2(xx);
        let new_angle = theta + angle.to_radians();
        (rad * new_angle.cos(), rad * new_angle.sin(), alpha)
    }

    /// Positions the text or drawing described by `settings` using the glyphs in `font`.
    ///
    /// Fails if the font lacks every requested character. Individually missing characters are
    /// reported in [`Design::missing_chars`] and skipped.
    pub fn layout(font: &Font, settings: &Settings) -> Result<Self> {
        settings.ensure_valid()?;
        if font.is_empty() {
            return Err(Error::NothingToCut(match settings.input_type {
                InputType::Text => "No font characters loaded".into(),
                InputType::Image => "No image loaded".into(),
            }));
        }
        let is_text = settings.input_type == InputType::Text;
        let radius_in = if is_text && !settings.show_font_index {
            settings.text_radius
        } else {
            0.0
        };
        let text = Self::text_to_lay_out(font, settings);
        let thick = if settings.cut_type == CutType::VCarve {
            0.0
        } else {
            settings.stroke_thickness
        };

        let metrics = Self::line_metrics(font, &text, settings.height_calc).ok_or_else(|| {
            Error::NothingToCut(if is_text {
                "Input characters were not found in the current font".into()
            } else {
                "Image contains no design information".into()
            })
        })?;

        let y_scale = if settings.use_image_size && !is_text {
            settings.y_scale / 100.0
        } else {
            let span = metrics.height - metrics.depth;
            let scale = if span != 0.0 {
                (settings.y_scale - thick) / span
            } else {
                0.1
            };
            if scale <= ZERO { 0.1 } else { scale }
        };
        let x_scale = settings.x_scale * y_scale / 100.0;

        let char_width = font
            .glyphs
            .values()
            .map(|g| g.x_max())
            .fold(f64::NEG_INFINITY, f64::max);
        let word_space = char_width * (settings.word_space / 100.0);
        let char_space = char_width * (settings.char_space / 100.0);
        let line_space = (metrics.height - metrics.depth + thick / y_scale) * settings.line_space;

        let radius = if radius_in == 0.0 {
            0.0
        } else {
            match (settings.outer, settings.upper) {
                (true, true) => radius_in + thick / 2.0 + y_scale * (-metrics.depth),
                (true, false) => -radius_in - thick / 2.0 - y_scale * metrics.height,
                (false, true) => radius_in - thick / 2.0 - y_scale * metrics.height,
                (false, false) => -radius_in + thick / 2.0 + y_scale * (-metrics.depth),
            }
        };

        // Place strokes, tracking the extent of each line.
        let mut segments: Vec<Segment> = Vec::new();
        let mut line_max_x: Vec<f64> = Vec::new();
        let mut missing_chars: Vec<char> = Vec::new();
        let (mut x_pos, mut y_pos) = (0.0_f64, 0.0_f64);
        let mut cur_line_max = f64::NEG_INFINITY;
        let mut line = 0_usize;
        for (index, ch) in text.chars().enumerate() {
            match ch {
                ' ' => x_pos += word_space,
                '\t' => x_pos += 3.0 * word_space,
                '\n' => {
                    x_pos = 0.0;
                    y_pos += line_space;
                    line += 1;
                    line_max_x.push(cur_line_max);
                    cur_line_max = f64::NEG_INFINITY;
                }
                _ => {
                    let Some(glyph) = font.glyph(ch) else {
                        if !missing_chars.contains(&ch) {
                            missing_chars.push(ch);
                        }
                        continue;
                    };
                    for stroke in &glyph.strokes {
                        let x1 = (stroke.x1 + x_pos) * x_scale;
                        let y1 = (stroke.y1 - y_pos) * y_scale;
                        let x2 = (stroke.x2 + x_pos) * x_scale;
                        let y2 = (stroke.y2 - y_pos) * y_scale;
                        segments.push(Segment::new(x1, y1, x2, y2, line, index + 1));
                        cur_line_max = cur_line_max.max(x1).max(x2);
                    }
                    x_pos += char_space + glyph.x_max();
                }
            }
        }
        line_max_x.push(cur_line_max);

        // Justify lines against the longest line.
        let longest = line_max_x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let shift = match settings.justify {
            Justify::Left => 0.0,
            Justify::Center => 0.5,
            Justify::Right => 1.0,
        };
        if shift != 0.0 {
            for s in &mut segments {
                let dx = (longest - line_max_x[s.line]) * shift;
                s.x1 += dx;
                s.x2 += dx;
            }
        }

        // Wrap onto a circle.
        if radius != 0.0 {
            let (mut min_a, mut max_a) = (f64::INFINITY, f64::NEG_INFINITY);
            for s in &mut segments {
                let (x1, y1, a1) = Self::wrap_rotate(s.x1, s.y1, 0.0, radius);
                let (x2, y2, a2) = Self::wrap_rotate(s.x2, s.y2, 0.0, radius);
                (s.x1, s.y1, s.x2, s.y2) = (x1, y1, x2, y2);
                max_a = max_a.max(a1).max(a2);
                min_a = min_a.min(a1).min(a2);
            }
            let spin = match settings.justify {
                Justify::Left => None,
                Justify::Center => Some((min_a + max_a) / 2.0),
                Justify::Right => Some(if settings.upper { max_a } else { min_a }),
            };
            if let Some(angle) = spin {
                for s in &mut segments {
                    let a = Point::new(s.x1, s.y1).rotated(angle);
                    let b = Point::new(s.x2, s.y2).rotated(angle);
                    (s.x1, s.y1, s.x2, s.y2) = (a.x, a.y, b.x, b.y);
                }
            }
        }

        // Rotate, mirror, and flip.
        let mut bounds = Bounds::EMPTY;
        let mut max_r2 = 0.0_f64;
        for s in &mut segments {
            if settings.angle != 0.0 {
                (s.x1, s.y1, _) = Self::wrap_rotate(s.x1, s.y1, settings.angle, 0.0);
                (s.x2, s.y2, _) = Self::wrap_rotate(s.x2, s.y2, settings.angle, 0.0);
            }
            if settings.mirror {
                s.x1 = -s.x1;
                s.x2 = -s.x2;
            }
            if settings.flip {
                s.y1 = -s.y1;
                s.y2 = -s.y2;
            }
            bounds.include(s.x1, s.y1);
            bounds.include(s.x2, s.y2);
            max_r2 = max_r2
                .max(s.x1 * s.x1 + s.y1 * s.y1)
                .max(s.x2 * s.x2 + s.y2 * s.y2);
        }
        if bounds.is_empty() {
            return Err(Error::NothingToCut("The design contains no strokes".into()));
        }
        bounds.min_x -= thick / 2.0;
        bounds.max_x += thick / 2.0;
        bounds.min_y -= thick / 2.0;
        bounds.max_y += thick / 2.0;

        // Box or circle.
        let mut circle_radius = 0.0;
        if settings.add_box {
            if radius_in == 0.0 || settings.cut_type == CutType::VCarve {
                let mut delta = thick / 2.0 + settings.box_gap;
                let (l, r) = (bounds.min_x - delta, bounds.max_x + delta);
                let (b, t) = (bounds.min_y - delta, bounds.max_y + delta);
                let corners: [(f64, f64, f64, f64); 4] = if settings.mirror ^ settings.flip {
                    [(l, b, l, t), (l, t, r, t), (r, t, r, b), (r, b, l, b)]
                } else {
                    [(l, b, r, b), (r, b, r, t), (r, t, l, t), (l, t, l, b)]
                };
                for (x1, y1, x2, y2) in corners {
                    segments.push(Segment::new(x1, y1, x2, y2, 0, 0));
                }
                if settings.cut_type != CutType::VCarve {
                    delta += thick / 2.0;
                }
                bounds.min_x -= delta;
                bounds.max_x += delta;
                bounds.min_y -= delta;
                bounds.max_y += delta;
            } else {
                circle_radius = max_r2.sqrt() + thick + settings.box_gap;
                let extent = circle_radius + thick / 2.0;
                bounds = Bounds {
                    min_x: -extent,
                    min_y: -extent,
                    max_x: extent,
                    max_y: extent,
                };
            }
        }

        // Move the chosen origin to the requested offset.
        let mid = bounds.center();
        let (x_zero, y_zero) = match settings.origin {
            Origin::TopLeft => (bounds.min_x, bounds.max_y),
            Origin::TopCenter => (mid.x, bounds.max_y),
            Origin::TopRight => (bounds.max_x, bounds.max_y),
            Origin::MidLeft => (bounds.min_x, mid.y),
            Origin::MidCenter => (mid.x, mid.y),
            Origin::MidRight => (bounds.max_x, mid.y),
            Origin::BotLeft => (bounds.min_x, bounds.min_y),
            Origin::BotCenter => (mid.x, bounds.min_y),
            Origin::BotRight => (bounds.max_x, bounds.min_y),
            Origin::ArcCenter | Origin::Default => (0.0, 0.0),
        };
        let dx = settings.x_origin - x_zero;
        let dy = settings.y_origin - y_zero;
        for s in &mut segments {
            s.x1 = s.x1 - x_zero + settings.x_origin;
            s.y1 = s.y1 - y_zero + settings.y_origin;
            s.x2 = s.x2 - x_zero + settings.x_origin;
            s.y2 = s.y2 - y_zero + settings.y_origin;
        }
        let machine_bounds = Bounds {
            min_x: bounds.min_x - x_zero + settings.x_origin,
            min_y: bounds.min_y - y_zero + settings.y_origin,
            max_x: bounds.max_x - x_zero + settings.x_origin,
            max_y: bounds.max_y - y_zero + settings.y_origin,
        };

        Ok(Self {
            segments,
            bounds: machine_bounds,
            circle_radius,
            circle_center: Point::new(dx, dy),
            missing_chars,
        })
    }

    /// Determines the character sequence to place.
    fn text_to_lay_out(font: &Font, settings: &Settings) -> String {
        if settings.input_type == InputType::Image {
            return IMAGE_GLYPH.to_string();
        }
        if !settings.show_font_index {
            return settings.text.clone();
        }
        let columns = if settings.extended_chars {
            (1.5 * (font.glyphs.len() as f64).sqrt()) as usize
        } else {
            15
        };
        let mut out = String::new();
        let mut count = 0;
        for ch in font
            .glyphs
            .keys()
            .filter(|&&k| settings.extended_chars || k < 256)
            .filter_map(|&k| char::from_u32(k))
        {
            count += 1;
            out.push(ch);
            if count > columns {
                out.push('\n');
                count = 0;
            }
        }
        out
    }

    /// Finds the height and depth that determine the scale, or `None` if nothing matches.
    fn line_metrics(font: &Font, text: &str, calc: HeightCalc) -> Option<LineMetrics> {
        let (height, depth) = match calc {
            HeightCalc::MaxAll => font
                .glyphs
                .values()
                .fold((f64::NEG_INFINITY, f64::INFINITY), |(h, d), g| {
                    (h.max(g.y_max()), d.min(g.y_min()))
                }),
            HeightCalc::MaxUsed => text
                .chars()
                .filter_map(|c| font.glyph(c))
                .fold((f64::NEG_INFINITY, f64::INFINITY), |(h, d), g| {
                    (h.max(g.y_max()), d.min(g.y_min()))
                }),
        };
        (height > f64::NEG_INFINITY).then_some(LineMetrics { height, depth })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Glyph, Stroke};
    use approx::assert_abs_diff_eq;

    /// A one-by-one square glyph per character, wound clockwise, with `w` wide and 9 tall.
    fn box_font(chars: &str) -> Font {
        let mut font = Font::default();
        for c in chars.chars() {
            font.glyphs.insert(
                c as u32,
                Glyph {
                    strokes: vec![
                        Stroke::new(0.0, 0.0, 0.0, 9.0),
                        Stroke::new(0.0, 9.0, 6.0, 9.0),
                        Stroke::new(6.0, 9.0, 6.0, 0.0),
                        Stroke::new(6.0, 0.0, 0.0, 0.0),
                    ],
                },
            );
        }
        font
    }

    fn base() -> Settings {
        Settings {
            text: "A".into(),
            y_scale: 9.0,
            stroke_thickness: 0.0,
            ..Settings::default()
        }
    }

    #[test]
    fn scale_maps_height_exactly() {
        let d = Design::layout(&box_font("A"), &base()).unwrap();
        assert_eq!(d.bounds.height(), 9.0);
        assert_eq!(d.bounds.width(), 6.0);
    }

    #[test]
    fn thickness_is_included_in_height() {
        let s = Settings {
            stroke_thickness: 1.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_abs_diff_eq!(d.bounds.height(), 9.0, epsilon = 1e-12);
    }

    #[test]
    fn v_carve_ignores_thickness() {
        let s = Settings {
            stroke_thickness: 1.0,
            cut_type: CutType::VCarve,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.bounds.height(), 9.0);
    }

    #[test]
    fn x_scale_stretches_width_only() {
        let s = Settings {
            x_scale: 50.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.bounds.width(), 3.0);
        assert_eq!(d.bounds.height(), 9.0);
    }

    #[test]
    fn char_and_word_spacing_advance() {
        let s = Settings {
            text: "A A".into(),
            char_space: 50.0,
            word_space: 100.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        // A(6) + char gap(3) + word(6) + A(6)
        assert_eq!(d.bounds.width(), 21.0);
        assert_eq!(d.segments[4].char_num, 3);
    }

    #[test]
    fn lines_stack_downward() {
        let s = Settings {
            text: "A\nA".into(),
            line_space: 1.5,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.bounds.max_y, 9.0);
        assert_eq!(d.bounds.min_y, -13.5);
        assert_eq!(d.segments[4].line, 1);
    }

    #[test]
    fn missing_characters_are_reported_once() {
        let s = Settings {
            text: "AzzA?".into(),
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.missing_chars, ['z', '?']);
        assert_eq!(d.segments.len(), 8);
    }

    #[test]
    fn all_missing_is_an_error() {
        let s = Settings {
            text: "zz".into(),
            ..base()
        };
        assert!(matches!(
            Design::layout(&box_font("A"), &s),
            Err(Error::NothingToCut(_))
        ));
    }

    #[test]
    fn empty_font_and_invalid_settings_are_errors() {
        assert!(Design::layout(&Font::default(), &base()).is_err());
        let s = Settings {
            y_scale: -1.0,
            ..base()
        };
        assert!(matches!(
            Design::layout(&box_font("A"), &s),
            Err(Error::InvalidSettings(_))
        ));
    }

    #[test]
    fn justification_aligns_short_lines() {
        let check = |justify, expected: f64| {
            let s = Settings {
                text: "AA\nA".into(),
                justify,
                char_space: 0.0,
                ..base()
            };
            let d = Design::layout(&box_font("A"), &s).unwrap();
            let second_line_min = d
                .segments
                .iter()
                .filter(|s| s.line == 1)
                .map(|s| s.x1.min(s.x2))
                .fold(f64::INFINITY, f64::min);
            assert_abs_diff_eq!(second_line_min, expected, epsilon = 1e-12);
        };
        check(Justify::Left, 0.0);
        check(Justify::Center, 3.0);
        check(Justify::Right, 6.0);
    }

    #[test]
    fn origin_positions() {
        let cases = [
            (Origin::Default, 0.0, 0.0),
            (Origin::TopLeft, 0.0, -9.0),
            (Origin::TopRight, -6.0, -9.0),
            (Origin::MidCenter, -3.0, -4.5),
            (Origin::BotRight, -6.0, 0.0),
            (Origin::BotCenter, -3.0, 0.0),
        ];
        for (origin, min_x, min_y) in cases {
            let s = Settings { origin, ..base() };
            let d = Design::layout(&box_font("A"), &s).unwrap();
            assert_eq!(d.bounds.min_x, min_x, "{origin}");
            assert_eq!(d.bounds.min_y, min_y, "{origin}");
        }
    }

    #[test]
    fn origin_offset_shifts_everything() {
        let s = Settings {
            x_origin: 10.0,
            y_origin: -5.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.bounds.min_x, 10.0);
        assert_eq!(d.segments[0].y1, -5.0);
    }

    #[test]
    fn rotation_quarter_turn() {
        let s = Settings {
            angle: 90.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_abs_diff_eq!(d.bounds.width(), 9.0, epsilon = 1e-9);
        assert_abs_diff_eq!(d.bounds.height(), 6.0, epsilon = 1e-9);
        assert_abs_diff_eq!(d.bounds.max_x, 0.0, epsilon = 1e-9);
    }

    #[test]
    fn mirror_and_flip_negate_coordinates() {
        let s = Settings {
            mirror: true,
            flip: true,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.bounds.max_x, 0.0);
        assert_eq!(d.bounds.min_x, -6.0);
        assert_eq!(d.bounds.max_y, 0.0);
    }

    #[test]
    fn circle_text_stays_at_radius() {
        let s = Settings {
            text_radius: 20.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        for seg in &d.segments {
            let r = seg.x1.hypot(seg.y1);
            assert!((20.0 - 1e-9..=29.0 + 1e-9).contains(&r), "radius {r}");
        }
    }

    #[test]
    fn inside_circle_text_extends_inward() {
        let s = Settings {
            text_radius: 20.0,
            outer: false,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        for seg in &d.segments {
            let r = seg.x1.hypot(seg.y1);
            assert!((11.0 - 1e-9..=20.0 + 1e-9).contains(&r), "radius {r}");
        }
    }

    #[test]
    fn centered_circle_text_is_symmetric_about_y_axis() {
        let s = Settings {
            text_radius: 20.0,
            justify: Justify::Center,
            text: "AAA".into(),
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_abs_diff_eq!(d.bounds.min_x, -d.bounds.max_x, epsilon = 1e-9);
    }

    #[test]
    fn box_surrounds_design_and_winds_by_mirroring() {
        let s = Settings {
            add_box: true,
            box_gap: 1.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.segments.len(), 8);
        assert_eq!(d.bounds.width(), 8.0);
        assert_eq!(d.bounds.height(), 11.0);
        // Counter-clockwise by default: the first box edge runs along +X.
        assert_eq!(d.segments[4].y1, d.segments[4].y2);
        assert_eq!(d.segments[4].char_num, 0);
        let m = Settings {
            add_box: true,
            box_gap: 1.0,
            mirror: true,
            ..base()
        };
        let dm = Design::layout(&box_font("A"), &m).unwrap();
        assert_eq!(dm.segments[4].x1, dm.segments[4].x2, "reversed winding");
    }

    #[test]
    fn engraved_circle_text_gets_circle_not_box() {
        let s = Settings {
            add_box: true,
            box_gap: 1.0,
            text_radius: 20.0,
            ..base()
        };
        let d = Design::layout(&box_font("A"), &s).unwrap();
        assert_eq!(d.segments.len(), 4);
        assert!(d.circle_radius > 29.0);
        assert_eq!(d.bounds.min_x, -d.circle_radius);
    }

    #[test]
    fn image_input_uses_drawing_glyph_and_percent_height() {
        let font = Font::from_drawing(vec![
            Stroke::new(0.0, 0.0, 0.0, 10.0),
            Stroke::new(0.0, 10.0, 20.0, 10.0),
        ]);
        let s = Settings {
            input_type: InputType::Image,
            use_image_size: true,
            y_scale: 50.0,
            stroke_thickness: 0.0,
            ..Settings::default()
        };
        let d = Design::layout(&font, &s).unwrap();
        assert_eq!(d.bounds.height(), 5.0);
        assert_eq!(d.bounds.width(), 10.0);
    }

    #[test]
    fn font_index_lays_out_all_characters_in_rows() {
        let font = box_font("ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        let s = Settings {
            show_font_index: true,
            ..base()
        };
        let d = Design::layout(&font, &s).unwrap();
        assert_eq!(d.segments.len(), 26 * 4);
        assert!(d.segments.iter().any(|s| s.line == 1));
    }

    #[test]
    fn wrap_rotate_identity_without_radius_or_angle() {
        let (x, y, a) = Design::wrap_rotate(3.0, 4.0, 0.0, 0.0);
        assert_abs_diff_eq!(x, 3.0, epsilon = 1e-12);
        assert_abs_diff_eq!(y, 4.0, epsilon = 1e-12);
        assert_eq!(a, 0.0);
    }
}

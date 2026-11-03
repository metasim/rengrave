//! G-code output: path simplification, arc fitting, multi-pass depth logic, and file assembly.

use crate::design::Design;
use crate::error::{Error, Result};
use crate::geom::{ZERO, angle_from_sin_cos};
use crate::settings::{ArcFit, BitShape, CleanupBit, CutType, InputType, Settings};
use crate::sort::PathRange;
use crate::vcarve::{CarvePath, CarvePoint};

const PROGRAM_NAME: &str = concat!("rengrave-", env!("CARGO_PKG_VERSION"));

type Xyz = [f64; 3];

/// A simplified move.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fitted {
    /// A straight cut.
    Line(Xyz),
    /// An arc to a point; `ccw` selects the direction and `offset` locates the center relative
    /// to the current point.
    Arc {
        to: Xyz,
        ccw: bool,
        offset: [f64; 2],
    },
}

/// One unit of pending simplification work.
enum Work {
    Visit { lo: usize, hi: usize, first: bool },
    Emit(usize),
}

impl Fitted {
    /// Simplifies a cut path with the Douglas-Peucker algorithm, optionally fitting arcs.
    ///
    /// The result never strays from the input by more than `tolerance`. A trailing section that
    /// returns to the first point of a span is emitted as a closing move to that point.
    fn simplify(points: &[Xyz], tolerance: f64, fit_arcs: bool) -> Vec<Self> {
        let mut out = Vec::new();
        if points.is_empty() {
            return out;
        }
        let mut stack = vec![Work::Visit {
            lo: 0,
            hi: points.len() - 1,
            first: true,
        }];
        // A closing move is queued as Emit(lo) beneath the work for a span.
        while let Some(work) = stack.pop() {
            let (lo, mut hi, first) = match work {
                Work::Emit(i) => {
                    out.push(Fitted::Line(points[i]));
                    continue;
                }
                Work::Visit { lo, hi, first } => (lo, hi, first),
            };
            if lo == hi {
                out.push(Fitted::Line(points[lo]));
                continue;
            }
            let start = points[lo];
            let coincident = |p: Xyz| {
                (start[0] - p[0]).abs() < ZERO
                    && (start[1] - p[1]).abs() < ZERO
                    && (start[2] - p[2]).abs() < ZERO
            };
            let mut closes = false;
            while hi > lo && coincident(points[hi]) {
                closes = true;
                hi -= 1;
            }
            if hi == lo {
                // Every point in the span coincides with the first.
                continue;
            }
            if closes {
                stack.push(Work::Emit(lo));
            }
            let end = points[hi];

            let mut worst_dist = 0.0;
            let mut worst_dist_z = 0.0;
            let mut worst = 0;
            let mut min_rad = f64::MAX;
            let mut max_arc = 0;
            for (i, &p) in points.iter().enumerate().take(hi).skip(lo + 1) {
                let dist = Fitted::dist_to_segment(start, end, p, false);
                let dist_z = Fitted::dist_to_segment(start, end, p, true);
                if dist > worst_dist {
                    worst = i;
                    worst_dist = dist;
                    if fit_arcs
                        && let Some(rad) = Fitted::circle_radius(start, p, end)
                        && rad < min_rad
                    {
                        max_arc = i;
                        min_rad = rad;
                    }
                }
                if dist_z > worst_dist_z {
                    worst_dist_z = dist_z;
                }
            }

            let mut arc: Option<((f64, f64), f64)> = None;
            if min_rad != f64::MAX
                && let Some(center) = Fitted::circle_center(start, points[max_arc], end)
                && Fitted::arc_is_fittable(center, start, points[max_arc], end)
            {
                let mut worst_arc = 0.0_f64;
                for p in &points[lo..=hi] {
                    let radial = ((center.0 - p[0]).hypot(center.1 - p[1]) - min_rad).abs();
                    let dist = (worst_dist_z * worst_dist_z + radial * radial).sqrt();
                    worst_arc = worst_arc.max(dist);
                }
                arc = Some((center, worst_arc));
            }

            match arc {
                Some((center, worst_arc)) if worst_arc < tolerance && worst_arc < worst_dist => {
                    let mid = points[max_arc];
                    let doubled_area = (start[0] * mid[1] - mid[0] * start[1])
                        + (mid[0] * end[1] - end[0] * mid[1])
                        + (end[0] * start[1] - start[0] * end[1]);
                    out.push(Fitted::Line(start));
                    out.push(Fitted::Arc {
                        to: end,
                        ccw: doubled_area > 0.0,
                        offset: [center.0 - start[0], center.1 - start[1]],
                    });
                }
                _ if worst_dist > tolerance => {
                    if first {
                        stack.push(Work::Emit(hi));
                    }
                    stack.push(Work::Visit {
                        lo: worst,
                        hi,
                        first: false,
                    });
                    stack.push(Work::Emit(worst));
                    stack.push(Work::Visit {
                        lo,
                        hi: worst,
                        first: false,
                    });
                    if first {
                        stack.push(Work::Emit(lo));
                    }
                }
                _ => {
                    if first {
                        out.push(Fitted::Line(start));
                        out.push(Fitted::Line(end));
                    }
                }
            }
        }
        out
    }
}

/// Accumulates cuts and writes them in modal form, omitting unchanged axis words.
struct MotionWriter {
    last: [Option<f64>; 3],
    last_feed: Option<String>,
    feed: Option<String>,
    cuts: Vec<Xyz>,
    tolerance: f64,
    arc_fit: ArcFit,
    decimals: usize,
}

impl MotionWriter {
    fn new(safe_z: f64, tolerance: f64, arc_fit: ArcFit, decimals: usize) -> Self {
        Self {
            last: [None, None, Some(safe_z)],
            last_feed: None,
            feed: None,
            cuts: Vec::new(),
            tolerance,
            arc_fit,
            decimals,
        }
    }

    /// Queues a cut. A missing Z repeats the most recent one.
    fn cut(&mut self, x: f64, y: f64, z: Option<f64>) {
        let last_z = self
            .cuts
            .last()
            .map(|c| c[2])
            .or(self.last[2])
            .unwrap_or_default();
        self.cuts.push([x, y, z.unwrap_or(last_z)]);
    }

    fn set_feed(&mut self, out: &mut Vec<String>, feed: &str) {
        self.flush(out);
        self.feed = Some(format!("F{feed}"));
        self.last_feed = None;
    }

    fn flush(&mut self, out: &mut Vec<String>) {
        if self.cuts.is_empty() {
            return;
        }
        let cuts = std::mem::take(&mut self.cuts);
        for fitted in Fitted::simplify(&cuts, self.tolerance, self.arc_fit != ArcFit::None) {
            match fitted {
                Fitted::Line(p) => self.write_move(out, "G1", p, None),
                Fitted::Arc { to, ccw, offset } => {
                    self.write_move(out, if ccw { "G3" } else { "G2" }, to, Some(offset));
                }
            }
        }
    }

    fn write_move(&mut self, out: &mut Vec<String>, code: &str, to: Xyz, offset: Option<[f64; 2]>) {
        let d = self.decimals;
        let mut feed_word = "";
        let feed_owned;
        if self.feed != self.last_feed {
            feed_owned = self.feed.clone().unwrap_or_default();
            feed_word = &feed_owned;
            self.last_feed.clone_from(&self.feed);
        }
        let mut radius_word = String::new();
        if let Some([i, j]) = offset {
            let xc = self.last[0].unwrap_or_default() + i;
            let yc = self.last[1].unwrap_or_default() + j;
            let r1 = (xc - self.last[0].unwrap_or_default())
                .hypot(yc - self.last[1].unwrap_or_default());
            let r2 = (xc - to[0]).hypot(yc - to[1]);
            radius_word = format!(" R{:.d$}", (r1 + r2) / 2.0);
        }
        let mut axis_words = String::new();
        for (axis, (value, last)) in ["X", "Y", "Z"]
            .iter()
            .zip(to.iter().copied().zip(self.last.iter_mut()))
        {
            if Some(value) != *last {
                axis_words.push_str(&format!(" {axis}{value:.d$}"));
                *last = Some(value);
            }
        }
        if axis_words.is_empty() && feed_word.is_empty() {
            return;
        }
        let center_words = offset
            .map(|[i, j]| format!(" I{i:.d$} J{j:.d$}"))
            .unwrap_or_default();
        out.push(if self.arc_fit == ArcFit::Radius {
            format!("{code}{axis_words}{radius_word}{feed_word}")
        } else {
            format!("{code}{axis_words}{center_words}{feed_word}")
        });
    }
}

/// Depth-pass state machine for roughing followed by a finish pass.
struct Passes {
    max_dz: f64,
    rough_stock: f64,
    zmin: f64,
    roughing: bool,
    rough_again: bool,
}

impl Passes {
    fn new(settings: &Settings) -> Self {
        let max_dz = settings.v_max_cut;
        let mut rough_stock = settings.v_rough_stock;
        let rough_again = rough_stock > 0.0;
        if rough_stock > 0.0 && -max_dz < rough_stock {
            rough_stock = -max_dz;
        }
        Self {
            max_dz,
            rough_stock,
            zmin: 0.0,
            roughing: true,
            rough_again,
        }
    }

    /// Moves to the next pass. Returns `false` when all passes are done.
    fn advance(&mut self) -> bool {
        if !(self.rough_again || self.roughing) {
            return false;
        }
        if !self.rough_again {
            self.roughing = false;
            self.max_dz = -99999.0;
        }
        self.rough_again = false;
        self.zmin += self.max_dz;
        true
    }
}

/// Assembled output files.
pub struct GcodeProgram;

impl GcodeProgram {
    /// The header comment block, including recoverable settings.
    fn settings_block(settings: &Settings) -> Vec<String> {
        let mut out = vec![
            format!("( Code generated by {PROGRAM_NAME}, a port of F-Engrave 1.78 by Scorch )"),
            "(Settings used when this file was created)".to_string(),
        ];
        if settings.input_type == InputType::Text {
            const MAX_LEN: usize = 40;
            let mut short: String = settings.text.chars().take(MAX_LEN).collect();
            if settings.text.chars().count() > MAX_LEN {
                short.push_str("___");
            }
            out.push(format!(
                "(Engrave Text:{} )",
                GcodeProgram::sanitized_comment(&short)
            ));
        }
        out.push("(=========================================================)".to_string());
        out.extend(settings.to_settings_lines());
        out.push(format!("( Fontfile: {} )", settings.font_file));
        out.push("(#########################################################)".to_string());
        out
    }

    /// Renders the settings alone, as stored in a configuration file.
    pub fn settings_file(settings: &Settings) -> Vec<String> {
        Self::settings_block(settings)
    }

    /// Generates the program for an engraved or V-carved design.
    ///
    /// `carve` holds the result of [`CarvePath::calculate`], and is required when
    /// `settings.cut_type` is [`CutType::VCarve`].
    pub fn generate(
        design: &Design,
        carve: Option<&CarvePath>,
        settings: &Settings,
    ) -> Result<Vec<String>> {
        settings.ensure_valid()?;
        let vcarving = settings.cut_type == CutType::VCarve;
        let empty = CarvePath::default();
        let carve = match carve {
            Some(c) => c,
            None if vcarving => {
                return Err(Error::NothingToCut(
                    "V-carve paths must be calculated before writing G-code".into(),
                ));
            }
            None => &empty,
        };

        let mut out = if settings.no_comments {
            Vec::new()
        } else {
            Self::settings_block(settings)
        };
        let dp = settings.units.coord_decimals();
        let dp_feed = settings.units.feed_decimals();
        let z_safe = settings.z_safe;
        let mut motion = MotionWriter::new(z_safe, settings.accuracy, settings.arc_fit, dp);
        if settings.arc_fit != ArcFit::None {
            out.push("G17".to_string());
        }
        let (safe_val, mut depth_val) = if settings.disable_variables {
            (format!("{z_safe:.dp$}"), format!("{:.dp$}", settings.z_cut))
        } else {
            out.push(format!("#1 = {z_safe:.dp$}  ( Safe Z )"));
            out.push(format!(
                "#2 = {:.dp$}  ( Engraving Depth Z )",
                settings.z_cut
            ));
            ("#1".to_string(), "#2".to_string())
        };
        Self::preamble(settings, &mut out);

        let feed_str = format!("{:.dp_feed$}", settings.feed);
        let mut plunge_str = format!("{:.dp_feed$}", settings.plunge);
        let zero_feed = format!("{:.dp_feed$}", 0.0);
        out.push(format!("F{feed_str}"));
        if plunge_str == zero_feed {
            plunge_str.clone_from(&feed_str);
        }

        if !vcarving || settings.bit_shape == BitShape::Flat {
            Self::outline_passes(
                design,
                carve,
                settings,
                &mut motion,
                &mut out,
                (&safe_val, &mut depth_val),
                (&feed_str, &plunge_str),
            );
        } else {
            plunge_str.clone_from(&feed_str);
            Self::carve_passes(carve, settings, &mut motion, &mut out, &safe_val);
        }

        if design.circle_radius != 0.0 && !vcarving {
            out.push(format!("G0 Z{safe_val}"));
            out.push(format!(
                "G0 X{:.dp$} Y{:.dp$}",
                design.circle_center.x - design.circle_radius,
                design.circle_center.y
            ));
            let same_feed = feed_str == plunge_str;
            out.push(if same_feed {
                format!("G1 Z{depth_val}")
            } else {
                format!("G1 Z{depth_val} F{plunge_str}")
            });
            let arc = format!("G2 I{:.dp$} J{:.dp$}", design.circle_radius, 0.0);
            out.push(if same_feed {
                arc
            } else {
                format!("{arc} F{feed_str}")
            });
        }

        out.push(format!("G0 Z{safe_val}"));
        GcodeProgram::split_lines(&settings.g_post, &mut out);
        Ok(out)
    }

    fn preamble(settings: &Settings, out: &mut Vec<String>) {
        out.push("G90".to_string());
        if settings.arc_fit == ArcFit::Center {
            out.push("G91.1".to_string());
        }
        out.push(match settings.units {
            crate::settings::Units::Inches => "G20".to_string(),
            crate::settings::Units::Millimeters => "G21".to_string(),
        });
        GcodeProgram::split_lines(&settings.g_pre, out);
    }

    /// Cuts outlines at constant depth: engraving, and flat-bottomed V-carve regions.
    fn outline_passes(
        design: &Design,
        carve: &CarvePath,
        settings: &Settings,
        motion: &mut MotionWriter,
        out: &mut Vec<String>,
        (safe_val, depth_val): (&str, &mut String),
        (feed_str, plunge_str): (&str, &str),
    ) {
        let dp = settings.units.coord_decimals();
        let flat = settings.cut_type != CutType::Engrave && settings.bit_shape == BitShape::Flat;
        let mut acc = settings.accuracy;
        let mut depth = settings.z_cut;
        let mut ecoords: Vec<(f64, f64, usize)> = Vec::new();
        let mut passes = Passes::new(settings);
        if flat {
            acc = settings.v_step_len * 1.5;
            let rbit = settings.effective_bit_dia() / 2.0;
            let mut loop_num = 0;
            if let Some(first) = carve.points.first() {
                let mut old = first.loop_num;
                for p in &carve.points[1..] {
                    if old != p.loop_num {
                        loop_num += 1;
                    }
                    if p.r >= rbit {
                        ecoords.push((p.x, p.y, loop_num));
                        old = p.loop_num;
                    } else {
                        loop_num += 1;
                    }
                }
            }
            depth = settings.max_cut_depth();
        } else {
            passes.rough_again = false;
            let mut loop_num = 0;
            let (mut old_x, mut old_y) = (-99990.0_f64, -99990.0_f64);
            let mut first_stroke = true;
            for s in &design.segments {
                let dist = (old_x - s.x1).hypot(old_y - s.y1);
                if dist > acc || first_stroke {
                    loop_num += 1;
                    first_stroke = false;
                    ecoords.push((s.x1, s.y1, loop_num));
                }
                ecoords.push((s.x2, s.y2, loop_num));
                old_x = s.x2;
                old_y = s.y2;
            }
        }
        let order = PathRange::order_loops(&ecoords, |c| (c.0, c.1), |c| c.2);

        while passes.advance() {
            let mut z1 = depth;
            if passes.roughing {
                z1 += passes.rough_stock;
            }
            if z1 < passes.zmin {
                z1 = passes.zmin;
                passes.rough_again = true;
            }
            if flat {
                *depth_val = format!("{z1:.dp$}");
            }
            let (mut last_x, mut last_y) = (-999.0_f64, -999.0_f64);
            for range in &order {
                let mut loop_old = None;
                for i in range.indices() {
                    let (x1, y1, loop_num) = ecoords[i];
                    if Some(loop_num) != loop_old {
                        motion.flush(out);
                        if (x1 - last_x).hypot(y1 - last_y) > acc {
                            out.push(format!("G0 Z{safe_val}"));
                            out.push(format!("G0 X{x1:.dp$} Y{y1:.dp$}"));
                            if feed_str == plunge_str {
                                out.push(format!("G1 Z{depth_val}"));
                            } else {
                                out.push(format!("G1 Z{depth_val} F{plunge_str}"));
                                motion.set_feed(out, feed_str);
                            }
                            last_x = x1;
                            last_y = y1;
                            motion.cut(x1, y1, None);
                        }
                    } else {
                        motion.cut(x1, y1, None);
                        last_x = x1;
                        last_y = y1;
                    }
                    loop_old = Some(loop_num);
                }
                motion.flush(out);
            }
        }
    }

    /// Cuts variable-depth V-carve loops.
    fn carve_passes(
        carve: &CarvePath,
        settings: &Settings,
        motion: &mut MotionWriter,
        out: &mut Vec<String>,
        safe_val: &str,
    ) {
        let dp = settings.units.coord_decimals();
        let points = Self::order_loops_by_begin(&carve.points);
        if points.is_empty() {
            return;
        }
        let half_angle = (settings.v_bit_angle / 2.0).to_radians();
        let bit_radius = settings.v_bit_dia / 2.0;
        let inlay_depth = settings.max_cut_depth();
        let z_of = |r: f64| match settings.bit_shape {
            BitShape::Ball => {
                let theta = (r / bit_radius).clamp(-1.0, 1.0).acos();
                -bit_radius * (1.0 - theta.sin())
            }
            _ => {
                let z = -r / half_angle.tan();
                if settings.inlay { z + inlay_depth } else { z }
            }
        };

        let mut passes = Passes::new(settings);
        while passes.advance() {
            let mut loop_old = None;
            for (i, p) in points.iter().enumerate() {
                let next_r = points.get(i + 1).map_or(0.0, |n| n.r);
                let mut z1 = z_of(p.r);
                let mut next_z = z_of(next_r);
                if passes.roughing {
                    z1 += passes.rough_stock;
                    next_z += passes.rough_stock;
                }
                if passes.zmin - z1 > 0.001 {
                    z1 = passes.zmin;
                    passes.rough_again = true;
                }
                if passes.zmin - next_z > 0.001 {
                    next_z = passes.zmin;
                    passes.rough_again = true;
                }
                let zmax = passes.zmin - passes.max_dz;
                if z1 > zmax && next_z > zmax && passes.roughing {
                    loop_old = None;
                    continue;
                }
                if Some(p.loop_num) != loop_old {
                    motion.flush(out);
                    out.push(format!("G0 Z{safe_val}"));
                    out.push(format!("G0 X{:.dp$} Y{:.dp$}", p.x, p.y));
                    out.push(format!("G1 Z{z1:.dp$}"));
                }
                motion.cut(p.x, p.y, Some(z1));
                loop_old = Some(p.loop_num);
            }
            motion.flush(out);
        }
    }

    /// Orders whole loops greedily by distance from the end of one to the beginning of the next.
    fn order_loops_by_begin(points: &[CarvePoint]) -> Vec<CarvePoint> {
        if points.is_empty() {
            return Vec::new();
        }
        let mut begins = vec![0_usize];
        let mut ends = Vec::new();
        for i in 1..points.len() {
            if points[i].loop_num != points[i - 1].loop_num {
                begins.push(i);
                ends.push(i - 1);
            }
        }
        ends.push(points.len() - 1);

        let xy = |i: &usize| (points[*i].x, points[*i].y);
        let begin_points: Vec<_> = begins.iter().map(xy).collect();
        let end_points: Vec<_> = ends.iter().map(xy).collect();
        PathRange::order_by_begin(&begin_points, &end_points)
            .into_iter()
            .flat_map(|k| points[begins[k]..=ends[k]].iter().copied())
            .collect()
    }

    /// Generates the program for a cleanup operation from paths computed by
    /// [`crate::cleanup::CleanupPaths::calculate`]. Fails unless the job is a V-carve.
    pub fn generate_cleanup(
        coords: &[CarvePoint],
        bit: CleanupBit,
        settings: &Settings,
    ) -> Result<Vec<String>> {
        settings.ensure_valid()?;
        if settings.cut_type != CutType::VCarve {
            return Err(Error::NothingToCut(
                "Cleanup operations apply only to V-carve jobs".into(),
            ));
        }
        let units = settings.units;
        let dp = units.coord_decimals();
        let mut depth = settings.max_cut_depth();
        if settings.inlay {
            depth += settings.allowance;
        }
        let mut out = Vec::new();
        if !settings.no_comments {
            out.push(format!("( Code generated by {PROGRAM_NAME} )"));
            out.push("( This file is a secondary operation for )".to_string());
            out.push("( cleaning up a V-carve. )".to_string());
            out.push("( The tool paths were calculated based )".to_string());
            match bit {
                CleanupBit::Straight => {
                    out.push("( on using a bit with a )".to_string());
                    out.push(format!(
                        "( Diameter of {:.4} {})",
                        settings.clean_dia, units
                    ));
                }
                CleanupBit::VBit => {
                    out.push("( on using a v-bit with a)".to_string());
                    out.push(format!("( angle of {:.4} Degrees)", settings.v_bit_angle));
                }
            }
            out.push("(==========================================)".to_string());
        }
        let z_safe = settings.z_safe;
        let safe_val = if settings.disable_variables {
            format!("{z_safe:.dp$}")
        } else {
            out.push(format!("#1 = {z_safe:.dp$}  ( Safe Z )"));
            "#1".to_string()
        };
        if !settings.no_comments {
            out.push("(##########################################)".to_string());
        }
        Self::preamble(settings, &mut out);

        let feed_str = format!("{:.dp$}", settings.feed);
        let mut plunge_str = format!("{:.dp$}", settings.plunge);
        let zero_feed = format!("{:.dp$}", 0.0);
        let mut feed_current = zero_feed.clone();
        if plunge_str == zero_feed {
            plunge_str.clone_from(&feed_str);
        }

        let mut coords_out = coords.to_vec();
        let mut passes = Passes::new(settings);
        while passes.advance() {
            let order = PathRange::order_loops(&coords_out, |c| (c.x, c.y), |c| c.loop_num);
            coords_out = order
                .iter()
                .flat_map(|r| r.indices().map(|i| coords_out[i]).collect::<Vec<_>>())
                .collect();

            let mut loop_old = None;
            for p in &coords_out {
                if Some(p.loop_num) != loop_old {
                    out.push(format!("G0 Z{safe_val}"));
                    out.push(format!("G0 X{:.dp$} Y{:.dp$}", p.x, p.y));
                    let mut z1 = depth;
                    if passes.roughing {
                        z1 = depth + passes.rough_stock;
                    }
                    if z1 < passes.zmin {
                        z1 = passes.zmin;
                        passes.rough_again = true;
                    }
                    let feed_word = if feed_current == plunge_str {
                        String::new()
                    } else {
                        feed_current.clone_from(&plunge_str);
                        format!(" F{plunge_str}")
                    };
                    out.push(format!("G1 Z{z1:.dp$}{feed_word}"));
                } else {
                    let feed_word = if feed_str == feed_current {
                        String::new()
                    } else {
                        feed_current.clone_from(&feed_str);
                        format!(" F{feed_str}")
                    };
                    out.push(format!("G1 X{:.dp$} Y{:.dp$}{feed_word}", p.x, p.y));
                }
                loop_old = Some(p.loop_num);
            }
        }
        out.push(format!("G0 Z{safe_val}"));
        GcodeProgram::split_lines(&settings.g_post, &mut out);
        Ok(out)
    }
}

impl Fitted {
    /// Three-dimensional distance from `p` to the segment `a..b`. With `z_only`, only the Z
    /// component of that displacement counts.
    fn dist_to_segment(a: Xyz, b: Xyz, p: Xyz, z_only: bool) -> f64 {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let dz = b[2] - a[2];
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 == 0.0 {
            return 0.0;
        }
        let t =
            ((dx * (p[0] - a[0]) + dy * (p[1] - a[1]) + dz * (p[2] - a[2])) / d2).clamp(0.0, 1.0);
        let dist2 = if z_only {
            (p[2] - a[2] - t * dz).powi(2)
        } else {
            (p[0] - a[0] - t * dx).powi(2)
                + (p[1] - a[1] - t * dy).powi(2)
                + (p[2] - a[2] - t * dz).powi(2)
        };
        dist2.powf(0.5)
    }

    /// Radius of the XY circle through three points, or `None` if they are nearly collinear.
    fn circle_radius(p1: Xyz, p2: Xyz, p3: Xyz) -> Option<f64> {
        let (x12, y12) = (p1[0] - p2[0], p1[1] - p2[1]);
        let (x23, y23) = (p2[0] - p3[0], p2[1] - p3[1]);
        let (x31, y31) = (p3[0] - p1[0], p3[1] - p1[1]);
        let den = (x12 * y23 - x23 * y12).abs();
        if den < 1e-5 {
            return None;
        }
        Some(x12.hypot(y12) * x23.hypot(y23) * x31.hypot(y31) / 2.0 / den)
    }

    /// Center of the XY circle through three points, or `None` if they are nearly collinear.
    fn circle_center(p1: Xyz, p2: Xyz, p3: Xyz) -> Option<(f64, f64)> {
        let sub = |a: Xyz, b: Xyz| (a[0] - b[0], a[1] - b[1]);
        let dot = |a: (f64, f64), b: (f64, f64)| a.0 * b.0 + a.1 * b.1;
        let mag2 = |a: (f64, f64)| a.0 * a.0 + a.1 * a.1;
        let (a12, a23) = (sub(p1, p2), sub(p2, p3));
        let den = (a12.0 * a23.1 - a12.1 * a23.0).abs();
        if den < 1e-5 {
            return None;
        }
        let alpha = mag2(sub(p2, p3)) * dot(sub(p1, p2), sub(p1, p3)) / 2.0 / den / den;
        let beta = mag2(sub(p1, p3)) * dot(sub(p2, p1), sub(p2, p3)) / 2.0 / den / den;
        let gamma = mag2(sub(p1, p2)) * dot(sub(p3, p1), sub(p3, p2)) / 2.0 / den / den;
        Some((
            alpha * p1[0] + beta * p2[0] + gamma * p3[0],
            alpha * p1[1] + beta * p2[1] + gamma * p3[1],
        ))
    }

    /// Returns `true` if three points on the circle lie in one quadrant, and the turn at the middle
    /// point is gentle enough that an arc is a reasonable fit.
    fn arc_is_fittable(center: (f64, f64), p1: Xyz, p2: Xyz, p3: Xyz) -> bool {
        let la = (p1[0] - p2[0]).hypot(p1[1] - p2[1]);
        let lb = (p3[0] - p2[0]).hypot(p3[1] - p2[1]);
        if la == 0.0 || lb == 0.0 {
            return false;
        }
        let theta_a = angle_from_sin_cos((p1[1] - p2[1]) / la, (p1[0] - p2[0]) / la);
        let theta_b = angle_from_sin_cos((p3[1] - p2[1]) / lb, (p3[0] - p2[0]) / lb);
        let angle = (theta_a - theta_b).abs();
        const TEST_ANGLE: f64 = 36.0;
        if angle > 180.0 + TEST_ANGLE || angle < 180.0 - TEST_ANGLE {
            return false;
        }

        let sign = |v: f64| {
            if v.abs() < 1e-5 {
                0
            } else if v < 0.0 {
                -1
            } else {
                1
            }
        };
        let mut signs: Vec<(i32, i32)> = [p1, p2, p3]
            .iter()
            .map(|p| (sign(p[0] - center.0), sign(p[1] - center.1)))
            .collect();
        signs.sort_unstable();
        signs.dedup();
        if signs.len() == 1 {
            return true;
        }
        // A point on an axis belongs to the neighboring quadrant that is present.
        for (qx, qy) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
            if signs.contains(&(qx, qy)) {
                signs.retain(|&s| s != (qx, 0) && s != (0, qy));
            }
        }
        signs.len() == 1
    }
}

impl GcodeProgram {
    fn split_lines(text: &str, out: &mut Vec<String>) {
        out.extend(text.split('|').map(str::to_string));
    }

    /// Keeps only word characters, collapsing other runs to single spaces.
    fn sanitized_comment(text: &str) -> String {
        let mut out = String::new();
        let mut in_gap = false;
        for ch in text.chars() {
            if ch.is_alphanumeric() || ch == '_' {
                out.push(ch);
                in_gap = false;
            } else if !in_gap {
                out.push(' ');
                in_gap = true;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Font, Stroke};
    use crate::progress::Progress;
    use crate::settings::Units;
    use approx::assert_abs_diff_eq;

    fn pts(list: &[(f64, f64)]) -> Vec<Xyz> {
        list.iter().map(|&(x, y)| [x, y, 0.0]).collect()
    }

    #[test]
    fn distance_to_segment_cases() {
        let a = [0.0, 0.0, 0.0];
        let b = [10.0, 0.0, 0.0];
        assert_abs_diff_eq!(Fitted::dist_to_segment(a, b, [5.0, 3.0, 0.0], false), 3.0);
        assert_abs_diff_eq!(
            Fitted::dist_to_segment(a, b, [-3.0, 4.0, 0.0], false),
            5.0,
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(
            Fitted::dist_to_segment(a, b, [13.0, 4.0, 0.0], false),
            5.0,
            epsilon = 1e-12
        );
        assert_eq!(Fitted::dist_to_segment(a, a, [13.0, 4.0, 0.0], false), 0.0);
        assert_abs_diff_eq!(Fitted::dist_to_segment(a, b, [5.0, 3.0, 2.0], true), 2.0);
    }

    #[test]
    fn circle_through_three_points() {
        let (a, b, c) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]);
        assert_abs_diff_eq!(
            Fitted::circle_radius(a, b, c).unwrap(),
            1.0,
            epsilon = 1e-12
        );
        let center = Fitted::circle_center(a, b, c).unwrap();
        assert_abs_diff_eq!(center.0, 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(center.1, 0.0, epsilon = 1e-12);
    }

    #[test]
    fn collinear_points_have_no_circle() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0, 0.0]);
        assert_eq!(Fitted::circle_radius(a, b, c), None);
        assert_eq!(Fitted::circle_center(a, b, c), None);
    }

    #[test]
    fn straight_run_reduces_to_endpoints() {
        let input = pts(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.00001), (3.0, 0.0)]);
        let out = Fitted::simplify(&input, 0.001, false);
        assert_eq!(out, [Fitted::Line(input[0]), Fitted::Line(input[3])]);
    }

    #[test]
    fn significant_corner_is_kept() {
        let input = pts(&[(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)]);
        let out = Fitted::simplify(&input, 0.001, false);
        assert!(out.contains(&Fitted::Line(input[1])));
    }

    #[test]
    fn deviation_stays_within_tolerance() {
        let input: Vec<Xyz> = (0..200)
            .map(|i| {
                let t = f64::from(i) * 0.05;
                [t, t.sin(), (t * 0.5).cos()]
            })
            .collect();
        let tol = 0.01;
        let kept: Vec<Xyz> = Fitted::simplify(&input, tol, false)
            .into_iter()
            .map(|f| match f {
                Fitted::Line(p) | Fitted::Arc { to: p, .. } => p,
            })
            .collect();
        assert!(kept.len() < input.len());
        for p in &input {
            let best = kept
                .windows(2)
                .map(|w| Fitted::dist_to_segment(w[0], w[1], *p, false))
                .fold(f64::INFINITY, f64::min);
            assert!(best <= tol + 1e-9, "deviation {best}");
        }
    }

    #[test]
    fn closed_loop_returns_to_start() {
        let input = pts(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]);
        let out = Fitted::simplify(&input, 0.001, false);
        assert_eq!(out.first(), Some(&Fitted::Line(input[0])));
        assert_eq!(out.last(), Some(&Fitted::Line(input[0])));
    }

    #[test]
    fn single_point_passes_through_and_coincident_points_vanish() {
        let one = pts(&[(1.0, 2.0)]);
        assert_eq!(Fitted::simplify(&one, 0.001, false), [Fitted::Line(one[0])]);
        let same = pts(&[(1.0, 2.0), (1.0, 2.0), (1.0, 2.0)]);
        assert!(Fitted::simplify(&same, 0.001, false).is_empty());
        assert!(Fitted::simplify(&[], 0.001, false).is_empty());
    }

    fn arc_points(cx: f64, cy: f64, r: f64, from: f64, to: f64, n: usize) -> Vec<Xyz> {
        (0..=n)
            .map(|i| {
                let a = (from + (to - from) * i as f64 / n as f64).to_radians();
                [cx + r * a.cos(), cy + r * a.sin(), 0.0]
            })
            .collect()
    }

    #[test]
    fn arc_fit_finds_ccw_arc() {
        let input = arc_points(5.0, 5.0, 2.0, 10.0, 80.0, 40);
        let out = Fitted::simplify(&input, 0.001, true);
        assert_eq!(out.len(), 2, "{out:?}");
        let Fitted::Arc { to, ccw, offset } = out[1] else {
            panic!("expected arc");
        };
        assert!(ccw);
        assert_eq!(to, input[40]);
        assert_abs_diff_eq!(input[0][0] + offset[0], 5.0, epsilon = 1e-6);
        assert_abs_diff_eq!(input[0][1] + offset[1], 5.0, epsilon = 1e-6);
    }

    #[test]
    fn arc_fit_finds_cw_arc() {
        let input = arc_points(0.0, 0.0, 3.0, 80.0, 10.0, 40);
        let out = Fitted::simplify(&input, 0.001, true);
        assert!(matches!(out[1], Fitted::Arc { ccw: false, .. }));
    }

    #[test]
    fn arc_fit_refuses_multiple_quadrants_as_single_arc() {
        let input = arc_points(0.0, 0.0, 3.0, 45.0, 135.0, 40);
        let out = Fitted::simplify(&input, 0.001, true);
        assert!(out.len() > 2, "should be split: {out:?}");
    }

    #[test]
    fn arc_fit_leaves_lines_alone() {
        let input = pts(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]);
        let out = Fitted::simplify(&input, 0.001, true);
        assert!(out.iter().all(|f| matches!(f, Fitted::Line(_))));
    }

    #[test]
    fn fittability_rejects_sharp_turn() {
        let center = (0.0, 0.0);
        let sharp =
            Fitted::arc_is_fittable(center, [1.0, 0.0, 0.0], [1.1, 0.1, 0.0], [1.0, 0.2, 0.0]);
        assert!(!sharp);
        let dup =
            Fitted::arc_is_fittable(center, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.2, 0.0]);
        assert!(!dup);
    }

    #[test]
    fn writer_omits_unchanged_axes() {
        let mut out = Vec::new();
        let mut w = MotionWriter::new(0.25, 0.001, ArcFit::None, 4);
        w.write_move(&mut out, "G1", [1.0, 2.0, 0.25], None);
        w.write_move(&mut out, "G1", [1.0, 3.0, 0.25], None);
        w.write_move(&mut out, "G1", [1.0, 3.0, 0.25], None);
        w.write_move(&mut out, "G1", [1.0, 3.0, -0.1], None);
        assert_eq!(out, ["G1 X1.0000 Y2.0000", "G1 Y3.0000", "G1 Z-0.1000"]);
    }

    #[test]
    fn writer_emits_feed_once_after_change() {
        let mut out = Vec::new();
        let mut w = MotionWriter::new(0.0, 0.001, ArcFit::None, 3);
        w.set_feed(&mut out, "5.0");
        w.write_move(&mut out, "G1", [1.0, 0.0, 0.0], None);
        w.write_move(&mut out, "G1", [2.0, 0.0, 0.0], None);
        assert_eq!(out, ["G1 X1.000 Y0.000F5.0", "G1 X2.000"]);
    }

    #[test]
    fn writer_arc_formats() {
        let mut out = Vec::new();
        let mut w = MotionWriter::new(0.0, 0.001, ArcFit::Center, 3);
        w.write_move(&mut out, "G1", [1.0, 0.0, 0.0], None);
        w.write_move(&mut out, "G3", [0.0, 1.0, 0.0], Some([-1.0, 0.0]));
        assert_eq!(out[1], "G3 X0.000 Y1.000 I-1.000 J0.000");

        let mut out = Vec::new();
        let mut w = MotionWriter::new(0.0, 0.001, ArcFit::Radius, 3);
        w.write_move(&mut out, "G1", [1.0, 0.0, 0.0], None);
        w.write_move(&mut out, "G2", [0.0, 1.0, 0.0], Some([-1.0, 0.0]));
        assert_eq!(out[1], "G2 X0.000 Y1.000 R1.000");
    }

    #[test]
    fn passes_single_when_no_stock() {
        let s = Settings::default();
        let mut p = Passes::new(&s);
        assert!(p.advance());
        assert!(!p.roughing);
        assert!(!p.advance());
    }

    #[test]
    fn passes_step_down_then_finish() {
        let s = Settings {
            v_rough_stock: 0.01,
            v_max_cut: -0.1,
            ..Settings::default()
        };
        let mut p = Passes::new(&s);
        let mut zmins = Vec::new();
        for _ in 0..3 {
            assert!(p.advance());
            assert!(p.roughing);
            zmins.push(p.zmin);
            p.rough_again = zmins.len() < 3;
        }
        assert_abs_diff_eq!(zmins[2], -0.3, epsilon = 1e-12);
        assert!(p.advance(), "finish pass runs after roughing");
        assert!(!p.roughing);
        assert!(!p.advance());
    }

    #[test]
    fn stock_is_capped_to_pass_depth() {
        let s = Settings {
            v_rough_stock: 0.5,
            v_max_cut: -0.1,
            ..Settings::default()
        };
        assert_abs_diff_eq!(Passes::new(&s).rough_stock, 0.1);
    }

    #[test]
    fn comment_text_is_sanitized() {
        assert_eq!(
            GcodeProgram::sanitized_comment("Hi (there)! a_b\n"),
            "Hi there a_b "
        );
    }

    fn drawing() -> Font {
        Font::from_drawing(vec![
            Stroke::new(0.0, 0.0, 0.0, 1.0),
            Stroke::new(0.0, 1.0, 1.0, 1.0),
            Stroke::new(1.0, 1.0, 1.0, 0.0),
            Stroke::new(1.0, 0.0, 0.0, 0.0),
        ])
    }

    fn engrave_settings() -> Settings {
        Settings {
            input_type: InputType::Image,
            y_scale: 1.0,
            stroke_thickness: 0.0,
            ..Settings::default()
        }
    }

    #[test]
    fn engrave_program_structure() {
        let s = engrave_settings();
        let d = Design::layout(&drawing(), &s).unwrap();
        let p = GcodeProgram::generate(&d, None, &s).unwrap();
        assert_eq!(p[0], "G90");
        assert_eq!(p[1], "G20");
        assert_eq!(p[2], "G17 G64 P0.001 M3 S3000");
        assert_eq!(p[3], "F5.00");
        assert_eq!(p[4], "G0 Z0.2500");
        assert_eq!(p[6], "G1 Z-0.0050");
        assert!(p.contains(&"G1 Y1.0000".to_string()));
        assert_eq!(p[p.len() - 3], "G0 Z0.2500");
        assert_eq!(&p[p.len() - 2..], ["M5", "M2"]);
    }

    #[test]
    fn metric_program_uses_g21_and_three_decimals() {
        let s = Settings {
            units: Units::Millimeters,
            y_scale: 10.0,
            ..engrave_settings()
        };
        let d = Design::layout(&drawing(), &s).unwrap();
        let p = GcodeProgram::generate(&d, None, &s).unwrap();
        assert!(p.contains(&"G21".to_string()));
        assert!(p.contains(&"F5.0".to_string()));
        assert!(p.contains(&"G0 Z0.250".to_string()));
    }

    #[test]
    fn variables_mode_uses_parameters() {
        let s = Settings {
            disable_variables: false,
            ..engrave_settings()
        };
        let d = Design::layout(&drawing(), &s).unwrap();
        let p = GcodeProgram::generate(&d, None, &s).unwrap();
        assert_eq!(p[0], "#1 = 0.2500  ( Safe Z )");
        assert_eq!(p[1], "#2 = -0.0050  ( Engraving Depth Z )");
        assert!(p.contains(&"G0 Z#1".to_string()));
        assert!(p.contains(&"G1 Z#2".to_string()));
    }

    #[test]
    fn plunge_rate_is_written_with_feed_restored() {
        let s = Settings {
            plunge: 2.0,
            ..engrave_settings()
        };
        let d = Design::layout(&drawing(), &s).unwrap();
        let p = GcodeProgram::generate(&d, None, &s).unwrap();
        assert!(p.contains(&"G1 Z-0.0050 F2.00".to_string()));
        assert!(
            p.iter()
                .any(|l| l.starts_with("G1 X") && l.ends_with("F5.00"))
        );
    }

    #[test]
    fn comments_embed_recoverable_settings() {
        let s = Settings {
            no_comments: false,
            text: "Hi!".into(),
            ..engrave_settings()
        };
        let d = Design::layout(&drawing(), &s).unwrap();
        let text = GcodeProgram::generate(&d, None, &s).unwrap().join("\n");
        assert!(text.contains("rengrave-"));
        let mut restored = Settings::default();
        assert!(restored.apply_settings_text(&text).is_empty());
        assert_eq!(restored, s);
    }

    #[test]
    fn pipes_split_header_and_footer() {
        let s = Settings {
            g_pre: "A|B".into(),
            g_post: "C|D".into(),
            ..engrave_settings()
        };
        let d = Design::layout(&drawing(), &s).unwrap();
        let p = GcodeProgram::generate(&d, None, &s).unwrap();
        assert_eq!(&p[2..4], ["A", "B"]);
        assert_eq!(&p[p.len() - 2..], ["C", "D"]);
    }

    #[test]
    fn engraved_circle_is_cut_with_g2() {
        let s = Settings {
            input_type: InputType::Text,
            text: "F".into(),
            text_radius: 3.0,
            add_box: true,
            ..engrave_settings()
        };
        let d = Design::layout(&drawing(), &s).unwrap();
        let p = GcodeProgram::generate(&d, None, &s).unwrap();
        let g2 = p.iter().find(|l| l.starts_with("G2 I")).expect("circle");
        assert!(g2.ends_with("J0.0000"), "{g2}");
    }

    fn carve_settings() -> Settings {
        Settings {
            cut_type: CutType::VCarve,
            ..engrave_settings()
        }
    }

    fn carved(s: &Settings) -> (Design, CarvePath) {
        let mut d = Design::layout(&drawing(), s).unwrap();
        let c = CarvePath::calculate(&mut d, s, &Progress::none()).unwrap();
        (d, c)
    }

    fn min_z(lines: &[String]) -> f64 {
        lines
            .iter()
            .filter_map(|l| l.split_whitespace().find_map(|w| w.strip_prefix('Z')))
            .filter_map(|z| z.parse::<f64>().ok())
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn v_carve_requires_paths() {
        let s = carve_settings();
        let d = Design::layout(&drawing(), &s).unwrap();
        assert!(GcodeProgram::generate(&d, None, &s).is_err());
    }

    #[test]
    fn v_carve_depth_matches_bit_geometry() {
        let s = carve_settings();
        let (d, c) = carved(&s);
        let p = GcodeProgram::generate(&d, Some(&c), &s).unwrap();
        assert_abs_diff_eq!(min_z(&p), s.max_cut_depth(), epsilon = 1e-3);
    }

    #[test]
    fn ball_depth_matches_bit_geometry() {
        let s = Settings {
            bit_shape: BitShape::Ball,
            ..carve_settings()
        };
        let (d, c) = carved(&s);
        let p = GcodeProgram::generate(&d, Some(&c), &s).unwrap();
        assert_abs_diff_eq!(min_z(&p), -0.25, epsilon = 1e-3);
    }

    #[test]
    fn flat_bit_cuts_at_full_depth_only() {
        let s = Settings {
            bit_shape: BitShape::Flat,
            v_bit_dia: 0.1,
            ..carve_settings()
        };
        let (d, c) = carved(&s);
        let p = GcodeProgram::generate(&d, Some(&c), &s).unwrap();
        assert!(p.contains(&"G1 Z-0.0500".to_string()));
    }

    #[test]
    fn multipass_limits_depth_of_first_pass() {
        let s = Settings {
            v_rough_stock: 0.02,
            v_max_cut: -0.1,
            ..carve_settings()
        };
        let (d, c) = carved(&s);
        let p = GcodeProgram::generate(&d, Some(&c), &s).unwrap();
        let first_final = p.iter().position(|l| l == "G0 Z0.2500").unwrap();
        // The deepest first-pass plunge is no deeper than the pass limit.
        let second_lift = p[first_final + 1..]
            .iter()
            .position(|l| l == "G0 Z0.2500")
            .unwrap()
            + first_final
            + 1;
        assert!(min_z(&p[first_final..second_lift]) >= -0.1 - 1e-3);
        assert_abs_diff_eq!(min_z(&p), s.max_cut_depth(), epsilon = 1e-3);
    }

    #[test]
    fn loops_are_ordered_by_begin_distance() {
        let cp = |x: f64, l: usize| CarvePoint {
            x,
            y: 0.0,
            r: 0.0,
            loop_num: l,
        };
        let points = [
            cp(0.0, 1),
            cp(1.0, 1),
            cp(50.0, 2),
            cp(51.0, 2),
            cp(2.0, 3),
            cp(3.0, 3),
        ];
        let out = GcodeProgram::order_loops_by_begin(&points);
        let xs: Vec<f64> = out.iter().map(|p| p.x).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0, 3.0, 50.0, 51.0]);
    }

    #[test]
    fn cleanup_program_depth_and_feeds() {
        let coords = [
            CarvePoint {
                x: 0.0,
                y: 0.0,
                r: 0.1,
                loop_num: 1,
            },
            CarvePoint {
                x: 1.0,
                y: 0.0,
                r: 0.1,
                loop_num: 1,
            },
        ];
        let s = Settings {
            plunge: 2.0,
            ..carve_settings()
        };
        let p = GcodeProgram::generate_cleanup(&coords, CleanupBit::Straight, &s).unwrap();
        assert!(p.contains(&"G1 Z-0.4330 F2.0000".to_string()));
        assert!(p.contains(&"G1 X1.0000 Y0.0000 F5.0000".to_string()));
        assert_eq!(p[p.len() - 3], "G0 Z0.2500");
    }

    #[test]
    fn cleanup_comments_name_the_tool() {
        let s = Settings {
            no_comments: false,
            ..carve_settings()
        };
        let v = GcodeProgram::generate_cleanup(&[], CleanupBit::VBit, &s)
            .unwrap()
            .join("\n");
        assert!(v.contains("angle of 60.0000 Degrees"));
        let f = GcodeProgram::generate_cleanup(&[], CleanupBit::Straight, &s)
            .unwrap()
            .join("\n");
        assert!(f.contains("Diameter of 0.2500 in"));
    }
}

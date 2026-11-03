//! Secondary cleanup operations that clear material a V-carve cannot reach.
//!
//! The area to clear is derived by offsetting the design's outlines. A straight bit then clears
//! flat-bottomed regions with perimeter, raster, and concentric cuts, and a small V-bit tidies
//! the band where those cuts meet the carved walls.

use crate::design::Design;
use crate::error::Result;
use crate::geom::{Point, Segment, ZERO};
use crate::settings::{CleanupBit, CleanupOps, CutType, Settings};
use crate::sort::PathRange;
use crate::vcarve::CarvePoint;
use clipper2_rust::{
    ClipperOffset, EndType, FillRule, JoinType, Path64, Paths64, Point64, difference_64,
};

/// Fixed-point scale for polygon offsetting.
const SCALE: f64 = 10000.0;

/// Fans a set of closed outlines out at different offsets.
struct Offsetter {
    paths: Paths64,
}

impl Offsetter {
    fn new(paths: Paths64) -> Self {
        Self { paths }
    }

    /// Offsets by `delta` in design units; positive grows outer loops.
    fn offset(&self, delta: f64) -> Paths64 {
        let mut clipper = ClipperOffset::new_default();
        clipper.add_paths(&self.paths, JoinType::Round, EndType::Polygon);
        let mut out = Paths64::new();
        clipper.execute(delta * SCALE, &mut out);
        out
    }
}

/// Converts sorted segments into one closed polygon per loop.
fn segments_to_paths(segments: &[Segment]) -> Paths64 {
    let fixed = |v: f64| (v * SCALE) as i64;
    let mut out = Paths64::new();
    let mut current = Path64::new();
    let mut last_loop = None;
    for s in segments {
        if last_loop.is_some() && last_loop != Some(s.line) {
            out.push(std::mem::take(&mut current));
        }
        current.push(Point64::new(fixed(s.x1), fixed(s.y1)));
        current.push(Point64::new(fixed(s.x2), fixed(s.y2)));
        last_loop = Some(s.line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn path_point(p: &Point64) -> Point {
    Point::new(p.x as f64 / SCALE, p.y as f64 / SCALE)
}

/// Expands polygons into closed perimeter runs, numbering loops after `last_loop`.
fn paths_to_loops(paths: &Paths64, radius: f64, last_loop: usize) -> Vec<CarvePoint> {
    let mut out = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let loop_num = last_loop + index + 1;
        let n = path.len();
        if n == 0 {
            continue;
        }
        for i in 0..=n {
            let p = path_point(&path[(i + 2 * n - 2) % n]);
            out.push(CarvePoint {
                x: p.x,
                y: p.y,
                r: radius,
                loop_num,
            });
        }
    }
    out
}

/// Expands polygons into their edges.
fn paths_to_edges(paths: &Paths64) -> Vec<Segment> {
    let mut out = Vec::new();
    for path in paths {
        let n = path.len();
        for i in 0..n {
            let a = path_point(&path[(i + n - 1) % n]);
            let b = path_point(&path[i]);
            out.push(Segment::new(a.x, a.y, b.x, b.y, 0, 0));
        }
    }
    out
}

/// Sorted intersections of the segment `a..b` with `edges`, as points along it.
fn line_intersections(a: Point, b: Point, edges: &[Segment]) -> Vec<Point> {
    const TOL: f64 = 1e-6;
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < TOL {
        return Vec::new();
    }
    let (sin, cos) = (dy / len, dx / len);
    let mut along: Vec<f64> = Vec::new();
    for e in edges {
        let xr0 = (e.x1 - a.x) * cos + (e.y1 - a.y) * sin;
        let yr0 = (e.x1 - a.x) * sin - (e.y1 - a.y) * cos;
        let xr1 = (e.x2 - a.x) * cos + (e.y2 - a.y) * sin;
        let yr1 = (e.x2 - a.x) * sin - (e.y2 - a.y) * cos;
        if yr0.min(yr1) < TOL && yr0.max(yr1) > TOL {
            let dxr = xr1 - xr0;
            let hit = if dxr.abs() < TOL {
                xr0
            } else {
                let m = (yr1 - yr0) / dxr;
                let b_r = yr1 - m * xr1;
                -b_r / m
            };
            if hit > TOL && hit < len - TOL {
                along.push(hit);
            }
        }
    }
    along.sort_by(f64::total_cmp);
    along
        .into_iter()
        .map(|t| Point::new(t * cos + a.x, t * sin + a.y))
        .collect()
}

/// Which axis raster passes run along.
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
}

/// Parameters shared by the raster cuts of one operation.
struct Raster<'a> {
    edges: &'a [Segment],
    extent: crate::geom::Bounds,
    step: f64,
    overlap: f64,
    skip_edge: usize,
    max_link: f64,
    radius: f64,
}

impl Raster<'_> {
    /// Generates parallel passes, then links nearby passes to reduce retracts.
    fn cut(&self, axis: Axis, loop_cnt: &mut usize, out: &mut Vec<CarvePoint>) {
        let (lo, hi) = match axis {
            Axis::X => (self.extent.min_y, self.extent.max_y),
            Axis::Y => (self.extent.min_x, self.extent.max_x),
        };
        let (lo, hi) = (lo + self.overlap, hi - self.overlap);
        let steps = ((hi - lo) / self.step).ceil();
        let mut passes: Vec<(f64, f64, usize)> = Vec::new();
        if steps > 0.0 && steps.is_finite() {
            for i in 0..=(steps as usize) {
                let at = lo + i as f64 / steps * (hi - lo);
                let (a, b) = match axis {
                    Axis::X => (
                        Point::new(self.extent.min_x - 1.0, at),
                        Point::new(self.extent.max_x + 1.0, at),
                    ),
                    Axis::Y => (
                        Point::new(at, self.extent.min_y - 1.0),
                        Point::new(at, self.extent.max_y + 1.0),
                    ),
                };
                let hits = line_intersections(a, b, self.edges);
                let mut j = self.skip_edge;
                while j + 1 + self.skip_edge < hits.len() {
                    let (p1, p2) = (hits[j], hits[j + 1]);
                    let span = match axis {
                        Axis::X => p2.x - p1.x,
                        Axis::Y => p2.y - p1.y,
                    };
                    if span > self.overlap * 2.0 {
                        *loop_cnt += 1;
                        match axis {
                            Axis::X => {
                                passes.push((p1.x + self.overlap, p1.y, *loop_cnt));
                                passes.push((p2.x - self.overlap, p2.y, *loop_cnt));
                            }
                            Axis::Y => {
                                passes.push((p1.x, p1.y + self.overlap, *loop_cnt));
                                passes.push((p2.x, p2.y - self.overlap, *loop_cnt));
                            }
                        }
                    }
                    j += 2;
                }
            }
        }
        *loop_cnt += 1;
        let order = PathRange::order_loops(&passes, |p| (p.0, p.1), |p| p.2);
        let (mut x_old, mut y_old) = (-999.0_f64, -999.0_f64);
        let mut loop_old = None;
        for range in order {
            for i in range.indices() {
                let (x, y, source) = passes[i];
                if (x - x_old).hypot(y - y_old) > self.max_link && Some(source) != loop_old {
                    *loop_cnt += 1;
                }
                out.push(CarvePoint {
                    x,
                    y,
                    r: self.radius,
                    loop_num: *loop_cnt,
                });
                x_old = x;
                y_old = y;
                loop_old = Some(source);
            }
        }
    }
}

/// The toolpaths for one cleanup operation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanupPaths {
    /// Tool positions, grouped by loop number.
    pub points: Vec<CarvePoint>,
}

impl CleanupPaths {
    /// Calculates cleanup toolpaths for a V-carved `design`, for the given bit.
    ///
    /// Returns an empty result for engraving jobs, when no cuts are enabled for the bit, or when
    /// the design leaves nothing to clear.
    pub fn calculate(design: &Design, settings: &Settings, bit: CleanupBit) -> Result<Self> {
        settings.ensure_valid()?;
        let ops: CleanupOps = match bit {
            CleanupBit::Straight => settings.clean_straight,
            CleanupBit::VBit => settings.clean_v_bit,
        };
        if settings.cut_type != CutType::VCarve || !ops.any() {
            return Ok(Self::default());
        }
        let flop = settings.effective_flop(true);
        let offset_sign = if flop { -1.0 } else { 1.0 };
        let clip = Segment::sort_into_loops(&design.segments, settings.accuracy);
        if clip.is_empty() {
            return Ok(Self::default());
        }
        let source = Offsetter::new(segments_to_paths(&clip));
        let rbit = settings.effective_bit_dia() / 2.0;

        let (clean_dia, clean_step) = match bit {
            CleanupBit::Straight => (settings.clean_dia, settings.clean_step / 100.0),
            CleanupBit::VBit => {
                if settings.clean_v < ZERO {
                    return Ok(Self::default());
                }
                (settings.clean_v, 1.0)
            }
        };
        let radius = clean_dia / 2.0;
        let dx = clean_dia * clean_step;
        let max_link = match bit {
            CleanupBit::Straight => clean_dia,
            CleanupBit::VBit => (dx * dx + dx * dx).sqrt() * 1.1,
        };
        // Loops are only meaningful when clearing inside the design with a straight bit.
        let wants_loops = ops.loops && (bit == CleanupBit::VBit || !flop);

        let clean_loops = match bit {
            CleanupBit::Straight => source.offset(offset_sign * -(clean_dia / 2.0 + rbit)),
            CleanupBit::VBit => {
                // Outward to the toolpath of full-depth cuts...
                let flat_r = settings.clean_dia / 2.0;
                let full_depth = source.offset(offset_sign * -(flat_r + rbit));
                // ...then back to the extent the straight bit reached, plus a little...
                let reached =
                    Offsetter::new(full_depth).offset(offset_sign * (flat_r + clean_dia / 4.0));
                // ...and subtract from the walls of the full-depth cut.
                let walls = source.offset(offset_sign * (-rbit - clean_dia / 4.0));
                if flop {
                    difference_64(&reached, &walls, FillRule::EvenOdd)
                } else {
                    difference_64(&walls, &reached, FillRule::EvenOdd)
                }
            }
        };

        let edges = paths_to_edges(&clean_loops);
        let perimeter = paths_to_loops(&clean_loops, radius, 0);
        let mut extent = crate::geom::Bounds::EMPTY;
        for p in &perimeter {
            extent.include(p.x, p.y);
        }
        let mut loop_cnt = 0;
        let mut out: Vec<CarvePoint> = Vec::new();
        if ops.perimeter || wants_loops {
            out.clone_from(&perimeter);
            if let Some(last) = out.last() {
                loop_cnt = last.loop_num;
            }
        }

        if extent.is_empty() {
            return Ok(Self { points: out });
        }

        // Loop numbers for raster passes are reserved before concentric loops are added, so the
        // passes are computed into their own buffers first.
        let raster = Raster {
            edges: &edges,
            extent,
            step: clean_dia * clean_step,
            overlap: dx / 2.0,
            skip_edge: usize::from(flop && bit == CleanupBit::Straight),
            max_link,
            radius,
        };
        if wants_loops {
            let dl = clean_dia * clean_step / 2.0;
            let inner = Offsetter::new(clean_loops.clone());
            let mut offset = 0.0;
            loop {
                offset -= dl;
                let rings = inner.offset(offset);
                if rings.is_empty() {
                    break;
                }
                let ring_points = paths_to_loops(&rings, radius, loop_cnt);
                if let Some(last) = ring_points.last() {
                    loop_cnt = last.loop_num;
                }
                out.extend(ring_points);
            }
        }
        if ops.x {
            raster.cut(Axis::X, &mut loop_cnt, &mut out);
        }
        if ops.y {
            raster.cut(Axis::Y, &mut loop_cnt, &mut out);
        }
        Ok(Self { points: out })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Font, Stroke};
    use crate::settings::InputType;
    use approx::assert_abs_diff_eq;

    /// A clockwise square, side `size`, with an optional clockwise-wound hole.
    fn font(size: f64, hole: Option<(f64, f64)>) -> Font {
        let mut strokes = vec![
            Stroke::new(0.0, 0.0, 0.0, size),
            Stroke::new(0.0, size, size, size),
            Stroke::new(size, size, size, 0.0),
            Stroke::new(size, 0.0, 0.0, 0.0),
        ];
        if let Some((a, b)) = hole {
            strokes.extend([
                Stroke::new(a, a, b, a),
                Stroke::new(b, a, b, b),
                Stroke::new(b, b, a, b),
                Stroke::new(a, b, a, a),
            ]);
        }
        Font::from_drawing(strokes)
    }

    fn settings() -> Settings {
        Settings {
            input_type: InputType::Image,
            cut_type: CutType::VCarve,
            y_scale: 10.0,
            v_bit_dia: 1.0,
            v_bit_angle: 90.0,
            clean_dia: 0.5,
            clean_step: 50.0,
            clean_v: 0.1,
            ..Settings::default()
        }
    }

    fn design(font: &Font, s: &Settings) -> Design {
        Design::layout(font, s).unwrap()
    }

    fn only(ops: CleanupOps) -> Settings {
        Settings {
            clean_straight: ops,
            clean_v_bit: ops,
            ..settings()
        }
    }

    const PERIMETER: CleanupOps = CleanupOps {
        perimeter: true,
        x: false,
        y: false,
        loops: false,
    };
    const X_ONLY: CleanupOps = CleanupOps {
        perimeter: false,
        x: true,
        y: false,
        loops: false,
    };
    const Y_ONLY: CleanupOps = CleanupOps {
        perimeter: false,
        x: false,
        y: true,
        loops: false,
    };
    const LOOPS: CleanupOps = CleanupOps {
        perimeter: false,
        x: false,
        y: false,
        loops: true,
    };

    fn bounds_of(points: &[CarvePoint]) -> crate::geom::Bounds {
        let mut b = crate::geom::Bounds::EMPTY;
        for p in points {
            b.include(p.x, p.y);
        }
        b
    }

    #[test]
    fn straight_perimeter_is_inset_by_bit_radii() {
        let s = only(PERIMETER);
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                .unwrap();
        let b = bounds_of(&paths.points);
        // Inset = cleanup radius (0.25) + carve radius (0.5).
        assert_abs_diff_eq!(b.min_x, 0.75, epsilon = 1e-3);
        assert_abs_diff_eq!(b.max_x, 9.25, epsilon = 1e-3);
        assert_abs_diff_eq!(b.min_y, 0.75, epsilon = 1e-3);
        assert_abs_diff_eq!(b.max_y, 9.25, epsilon = 1e-3);
        assert!(paths.points.iter().all(|p| p.r == 0.25));
    }

    #[test]
    fn perimeter_loop_closes() {
        let s = only(PERIMETER);
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                .unwrap();
        let (first, last) = (paths.points[0], paths.points[paths.points.len() - 1]);
        assert_eq!((first.x, first.y), (last.x, last.y));
    }

    #[test]
    fn holes_grow_when_outer_shrinks() {
        let s = only(PERIMETER);
        let f = font(10.0, Some((4.0, 6.0)));
        let paths = CleanupPaths::calculate(&design(&f, &s), &s, CleanupBit::Straight).unwrap();
        let ids: std::collections::BTreeSet<_> = paths.points.iter().map(|p| p.loop_num).collect();
        assert_eq!(ids.len(), 2, "outer and hole loops");
        let hole: Vec<_> = ids
            .iter()
            .map(|id| {
                let pts: Vec<_> = paths
                    .points
                    .iter()
                    .copied()
                    .filter(|p| p.loop_num == *id)
                    .collect();
                bounds_of(&pts)
            })
            .min_by(|a, b| a.width().total_cmp(&b.width()))
            .into_iter()
            .collect();
        // Hole 2 wide, grown by 0.75 each side.
        assert_abs_diff_eq!(hole[0].width(), 3.5, epsilon = 1e-2);
    }

    #[test]
    fn raster_passes_stay_within_cleared_area() {
        for (ops, axis_x) in [(X_ONLY, true), (Y_ONLY, false)] {
            let s = only(ops);
            let paths =
                CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                    .unwrap();
            assert!(!paths.points.is_empty());
            let b = bounds_of(&paths.points);
            // Passes run from wall to wall, less half a step.
            let (lo, hi) = (0.75 + 0.125, 9.25 - 0.125);
            let (along_lo, along_hi) = if axis_x {
                (b.min_x, b.max_x)
            } else {
                (b.min_y, b.max_y)
            };
            assert_abs_diff_eq!(along_lo, lo, epsilon = 1e-3);
            assert_abs_diff_eq!(along_hi, hi, epsilon = 1e-3);
        }
    }

    #[test]
    fn raster_spacing_matches_step_over() {
        let s = only(X_ONLY);
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                .unwrap();
        let mut ys: Vec<f64> = paths.points.iter().map(|p| p.y).collect();
        ys.sort_by(f64::total_cmp);
        ys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        for pair in ys.windows(2) {
            assert!(
                pair[1] - pair[0] <= 0.25 + 1e-9,
                "gap {}",
                pair[1] - pair[0]
            );
        }
    }

    #[test]
    fn raster_avoids_islands() {
        let s = only(X_ONLY);
        let f = font(10.0, Some((4.0, 6.0)));
        let paths = CleanupPaths::calculate(&design(&f, &s), &s, CleanupBit::Straight).unwrap();
        // Tool centers stay at least the two radii (0.75) from the island.
        for p in &paths.points {
            let dx = (4.0 - p.x).max(p.x - 6.0).max(0.0);
            let dy = (4.0 - p.y).max(p.y - 6.0).max(0.0);
            assert!(dx.hypot(dy) >= 0.75 - 1e-2, "{p:?}");
        }
    }

    #[test]
    fn concentric_loops_shrink_until_gone() {
        let s = only(LOOPS);
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                .unwrap();
        let ids: std::collections::BTreeSet<_> = paths.points.iter().map(|p| p.loop_num).collect();
        // 8.5 wide area, shrinking 0.125 per side per ring.
        assert!(ids.len() > 25, "{}", ids.len());
    }

    #[test]
    fn flipped_straight_loop_cleanup_is_disabled() {
        let s = Settings {
            v_flop: true,
            add_box: true,
            ..only(LOOPS)
        };
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                .unwrap();
        assert!(paths.points.is_empty());
    }

    #[test]
    fn v_bit_band_follows_carved_walls() {
        let s = only(PERIMETER);
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::VBit).unwrap();
        assert!(!paths.points.is_empty());
        let b = bounds_of(&paths.points);
        // Outside the straight perimeter (0.75) and within the walls of the carve (0.5+0.025).
        assert!(b.min_x >= 0.5 && b.min_x < 0.75, "{}", b.min_x);
        assert!(paths.points.iter().all(|p| p.r == 0.05));
    }

    #[test]
    fn v_bit_zero_width_disables_operation() {
        let s = Settings {
            clean_v: 0.0,
            ..only(PERIMETER)
        };
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::VBit).unwrap();
        assert!(paths.points.is_empty());
    }

    #[test]
    fn engrave_jobs_and_disabled_ops_have_no_cleanup() {
        let none = CleanupOps {
            perimeter: false,
            x: false,
            y: false,
            loops: false,
        };
        let s = only(none);
        let d = design(&font(10.0, None), &s);
        assert!(
            CleanupPaths::calculate(&d, &s, CleanupBit::Straight)
                .unwrap()
                .points
                .is_empty()
        );
        let e = Settings {
            cut_type: CutType::Engrave,
            ..settings()
        };
        assert!(
            CleanupPaths::calculate(&d, &e, CleanupBit::Straight)
                .unwrap()
                .points
                .is_empty()
        );
    }

    #[test]
    fn oversized_bit_leaves_nothing() {
        let s = Settings {
            clean_dia: 30.0,
            ..only(PERIMETER)
        };
        let paths =
            CleanupPaths::calculate(&design(&font(10.0, None), &s), &s, CleanupBit::Straight)
                .unwrap();
        assert!(paths.points.is_empty());
    }

    #[test]
    fn intersections_are_sorted_and_skip_endpoints() {
        let edges = [
            Segment::new(5.0, -1.0, 5.0, 1.0, 0, 0),
            Segment::new(2.0, -1.0, 2.0, 1.0, 0, 0),
            Segment::new(8.0, 1.0, 9.0, 2.0, 0, 0),
        ];
        let hits = line_intersections(Point::new(0.0, 0.0), Point::new(10.0, 0.0), &edges);
        assert_eq!(hits.len(), 2);
        assert_abs_diff_eq!(hits[0].x, 2.0, epsilon = 1e-9);
        assert_abs_diff_eq!(hits[1].x, 5.0, epsilon = 1e-9);
    }

    #[test]
    fn intersection_of_degenerate_line_is_empty() {
        let edges = [Segment::new(0.0, -1.0, 0.0, 1.0, 0, 0)];
        assert!(line_intersections(Point::new(1.0, 1.0), Point::new(1.0, 1.0), &edges).is_empty());
    }

    #[test]
    fn loop_expansion_starts_at_second_to_last_vertex() {
        let square: Path64 = vec![
            Point64::new(0, 0),
            Point64::new(10000, 0),
            Point64::new(10000, 10000),
            Point64::new(0, 10000),
        ];
        let out = paths_to_loops(&vec![square], 0.1, 5);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0].loop_num, 6);
        assert_eq!((out[0].x, out[0].y), (1.0, 1.0));
        assert_eq!((out[4].x, out[4].y), (1.0, 1.0));
    }
}

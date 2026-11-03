//! V-carve toolpath calculation.
//!
//! At each sample along an outline, the engine finds the largest circle tangent to the outline
//! at that sample which touches no other outline segment. The center of the circle is the tool
//! position, and its radius sets the depth.

use crate::design::Design;
use crate::error::Result;
use crate::geom::{Point, Segment, ZERO, angle_from_sin_cos};
use crate::progress::Progress;
use crate::settings::{BitShape, CheckScope, InputType, Settings};
use crate::sort::sort_for_v_carve;

/// A tool position with its cone radius at the material surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarvePoint {
    /// Tool X.
    pub x: f64,
    /// Tool Y.
    pub y: f64,
    /// Radius of the cut at the surface; depth derives from this and the bit shape.
    pub r: f64,
    /// Number of the toolpath loop this point belongs to.
    pub loop_num: usize,
}

/// The result of a V-carve calculation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CarvePath {
    /// Tool positions in cutting order.
    pub points: Vec<CarvePoint>,
}

/// One segment's bounding circle, used for fast rejection.
#[derive(Clone, Copy)]
struct SegmentCircle {
    x: f64,
    y: f64,
    r: f64,
}

/// Which outline neighbors to test, by uniform grid cell.
struct Partitions {
    cells: Vec<Vec<u32>>,
    x_cells: usize,
    y_cells: usize,
    x_len: f64,
    y_len: f64,
    min_x: f64,
    min_y: f64,
}

impl Partitions {
    fn new(segments: &[Segment], design: &Design, rmax: f64, dline: f64) -> Self {
        let x_length = design.bounds.width();
        let y_length = design.bounds.height();
        let x_minus_1 = ((x_length / ((2.0 * rmax + dline) * 1.1)) as usize).max(1);
        let y_minus_1 = ((y_length / ((2.0 * rmax + dline) * 1.1)) as usize).max(1);
        let mut x_len = x_length / x_minus_1 as f64;
        let mut y_len = y_length / y_minus_1 as f64;
        let x_cells = x_minus_1 + 1;
        let y_cells = y_minus_1 + 1;
        if x_len < ZERO {
            x_len = 1.0;
        }
        if y_len < ZERO {
            y_len = 1.0;
        }
        let mut this = Self {
            cells: vec![Vec::new(); x_cells * y_cells],
            x_cells,
            y_cells,
            x_len,
            y_len,
            min_x: design.bounds.min_x,
            min_y: design.bounds.min_y,
        };
        let mut occupied: Vec<usize> = Vec::new();
        for (index, s) in segments.iter().enumerate() {
            occupied.clear();
            this.cells_for_segment(s, &mut occupied);
            occupied.sort_unstable();
            occupied.dedup();
            for &cell in &occupied {
                this.cells[cell].push(index as u32);
            }
        }
        this
    }

    /// Collects each cell the segment crosses, plus all adjacent cells.
    fn cells_for_segment(&self, s: &Segment, out: &mut Vec<usize>) {
        let x1 = s.x1 - self.min_x;
        let y1 = s.y1 - self.min_y;
        let x2 = s.x2 - self.min_x;
        let y2 = s.y2 - self.min_y;

        let xi1 = (x1 / self.x_len) as i64;
        let xi2 = (x2 / self.x_len) as i64;
        let yi1 = (y1 / self.y_len) as i64;
        let yi2 = (y2 / self.y_len) as i64;
        let (x_min, x_max) = (xi1.min(xi2), xi1.max(xi2));
        let (y_min, y_max) = (yi1.min(yi2), yi1.max(yi2));

        let mut mark = |xc: i64, yc: i64| {
            let x_hi = (xc + 2).min(self.x_cells as i64);
            let y_hi = (yc + 2).min(self.y_cells as i64);
            for i in (xc - 1).max(0)..x_hi {
                for j in (yc - 1).max(0)..y_hi {
                    out.push(i as usize + j as usize * self.x_cells);
                }
            }
        };

        if x_max > x_min && (x2 - x1).abs() > ZERO {
            if y_max > y_min && (y2 - y1).abs() > ZERO {
                mark(xi1, yi1);
                mark(xi2, yi2);
                let m = (y2 - y1) / (x2 - x1);
                let b = y1 - m * x1;
                for xc in x_min + 1..x_max {
                    let x = xc as f64 * self.x_len;
                    mark(xc, ((m * x + b) / self.y_len) as i64);
                }
                for yc in y_min + 1..y_max {
                    let y = yc as f64 * self.y_len;
                    mark(((y - b) / m / self.x_len) as i64, yc);
                }
            } else {
                for xc in x_min..=x_max {
                    mark(xc, y_min);
                }
            }
        } else {
            for yc in y_min..=y_max {
                mark(x_min, yc);
            }
        }
    }

    fn at(&self, x: f64, y: f64) -> &[u32] {
        let xi = (((x - self.min_x) / self.x_len) as usize).min(self.x_cells - 1);
        let yi = (((y - self.min_y) / self.y_len) as usize).min(self.y_cells - 1);
        &self.cells[xi + yi * self.x_cells]
    }
}

/// Holds the outline being carved and answers maximum-circle queries.
struct Carver<'a> {
    segments: &'a [Segment],
    circles: Vec<SegmentCircle>,
    partitions: Partitions,
    rmax: f64,
    scope: CheckScope,
}

impl Carver<'_> {
    /// Largest circle radius, up to the tool's, at `(x, y)` tangent to a line with the given
    /// transformed orientation. `corner` excludes segments that meet at the point.
    fn max_circle(&self, x: f64, y: f64, char_num: usize, sin: f64, cos: f64, corner: bool) -> f64 {
        let mut rmin = self.rmax;
        let reach = self.rmax.abs();
        for &index in self.partitions.at(x, y) {
            let circle = self.circles[index as usize];
            let gap = ((circle.x - x) * (circle.x - x) + (circle.y - y) * (circle.y - y)).sqrt();
            if gap >= (reach + circle.r).abs() {
                continue;
            }
            let s = &self.segments[index as usize];
            if x < s.x1.min(s.x2) - rmin * 2.0
                || y < s.y1.min(s.y2) - rmin * 2.0
                || x > s.x1.max(s.x2) + rmin * 2.0
                || y > s.y1.max(s.y2) + rmin * 2.0
            {
                continue;
            }
            if self.scope == CheckScope::Char && char_num != s.char_num {
                continue;
            }
            if corner {
                let at_start = (x - s.x1).abs() <= ZERO && (y - s.y1).abs() <= ZERO;
                let at_end = (x - s.x2).abs() <= ZERO && (y - s.y2).abs() <= ZERO;
                if at_start || at_end {
                    continue;
                }
            }
            rmin = rmin.min(Self::tangent_limit(s, x, y, sin, cos, rmin));
        }
        rmin
    }

    /// Limit on the circle radius imposed by one segment.
    ///
    /// In the local frame the sample is the origin and the circle's center lies on +Y.
    fn tangent_limit(s: &Segment, x: f64, y: f64, sin: f64, cos: f64, start: f64) -> f64 {
        let mut rmin = start;
        let xc1 = (s.x1 - x) * cos - (s.y1 - y) * sin;
        let yc1 = (s.x1 - x) * sin + (s.y1 - y) * cos;
        let xc2 = (s.x2 - x) * cos - (s.y2 - y) * sin;
        let yc2 = (s.x2 - x) * sin + (s.y2 - y) * cos;

        if (xc2 - xc1).abs() < ZERO && (yc2 - yc1).abs() > ZERO {
            let r = xc1.abs();
            if yc1.max(yc2) >= r && yc1.min(yc2) <= r {
                rmin = rmin.min(r);
            }
        } else if (yc2 - yc1).abs() < ZERO
            && (xc2 - xc1).abs() > ZERO
            && xc1.max(xc2) >= 0.0
            && xc1.min(xc2) <= 0.0
            && yc1 > ZERO
        {
            rmin = rmin.min(yc1 / 2.0);
        }

        if (yc2 - yc1).abs() > ZERO && (xc2 - xc1).abs() > ZERO {
            let m = (yc2 - yc1) / (xc2 - xc1);
            let b = yc1 - m * xc1;
            let sq = m + 1.0 / m;
            let a = 1.0 + m * m - 2.0 * m * sq;
            let bb = -2.0 * b * sq;
            let c = -b * b;
            let disc = bb * bb - 4.0 * a * c;
            if disc >= 0.0 {
                let root = disc.sqrt();
                for xq in [(-bb + root) / (2.0 * a), (-bb - root) / (2.0 * a)] {
                    if xq >= xc1.min(xc2) && xq <= xc1.max(xc2) {
                        let r = xq * sq + b;
                        if r >= 0.0 {
                            rmin = rmin.min(r);
                        }
                    }
                }
            }
        }

        if yc1 > ZERO {
            rmin = rmin.min((xc1 * xc1 + yc1 * yc1) / (2.0 * yc1));
        }
        if yc2 > ZERO {
            rmin = rmin.min((xc2 * xc2 + yc2 * yc2) / (2.0 * yc2));
        }

        if yc1.abs() < ZERO && xc1.abs() < ZERO && yc2 > ZERO {
            rmin = 0.0;
        }
        if yc2.abs() < ZERO && xc2.abs() < ZERO && yc1 > ZERO {
            rmin = 0.0;
        }
        rmin
    }
}

impl CarvePath {
    /// Records a tool position offset by `r` from `(x, y)` along the normal at `phi` radians.
    fn record(&mut self, x: f64, y: f64, phi: f64, r: f64, loop_num: usize) {
        let offset = Point::new(0.0, r).rotated(-phi);
        self.points.push(CarvePoint {
            x: x + offset.x,
            y: y + offset.y,
            r,
            loop_num,
        });
    }

    /// Calculates the V-carve toolpath for `design`.
    ///
    /// For image input, `design`'s segments are first replaced by closed, oriented loops, as
    /// produced by [`sort_for_v_carve`]. Progress is reported in terms of outline length, and
    /// canceling returns [`Error::Canceled`].
    pub fn calculate(
        design: &mut Design,
        settings: &Settings,
        progress: &Progress,
    ) -> Result<Self> {
        settings.ensure_valid()?;
        if settings.input_type == InputType::Image {
            design.segments = sort_for_v_carve(&design.segments, settings.accuracy);
        }

        let mut path = Self::default();
        let total_length: f64 = design.segments.iter().map(Segment::length).sum();
        if design.segments.is_empty() || total_length <= 0.0 {
            return Ok(path);
        }

        let rmax = settings.effective_bit_dia() / 2.0;
        let dline = settings.v_step_len.max(settings.units.min_step_len());
        let dangle = (dline / rmax).to_degrees().max(2.0);
        let step_corner = settings.v_step_corner;
        let drive_corner = if settings.inlay {
            360.0 - step_corner
        } else {
            settings.v_drive_corner
        };
        let not_ball = settings.bit_shape != BitShape::Ball;
        let drives_to_corner = settings.v_bit_angle != 0.0 && not_ball;
        let scope = if settings.input_type == InputType::Text {
            settings.v_check_scope
        } else {
            CheckScope::All
        };
        let flop = settings.effective_flop(false);

        let segments = design.segments.as_slice();
        let carver = Carver {
            segments,
            circles: segments
                .iter()
                .map(|s| SegmentCircle {
                    x: (s.x1 + s.x2) / 2.0,
                    y: (s.y1 + s.y2) / 2.0,
                    r: s.length() / 2.0 + rmax,
                })
                .collect(),
            partitions: Partitions::new(segments, design, rmax, dline),
            rmax,
            scope,
        };

        // Walk state: the start of the current loop and the previously processed segment.
        let (mut xa, mut ya, mut xb, mut yb) = (9999.0, 9999.0, 9999.0, 9999.0);
        let (mut x0, mut y0) = (9999.0, 9999.0);
        let (mut sin0, mut cos0) = (2.0_f64, 2.0_f64);
        let mut char0: Option<usize> = None;
        let mut theta = 9999.0;
        let mut loop_cnt = 0_usize;
        let (mut xpta, mut ypta, mut phi2a, mut routa) = (0.0, 0.0, 0.0, 0.0);
        let mut cur_length = 0.0;

        for i in 0..segments.len() {
            progress.check_canceled()?;
            progress.report(cur_length / total_length);

            let s = if flop {
                segments[segments.len() - 1 - i]
            } else {
                segments[i]
            };
            let (x1, y1, x2, y2) = if flop {
                (s.x2, s.y2, s.x1, s.y1)
            } else {
                (s.x1, s.y1, s.x2, s.y2)
            };
            let char_num = s.char_num;
            let dx = x2 - x1;
            let dy = y2 - y1;
            let lseg = (dx * dx + dy * dy).sqrt();
            if lseg < ZERO {
                continue;
            }
            let seg_sin = dy / lseg;
            let seg_cos = -dx / lseg;
            let phi = angle_from_sin_cos(seg_sin, seg_cos);
            cur_length += lseg;

            let new_loop =
                (x1 - x0).abs() > ZERO || (y1 - y0).abs() > ZERO || Some(char_num) != char0;
            if new_loop {
                loop_cnt += 1;
                xa = x1;
                ya = y1;
                xb = x2;
                yb = y2;
                theta = 9999.0;
                sin0 = 2.0;
                cos0 = 2.0;
            }

            let delta = if cos0 > 1.0 {
                180.0
            } else {
                turn_angle(x2 - x1, y2 - y1, sin0, cos0)
            };
            if delta < drive_corner && drives_to_corner {
                path.points.push(CarvePoint {
                    x: x1,
                    y: y1,
                    r: 0.0,
                    loop_num: loop_cnt,
                });
            }
            if delta > step_corner {
                path.corner_steps(&carver, x1, y1, delta, dangle, theta, char_num, loop_cnt);
            }
            theta = phi;
            x0 = x2;
            y0 = y2;
            sin0 = seg_sin;
            cos0 = seg_cos;
            char0 = Some(char_num);

            let nsteps = (lseg / dline).floor().max(2.0);
            let dxpt = dx / nsteps;
            let dypt = dy / nsteps;
            let mut cnt: i64 = if new_loop && drives_to_corner { -1 } else { 0 };
            let phi2 = phi.to_radians();
            let last_step = nsteps as i64 - 1;
            while cnt < last_step {
                cnt += 1;
                let xpt = x1 + dxpt * cnt as f64;
                let ypt = y1 + dypt * cnt as f64;
                let mut rout = carver.max_circle(xpt, ypt, char_num, seg_sin, seg_cos, false);
                if cnt == 0 && not_ball {
                    rout = 0.0;
                }
                path.record(xpt, ypt, phi2, rout, loop_cnt);
                if new_loop && cnt == 1 {
                    xpta = xpt;
                    ypta = ypt;
                    phi2a = phi2;
                    routa = rout;
                }
            }

            // Close a loop that has returned to its start.
            if (x2 - xa).abs() < ZERO && (y2 - ya).abs() < ZERO {
                let delta = turn_angle(xb - xa, yb - ya, sin0, cos0);
                if delta < drive_corner {
                    path.points.push(CarvePoint {
                        x: xa,
                        y: ya,
                        r: 0.0,
                        loop_num: loop_cnt,
                    });
                } else if delta > step_corner {
                    path.corner_steps(&carver, xa, ya, delta, dangle, theta, char_num, loop_cnt);
                    path.record(xpta, ypta, phi2a, routa, loop_cnt);
                } else {
                    path.record(xpta, ypta, phi2a, routa, loop_cnt);
                }
            }
        }
        progress.report(1.0);
        Ok(path)
    }

    /// Adds samples that sweep the tool normal around an outside corner.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors one walk step's local state"
    )]
    fn corner_steps(
        &mut self,
        carver: &Carver<'_>,
        x: f64,
        y: f64,
        delta: f64,
        dangle: f64,
        theta: f64,
        char_num: usize,
        loop_num: usize,
    ) {
        let phisteps = ((delta - 180.0) / dangle).floor().max(2.0);
        let step_phi = (delta - 180.0) / phisteps;
        let mut pcnt = 0.0;
        while pcnt < phisteps - 1.0 {
            pcnt += 1.0;
            let sub_phi = (-pcnt * step_phi + theta).to_radians();
            let rout = carver.max_circle(x, y, char_num, sub_phi.sin(), sub_phi.cos(), true);
            self.record(x, y, sub_phi, rout, loop_num);
        }
    }
}

/// The turn between the previous segment's frame and a new direction, in degrees.
fn turn_angle(dx: f64, dy: f64, sin0: f64, cos0: f64) -> f64 {
    let xt = dx * cos0 - dy * sin0;
    let yt = dx * sin0 + dy * cos0;
    let l = (xt * xt + yt * yt).sqrt();
    angle_from_sin_cos(yt / l, xt / l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::font::{Font, Stroke};
    use crate::settings::{CutType, Units};
    use approx::assert_abs_diff_eq;

    fn square_font(size: f64) -> Font {
        Font::from_drawing(vec![
            Stroke::new(0.0, 0.0, 0.0, size),
            Stroke::new(0.0, size, size, size),
            Stroke::new(size, size, size, 0.0),
            Stroke::new(size, 0.0, 0.0, 0.0),
        ])
    }

    fn carve_settings() -> Settings {
        Settings {
            input_type: InputType::Image,
            cut_type: CutType::VCarve,
            y_scale: 1.0,
            v_step_len: 0.01,
            ..Settings::default()
        }
    }

    fn carve(font: &Font, settings: &Settings) -> CarvePath {
        let mut design = Design::layout(font, settings).unwrap();
        CarvePath::calculate(&mut design, settings, &Progress::none()).unwrap()
    }

    #[test]
    fn square_centerline_reaches_inscribed_radius() {
        let path = carve(&square_font(1.0), &carve_settings());
        let max_r = path.points.iter().map(|p| p.r).fold(0.0, f64::max);
        // 0.5-diameter bit: the tool's radius (0.25) is less than the inscribed radius (0.5).
        assert_abs_diff_eq!(max_r, 0.25, epsilon = 1e-12);
        for p in &path.points {
            assert!(
                (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y),
                "{p:?}"
            );
        }
    }

    #[test]
    fn small_square_limited_by_opposite_walls() {
        let settings = Settings {
            y_scale: 0.2,
            ..carve_settings()
        };
        let path = carve(&square_font(1.0), &settings);
        let max_r = path.points.iter().map(|p| p.r).fold(0.0, f64::max);
        assert_abs_diff_eq!(max_r, 0.1, epsilon = 1e-3);
    }

    #[test]
    fn tool_circle_never_crosses_outline() {
        // Every carve circle must stay inside the square it is cut in.
        let settings = Settings {
            y_scale: 0.3,
            ..carve_settings()
        };
        let path = carve(&square_font(1.0), &settings);
        assert!(!path.points.is_empty());
        for p in &path.points {
            let wall = p.x.min(p.y).min(0.3 - p.x).min(0.3 - p.y);
            assert!(
                p.r <= wall + 1e-3,
                "r {} exceeds wall {} at {p:?}",
                p.r,
                wall
            );
        }
    }

    #[test]
    fn loop_starts_at_surface() {
        let path = carve(&square_font(1.0), &carve_settings());
        assert_eq!(path.points[0].r, 0.0);
        assert_eq!(path.points[0].loop_num, 1);
    }

    #[test]
    fn single_loop_for_single_closed_outline() {
        let path = carve(&square_font(1.0), &carve_settings());
        assert!(path.points.iter().all(|p| p.loop_num == 1));
    }

    #[test]
    fn corner_drive_touches_corners() {
        let path = carve(&square_font(1.0), &carve_settings());
        let touches = |x: f64, y: f64| {
            path.points
                .iter()
                .any(|p| p.r == 0.0 && (p.x - x).abs() < 1e-9 && (p.y - y).abs() < 1e-9)
        };
        assert!(touches(0.0, 1.0) && touches(1.0, 1.0) && touches(1.0, 0.0));
    }

    #[test]
    fn ball_bit_skips_drive_to_corner_and_first_plunge() {
        let settings = Settings {
            bit_shape: BitShape::Ball,
            ..carve_settings()
        };
        let path = carve(&square_font(1.0), &settings);
        assert!(path.points[0].r > 0.0);
    }

    #[test]
    fn flop_cuts_outside_the_outline() {
        let settings = Settings {
            v_flop: true,
            ..carve_settings()
        };
        let path = carve(&square_font(1.0), &settings);
        assert!(
            path.points
                .iter()
                .any(|p| p.x > 1.0 + 1e-3 || p.x < -1e-3 || p.y > 1.0 + 1e-3 || p.y < -1e-3),
            "flopped path should leave the square"
        );
    }

    #[test]
    fn empty_outline_gives_empty_path() {
        let settings = carve_settings();
        let mut design = Design::layout(&square_font(1.0), &settings).unwrap();
        design.segments.clear();
        let path = CarvePath::calculate(&mut design, &settings, &Progress::none()).unwrap();
        assert!(path.points.is_empty());
    }

    #[test]
    fn cancel_stops_calculation() {
        let settings = carve_settings();
        let mut design = Design::layout(&square_font(1.0), &settings).unwrap();
        let progress = Progress::none();
        progress.cancel();
        assert!(matches!(
            CarvePath::calculate(&mut design, &settings, &progress),
            Err(Error::Canceled)
        ));
    }

    #[test]
    fn progress_is_monotonic_and_finishes() {
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let progress = Progress::with_callback(move |f| sink.lock().unwrap().push(f));
        let settings = carve_settings();
        let mut design = Design::layout(&square_font(1.0), &settings).unwrap();
        CarvePath::calculate(&mut design, &settings, &progress).unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*seen.last().unwrap(), 1.0);
    }

    #[test]
    fn minimum_step_is_enforced() {
        let settings = Settings {
            v_step_len: 1e-9,
            y_scale: 0.05,
            ..carve_settings()
        };
        let path = carve(&square_font(1.0), &settings);
        let expected_max = (4.0 * 0.05 / Units::Inches.min_step_len()) as usize + 32;
        assert!(path.points.len() <= expected_max, "{}", path.points.len());
    }

    #[test]
    fn tangent_limit_parallel_wall_halves_gap() {
        // Local frame: sample at origin, center on +Y; a wall along X at y = 1.
        let s = Segment::new(-5.0, 1.0, 5.0, 1.0, 0, 0);
        assert_abs_diff_eq!(
            Carver::tangent_limit(&s, 0.0, 0.0, 0.0, 1.0, 9.0),
            0.5,
            epsilon = 1e-12
        );
    }

    #[test]
    fn tangent_limit_endpoint_circle_through_point() {
        // A single point at (1, 1): the tangent circle through it has radius 1.
        let s = Segment::new(1.0, 1.0, 1.0, 5.0, 0, 0);
        assert_abs_diff_eq!(
            Carver::tangent_limit(&s, 0.0, 0.0, 0.0, 1.0, 9.0),
            1.0,
            epsilon = 1e-12
        );
    }

    #[test]
    fn tangent_limit_slanted_line_tangency() {
        // A 45-degree line y = x - 2 does not limit; y = x + 2 is tangent to a circle at
        // center (0, r) when r / sqrt(2) + ... solved: r = 2 / (sqrt(2) - 1)... verify by
        // distance from center to line equals r.
        let s = Segment::new(-20.0, -18.0, 20.0, 22.0, 0, 0);
        let r = Carver::tangent_limit(&s, 0.0, 0.0, 0.0, 1.0, 100.0);
        let distance = (0.0 - r + 2.0_f64).abs() / 2.0_f64.sqrt();
        assert_abs_diff_eq!(distance, r, epsilon = 1e-9);
    }

    #[test]
    fn tangent_limit_ignores_segments_behind_tangent() {
        let s = Segment::new(-5.0, -1.0, 5.0, -1.0, 0, 0);
        assert_eq!(Carver::tangent_limit(&s, 0.0, 0.0, 0.0, 1.0, 3.0), 3.0);
    }

    #[test]
    fn tangent_limit_connected_segment_forces_zero_radius() {
        let s = Segment::new(0.0, 0.0, 1.0, 1.0, 0, 0);
        assert_eq!(Carver::tangent_limit(&s, 0.0, 0.0, 0.0, 1.0, 3.0), 0.0);
    }

    #[test]
    fn partitions_find_nearby_segments() {
        let settings = carve_settings();
        let mut design = Design::layout(&square_font(1.0), &settings).unwrap();
        design.segments = sort_for_v_carve(&design.segments, settings.accuracy);
        let parts = Partitions::new(&design.segments, &design, 0.1, 0.01);
        // Any point near a segment must find it in the cell.
        for (i, s) in design.segments.iter().enumerate() {
            let mid = ((s.x1 + s.x2) / 2.0, (s.y1 + s.y2) / 2.0);
            assert!(parts.at(mid.0, mid.1).contains(&(i as u32)));
        }
    }

    #[test]
    fn partitions_clamp_out_of_range_queries() {
        let settings = carve_settings();
        let design = Design::layout(&square_font(1.0), &settings).unwrap();
        let parts = Partitions::new(&design.segments, &design, 0.1, 0.01);
        let _ = parts.at(1e6, 1e6);
    }

    #[test]
    fn turn_angle_measures_relative_direction() {
        // Previous direction +X: cos0 = -1, sin0 = 0.
        assert_abs_diff_eq!(turn_angle(1.0, 0.0, 0.0, -1.0), 180.0, epsilon = 1e-9);
        assert_abs_diff_eq!(turn_angle(0.0, 1.0, 0.0, -1.0), 270.0, epsilon = 1e-9);
        assert_abs_diff_eq!(turn_angle(0.0, -1.0, 0.0, -1.0), 90.0, epsilon = 1e-9);
    }
}

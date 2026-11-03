// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2011-2026 Scorch
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
//
// Ported from `Transform`, `Get_Angle`, and `point_inside_polygon` of F-Engrave 1.78.

//! Planar geometry primitives shared by the toolpath algorithms.

/// Distance below which two coordinates are considered coincident.
pub const ZERO: f64 = 0.00001;

/// A point in the design plane.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

impl Point {
    /// Creates a point from its coordinates.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns this point rotated counter-clockwise about the origin by `angle` radians.
    pub fn rotated(self, angle: f64) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self::new(self.x * cos - self.y * sin, self.x * sin + self.y * cos)
    }

    /// Euclidean distance to `other`.
    pub fn distance(self, other: Self) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }

    /// Returns `true` if this point lies inside the closed polygon `poly`, by the even-odd rule.
    ///
    /// The closing edge from the last vertex back to the first is implied.
    pub fn is_inside(self, poly: &[Self]) -> bool {
        let Self { x, y } = self;
        let n = poly.len();
        if n == 0 {
            return false;
        }
        let mut inside = false;
        let mut p1 = poly[0];
        let mut xinters = 0.0;
        for i in 0..=n {
            let p2 = poly[i % n];
            if y > p1.y.min(p2.y) && y <= p1.y.max(p2.y) && x <= p1.x.max(p2.x) {
                if p1.y != p2.y {
                    xinters = (y - p1.y) * (p2.x - p1.x) / (p2.y - p1.y) + p1.x;
                }
                if p1.x == p2.x || x <= xinters {
                    inside = !inside;
                }
            }
            p1 = p2;
        }
        inside
    }
}

/// Converts a sine/cosine pair to an angle in degrees, in the range `0..=360`.
///
/// Values marginally outside `-1..=1` are clamped, so rounding error cannot produce NaN.
pub fn angle_from_sin_cos(sin: f64, cos: f64) -> f64 {
    let acos = cos.clamp(-1.0, 1.0).acos().to_degrees();
    if sin >= 0.0 { acos } else { 360.0 - acos }
}

/// An axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Smallest X coordinate.
    pub min_x: f64,
    /// Smallest Y coordinate.
    pub min_y: f64,
    /// Largest X coordinate.
    pub max_x: f64,
    /// Largest Y coordinate.
    pub max_y: f64,
}

impl Bounds {
    /// A box containing nothing; the first [`Bounds::include`] replaces it.
    pub const EMPTY: Self = Self {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };

    /// Grows the box to contain `(x, y)`.
    pub fn include(&mut self, x: f64, y: f64) {
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_y = self.min_y.min(y);
        self.max_y = self.max_y.max(y);
    }

    /// Returns `true` if nothing has been included.
    pub fn is_empty(&self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }

    /// Width of the box.
    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    /// Height of the box.
    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    /// Center of the box.
    pub fn center(&self) -> Point {
        Point::new(
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }
}

/// A straight piece of a design outline, with the bookkeeping the toolpath stages need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Start X.
    pub x1: f64,
    /// Start Y.
    pub y1: f64,
    /// End X.
    pub x2: f64,
    /// End Y.
    pub y2: f64,
    /// Line of text the segment came from, or the loop number after loop sorting.
    pub line: usize,
    /// One-based position of the source character in the text; zero for generated geometry.
    pub char_num: usize,
}

impl Segment {
    /// Creates a segment with its endpoints and bookkeeping.
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64, line: usize, char_num: usize) -> Self {
        Self {
            x1,
            y1,
            x2,
            y2,
            line,
            char_num,
        }
    }

    /// Length of the segment.
    pub fn length(&self) -> f64 {
        ((self.x2 - self.x1).powi(2) + (self.y2 - self.y1).powi(2)).sqrt()
    }
}

/// Rounds `value` to `digits` significant digits, as `printf("%.{digits}g")` would.
pub fn round_significant(value: f64, digits: i32) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    format!("{:.*e}", (digits - 1).max(0) as usize, value)
        .parse()
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn rotation_quarter_turn() {
        let p = Point::new(1.0, 0.0).rotated(std::f64::consts::FRAC_PI_2);
        assert_abs_diff_eq!(p.x, 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(p.y, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn angle_covers_all_quadrants() {
        for deg in [0.0_f64, 30.0, 90.0, 135.0, 180.0, 225.0, 270.0, 300.0] {
            let (s, c) = deg.to_radians().sin_cos();
            assert_abs_diff_eq!(angle_from_sin_cos(s, c), deg, epsilon = 1e-9);
        }
    }

    #[test]
    fn angle_tolerates_rounding_past_unit_range() {
        assert_eq!(angle_from_sin_cos(0.0, 1.0 + 1e-15), 0.0);
        assert_eq!(angle_from_sin_cos(0.0, -1.0 - 1e-15), 180.0);
    }

    #[test]
    fn bounds_track_points() {
        let mut b = Bounds::EMPTY;
        assert!(b.is_empty());
        b.include(1.0, -2.0);
        b.include(-3.0, 4.0);
        assert!(!b.is_empty());
        assert_eq!(b.width(), 4.0);
        assert_eq!(b.height(), 6.0);
        assert_eq!(b.center(), Point::new(-1.0, 1.0));
    }

    #[test]
    fn point_in_polygon_square() {
        let square = [
            Point::new(0.0, 0.0),
            Point::new(2.0, 0.0),
            Point::new(2.0, 2.0),
            Point::new(0.0, 2.0),
        ];
        assert!(Point::new(1.0, 1.0).is_inside(&square));
        assert!(!Point::new(3.0, 1.0).is_inside(&square));
        assert!(!Point::new(-1.0, 1.0).is_inside(&square));
        assert!(!Point::new(1.0, 3.0).is_inside(&square));
    }

    #[test]
    fn point_in_polygon_concave() {
        let u = [
            Point::new(0.0, 0.0),
            Point::new(3.0, 0.0),
            Point::new(3.0, 3.0),
            Point::new(2.0, 3.0),
            Point::new(2.0, 1.0),
            Point::new(1.0, 1.0),
            Point::new(1.0, 3.0),
            Point::new(0.0, 3.0),
        ];
        assert!(Point::new(0.5, 2.0).is_inside(&u));
        assert!(!Point::new(1.5, 2.0).is_inside(&u));
        assert!(Point::new(1.5, 0.5).is_inside(&u));
    }

    #[test]
    fn significant_rounding_matches_percent_g() {
        assert_eq!(round_significant(50.8, 3), 50.8);
        assert_eq!(round_significant(0.0254, 3), 0.0254);
        assert_eq!(round_significant(127.0001, 3), 127.0);
        assert_eq!(round_significant(0.00039370, 3), 0.000394);
        assert_eq!(round_significant(0.0, 3), 0.0);
    }
}

//! Reads DXF drawings into straight strokes.
//!
//! Supported entities are `LINE`, `ARC`, `CIRCLE`, `LWPOLYLINE` and `POLYLINE` (with bulges),
//! `SPLINE`, `ELLIPSE`, `LEADER`, `SOLID`, and `INSERT` block references. Anything else is
//! reported in [`DxfDrawing::ignored`].

use crate::error::{Error, Result};
use crate::font::Stroke;
use crate::geom::{Point, ZERO, angle_from_sin_cos};
use std::collections::{BTreeMap, BTreeSet};

/// Limit on nested `INSERT`s, which also stops self-referencing blocks.
const MAX_INSERT_DEPTH: u32 = 32;

/// The result of reading a DXF file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DxfDrawing {
    /// Strokes in file order.
    pub strokes: Vec<Stroke>,
    /// Entity types that were skipped.
    pub ignored: BTreeSet<String>,
}

/// A group value, typed by its group code.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Text(String),
    Number(f64),
    Absent,
}

impl Value {
    fn parse(code: u32, raw: &str) -> Option<Self> {
        match code {
            0..=9 | 100 | 102 | 105 | 300..=368 | 999 | 1000..=1009 => {
                Some(Self::Text(raw.to_string()))
            }
            10..=59 | 140..=147 | 1010..=1059 => raw.parse().ok().map(Self::Number),
            60..=79 | 90..=99 | 170..=175 | 280..=289 | 1060..=1071 => {
                raw.parse::<f64>().ok().map(|v| Self::Number(v.trunc()))
            }
            _ => Some(Self::Absent),
        }
    }
}

/// An entity's data: each group code with all of its values, in order.
#[derive(Debug, Clone, Default)]
struct Entity {
    kind: String,
    data: BTreeMap<u32, Vec<Value>>,
}

impl Entity {
    fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_string(),
            data: BTreeMap::new(),
        }
    }

    fn push(&mut self, code: u32, value: Value) {
        self.data.entry(code).or_default().push(value);
    }

    fn numbers(&self, code: u32) -> Vec<f64> {
        self.data
            .get(&code)
            .map(|vals| {
                vals.iter()
                    .filter_map(|v| match v {
                        Value::Number(n) => Some(*n),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn number(&self, code: u32) -> Option<f64> {
        self.numbers(code).first().copied()
    }

    /// A required number; its absence makes the entity unusable.
    fn req(&self, code: u32) -> Result<f64> {
        self.number(code)
            .ok_or_else(|| Error::Import(format!("{} entity is missing group {code}", self.kind)))
    }

    fn text(&self, code: u32) -> Option<&str> {
        self.data.get(&code)?.iter().find_map(|v| match v {
            Value::Text(t) => Some(t.as_str()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Default)]
struct Block {
    base: (f64, f64),
    entities: Vec<Entity>,
}

/// The placement applied by an enclosing `INSERT`.
#[derive(Debug, Clone, Copy)]
struct Placement {
    offset: (f64, f64),
    scale: (f64, f64),
    rotate: f64,
}

impl Placement {
    const IDENTITY: Self = Self {
        offset: (0.0, 0.0),
        scale: (1.0, 1.0),
        rotate: 0.0,
    };

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let mut p = Point::new(x * self.scale.0, y * self.scale.1);
        if self.rotate.abs() > ZERO {
            p = p.rotated(self.rotate.to_radians());
        }
        (p.x + self.offset.0, p.y + self.offset.1)
    }
}

/// State of a `POLYLINE` being assembled from its `VERTEX` entities.
#[derive(Debug, Clone, Copy, Default)]
struct OpenPolyline {
    closed: bool,
    started: bool,
    first: (f64, f64),
    last: (f64, f64),
    bulge: f64,
}

struct Reader {
    blocks: BTreeMap<String, Block>,
    tol_deg: f64,
    out: DxfDrawing,
    polyline: Option<OpenPolyline>,
}

/// Parses DXF text, flattening curves to within `tol_deg` degrees.
pub fn read_dxf(text: &str, tol_deg: f64) -> Result<DxfDrawing> {
    let groups = read_groups(text);
    let mut blocks: BTreeMap<String, Block> = BTreeMap::new();
    let mut entities: Vec<Entity> = Vec::new();
    let mut saw_section = false;

    let mut i = 0;
    while i < groups.len() {
        let (code, value) = &groups[i];
        i += 1;
        if *code != 0 || !matches!(value, Value::Text(t) if t == "SECTION") {
            continue;
        }
        saw_section = true;
        let name = match groups.get(i) {
            Some((2, Value::Text(n))) => {
                i += 1;
                n.as_str()
            }
            _ => "",
        };
        let start = i;
        while i < groups.len()
            && !(groups[i].0 == 0 && matches!(&groups[i].1, Value::Text(t) if t == "ENDSEC"))
        {
            i += 1;
        }
        let section = &groups[start..i];
        i += 1;
        match name {
            "BLOCKS" => parse_blocks(section, &mut blocks),
            "ENTITIES" => entities = parse_entities(section),
            _ => {}
        }
    }
    if !saw_section {
        return Err(Error::Import("no DXF sections found".into()));
    }

    let mut reader = Reader {
        blocks,
        tol_deg: tol_deg.max(0.01),
        out: DxfDrawing::default(),
        polyline: None,
    };
    for entity in &entities {
        // A damaged entity should not lose the rest of the drawing.
        if reader.eval(entity, Placement::IDENTITY, 0).is_err() {
            reader.out.ignored.insert(entity.kind.clone());
        }
    }
    Ok(reader.out)
}

/// Splits the file into (group code, value) pairs, skipping anything unreadable.
fn read_groups(text: &str) -> Vec<(u32, Value)> {
    let mut groups = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Ok(code) = line.trim().parse::<u32>() else {
            continue;
        };
        let Some(raw) = lines.next() else { break };
        if let Some(value) = Value::parse(code, raw.trim_matches([' ', '\r'])) {
            groups.push((code, value));
        }
    }
    groups
}

fn parse_blocks(section: &[(u32, Value)], blocks: &mut BTreeMap<String, Block>) {
    let mut current: Option<(Option<String>, Block, Option<Entity>)> = None;
    let finish = |current: &mut Option<(Option<String>, Block, Option<Entity>)>,
                  blocks: &mut BTreeMap<String, Block>| {
        if let Some((name, mut block, last)) = current.take() {
            block.entities.extend(last);
            if let Some(name) = name {
                blocks.insert(name, block);
            }
        }
    };
    let mut pending_x = 0.0;
    for (code, value) in section {
        match (code, value) {
            (0, Value::Text(t)) if t == "BLOCK" => {
                finish(&mut current, blocks);
                current = Some((None, Block::default(), None));
                pending_x = 0.0;
            }
            (0, Value::Text(t)) if t == "ENDBLK" => finish(&mut current, blocks),
            (0, Value::Text(t)) => {
                if let Some((_, block, last)) = &mut current {
                    block.entities.extend(last.take());
                    *last = Some(Entity::new(t));
                }
            }
            _ => {
                let Some((name, block, last)) = &mut current else {
                    continue;
                };
                if let Some(entity) = last {
                    entity.push(*code, value.clone());
                    continue;
                }
                match (code, value) {
                    (2, Value::Text(n)) => *name = Some(n.clone()),
                    (10, Value::Number(x)) => pending_x = *x,
                    (20, Value::Number(y)) => block.base = (pending_x, *y),
                    _ => {}
                }
            }
        }
    }
    finish(&mut current, blocks);
}

fn parse_entities(section: &[(u32, Value)]) -> Vec<Entity> {
    let mut entities: Vec<Entity> = Vec::new();
    let mut last_code = None;
    for (code, value) in section {
        match (code, value) {
            (0, Value::Text(kind)) => entities.push(Entity::new(kind)),
            _ => {
                if let Some(entity) = entities.last_mut() {
                    // Keep bulge values aligned with vertices that have none.
                    if entity.kind == "LWPOLYLINE" && *code == 10 && last_code == Some(20) {
                        entity.push(42, Value::Number(0.0));
                    }
                    entity.push(*code, value.clone());
                }
            }
        }
        last_code = Some(*code);
    }
    entities
}

impl Reader {
    fn add(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, place: Placement) {
        let (ax, ay) = place.apply(x0, y0);
        let (bx, by) = place.apply(x1, y1);
        self.out.strokes.push(Stroke::new(ax, ay, bx, by));
    }

    fn add_bulge(&mut self, a: (f64, f64), b: (f64, f64), bulge: f64, place: Placement) {
        if bulge == 0.0 {
            self.add(a.0, a.1, b.0, b.1, place);
        } else {
            for s in bulge_segments(a, b, bulge, self.tol_deg) {
                self.add(s.x1, s.y1, s.x2, s.y2, place);
            }
        }
    }

    fn eval(&mut self, e: &Entity, place: Placement, depth: u32) -> Result<()> {
        match e.kind.as_str() {
            "LINE" => {
                let (x0, y0, x1, y1) = (e.req(10)?, e.req(20)?, e.req(11)?, e.req(21)?);
                self.add(x0, y0, x1, y1, place);
            }
            "ARC" => {
                let (x, y, r) = (e.req(10)?, e.req(20)?, e.req(40)?);
                let (start, mut end) = (e.req(50)?, e.req(51)?);
                if end < start {
                    end += 360.0;
                }
                let steps = ((end - start) / self.tol_deg).floor().max(2.0);
                self.add_circular(x, y, r, start, (end - start) / steps, steps as usize, place);
            }
            "CIRCLE" => {
                let (x, y, r) = (e.req(10)?, e.req(20)?, e.req(40)?);
                let steps = (360.0 / self.tol_deg).ceil().max(2.0);
                self.add_circular(x, y, r, 0.0, 360.0 / steps, steps as usize, place);
            }
            "LWPOLYLINE" => self.eval_lwpolyline(e, place),
            "SPLINE" => self.eval_spline(e, place)?,
            "ELLIPSE" => self.eval_ellipse(e, place)?,
            "LEADER" => {
                let pts: Vec<_> = e.numbers(10).into_iter().zip(e.numbers(20)).collect();
                for w in pts.windows(2) {
                    self.add(w[0].0, w[0].1, w[1].0, w[1].1, place);
                }
            }
            "POLYLINE" => {
                let flags = e.number(70).unwrap_or(0.0) as i64;
                self.polyline = Some(OpenPolyline {
                    closed: flags & 1 != 0,
                    ..OpenPolyline::default()
                });
            }
            "VERTEX" => {
                let (x, y) = (e.req(10)?, e.req(20)?);
                let bulge = e.number(42).unwrap_or(0.0);
                let Some(mut poly) = self.polyline else {
                    self.out.ignored.insert(e.kind.clone());
                    return Ok(());
                };
                if poly.started {
                    self.add_bulge(poly.last, (x, y), poly.bulge, place);
                } else {
                    poly.first = (x, y);
                    poly.started = true;
                }
                poly.last = (x, y);
                poly.bulge = bulge;
                self.polyline = Some(poly);
            }
            "SEQEND" => {
                if let Some(poly) = self.polyline.take()
                    && poly.closed
                    && poly.started
                {
                    self.add_bulge(poly.last, poly.first, poly.bulge, place);
                }
            }
            "INSERT" => self.eval_insert(e, place, depth)?,
            "SOLID" => {
                let a = (e.req(10)?, e.req(20)?);
                let b = (e.req(11)?, e.req(21)?);
                let c = (e.req(12)?, e.req(22)?);
                let d = (e.number(13).unwrap_or(c.0), e.number(23).unwrap_or(c.1));
                self.add(a.0, a.1, b.0, b.1, place);
                self.add(b.0, b.1, d.0, d.1, place);
                self.add(d.0, d.1, c.0, c.1, place);
                self.add(c.0, c.1, a.0, a.1, place);
            }
            "HATCH" => {}
            other => {
                self.out.ignored.insert(other.to_string());
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments, reason = "plain circle parameters")]
    fn add_circular(
        &mut self,
        x: f64,
        y: f64,
        r: f64,
        start_deg: f64,
        step_deg: f64,
        steps: usize,
        place: Placement,
    ) {
        let start = start_deg.to_radians();
        let step = step_deg.to_radians();
        let mut p0 = (x + r * start.cos(), y + r * start.sin());
        for i in 1..=steps {
            let phi = start + i as f64 * step;
            let p1 = (x + r * phi.cos(), y + r * phi.sin());
            self.add(p0.0, p0.1, p1.0, p1.1, place);
            p0 = p1;
        }
    }

    fn eval_lwpolyline(&mut self, e: &Entity, place: Placement) {
        let xs = e.numbers(10);
        let ys = e.numbers(20);
        let bulges = e.numbers(42);
        let points: Vec<(f64, f64)> = xs.into_iter().zip(ys).collect();
        let bulge_at = |i: usize| bulges.get(i).copied().unwrap_or(0.0);
        for (i, w) in points.windows(2).enumerate() {
            self.add_bulge(w[0], w[1], bulge_at(i), place);
        }
        if e.number(70).unwrap_or(0.0) != 0.0
            && let (Some(&first), Some(&last)) = (points.first(), points.last())
        {
            self.add_bulge(last, first, bulge_at(points.len() - 1), place);
        }
    }

    fn eval_spline(&mut self, e: &Entity, place: Placement) -> Result<()> {
        let degree = e.req(71)? as usize;
        let knots = e.numbers(40);
        let points: Vec<(f64, f64)> = e.numbers(10).into_iter().zip(e.numbers(20)).collect();
        let weights = e.numbers(41);
        let weights = if weights.len() >= points.len() {
            weights
        } else {
            vec![1.0; points.len()]
        };
        let curve = Nurbs::new(degree, knots, weights, points)?;
        let pts = curve.flatten(self.tol_deg);
        for w in pts.windows(2) {
            self.add(w[0].0, w[0].1, w[1].0, w[1].1, place);
        }
        Ok(())
    }

    fn eval_ellipse(&mut self, e: &Entity, place: Placement) -> Result<()> {
        let (xc, yc) = (e.req(10)?, e.req(20)?);
        let (xm, ym) = (e.req(11)?, e.req(21)?);
        let ratio = e.req(40)?;
        let start = e.req(41)?;
        let mut end = e.req(42)?;
        if end < start {
            end += std::f64::consts::TAU;
        }
        let rotation = ym.atan2(xm);
        let a = xm.hypot(ym);
        let b = a * ratio;
        let at = |phi: f64| {
            (
                xc + (a * phi.cos() * rotation.cos() - b * phi.sin() * rotation.sin()),
                yc + (a * phi.cos() * rotation.sin() + b * phi.sin() * rotation.cos()),
            )
        };
        let tol = self.tol_deg.to_radians();
        let mut phi = start;
        let mut p1 = at(phi);
        let mut step = tol;
        const MIN_STEP: f64 = 1e-6;
        while phi < end - 1e-12 {
            if phi + step > end {
                step = end - phi;
            }
            let p2 = at(phi + step);
            let test = at(phi + step / 2.0);
            let (dx1, dy1) = (test.0 - p1.0, test.1 - p1.1);
            let (dx2, dy2) = (p2.0 - test.0, p2.1 - test.1);
            let (l1, l2) = (dx1.hypot(dy1), dx2.hypot(dy2));
            let angle = if l1 > 0.0 && l2 > 0.0 {
                (dx1 / l1 * dx2 / l2 + dy1 / l1 * dy2 / l2)
                    .clamp(-1.0, 1.0)
                    .acos()
            } else {
                0.0
            };
            if angle > tol && step > MIN_STEP {
                step /= 2.0;
            } else {
                phi += step;
                self.add(p1.0, p1.1, p2.0, p2.1, place);
                step *= 2.0;
                p1 = p2;
            }
        }
        Ok(())
    }

    fn eval_insert(&mut self, e: &Entity, place: Placement, depth: u32) -> Result<()> {
        if depth >= MAX_INSERT_DEPTH {
            return Err(Error::Import("INSERT nesting is too deep".into()));
        }
        let name = e
            .text(2)
            .ok_or_else(|| Error::Import("INSERT without a block name".into()))?;
        let Some(block) = self.blocks.get(name).cloned() else {
            return Err(Error::Import(format!("INSERT of unknown block {name:?}")));
        };
        let inner = Placement {
            offset: (
                e.req(10)? + place.offset.0 - block.base.0,
                e.req(20)? + place.offset.1 - block.base.1,
            ),
            scale: (e.number(41).unwrap_or(1.0), e.number(42).unwrap_or(1.0)),
            rotate: e.number(50).unwrap_or(0.0),
        };
        for child in &block.entities {
            self.eval(child, inner, depth + 1)?;
        }
        Ok(())
    }
}

/// Flattens a polyline bulge (the tangent of a quarter of the included angle) into segments.
fn bulge_segments(a: (f64, f64), b: (f64, f64), bulge: f64, tol_deg: f64) -> Vec<Stroke> {
    let sign = if bulge < 0.0 { 1.0 } else { -1.0 };
    let bulge = bulge.abs();
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let c = dx.hypot(dy);
    if c < ZERO {
        return vec![Stroke::new(a.0, a.1, b.0, b.1)];
    }
    let alpha = 2.0 * bulge.atan();
    let radius = c / (2.0 * alpha.sin());
    let l = radius * alpha.cos();
    let steps = (2.0 * alpha / tol_deg.to_radians()).ceil().max(1.0);
    let phi = angle_from_sin_cos(dy / c, dx / c).to_radians();
    let d_theta = 2.0 * alpha / steps;
    let mut theta = alpha - d_theta;
    let mut out = Vec::with_capacity(steps as usize);
    let mut cur = a;
    for _ in 1..steps as usize {
        let xp = c / 2.0 - radius * theta.sin();
        let yp = radius * theta.cos() - l;
        let p = Point::new(xp, yp * sign).rotated(phi);
        let next = (p.x + a.0, p.y + a.1);
        out.push(Stroke::new(cur.0, cur.1, next.0, next.1));
        cur = next;
        theta -= d_theta;
    }
    out.push(Stroke::new(cur.0, cur.1, b.0, b.1));
    out
}

/// A rational B-spline curve in the plane.
struct Nurbs {
    degree: usize,
    knots: Vec<f64>,
    /// Homogeneous control points: x*w, y*w, w.
    control: Vec<[f64; 3]>,
}

impl Nurbs {
    fn new(
        degree: usize,
        mut knots: Vec<f64>,
        weights: Vec<f64>,
        points: Vec<(f64, f64)>,
    ) -> Result<Self> {
        if points.is_empty() || knots.len() != points.len() + degree + 1 {
            return Err(Error::Import("SPLINE knot/control point mismatch".into()));
        }
        let kmin = knots.iter().copied().fold(f64::INFINITY, f64::min);
        let kmax = knots.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if kmax <= kmin {
            return Err(Error::Import("SPLINE has a degenerate knot vector".into()));
        }
        for k in &mut knots {
            *k = (*k - kmin) / (kmax - kmin);
        }
        let control = points
            .iter()
            .zip(&weights)
            .map(|(&(x, y), &w)| [x * w, y * w, w])
            .collect();
        Ok(Self {
            degree,
            knots,
            control,
        })
    }

    /// Algorithm A2.1 from The NURBS Book.
    fn find_span(&self, u: f64) -> usize {
        let n = self.control.len() - 1;
        if u >= self.knots[n + 1] {
            return n;
        }
        let mut low = self.degree;
        let mut high = n + 1;
        let mut mid = usize::midpoint(low, high);
        while u < self.knots[mid] || u >= self.knots[mid + 1] {
            if u < self.knots[mid] {
                high = mid;
            } else {
                low = mid;
            }
            mid = usize::midpoint(low, high);
        }
        mid
    }

    /// Evaluates the curve at parameter `u`.
    fn eval(&self, u: f64) -> (f64, f64) {
        let p = self.degree;
        let span = self.find_span(u);
        // Algorithm A2.2: the nonzero basis functions.
        let mut basis = vec![0.0; p + 1];
        let mut left = vec![0.0; p + 1];
        let mut right = vec![0.0; p + 1];
        basis[0] = 1.0;
        for j in 1..=p {
            left[j] = u - self.knots[span + 1 - j];
            right[j] = self.knots[span + j] - u;
            let mut saved = 0.0;
            for r in 0..j {
                let den = right[r + 1] + left[j - r];
                let temp = if den != 0.0 { basis[r] / den } else { 0.0 };
                basis[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            basis[j] = saved;
        }
        let mut h = [0.0; 3];
        for (j, b) in basis.iter().enumerate() {
            let c = self.control[span - p + j];
            h[0] += b * c[0];
            h[1] += b * c[1];
            h[2] += b * c[2];
        }
        (h[0] / h[2], h[1] / h[2])
    }

    /// Samples the curve so that consecutive chords subtend at most `tol_deg` degrees.
    fn flatten(&self, tol_deg: f64) -> Vec<(f64, f64)> {
        const MIN_STEP: f64 = 1e-6;
        let tol = tol_deg.to_radians();
        let first_knot = self.knots.iter().copied().find(|&k| k > 0.0).unwrap_or(1.0);
        let mut step = first_knot / 3.0;
        let mut u = 0.0;
        let mut p1 = self.eval(0.0);
        let mut points = vec![p1];
        while u < 1.0 {
            if u + step > 1.0 {
                step = 1.0 - u;
            }
            let p2 = self.eval(u + step);
            let test = self.eval(u + step / 2.0);
            let chord = (p2.0 - p1.0).hypot(p2.1 - p1.1);
            let t = (test.0 - (p1.0 + p2.0) / 2.0).hypot(test.1 - (p1.1 + p2.1) / 2.0);
            let radius = if t.abs() > ZERO {
                (chord * chord / 4.0 + t * t) / (2.0 * t)
            } else {
                0.0
            };
            let l1 = (test.0 - p1.0).hypot(test.1 - p1.1);
            let l2 = (p2.0 - test.0).hypot(p2.1 - test.1);
            let angle = if l1 > ZERO && l2 > ZERO && radius > ZERO {
                let ratio = (chord / 2.0) / radius;
                2.0 * if ratio.abs() > 1.0 { 0.0 } else { ratio.asin() }
            } else {
                0.0
            };
            if angle > tol && step > MIN_STEP {
                step /= 2.0;
            } else {
                u += step;
                points.push(p2);
                step *= 2.0;
                p1 = p2;
            }
        }
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    fn dxf(entities: &str) -> String {
        format!("999\ncomment\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n")
    }

    fn assert_chained(strokes: &[Stroke]) {
        for w in strokes.windows(2) {
            assert_abs_diff_eq!(w[0].x2, w[1].x1, epsilon = 1e-9);
            assert_abs_diff_eq!(w[0].y2, w[1].y1, epsilon = 1e-9);
        }
    }

    #[test]
    fn reads_line() {
        let d = read_dxf(
            &dxf("0\nLINE\n8\nlayer\n10\n1.5\n20\n2\n30\n0\n11\n3\n21\n4\n31\n0\n"),
            5.0,
        )
        .unwrap();
        assert_eq!(d.strokes, [Stroke::new(1.5, 2.0, 3.0, 4.0)]);
    }

    #[test]
    fn tolerates_crlf_and_indented_codes() {
        let text = dxf("  0\r\nLINE\r\n 10\r\n1\r\n 20\r\n2\r\n 11\r\n3\r\n 21\r\n4\r\n");
        assert_eq!(read_dxf(&text, 5.0).unwrap().strokes.len(), 1);
    }

    #[test]
    fn non_dxf_text_is_an_error() {
        assert!(read_dxf("hello\nworld\n", 5.0).is_err());
    }

    #[test]
    fn unknown_and_damaged_entities_do_not_lose_the_rest() {
        let d = read_dxf(
            &dxf(
                "0\nMTEXT\n1\nhi\n0\nLINE\n10\n1\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n0\nHATCH\n",
            ),
            5.0,
        )
        .unwrap();
        assert_eq!(d.strokes.len(), 1);
        assert!(d.ignored.contains("MTEXT"));
        assert!(d.ignored.contains("LINE"));
        assert!(!d.ignored.contains("HATCH"));
    }

    #[test]
    fn arc_stays_on_circle_and_wraps() {
        let d = read_dxf(&dxf("0\nARC\n10\n1\n20\n1\n40\n2\n50\n270\n51\n90\n"), 10.0).unwrap();
        assert_eq!(d.strokes.len(), 18);
        assert_chained(&d.strokes);
        for s in &d.strokes {
            assert_abs_diff_eq!((s.x1 - 1.0).hypot(s.y1 - 1.0), 2.0, epsilon = 1e-9);
            assert!(s.x1 >= 1.0 - 1e-9, "should go through +X");
        }
    }

    #[test]
    fn circle_closes_for_any_tolerance() {
        for tol in [5.0, 7.0, 11.0] {
            let d = read_dxf(&dxf("0\nCIRCLE\n10\n0\n20\n0\n40\n3\n"), tol).unwrap();
            let (first, last) = (d.strokes[0], *d.strokes.last().unwrap());
            assert_abs_diff_eq!(first.x1, last.x2, epsilon = 1e-9);
            assert_abs_diff_eq!(first.y1, last.y2, epsilon = 1e-9);
            assert_chained(&d.strokes);
        }
    }

    #[test]
    fn lwpolyline_closed_and_open() {
        let verts = "10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n";
        let open = read_dxf(&dxf(&format!("0\nLWPOLYLINE\n70\n0\n{verts}")), 5.0).unwrap();
        assert_eq!(open.strokes.len(), 2);
        let closed = read_dxf(&dxf(&format!("0\nLWPOLYLINE\n70\n1\n{verts}")), 5.0).unwrap();
        assert_eq!(closed.strokes.len(), 3);
        assert_eq!(closed.strokes[2], Stroke::new(1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn lwpolyline_bulge_is_a_semicircle() {
        // Bulge 1 between (0,0) and (2,0) is a semicircle of radius 1 below the chord.
        let d = read_dxf(
            &dxf("0\nLWPOLYLINE\n70\n0\n10\n0\n20\n0\n42\n1\n10\n2\n20\n0\n"),
            10.0,
        )
        .unwrap();
        assert!(d.strokes.len() >= 18);
        assert_chained(&d.strokes);
        assert!(
            d.strokes.iter().all(|s| s.y1 <= 1e-9),
            "positive bulge is to the right of travel"
        );
        for s in &d.strokes {
            assert_abs_diff_eq!((s.x1 - 1.0).hypot(s.y1), 1.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn lwpolyline_bulges_align_with_their_vertices() {
        // First vertex has no bulge, second does.
        let d = read_dxf(
            &dxf("0\nLWPOLYLINE\n70\n0\n10\n0\n20\n0\n10\n2\n20\n0\n42\n-1\n10\n4\n20\n0\n"),
            30.0,
        )
        .unwrap();
        assert_eq!(d.strokes[0], Stroke::new(0.0, 0.0, 2.0, 0.0));
        assert!(d.strokes.len() > 2);
        assert!(d.strokes[1..].iter().all(|s| s.y2 >= -1e-9));
    }

    #[test]
    fn polyline_with_vertices() {
        let d = read_dxf(
            &dxf("0\nPOLYLINE\n70\n1\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n1\n20\n0\n0\nVERTEX\n10\n1\n20\n1\n0\nSEQEND\n"),
            5.0,
        )
        .unwrap();
        assert_eq!(d.strokes.len(), 3);
        assert_eq!(d.strokes[2], Stroke::new(1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn stray_vertex_is_ignored() {
        let d = read_dxf(&dxf("0\nVERTEX\n10\n0\n20\n0\n"), 5.0).unwrap();
        assert!(d.strokes.is_empty());
        assert!(d.ignored.contains("VERTEX"));
    }

    #[test]
    fn ellipse_points_satisfy_equation() {
        let d = read_dxf(
            &dxf("0\nELLIPSE\n10\n1\n20\n2\n11\n4\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n"),
            5.0,
        )
        .unwrap();
        assert!(d.strokes.len() > 20);
        assert_chained(&d.strokes);
        for s in &d.strokes {
            let (x, y) = (s.x1 - 1.0, s.y1 - 2.0);
            assert_abs_diff_eq!(x * x / 16.0 + y * y / 4.0, 1.0, epsilon = 1e-9);
        }
        let last = d.strokes.last().unwrap();
        assert_abs_diff_eq!(last.x2, d.strokes[0].x1, epsilon = 1e-9);
    }

    #[test]
    fn leader_and_solid() {
        let d = read_dxf(
            &dxf("0\nLEADER\n10\n0\n20\n0\n10\n1\n20\n1\n0\nSOLID\n10\n0\n20\n0\n11\n1\n21\n0\n12\n0\n22\n1\n13\n1\n23\n1\n"),
            5.0,
        )
        .unwrap();
        assert_eq!(d.strokes.len(), 5);
    }

    #[test]
    fn insert_places_block_contents() {
        let text = "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB1\n10\n1\n20\n1\n0\nLINE\n10\n1\n20\n1\n11\n2\n21\n1\n0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nB1\n10\n5\n20\n5\n0\nENDSEC\n0\nEOF\n";
        let d = read_dxf(text, 5.0).unwrap();
        assert_eq!(d.strokes, [Stroke::new(5.0, 5.0, 6.0, 5.0)]);
    }

    #[test]
    fn insert_scale_and_rotation() {
        let text = "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nB\n10\n0\n20\n0\n41\n2\n42\n2\n50\n90\n0\nENDSEC\n0\nEOF\n";
        let s = read_dxf(text, 5.0).unwrap().strokes[0];
        assert_abs_diff_eq!(s.x2, 0.0, epsilon = 1e-9);
        assert_abs_diff_eq!(s.y2, 2.0, epsilon = 1e-9);
    }

    #[test]
    fn recursive_insert_terminates() {
        let text = "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n0\nINSERT\n2\nB\n10\n0\n20\n0\n0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nB\n10\n0\n20\n0\n0\nENDSEC\n0\nEOF\n";
        let d = read_dxf(text, 5.0).unwrap();
        assert!(d.strokes.is_empty());
        assert!(d.ignored.contains("INSERT"));
    }

    #[test]
    fn missing_block_is_ignored() {
        let d = read_dxf(&dxf("0\nINSERT\n2\nNOPE\n10\n0\n20\n0\n"), 5.0).unwrap();
        assert!(d.ignored.contains("INSERT"));
    }

    #[test]
    fn nurbs_degree_one_is_a_polyline() {
        let curve = Nurbs::new(
            1,
            vec![0.0, 0.0, 1.0, 2.0, 2.0],
            vec![1.0; 3],
            vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)],
        )
        .unwrap();
        let mid = curve.eval(0.5);
        assert_abs_diff_eq!(mid.0, 1.0, epsilon = 1e-12);
        assert_abs_diff_eq!(mid.1, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn nurbs_quadratic_rational_is_a_true_circular_arc() {
        // The classic rational quadratic quarter circle.
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let curve = Nurbs::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![1.0, w, 1.0],
            vec![(1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        )
        .unwrap();
        for u in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
            let p = curve.eval(u);
            assert_abs_diff_eq!(p.0.hypot(p.1), 1.0, epsilon = 1e-12);
        }
        let pts = curve.flatten(5.0);
        assert!(pts.len() >= 10);
        assert_abs_diff_eq!(pts[pts.len() - 1].1, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn bad_splines_are_rejected() {
        assert!(Nurbs::new(3, vec![0.0, 1.0], vec![1.0], vec![(0.0, 0.0)]).is_err());
        assert!(
            Nurbs::new(
                1,
                vec![1.0; 4],
                vec![1.0, 1.0],
                vec![(0.0, 0.0), (1.0, 0.0)]
            )
            .is_err()
        );
    }

    #[test]
    fn spline_entity_is_flattened() {
        let d = read_dxf(
            &dxf("0\nSPLINE\n70\n8\n71\n2\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n2\n10\n2\n20\n0\n"),
            5.0,
        )
        .unwrap();
        assert!(d.strokes.len() > 3);
        assert_chained(&d.strokes);
        assert_abs_diff_eq!(d.strokes.last().unwrap().x2, 2.0, epsilon = 1e-9);
    }

    #[test]
    fn zero_length_bulge_is_a_point() {
        let s = bulge_segments((1.0, 1.0), (1.0, 1.0), 0.5, 5.0);
        assert_eq!(s.len(), 1);
    }
}

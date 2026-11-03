//! Path ordering: closing and orienting loops for V-carving, and minimizing rapid moves.

use crate::geom::{Point, Segment, ZERO, point_in_polygon};

/// A contiguous run of points sharing a loop number, as inclusive indices.
///
/// `start` exceeds `end` when the run should be traversed backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathRange {
    /// Index of the first point to visit.
    pub start: usize,
    /// Index of the last point to visit.
    pub end: usize,
}

impl PathRange {
    /// Iterates over the indices in traversal order.
    pub fn indices(self) -> Box<dyn Iterator<Item = usize>> {
        if self.start > self.end {
            Box::new((self.end..=self.start).rev())
        } else {
            Box::new(self.start..=self.end)
        }
    }
}

/// Orders the loops in `items` so that each begins near where the last ended, reversing a loop
/// when that is shorter. `xy` extracts a point's position and `loop_of` its loop number.
///
/// The first loop always comes first and keeps its direction.
pub fn sort_paths<T>(
    items: &[T],
    xy: impl Fn(&T) -> (f64, f64),
    loop_of: impl Fn(&T) -> usize,
) -> Vec<PathRange> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut begins = vec![0_usize];
    let mut ends = Vec::new();
    let mut last_loop = loop_of(&items[0]);
    for (i, item) in items.iter().enumerate().skip(1) {
        let current = loop_of(item);
        if current != last_loop {
            begins.push(i);
            ends.push(i - 1);
        }
        last_loop = current;
    }
    ends.push(items.len() - 1);

    let dist2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);

    let mut order = vec![PathRange {
        start: begins[0],
        end: ends[0],
    }];
    let mut next = 0;
    let mut use_begin = false;
    for _ in 1..begins.len() {
        let from = if use_begin {
            ends.remove(next);
            begins.remove(next)
        } else {
            begins.remove(next);
            ends.remove(next)
        };
        let cur = xy(&items[from]);

        let mut min_begin = dist2(cur, xy(&items[begins[0]]));
        let mut min_end = dist2(cur, xy(&items[ends[0]]));
        let mut next_begin = 0;
        let mut next_end = 0;
        for j in 1..begins.len() {
            let d = dist2(cur, xy(&items[begins[j]]));
            if d < min_begin {
                min_begin = d;
                next_begin = j;
            }
            let d = dist2(cur, xy(&items[ends[j]]));
            if d < min_end {
                min_end = d;
                next_end = j;
            }
        }
        if min_end < min_begin {
            next = next_end;
            order.push(PathRange {
                start: ends[next],
                end: begins[next],
            });
            use_begin = true;
        } else {
            next = next_begin;
            order.push(PathRange {
                start: begins[next],
                end: ends[next],
            });
            use_begin = false;
        }
    }
    order
}

/// Prepares arbitrary outlines for V-carving.
///
/// Open chains are stitched into closed loops; features shorter than `accuracy` are removed;
/// loops are oriented so that the carved side alternates with nesting depth (outermost loops
/// run clockwise); and loops are reordered to shorten rapids. In the returned segments,
/// [`Segment::line`] holds the loop number.
pub fn sort_for_v_carve(input: &[Segment], accuracy: f64) -> Vec<Segment> {
    if input.is_empty() {
        return Vec::new();
    }
    // Flatten into one point list, recording where each connected chain begins and ends.
    let mut pts: Vec<Point> = Vec::with_capacity(input.len() * 2);
    let mut chain_begin: Vec<usize> = vec![0];
    let mut chain_end: Vec<usize> = Vec::new();
    pts.push(Point::new(input[0].x1, input[0].y1));
    pts.push(Point::new(input[0].x2, input[0].y2));
    let mut old = pts[1];
    for s in &input[1..] {
        let a = Point::new(s.x1, s.y1);
        if old.distance(a) > ZERO {
            chain_end.push(pts.len() - 1);
            chain_begin.push(pts.len());
            pts.push(a);
        }
        old = Point::new(s.x2, s.y2);
        pts.push(old);
    }
    chain_end.push(pts.len() - 1);

    // Separate closed loops from open chains.
    let mut loop_begin: Vec<usize> = Vec::new();
    let mut loop_end: Vec<usize> = Vec::new();
    let mut open_begin: Vec<usize> = Vec::new();
    let mut open_end: Vec<usize> = Vec::new();
    for (&b, &e) in chain_begin.iter().zip(&chain_end) {
        if pts[b].distance(pts[e]) <= ZERO {
            pts[e] = pts[b];
            loop_begin.push(b);
            loop_end.push(e);
        } else {
            open_begin.push(b);
            open_end.push(e);
        }
    }

    // Join each open chain to the nearest chain end, until it closes.
    struct Piece {
        begin: usize,
        end: usize,
        group: usize,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut group = 0;
    while !open_begin.is_empty() {
        let start = open_begin.remove(0);
        let mut end = open_end.remove(0);
        group += 1;
        pieces.push(Piece {
            begin: start,
            end,
            group,
        });
        let first = pts[start];
        let mut open = true;
        while open && !open_begin.is_empty() {
            let tail = pts[end];
            let mut d_begin_min = tail.distance(first);
            let mut d_end_min = d_begin_min;
            let mut k_begin = None;
            let mut k_end = None;
            for k in 0..open_begin.len() {
                let d_begin = tail.distance(pts[open_begin[k]]);
                let d_end = tail.distance(pts[open_end[k]]);
                if d_begin < d_begin_min {
                    d_begin_min = d_begin;
                    k_begin = Some(k);
                }
                if d_end < d_end_min {
                    d_end_min = d_end;
                    k_end = Some(k);
                }
            }
            let chosen = match (k_begin, k_end) {
                (None, None) => None,
                (_, Some(k)) if d_end_min < d_begin_min => {
                    let k_b = open_begin.remove(k);
                    let k_e = open_end.remove(k);
                    Some((k_e, k_b))
                }
                (Some(k), _) => {
                    let k_b = open_begin.remove(k);
                    let k_e = open_end.remove(k);
                    Some((k_b, k_e))
                }
                // Equal distances with only an end candidate: traverse that chain backwards.
                (None, Some(k)) => {
                    let k_b = open_begin.remove(k);
                    let k_e = open_end.remove(k);
                    Some((k_e, k_b))
                }
            };
            let bridge_from = pts[end];
            pts.push(bridge_from);
            match chosen {
                None => {
                    pts.push(pts[start]);
                    pieces.push(Piece {
                        begin: pts.len() - 2,
                        end: pts.len() - 1,
                        group,
                    });
                    open = false;
                }
                Some((next_begin, next_end)) => {
                    pts.push(pts[next_begin]);
                    pieces.push(Piece {
                        begin: pts.len() - 2,
                        end: pts.len() - 1,
                        group,
                    });
                    pieces.push(Piece {
                        begin: next_begin,
                        end: next_end,
                        group,
                    });
                    end = next_end;
                }
            }
        }
        if open {
            pts.push(pts[end]);
            pts.push(pts[start]);
            pieces.push(Piece {
                begin: pts.len() - 2,
                end: pts.len() - 1,
                group,
            });
        }
    }

    // Collapse features smaller than the accuracy within already closed loops.
    for (&b, &e) in loop_begin.iter().zip(&loop_end) {
        let mut anchor = pts[b];
        for i in b + 1..=e {
            if anchor.distance(pts[i]) >= accuracy {
                anchor = pts[i];
            } else if i != e {
                pts[i] = anchor;
            } else {
                pts[e] = pts[b];
            }
        }
    }

    // Append the stitched chains as new sequential loops.
    let mut last_group = None;
    for piece in &pieces {
        if last_group != Some(piece.group) {
            if last_group.is_some() {
                loop_end.push(pts.len() - 1);
            }
            loop_begin.push(pts.len());
            last_group = Some(piece.group);
        }
        let range = PathRange {
            start: piece.begin,
            end: piece.end,
        };
        for i in range.indices() {
            pts.push(pts[i]);
        }
    }
    if loop_begin.len() > loop_end.len() {
        loop_end.push(pts.len() - 1);
    }

    // Decide direction: start from clockwise, then toggle once per enclosing loop.
    let loop_count = loop_begin.len();
    let mut flip: Vec<bool> = (0..loop_count)
        .map(|k| {
            let mut area = 0.0;
            for i in loop_begin[k]..loop_end[k] {
                area += (pts[i + 1].x - pts[i].x) * (pts[i + 1].y + pts[i].y);
            }
            area <= 0.0
        })
        .collect();
    for i in 0..loop_count {
        let poly = &pts[loop_begin[i]..loop_end[i]];
        if poly.is_empty() {
            continue;
        }
        for j in 0..loop_count {
            if j != i {
                let probe = pts[loop_begin[j]];
                if point_in_polygon(probe.x, probe.y, poly) {
                    flip[j] = !flip[j];
                }
            }
        }
    }

    // Order loops by distance from the end of one to the beginning of the next.
    let mut remaining: Vec<usize> = (0..loop_count).collect();
    let mut order: Vec<(usize, usize, usize)> = Vec::with_capacity(loop_count);
    let directed = |k: usize| {
        if flip[k] {
            (loop_end[k], loop_begin[k], k)
        } else {
            (loop_begin[k], loop_end[k], k)
        }
    };
    if loop_count > 0 {
        order.push(directed(0));
    }
    let mut next = 0;
    while remaining.len() > 1 {
        let k = remaining.remove(next);
        let cur = pts[loop_end[k]];
        let mut best = f64::INFINITY;
        next = 0;
        for (j, &candidate) in remaining.iter().enumerate() {
            let p = pts[loop_begin[candidate]];
            let d = (cur.x - p.x).powi(2) + (cur.y - p.y).powi(2);
            if d < best {
                best = d;
                next = j;
            }
        }
        order.push(directed(remaining[next]));
    }

    // Emit segments, merging points closer than ZERO into the following segment.
    let mut out: Vec<Segment> = Vec::with_capacity(pts.len());
    for &(start, end, loop_num) in &order {
        let range = PathRange { start, end };
        let first = pts[start];
        let mut prev = start;
        let mut pending: Option<Point> = None;
        let mut last_from = first;
        for i in range.indices().skip(1) {
            let from = pending.unwrap_or(pts[prev]);
            let to = pts[i];
            if from.distance(to) >= ZERO {
                out.push(Segment::new(from.x, from.y, to.x, to.y, loop_num, 0));
                pending = None;
            } else {
                pending = Some(from);
            }
            last_from = from;
            prev = i;
        }
        if pending.is_some() && out.len() > 1 {
            if last_from.distance(first) <= accuracy && out[out.len() - 1].line == loop_num {
                let last = out.len() - 1;
                out[last].x2 = first.x;
                out[last].y2 = first.y;
            } else {
                out.push(Segment::new(
                    last_from.x,
                    last_from.y,
                    first.x,
                    first.y,
                    loop_num,
                    0,
                ));
            }
        }
    }

    // Drop loops too small to enclose anything, except the first loop emitted.
    let mut count = 1;
    if let Some(last) = out.last() {
        let mut loop_last = last.line;
        let mut i = out.len() as isize - 2;
        while i >= 0 {
            let line = out[i as usize].line;
            if line == loop_last {
                count += 1;
            } else {
                if count < 3 {
                    let del = i as usize + 1;
                    while del < out.len() && out[del].line == loop_last {
                        out.remove(del);
                    }
                }
                count = 1;
                loop_last = line;
            }
            i -= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, size: f64, clockwise: bool) -> Vec<Segment> {
        let c = [
            (x, y),
            (x, y + size),
            (x + size, y + size),
            (x + size, y),
            (x, y),
        ];
        let mut segs: Vec<Segment> = c
            .windows(2)
            .map(|w| Segment::new(w[0].0, w[0].1, w[1].0, w[1].1, 0, 0))
            .collect();
        if !clockwise {
            segs.reverse();
            for s in &mut segs {
                std::mem::swap(&mut s.x1, &mut s.x2);
                std::mem::swap(&mut s.y1, &mut s.y2);
            }
        }
        segs
    }

    fn area_of_loop(segs: &[Segment], loop_num: usize) -> f64 {
        segs.iter()
            .filter(|s| s.line == loop_num)
            .map(|s| s.x1 * s.y2 - s.x2 * s.y1)
            .sum::<f64>()
            / 2.0
    }

    fn loops(segs: &[Segment]) -> Vec<usize> {
        let mut out: Vec<usize> = segs.iter().map(|s| s.line).collect();
        out.dedup();
        out
    }

    #[test]
    fn counter_clockwise_outer_loop_becomes_clockwise() {
        let out = sort_for_v_carve(&square(0.0, 0.0, 1.0, false), 0.001);
        assert_eq!(out.len(), 4);
        assert!(area_of_loop(&out, out[0].line) < 0.0);
    }

    #[test]
    fn clockwise_outer_loop_is_preserved() {
        let input = square(0.0, 0.0, 1.0, true);
        let out = sort_for_v_carve(&input, 0.001);
        assert_eq!(out, input);
    }

    #[test]
    fn nesting_alternates_direction() {
        let mut input = square(0.0, 0.0, 10.0, true);
        input.extend(square(2.0, 2.0, 6.0, true));
        input.extend(square(4.0, 4.0, 2.0, true));
        let out = sort_for_v_carve(&input, 0.001);
        let ids = loops(&out);
        assert_eq!(ids.len(), 3);
        let by_size = |size: f64| {
            let id = *ids
                .iter()
                .find(|&&id| (area_of_loop(&out, id).abs() - size * size).abs() < 1e-9)
                .unwrap();
            area_of_loop(&out, id)
        };
        assert!(by_size(10.0) < 0.0);
        assert!(by_size(6.0) > 0.0);
        assert!(by_size(2.0) < 0.0);
    }

    #[test]
    fn open_chains_are_stitched_closed() {
        // Two arcs of a rectangle, the second running the "wrong" way.
        let input = vec![
            Segment::new(0.0, 0.0, 0.0, 5.0, 0, 0),
            Segment::new(0.0, 5.0, 5.0, 5.0, 0, 0),
            Segment::new(5.0, 0.0, 5.0, 5.0, 0, 0),
            Segment::new(0.0, 0.0, 5.0, 0.0, 0, 0),
        ];
        let out = sort_for_v_carve(&input, 0.001);
        assert_eq!(loops(&out).len(), 1);
        assert_eq!(out.len(), 4);
        for pair in out.windows(2) {
            assert_eq!((pair[0].x2, pair[0].y2), (pair[1].x1, pair[1].y1));
        }
        let (first, last) = (out[0], out[3]);
        assert_eq!((last.x2, last.y2), (first.x1, first.y1));
    }

    #[test]
    fn lone_open_chain_closes_back_on_itself() {
        let input = vec![
            Segment::new(0.0, 0.0, 4.0, 0.0, 0, 0),
            Segment::new(4.0, 0.0, 4.0, 4.0, 0, 0),
        ];
        let out = sort_for_v_carve(&input, 0.001);
        assert_eq!(out.len(), 3);
        let last = out[out.len() - 1];
        assert_eq!((last.x2, last.y2), (out[0].x1, out[0].y1));
    }

    #[test]
    fn tiny_segments_collapse_without_breaking_closure() {
        let input = vec![
            Segment::new(0.0, 0.0, 0.0, 5.0, 0, 0),
            Segment::new(0.0, 5.0, 0.0001, 5.0, 0, 0),
            Segment::new(0.0001, 5.0, 5.0, 5.0, 0, 0),
            Segment::new(5.0, 5.0, 0.0, 0.0, 0, 0),
        ];
        let out = sort_for_v_carve(&input, 0.01);
        assert_eq!(out.len(), 3);
        for pair in out.windows(2) {
            assert_eq!((pair[0].x2, pair[0].y2), (pair[1].x1, pair[1].y1));
        }
    }

    #[test]
    fn nearest_loop_visited_next() {
        let mut input = square(0.0, 0.0, 1.0, true);
        input.extend(square(100.0, 0.0, 1.0, true));
        input.extend(square(2.0, 0.0, 1.0, true));
        let out = sort_for_v_carve(&input, 0.001);
        let visited: Vec<f64> = {
            let ids = loops(&out);
            ids.iter()
                .map(|id| out.iter().find(|s| s.line == *id).unwrap().x1)
                .collect()
        };
        assert_eq!(visited, [0.0, 2.0, 100.0]);
    }

    #[test]
    fn degenerate_two_segment_loop_after_first_is_removed() {
        let mut input = square(0.0, 0.0, 5.0, true);
        input.push(Segment::new(10.0, 10.0, 11.0, 10.0, 0, 0));
        input.push(Segment::new(11.0, 10.0, 10.0, 10.0, 0, 0));
        input.extend(square(20.0, 0.0, 5.0, true));
        let out = sort_for_v_carve(&input, 0.001);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(sort_for_v_carve(&[], 0.001).is_empty());
    }

    #[derive(Clone)]
    struct P(f64, f64, usize);

    #[test]
    fn sort_paths_chooses_nearest_end_and_reverses() {
        let items = vec![
            P(0.0, 0.0, 1),
            P(1.0, 0.0, 1),
            P(50.0, 0.0, 2),
            P(2.0, 0.0, 2),
            P(3.0, 0.0, 3),
            P(4.0, 0.0, 3),
        ];
        let order = sort_paths(&items, |p| (p.0, p.1), |p| p.2);
        // Loop 2 ends at x = 50 after reversal, from which loop 3's far end is nearest.
        assert_eq!(
            order,
            [
                PathRange { start: 0, end: 1 },
                PathRange { start: 3, end: 2 },
                PathRange { start: 5, end: 4 },
            ]
        );
    }

    #[test]
    fn sort_paths_handles_small_inputs() {
        let none: Vec<P> = Vec::new();
        assert!(sort_paths(&none, |p| (p.0, p.1), |p| p.2).is_empty());
        let one = vec![P(0.0, 0.0, 1)];
        assert_eq!(
            sort_paths(&one, |p| (p.0, p.1), |p| p.2),
            [PathRange { start: 0, end: 0 }]
        );
    }

    #[test]
    fn path_range_iterates_in_direction() {
        let f: Vec<_> = PathRange { start: 1, end: 3 }.indices().collect();
        assert_eq!(f, [1, 2, 3]);
        let b: Vec<_> = PathRange { start: 3, end: 1 }.indices().collect();
        assert_eq!(b, [3, 2, 1]);
    }
}

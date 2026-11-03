//! Converts raster images to outline strokes, natively or with the external `potrace` program.

use crate::dxf::read_dxf;
use crate::error::{Error, Result};
use crate::font::{Stroke, extension_upper};
use crate::settings::{Settings, Tracer, TurnPolicy};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// A black-and-white image: `true` means the pixel belongs to the design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    width: usize,
    height: usize,
    pixels: Vec<bool>,
}

impl Bitmap {
    /// Creates a bitmap from rows of `true`/`false`, top row first.
    ///
    /// # Panics
    ///
    /// Panics if `pixels` does not hold exactly `width * height` values.
    pub fn new(width: usize, height: usize, pixels: Vec<bool>) -> Self {
        assert_eq!(pixels.len(), width * height, "pixel count mismatch");
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Loads an image file, compositing transparency over white and thresholding at mid-gray.
    pub fn load(path: &Path) -> Result<Self> {
        let image = image::open(path)
            .map_err(|e| Error::Import(format!("{}: {e}", path.display())))?
            .into_rgba8();
        let (width, height) = (image.width() as usize, image.height() as usize);
        let pixels = image
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0.map(f64::from);
                let alpha = a / 255.0;
                let luma = (0.299 * r + 0.587 * g + 0.114 * b) * alpha + 255.0 * (1.0 - alpha);
                luma < 128.0
            })
            .collect();
        Ok(Self::new(width, height, pixels))
    }

    /// Returns the pixel in column `x` and row `y` counted from the bottom; `false` outside.
    fn at(&self, x: isize, y: isize) -> bool {
        if x < 0 || y < 0 || x >= self.width as isize || y >= self.height as isize {
            return false;
        }
        let row = self.height - 1 - y as usize;
        self.pixels[row * self.width + x as usize]
    }

    /// Whether ambiguous corners should connect diagonally adjacent `true` pixels.
    fn connect_set_pixels(&self, policy: TurnPolicy) -> bool {
        let set = self.pixels.iter().filter(|&&p| p).count();
        match policy {
            TurnPolicy::Black | TurnPolicy::Right | TurnPolicy::Random => true,
            TurnPolicy::White | TurnPolicy::Left => false,
            TurnPolicy::Minority => set * 2 <= self.pixels.len(),
            TurnPolicy::Majority => set * 2 > self.pixels.len(),
        }
    }

    /// Traces pixel boundaries into closed loops, in pixel coordinates with Y up.
    ///
    /// Outer boundaries run clockwise and holes counter-clockwise.
    fn boundary_loops(&self, policy: TurnPolicy) -> Vec<Vec<(i32, i32)>> {
        let connect = self.connect_set_pixels(policy);
        // Directed cracks between pixels, keeping the set pixel on the right.
        let mut cracks: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for y in 0..self.height as isize {
            for x in 0..self.width as isize {
                if !self.at(x, y) {
                    continue;
                }
                let (cx, cy) = (x as i32, y as i32);
                if !self.at(x, y + 1) {
                    cracks
                        .entry((cx, cy + 1))
                        .or_default()
                        .push((cx + 1, cy + 1));
                }
                if !self.at(x + 1, y) {
                    cracks
                        .entry((cx + 1, cy + 1))
                        .or_default()
                        .push((cx + 1, cy));
                }
                if !self.at(x, y - 1) {
                    cracks.entry((cx + 1, cy)).or_default().push((cx, cy));
                }
                if !self.at(x - 1, y) {
                    cracks.entry((cx, cy)).or_default().push((cx, cy + 1));
                }
            }
        }

        let mut starts: Vec<(i32, i32)> = cracks.keys().copied().collect();
        starts.sort_unstable();
        let mut loops = Vec::new();
        for start in starts {
            while cracks.get(&start).is_some_and(|c| !c.is_empty()) {
                let mut path = vec![start];
                let mut prev_dir = (0, 0);
                let mut cur = start;
                while let Some(options) = cracks.get_mut(&cur).filter(|o| !o.is_empty()) {
                    let index = Self::choose_exit(options, cur, prev_dir, connect);
                    let next = options.swap_remove(index);
                    prev_dir = (next.0 - cur.0, next.1 - cur.1);
                    cur = next;
                    if cur == start {
                        break;
                    }
                    path.push(cur);
                }
                loops.push(path);
            }
        }
        loops
    }

    /// At a pixel corner with two exits, a right turn keeps diagonal set pixels apart and a left
    /// turn joins them.
    fn choose_exit(
        options: &[(i32, i32)],
        at: (i32, i32),
        came: (i32, i32),
        connect: bool,
    ) -> usize {
        if options.len() < 2 {
            return 0;
        }
        let cross = |o: &(i32, i32)| came.0 * (o.1 - at.1) - came.1 * (o.0 - at.0);
        options
            .iter()
            .position(|o| if connect { cross(o) > 0 } else { cross(o) < 0 })
            .unwrap_or(0)
    }
}

/// Drops pixel-staircase vertices from a closed loop with Douglas-Peucker simplification.
fn simplify_loop(points: &[(f64, f64)], tolerance: f64) -> Vec<(f64, f64)> {
    if points.len() < 4 {
        return points.to_vec();
    }
    let dist2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
    // Split the loop at the two mutually distant points, so each half is a simple chain.
    let far = (1..points.len())
        .max_by(|&i, &j| dist2(points[0], points[i]).total_cmp(&dist2(points[0], points[j])))
        .unwrap_or(1);
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[far] = true;
    let n = points.len();
    let mut stack = vec![(0_usize, far), (far, n)];
    while let Some((lo, hi)) = stack.pop() {
        let a = points[lo];
        let b = points[hi % n];
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        let mut worst = 0.0;
        let mut worst_i = None;
        for (i, p) in points.iter().enumerate().take(hi).skip(lo + 1) {
            let d = if len == 0.0 {
                (p.0 - a.0).hypot(p.1 - a.1)
            } else {
                ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len
            };
            if d > worst {
                worst = d;
                worst_i = Some(i);
            }
        }
        if worst > tolerance
            && let Some(i) = worst_i
        {
            keep[i] = true;
            stack.push((lo, i));
            stack.push((i, hi));
        }
    }
    points
        .iter()
        .zip(&keep)
        .filter(|&(_, &k)| k)
        .map(|(&p, _)| p)
        .collect()
}

/// Corner-cuts gentle vertices of a closed loop; turns sharper than `max_turn_deg` stay sharp.
fn smooth_loop(points: &[(f64, f64)], max_turn_deg: f64) -> Vec<(f64, f64)> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let prev = points[(i + n - 1) % n];
        let cur = points[i];
        let next = points[(i + 1) % n];
        let a = (cur.0 - prev.0, cur.1 - prev.1);
        let b = (next.0 - cur.0, next.1 - cur.1);
        let turn = (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1).abs();
        if turn.to_degrees() <= max_turn_deg {
            out.push((cur.0 - a.0 * 0.25, cur.1 - a.1 * 0.25));
            out.push((cur.0 + b.0 * 0.25, cur.1 + b.1 * 0.25));
        } else {
            out.push(cur);
        }
    }
    out
}

fn loop_area(points: &[(i32, i32)]) -> f64 {
    let mut doubled = 0_i64;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        doubled += i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1);
    }
    doubled as f64 / 2.0
}

impl Bitmap {
    /// Traces the bitmap to closed outlines in pixel units.
    ///
    /// Loops enclosing `turd_size` pixels or fewer are dropped as speckles. `alpha_max` from 0
    /// to 1.33 increases smoothing of corners, and `opt_tolerance` adjusts the staircase removal
    /// when `long_curve` is enabled.
    pub fn trace(
        &self,
        policy: TurnPolicy,
        turd_size: f64,
        alpha_max: f64,
        opt_tolerance: f64,
        long_curve: bool,
    ) -> Vec<Stroke> {
        // Half a pixel diagonal removes stairs; optimization may relax that.
        let tolerance = 0.75 + if long_curve { opt_tolerance } else { 0.0 };
        let max_turn = alpha_max.clamp(0.0, 4.0 / 3.0) * 60.0;
        let mut strokes = Vec::new();
        for pixel_loop in self.boundary_loops(policy) {
            if loop_area(&pixel_loop).abs() <= turd_size {
                continue;
            }
            let as_float: Vec<(f64, f64)> = pixel_loop
                .iter()
                .map(|&(x, y)| (f64::from(x), f64::from(y)))
                .collect();
            let mut outline = simplify_loop(&as_float, tolerance);
            if outline.len() < 3 {
                outline = as_float;
            }
            if max_turn > 0.0 {
                outline = smooth_loop(&smooth_loop(&outline, max_turn), max_turn);
            }
            for i in 0..outline.len() {
                let a = outline[i];
                let b = outline[(i + 1) % outline.len()];
                strokes.push(Stroke::new(a.0, a.1, b.0, b.1));
            }
        }
        strokes
    }

    /// Serializes the bitmap as a plain PBM image, the input format of `potrace`.
    fn to_pbm(&self) -> Vec<u8> {
        let mut out = format!("P4\n{} {}\n", self.width, self.height).into_bytes();
        for row in self.pixels.chunks(self.width.max(1)) {
            for byte in row.chunks(8) {
                let mut value = 0_u8;
                for (i, &bit) in byte.iter().enumerate() {
                    if bit {
                        value |= 0x80 >> i;
                    }
                }
                out.push(value);
            }
        }
        out
    }
}

/// Runs `potrace` on `bitmap`, returning its result as strokes in pixel units.
fn potrace(bitmap: &Bitmap, settings: &Settings) -> Result<Vec<Stroke>> {
    let mut command = Command::new("potrace");
    command
        .args(["-z", settings.bmp_turn_policy.name()])
        .args(["-t", &settings.bmp_turd_size.to_string()])
        .args(["-a", &settings.bmp_alpha_max.to_string()]);
    if settings.bmp_long_curve {
        command.args(["-O", &settings.bmp_opt_tolerance.to_string()]);
    } else {
        command.arg("-n");
    }
    command.args(["-b", "dxf", "-", "-o", "-"]);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Import(format!("unable to run potrace: {e}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Import("unable to write to potrace".into()))?;
    let pbm = bitmap.to_pbm();
    // Write from another thread: potrace may fill its output pipe before reading all its input.
    let writer = std::thread::spawn(move || stdin.write_all(&pbm));
    let output = child
        .wait_with_output()
        .map_err(|e| Error::Import(format!("potrace failed: {e}")))?;
    let write_result = writer
        .join()
        .map_err(|_| Error::Import("potrace input thread panicked".into()))?;
    if !output.status.success() {
        return Err(Error::Import(format!(
            "potrace failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    write_result.map_err(|e| Error::Import(format!("unable to write to potrace: {e}")))?;
    Ok(read_dxf(&String::from_utf8_lossy(&output.stdout), settings.seg_arc)?.strokes)
}

/// The strokes imported from a drawing or image, with any notes for the user.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportedDrawing {
    /// Outline strokes.
    pub strokes: Vec<Stroke>,
    /// Non-fatal observations about the import.
    pub warnings: Vec<String>,
}

impl ImportedDrawing {
    /// Reads a DXF drawing or traces a raster image, chosen by file extension.
    pub fn load(path: &Path, settings: &Settings) -> Result<Self> {
        match extension_upper(path).as_str() {
            "DXF" => {
                let bytes = std::fs::read(path).map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                let drawing = read_dxf(&String::from_utf8_lossy(&bytes), settings.seg_arc)?;
                let warnings = drawing
                    .ignored
                    .iter()
                    .map(|kind| format!("Ignored DXF entity type {kind}"))
                    .collect();
                Ok(Self {
                    strokes: drawing.strokes,
                    warnings,
                })
            }
            "BMP" | "PBM" | "PGM" | "PPM" | "PNM" | "PNG" | "JPG" | "JPEG" | "GIF" | "TIF"
            | "TIFF" => {
                let bitmap = Bitmap::load(path)?;
                let strokes = match settings.tracer {
                    Tracer::Native => bitmap.trace(
                        settings.bmp_turn_policy,
                        settings.bmp_turd_size,
                        settings.bmp_alpha_max,
                        settings.bmp_opt_tolerance,
                        settings.bmp_long_curve,
                    ),
                    Tracer::Potrace => potrace(&bitmap, settings)?,
                };
                Ok(Self {
                    strokes,
                    warnings: Vec::new(),
                })
            }
            other => Err(Error::Import(format!(
                "unsupported image type {other:?}: {}",
                path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    fn bitmap(rows: &[&str]) -> Bitmap {
        let width = rows[0].len();
        let pixels = rows
            .iter()
            .flat_map(|r| r.chars().map(|c| c == '#'))
            .collect();
        Bitmap::new(width, rows.len(), pixels)
    }

    fn signed_area(strokes: &[Stroke]) -> f64 {
        strokes
            .iter()
            .map(|s| s.x1 * s.y2 - s.x2 * s.y1)
            .sum::<f64>()
            / 2.0
    }

    fn trace_sharp(b: &Bitmap) -> Vec<Stroke> {
        b.trace(TurnPolicy::Minority, 0.0, 0.0, 0.0, false)
    }

    #[test]
    fn square_traces_to_clockwise_outline() {
        let b = bitmap(&["....", ".##.", ".##.", "...."]);
        let s = trace_sharp(&b);
        assert_eq!(s.len(), 4);
        assert_abs_diff_eq!(signed_area(&s), -4.0);
        let xs: Vec<f64> = s.iter().flat_map(|c| [c.x1, c.x2]).collect();
        assert_eq!(xs.iter().copied().fold(f64::INFINITY, f64::min), 1.0);
        assert_eq!(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max), 3.0);
    }

    #[test]
    fn hole_produces_opposite_winding() {
        let b = bitmap(&["#####", "#...#", "#...#", "#...#", "#####"]);
        let loops = b.boundary_loops(TurnPolicy::Minority);
        assert_eq!(loops.len(), 2);
        let mut areas: Vec<f64> = loops.iter().map(|l| loop_area(l)).collect();
        areas.sort_by(f64::total_cmp);
        assert_abs_diff_eq!(areas[0], -25.0);
        assert_abs_diff_eq!(areas[1], 9.0);
    }

    #[test]
    fn image_row_order_is_flipped_to_y_up() {
        // A pixel in the top image row sits at the top of design space.
        let b = bitmap(&["#.", ".."]);
        assert!(b.at(0, 1));
        assert!(!b.at(0, 0));
        assert!(!b.at(5, 5));
        assert!(!b.at(-1, 0));
    }

    #[test]
    fn speckles_are_removed() {
        let b = bitmap(&["#.....", "......", "..####", "..####"]);
        assert_eq!(b.trace(TurnPolicy::Minority, 2.0, 0.0, 0.0, false).len(), 4);
        assert_eq!(b.trace(TurnPolicy::Minority, 0.5, 0.0, 0.0, false).len(), 8);
    }

    #[test]
    fn diagonal_pixels_follow_the_turn_policy() {
        let b = bitmap(&["#.", ".#"]);
        assert_eq!(b.boundary_loops(TurnPolicy::Left).len(), 2);
        assert_eq!(b.boundary_loops(TurnPolicy::Right).len(), 1);
    }

    #[test]
    fn staircase_is_simplified() {
        let rows: Vec<String> = (0..20)
            .map(|y| {
                (0..20)
                    .map(|x| if x + y < 20 { '#' } else { '.' })
                    .collect()
            })
            .collect();
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let b = bitmap(&refs);
        let raw: usize = b
            .boundary_loops(TurnPolicy::Minority)
            .iter()
            .map(Vec::len)
            .sum();
        let s = trace_sharp(&b);
        assert!(s.len() * 4 < raw, "{} vs {raw}", s.len());
    }

    #[test]
    fn smoothing_cuts_gentle_corners_only() {
        let octagon_like = [(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)];
        // Right-angle corners survive a 45-degree threshold...
        assert_eq!(smooth_loop(&octagon_like, 45.0).len(), 4);
        // ...and are cut at 90 degrees.
        assert_eq!(smooth_loop(&octagon_like, 90.0).len(), 8);
    }

    #[test]
    fn traced_output_is_closed() {
        let b = bitmap(&["..##..", ".####.", "######", ".####.", "..##.."]);
        for alpha in [0.0, 1.0] {
            let s = b.trace(TurnPolicy::Minority, 0.0, alpha, 0.2, true);
            for pair in s.windows(2) {
                assert_eq!((pair[0].x2, pair[0].y2), (pair[1].x1, pair[1].y1));
            }
            let (f, l) = (s[0], s[s.len() - 1]);
            assert_eq!((l.x2, l.y2), (f.x1, f.y1));
        }
    }

    #[test]
    fn empty_bitmap_has_no_strokes() {
        assert!(trace_sharp(&bitmap(&["..", ".."])).is_empty());
    }

    #[test]
    fn pbm_encoding_packs_rows() {
        let b = bitmap(&["#.......#", "........."]);
        let pbm = b.to_pbm();
        assert_eq!(&pbm[..7], b"P4\n9 2\n");
        assert_eq!(&pbm[7..], &[0x80, 0x80, 0x00, 0x00]);
    }

    #[test]
    fn unsupported_extension_is_an_error() {
        let result = ImportedDrawing::load(Path::new("x.doc"), &Settings::default());
        assert!(matches!(result, Err(Error::Import(_))));
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(
            ImportedDrawing::load(Path::new("/nonexistent/x.dxf"), &Settings::default()).is_err()
        );
        assert!(
            ImportedDrawing::load(Path::new("/nonexistent/x.png"), &Settings::default()).is_err()
        );
    }

    #[test]
    fn png_import_traces_dark_pixels_and_composites_alpha() {
        let dir = std::env::temp_dir().join(format!("rengrave_trace_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.png");
        let mut img = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
        for y in 2..6 {
            for x in 2..6 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        // Fully transparent black must read as background.
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        img.save(&path).unwrap();
        let settings = Settings {
            bmp_alpha_max: 0.0,
            ..Settings::default()
        };
        let imported = ImportedDrawing::load(&path, &settings).unwrap();
        assert_abs_diff_eq!(signed_area(&imported.strokes), -16.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

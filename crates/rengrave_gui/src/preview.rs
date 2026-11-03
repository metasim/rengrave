// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! The interactive design preview: a pannable, zoomable view of outlines and toolpaths.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use rengrave_core::geom::Bounds;
use rengrave_core::{CarvePath, CarvePoint, CutType, Design, Settings};
use std::sync::Arc;

/// Colors that hold up against the theme's panel color.
struct Palette {
    background: Color32,
    extent: Color32,
    grid: Color32,
    grid_major: Color32,
    label: Color32,
    outline: Color32,
    carve_area: Color32,
    carve_path: Color32,
    clean_straight: Color32,
    clean_v: Color32,
    x_axis: Color32,
    y_axis: Color32,
}

impl Palette {
    fn for_mode(dark: bool) -> Self {
        if dark {
            Self {
                background: Color32::from_gray(32),
                // Translucent, so the grid shows through, with a slight warmth like stock.
                extent: Color32::from_rgba_unmultiplied(170, 120, 70, 36),
                grid: Color32::from_gray(50),
                grid_major: Color32::from_gray(66),
                label: Color32::from_gray(150),
                outline: Color32::from_gray(235),
                carve_area: crate::style::BLUE_DARK_THEME,
                carve_path: Color32::from_rgb(255, 214, 102),
                clean_straight: Color32::from_rgb(168, 112, 66),
                clean_v: Color32::from_rgb(250, 150, 90),
                x_axis: Color32::from_rgb(235, 87, 87),
                y_axis: Color32::from_rgb(90, 200, 120),
            }
        } else {
            Self {
                background: Color32::from_gray(246),
                extent: Color32::from_rgba_unmultiplied(190, 130, 70, 40),
                grid: Color32::from_gray(232),
                grid_major: Color32::from_gray(212),
                label: Color32::from_gray(100),
                outline: Color32::from_gray(25),
                carve_area: crate::style::BLUE_LIGHT_THEME,
                carve_path: Color32::from_rgb(180, 40, 40),
                clean_straight: Color32::from_rgb(170, 120, 80),
                clean_v: Color32::from_rgb(220, 110, 40),
                x_axis: Color32::from_rgb(210, 50, 50),
                y_axis: Color32::from_rgb(40, 150, 70),
            }
        }
    }
}

/// The mapping from design to screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    /// Pixels per design unit.
    pub scale: f32,
    /// Design coordinates at the center of the canvas.
    pub center: (f64, f64),
    /// Center of the canvas, in screen coordinates.
    pub origin: Pos2,
}

impl ViewTransform {
    /// Converts a design point to screen coordinates (Y points up in the design).
    pub fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        pos2(
            self.origin.x + ((x - self.center.0) as f32) * self.scale,
            self.origin.y - ((y - self.center.1) as f32) * self.scale,
        )
    }

    /// Converts a screen position to design coordinates.
    pub fn to_design(&self, p: Pos2) -> (f64, f64) {
        (
            f64::from((p.x - self.origin.x) / self.scale) + self.center.0,
            f64::from((self.origin.y - p.y) / self.scale) + self.center.1,
        )
    }

    /// The transform that fits `bounds` in `canvas` with a margin.
    pub fn fit(bounds: &Bounds, canvas: Rect) -> Self {
        const MARGIN: f32 = 28.0;
        let available = (canvas.size() - Vec2::splat(MARGIN * 2.0)).max(Vec2::splat(16.0));
        let (w, h) = (
            bounds.width().max(1e-6) as f32,
            bounds.height().max(1e-6) as f32,
        );
        let mid = bounds.center();
        Self {
            scale: (available.x / w).min(available.y / h),
            center: (mid.x, mid.y),
            origin: canvas.center(),
        }
    }
}

/// Which layers to draw that are not stored in [`Settings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layers {
    /// Draw straight-bit cleanup paths.
    pub clean_straight: bool,
    /// Draw V-bit cleanup paths.
    pub clean_v_bit: bool,
    /// Draw the grid.
    pub grid: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            clean_straight: true,
            clean_v_bit: true,
            grid: true,
        }
    }
}

/// Everything the canvas draws.
#[derive(Default, Clone)]
pub struct Scene {
    /// The positioned outline.
    pub design: Option<Arc<Design>>,
    /// The V-carve toolpath.
    pub carve: Option<Arc<CarvePath>>,
    /// Straight-bit cleanup positions.
    pub clean_straight: Option<Arc<Vec<CarvePoint>>>,
    /// V-bit cleanup positions.
    pub clean_v_bit: Option<Arc<Vec<CarvePoint>>>,
}

/// Identifies a drawn state, so shapes are only rebuilt when something changes.
#[derive(Clone, PartialEq)]
struct CacheKey {
    transform: ViewTransform,
    revision: u64,
    show: [bool; 6],
    dark: bool,
    thickness: f64,
    cut_type: CutType,
    circle: bool,
}

/// A canvas prepared for drawing by [`Preview::begin_canvas`].
pub(crate) struct Canvas {
    pub(crate) response: Response,
    pub(crate) painter: egui::Painter,
    pub(crate) transform: ViewTransform,
    rect: Rect,
    palette: Palette,
}

impl Canvas {
    /// Draws arrows along the X and Y axes at the origin.
    pub(crate) fn draw_axes(&self) {
        const LEN: f32 = 36.0;
        let o = self.transform.to_screen(0.0, 0.0);
        let (painter, palette) = (&self.painter, &self.palette);
        painter.arrow(o, vec2(LEN, 0.0), Stroke::new(2.0, palette.x_axis));
        painter.arrow(o, vec2(0.0, -LEN), Stroke::new(2.0, palette.y_axis));
    }
}

/// Interactive state of the canvas.
#[derive(Default)]
pub struct Preview {
    /// A user-set view; `None` keeps the design fitted to the canvas.
    custom_view: Option<(f32, (f64, f64))>,
    /// The design position under the pointer.
    pub hover_position: Option<(f64, f64)>,
    /// The current view, valid after the first frame.
    pub transform: Option<ViewTransform>,
    cache: Option<(CacheKey, Arc<Vec<Shape>>)>,
    /// Layer visibility not stored in the settings.
    pub layers: Layers,
    pending_zoom: f32,
}

impl Preview {
    /// Returns to fitting the design in the canvas.
    pub fn fit(&mut self) {
        self.custom_view = None;
    }

    /// Returns `true` if the view tracks the design automatically.
    pub fn is_fitted(&self) -> bool {
        self.custom_view.is_none()
    }

    /// Zooms about the center of the view by `factor` on the next frame.
    pub fn zoom_by(&mut self, factor: f32) {
        self.pending_zoom = if self.pending_zoom == 0.0 {
            factor
        } else {
            self.pending_zoom * factor
        };
    }

    /// Allocates a canvas filling the available space, handles pointer input, and draws the
    /// background and grid. `bounds` is what the view fits to, and `label` its accessible name.
    pub(crate) fn begin_canvas(
        &mut self,
        ui: &mut Ui,
        bounds: Option<Bounds>,
        label: &'static str,
    ) -> Canvas {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, label));
        let rect = response.rect;
        let palette = Palette::for_mode(ui.visuals().dark_mode);
        painter.rect_filled(rect, 0.0, palette.background);

        let fitted = bounds.map_or_else(
            || ViewTransform {
                scale: 50.0,
                center: (0.0, 0.0),
                origin: rect.center(),
            },
            |b| ViewTransform::fit(&b, rect),
        );
        let mut transform = match self.custom_view {
            Some((scale, center)) => ViewTransform {
                scale,
                center,
                origin: rect.center(),
            },
            None => fitted,
        };

        if response.dragged() {
            let delta = response.drag_delta();
            transform.center.0 -= f64::from(delta.x / transform.scale);
            transform.center.1 += f64::from(delta.y / transform.scale);
            self.custom_view = Some((transform.scale, transform.center));
        }
        if response.double_clicked() {
            self.fit();
            transform = fitted;
        }
        if self.pending_zoom != 0.0 {
            transform.scale = (transform.scale * self.pending_zoom).clamp(1e-3, 1e6);
            self.custom_view = Some((transform.scale, transform.center));
            self.pending_zoom = 0.0;
        }
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta().y, i.zoom_delta()));
            let factor = pinch * (scroll * 0.002).exp();
            if (factor - 1.0).abs() > 1e-4 {
                // Keep the point under the pointer stationary.
                let before = transform.to_design(pointer);
                transform.scale = (transform.scale * factor).clamp(1e-3, 1e6);
                let after = transform.to_design(pointer);
                transform.center.0 += before.0 - after.0;
                transform.center.1 += before.1 - after.1;
                self.custom_view = Some((transform.scale, transform.center));
            }
            self.hover_position = Some(transform.to_design(pointer));
        } else {
            self.hover_position = None;
        }
        self.transform = Some(transform);

        let painter = painter.with_clip_rect(rect);
        if self.layers.grid {
            Self::draw_grid(&painter, &transform, rect, &palette);
        }
        Canvas {
            response,
            painter,
            transform,
            rect,
            palette,
        }
    }

    /// Draws the design preview and handles pointer input.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        scene: &Scene,
        settings: &Settings,
        revision: u64,
    ) -> Response {
        let canvas = self.begin_canvas(
            ui,
            scene.design.as_ref().map(|d| d.bounds),
            "Design preview",
        );
        let (transform, palette) = (canvas.transform, &canvas.palette);
        let rect = canvas.rect;
        let key = CacheKey {
            transform,
            revision,
            show: [
                settings.show_box,
                settings.show_thick,
                settings.show_v_area,
                settings.show_v_path,
                self.layers.clean_straight,
                self.layers.clean_v_bit,
            ],
            dark: ui.visuals().dark_mode,
            thickness: settings.stroke_thickness,
            cut_type: settings.cut_type,
            circle: scene
                .design
                .as_ref()
                .is_some_and(|d| d.circle_radius != 0.0),
        };
        let shapes = match &self.cache {
            Some((cached, shapes)) if *cached == key => shapes.clone(),
            _ => {
                let shapes = Arc::new(Self::build_shapes(
                    scene,
                    settings,
                    &transform,
                    palette,
                    &self.layers,
                    rect,
                ));
                self.cache = Some((key, shapes.clone()));
                shapes
            }
        };
        canvas.painter.extend(shapes.iter().cloned());
        if settings.show_axis {
            canvas.draw_axes();
        }
        canvas.response
    }

    /// Picks a grid spacing of 1, 2, or 5 times a power of ten, at least `min_px` wide.
    fn grid_step(scale: f32, min_px: f32) -> f64 {
        let raw = f64::from(min_px / scale);
        let magnitude = 10f64.powf(raw.log10().floor());
        for mult in [1.0, 2.0, 5.0, 10.0] {
            if magnitude * mult >= raw {
                return magnitude * mult;
            }
        }
        magnitude * 10.0
    }

    fn draw_grid(painter: &egui::Painter, t: &ViewTransform, canvas: Rect, palette: &Palette) {
        let step = Self::grid_step(t.scale, 48.0);
        let (min_x, min_y) = t.to_design(canvas.left_bottom());
        let (max_x, max_y) = t.to_design(canvas.right_top());
        let decimals = (-step.log10().floor()).max(0.0) as usize;
        let font = egui::FontId::proportional(10.0);
        let is_major = |index: i64| index.rem_euclid(5) == 0;

        let mut xi = (min_x / step).floor() as i64;
        while (xi as f64) * step <= max_x {
            let x = xi as f64 * step;
            let sx = t.to_screen(x, 0.0).x;
            let color = if is_major(xi) {
                palette.grid_major
            } else {
                palette.grid
            };
            painter.line_segment(
                [pos2(sx, canvas.top()), pos2(sx, canvas.bottom())],
                Stroke::new(1.0, color),
            );
            painter.text(
                pos2(sx + 2.0, canvas.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{x:.decimals$}"),
                font.clone(),
                palette.label,
            );
            xi += 1;
        }
        let mut yi = (min_y / step).floor() as i64;
        while (yi as f64) * step <= max_y {
            let y = yi as f64 * step;
            let sy = t.to_screen(0.0, y).y;
            let color = if is_major(yi) {
                palette.grid_major
            } else {
                palette.grid
            };
            painter.line_segment(
                [pos2(canvas.left(), sy), pos2(canvas.right(), sy)],
                Stroke::new(1.0, color),
            );
            painter.text(
                pos2(canvas.left() + 2.0, sy - 1.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{y:.decimals$}"),
                font.clone(),
                palette.label,
            );
            yi += 1;
        }
    }

    fn build_shapes(
        scene: &Scene,
        settings: &Settings,
        t: &ViewTransform,
        palette: &Palette,
        layers: &Layers,
        canvas: Rect,
    ) -> Vec<Shape> {
        let mut shapes = Vec::new();
        let Some(design) = &scene.design else {
            return shapes;
        };
        if settings.show_box {
            let b = design.bounds;
            let rect =
                Rect::from_two_pos(t.to_screen(b.min_x, b.min_y), t.to_screen(b.max_x, b.max_y));
            shapes.push(Shape::rect_filled(rect, 0.0, palette.extent));
        }

        let is_carve = settings.cut_type == CutType::VCarve;
        if is_carve
            && settings.show_v_area
            && let Some(carve) = &scene.carve
        {
            let is_inlay = settings.inlay;
            let r_trim = if is_inlay {
                (settings.v_bit_angle / 2.0).to_radians().tan() * settings.max_cut_depth()
            } else {
                0.0
            };
            shapes.push(Self::discs_mesh(
                &carve.points,
                r_trim,
                t,
                palette.carve_area,
                canvas,
            ));
        }

        // The outline, with engraved thickness where it applies.
        let width = if !is_carve && settings.show_thick {
            ((settings.stroke_thickness as f32) * t.scale).max(1.0)
        } else {
            1.0
        };
        for s in &design.segments {
            let (a, b) = (t.to_screen(s.x1, s.y1), t.to_screen(s.x2, s.y2));
            if canvas.intersects(Rect::from_two_pos(a, b).expand(width)) {
                shapes.push(Shape::line_segment(
                    [a, b],
                    Stroke::new(width, palette.outline),
                ));
            }
        }
        if !is_carve && design.circle_radius != 0.0 {
            let c = design.circle_center;
            shapes.push(Shape::circle_stroke(
                t.to_screen(c.x, c.y),
                design.circle_radius as f32 * t.scale,
                Stroke::new(width, palette.outline),
            ));
        }

        if is_carve
            && settings.show_v_path
            && let Some(carve) = &scene.carve
        {
            Self::path_lines(
                &carve.points,
                t,
                Stroke::new(1.0, palette.carve_path),
                &mut shapes,
            );
        }
        if layers.clean_straight
            && let Some(points) = &scene.clean_straight
        {
            let width = (settings.clean_dia as f32 * t.scale).max(1.0);
            Self::path_lines(
                points,
                t,
                Stroke::new(width, palette.clean_straight),
                &mut shapes,
            );
        }
        if layers.clean_v_bit
            && let Some(points) = &scene.clean_v_bit
        {
            Self::path_lines(points, t, Stroke::new(1.0, palette.clean_v), &mut shapes);
        }
        shapes
    }

    /// Converts tool positions to screen polylines, one or more per loop.
    fn polylines(points: &[CarvePoint], t: &ViewTransform) -> Vec<Vec<Pos2>> {
        // The tessellator divides by a near-zero length at a reversal or at repeated points, which
        // produces stray or NaN vertices. V-carve paths reverse at every corner they drive into,
        // so merge points on the same spot and split lines at corners sharper than a right angle.
        const MIN_SEGMENT_PX: f32 = 0.1;
        let mut lines: Vec<Vec<Pos2>> = Vec::new();
        let mut line: Vec<Pos2> = Vec::new();
        let mut last_loop = None;
        for p in points {
            let pos = t.to_screen(p.x, p.y);
            if last_loop != Some(p.loop_num) {
                lines.push(std::mem::take(&mut line));
                last_loop = Some(p.loop_num);
            }
            match line[..] {
                [.., last] if (pos - last).length() < MIN_SEGMENT_PX => continue,
                [.., before, last] if (last - before).dot(pos - last) <= 0.0 => {
                    lines.push(std::mem::replace(&mut line, vec![last]));
                }
                _ => {}
            }
            line.push(pos);
        }
        lines.push(line);
        lines.retain(|l| l.len() >= 2);
        lines
    }

    /// Draws the path of a tool.
    pub(crate) fn path_lines(
        points: &[CarvePoint],
        t: &ViewTransform,
        stroke: Stroke,
        out: &mut Vec<Shape>,
    ) {
        out.extend(
            Self::polylines(points, t)
                .into_iter()
                .map(|line| Shape::line(line, stroke)),
        );
    }

    /// Builds one mesh containing a filled disc per tool position, skipping those that barely
    /// differ from the last one drawn.
    fn discs_mesh(
        points: &[CarvePoint],
        r_trim: f64,
        t: &ViewTransform,
        color: Color32,
        canvas: Rect,
    ) -> Shape {
        let mut mesh = Mesh::default();
        let mut last: Option<(Pos2, f32)> = None;
        for p in points {
            let radius = ((p.r - r_trim) as f32 * t.scale).abs();
            if radius < 0.5 {
                continue;
            }
            let center = t.to_screen(p.x, p.y);
            if !canvas.expand(radius).contains(center) {
                continue;
            }
            if let Some((c, r)) = last
                && (c - center).length() < 0.75
                && (r - radius).abs() < 0.75
            {
                continue;
            }
            last = Some((center, radius));
            let sides = (radius.sqrt() as u32 * 3).clamp(8, 48);
            let base = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex {
                pos: center,
                uv: WHITE_UV,
                color,
            });
            for i in 0..sides {
                let a = i as f32 / sides as f32 * std::f32::consts::TAU;
                mesh.vertices.push(Vertex {
                    pos: center + radius * Vec2::angled(a),
                    uv: WHITE_UV,
                    color,
                });
                let next = (i + 1) % sides;
                mesh.indices.extend([base, base + 1 + i, base + 1 + next]);
            }
        }
        Shape::mesh(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tessellates `shapes` as the renderer would, returning how far any vertex strays outside
    /// `allowed`. A NaN vertex counts as infinitely far.
    fn worst_stray(shapes: Vec<Shape>, allowed: Rect) -> f32 {
        let mut tessellator = egui::epaint::Tessellator::new(
            1.0,
            egui::epaint::TessellationOptions::default(),
            [1, 1],
            Vec::new(),
        );
        let mut worst = 0.0_f32;
        for shape in shapes {
            let mut mesh = Mesh::default();
            tessellator.tessellate_shape(shape, &mut mesh);
            for v in &mesh.vertices {
                if v.pos.any_nan() {
                    return f32::INFINITY;
                }
                let dx = (allowed.left() - v.pos.x)
                    .max(v.pos.x - allowed.right())
                    .max(0.0);
                let dy = (allowed.top() - v.pos.y)
                    .max(v.pos.y - allowed.bottom())
                    .max(0.0);
                worst = worst.max(dx.max(dy));
            }
        }
        worst
    }

    fn unit_view() -> ViewTransform {
        ViewTransform {
            scale: 1.0,
            center: (0.0, 0.0),
            origin: pos2(0.0, 0.0),
        }
    }

    fn at(list: &[(f64, f64, usize)]) -> Vec<CarvePoint> {
        list.iter()
            .map(|&(x, y, loop_num)| CarvePoint {
                x,
                y,
                r: 0.0,
                loop_num,
            })
            .collect()
    }

    #[test]
    fn polylines_split_at_reversals_and_sharp_corners() {
        let reversal = at(&[(0.0, 0.0, 1), (10.0, 10.0, 1), (0.0, 0.0, 1)]);
        assert_eq!(Preview::polylines(&reversal, &unit_view()).len(), 2);
        let sharp = at(&[(0.0, 0.0, 1), (10.0, 0.0, 1), (0.0, 1.0, 1)]);
        assert_eq!(Preview::polylines(&sharp, &unit_view()).len(), 2);
        // The line continues from the corner, leaving no gap.
        let lines = Preview::polylines(&sharp, &unit_view());
        assert_eq!(lines[0].last(), lines[1].first());
        let gentle = at(&[(0.0, 0.0, 1), (10.0, 0.0, 1), (20.0, 5.0, 1)]);
        assert_eq!(Preview::polylines(&gentle, &unit_view()).len(), 1);
    }

    #[test]
    fn polylines_merge_repeated_points_and_drop_single_points() {
        let repeats = at(&[
            (0.0, 0.0, 1),
            (5.0, 0.0, 1),
            (5.0, 0.0, 1),
            (5.0, 0.0, 1),
            (9.0, 0.0, 1),
        ]);
        let lines = Preview::polylines(&repeats, &unit_view());
        assert_eq!(
            lines,
            [vec![pos2(0.0, 0.0), pos2(5.0, 0.0), pos2(9.0, 0.0)]]
        );
        assert!(Preview::polylines(&at(&[(1.0, 1.0, 1), (1.0, 1.0, 1)]), &unit_view()).is_empty());
        assert!(Preview::polylines(&[], &unit_view()).is_empty());
    }

    #[test]
    fn polylines_keep_loops_separate() {
        let loops = at(&[(0.0, 0.0, 1), (5.0, 0.0, 1), (9.0, 9.0, 2), (9.0, 20.0, 2)]);
        assert_eq!(Preview::polylines(&loops, &unit_view()).len(), 2);
    }

    #[test]
    fn degenerate_paths_tessellate_without_stray_vertices() {
        // A diagonal reversal and triplicated points, at scales that exercise float rounding.
        for scale in [0.37_f32, 1.0, 3.3, 41.7] {
            let t = ViewTransform {
                scale,
                ..unit_view()
            };
            let points = at(&[
                (1.1, 2.3, 1),
                (7.7, 8.9, 1),
                (7.7, 8.9, 1),
                (7.7, 8.9, 1),
                (1.1, 2.3, 1),
                (3.0, 2.3, 1),
            ]);
            let mut shapes = Vec::new();
            Preview::path_lines(&points, &t, Stroke::new(1.0, Color32::WHITE), &mut shapes);
            let extent = Rect::from_two_pos(t.to_screen(1.1, 2.3), t.to_screen(7.7, 8.9));
            let stray = worst_stray(shapes, extent);
            assert!(stray < 4.0, "strays {stray} px at scale {scale}");
        }
    }

    #[test]
    fn tool_paths_of_real_glyphs_draw_within_their_extent() {
        use rengrave_core::{BuiltinFont, InputType, Job, Progress, Units};
        let settings = Settings {
            input_type: InputType::Text,
            cut_type: CutType::VCarve,
            font_file: BuiltinFont::Hack.name().into(),
            extended_chars: true,
            show_font_index: true,
            ..Settings::with_units(Units::Millimeters)
        };
        let job = Job::load(Settings {
            y_scale: 25.4,
            v_step_len: 0.254,
            ..settings
        })
        .unwrap();
        let (design, carve) = job.calculate(&Progress::none()).unwrap();
        let carve = carve.unwrap();
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 900.0));
        let t = ViewTransform::fit(&design.bounds, canvas);
        let mut shapes = Vec::new();
        Preview::path_lines(
            &carve.points,
            &t,
            Stroke::new(1.0, Color32::WHITE),
            &mut shapes,
        );
        let b = design.bounds;
        let allowed =
            Rect::from_two_pos(t.to_screen(b.min_x, b.min_y), t.to_screen(b.max_x, b.max_y));
        let stray = worst_stray(shapes, allowed);
        assert!(stray < 8.0, "a vertex strays {stray} px outside the design");
    }

    #[test]
    fn stock_area_lets_the_grid_show_through_and_is_warm() {
        for dark in [true, false] {
            let extent = Palette::for_mode(dark).extent;
            assert!(extent.a() > 0 && extent.a() < 128, "{extent:?}");
            let [r, g, b, _] = extent.to_srgba_unmultiplied();
            assert!(r > g && g > b, "{extent:?}");
        }
    }

    #[test]
    fn transform_round_trips_and_flips_y() {
        let t = ViewTransform {
            scale: 10.0,
            center: (1.0, 2.0),
            origin: pos2(100.0, 100.0),
        };
        let s = t.to_screen(2.0, 3.0);
        assert_eq!(s, pos2(110.0, 90.0));
        let (x, y) = t.to_design(s);
        assert!((x - 2.0).abs() < 1e-5 && (y - 3.0).abs() < 1e-5);
    }

    #[test]
    fn fit_contains_the_bounds() {
        let bounds = Bounds {
            min_x: -1.0,
            min_y: 0.0,
            max_x: 3.0,
            max_y: 1.0,
        };
        let canvas = Rect::from_min_size(pos2(10.0, 10.0), vec2(400.0, 200.0));
        let t = ViewTransform::fit(&bounds, canvas);
        assert!(canvas.contains(t.to_screen(-1.0, 0.0)));
        assert!(canvas.contains(t.to_screen(3.0, 1.0)));
        assert_eq!(t.to_screen(1.0, 0.5), canvas.center());
    }

    #[test]
    fn fit_tolerates_zero_sized_design() {
        let bounds = Bounds {
            min_x: 1.0,
            min_y: 1.0,
            max_x: 1.0,
            max_y: 1.0,
        };
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        assert!(ViewTransform::fit(&bounds, canvas).scale.is_finite());
    }

    #[test]
    fn grid_steps_are_round_numbers() {
        for scale in [0.5_f32, 7.0, 33.0, 180.0, 900.0, 12000.0] {
            let step = Preview::grid_step(scale, 48.0);
            assert!(step * f64::from(scale) >= 48.0);
            let mantissa = step / 10f64.powf(step.log10().floor());
            assert!(
                [1.0, 2.0, 5.0, 10.0]
                    .iter()
                    .any(|m| (mantissa - m).abs() < 1e-6),
                "{step}"
            );
        }
    }

    #[test]
    fn disc_mesh_thins_duplicates_and_skips_small_radii() {
        let t = ViewTransform {
            scale: 100.0,
            center: (0.0, 0.0),
            origin: pos2(100.0, 100.0),
        };
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0));
        let cp = |x: f64, r: f64| CarvePoint {
            x,
            y: 0.0,
            r,
            loop_num: 1,
        };
        let points = [
            cp(0.0, 0.0),
            cp(0.0, 0.2),
            cp(0.0001, 0.2),
            cp(0.5, 0.2),
            cp(50.0, 0.1),
        ];
        let Shape::Mesh(mesh) = Preview::discs_mesh(&points, 0.0, &t, Color32::RED, canvas) else {
            panic!("expected a mesh");
        };
        let center_count = mesh.indices.len() / 3;
        // Two discs of radius 20 px survive (3 * sqrt(20) -> 12 sides).
        assert_eq!(center_count, 24);
    }
}

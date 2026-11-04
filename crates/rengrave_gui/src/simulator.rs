// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! The toolpath simulator view: plays the G-code program and shows the carved stock.

use crate::icons;
use crate::preview::{Preview, ViewTransform};
use crate::widgets::UiExt;
use egui::{Color32, ColorImage, Pos2, Rect, RichText, Stroke, TextureHandle, TextureOptions, Ui};
use rengrave_core::simulate::{CellRect, SimMove};
use rengrave_core::{CarvePoint, Design, Settings, Simulation};
use std::sync::Arc;
use std::time::Duration;

/// Playback speeds offered, as multiples of real time.
const SPEEDS: [f64; 4] = [1.0, 10.0, 100.0, 1000.0];
/// Length a program should play for at the initially selected speed, in seconds.
const COMFORTABLE_PLAY_SECONDS: f64 = 60.0;
/// Longest time spent cutting stock per frame, which keeps the interface responsive.
const FRAME_BUDGET: Duration = Duration::from_millis(8);

/// The state of the simulator tab.
pub struct SimulatorView {
    sim: Option<Simulation>,
    source: Option<Arc<Vec<String>>>,
    error: Option<String>,
    texture: Option<TextureHandle>,
    playing: bool,
    speed: f64,
    target: f64,
    show_path: bool,
    resolution: usize,
    relief: f32,
    smooth: bool,
    keep_position: bool,
    path_cache: Option<(ViewTransform, Arc<Vec<egui::Shape>>)>,
}

impl Default for SimulatorView {
    fn default() -> Self {
        Self {
            sim: None,
            source: None,
            error: None,
            texture: None,
            playing: false,
            speed: SPEEDS[2],
            target: 0.0,
            show_path: false,
            resolution: Simulation::DEFAULT_RESOLUTION,
            relief: 1.0,
            smooth: true,
            keep_position: false,
            path_cache: None,
        }
    }
}

impl SimulatorView {
    /// The running simulation, once the program has been interpreted.
    pub fn simulation(&self) -> Option<&Simulation> {
        self.sim.as_ref()
    }

    /// Why the program could not be simulated, if it could not.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether playback is running.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The selected speed, as a multiple of real time.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Grid cells along the longer side of the simulated stock.
    pub fn resolution(&self) -> usize {
        self.resolution
    }

    /// How much the shading exaggerates the depth of the cuts, where 1 is true to scale.
    pub fn relief(&self) -> f32 {
        self.relief
    }

    /// Whether the stock texture is filtered smoothly, rather than as sharp cells.
    pub fn is_smooth(&self) -> bool {
        self.smooth
    }

    /// The playback position, in seconds of program time.
    pub fn position(&self) -> f64 {
        self.target
    }

    /// Moves playback to `seconds` of program time.
    pub fn seek(&mut self, seconds: f64) {
        self.target = seconds.max(0.0);
    }

    /// Whether the stock has been cut up to the playback position.
    pub fn has_caught_up(&self) -> bool {
        self.sim
            .as_ref()
            .is_none_or(|s| s.time() >= self.target.min(s.total_seconds()))
    }

    /// Starts over with a new program, if `lines` is not the program already loaded.
    pub(crate) fn load(&mut self, lines: &Arc<Vec<String>>, design: &Design, settings: &Settings) {
        if self.source.as_ref().is_some_and(|s| Arc::ptr_eq(s, lines)) {
            return;
        }
        self.source = Some(lines.clone());
        self.texture = None;
        self.path_cache = None;
        // A change of resolution rebuilds the same program, so stay where playback was.
        let keep = std::mem::take(&mut self.keep_position);
        if !keep {
            self.playing = false;
            self.target = 0.0;
        }
        match Simulation::new(lines, design, settings, self.resolution) {
            Ok(sim) => {
                if !keep {
                    self.speed = Self::comfortable_speed(sim.total_seconds());
                }
                self.sim = Some(sim);
                self.error = None;
            }
            Err(e) => {
                self.sim = None;
                self.error = Some(e.to_string());
            }
        }
    }

    /// The slowest speed that plays a program of this length in a comfortable time.
    fn comfortable_speed(total_seconds: f64) -> f64 {
        SPEEDS
            .into_iter()
            .find(|s| total_seconds / s <= COMFORTABLE_PLAY_SECONDS)
            .unwrap_or(SPEEDS[SPEEDS.len() - 1])
    }

    /// Formats a time as minutes and seconds, or hours when needed.
    fn format_time(seconds: f64) -> String {
        let total = seconds.max(0.0).round() as u64;
        let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
    }

    /// Draws the playback buttons for the toolbar.
    pub(crate) fn toolbar(&mut self, ui: &mut Ui) {
        let total = self.sim.as_ref().map_or(0.0, Simulation::total_seconds);
        let ready = self.sim.is_some();
        let (icon, label) = if self.playing {
            (icons::PAUSE, "Pause")
        } else {
            (icons::PLAY, "Play")
        };
        if ui.icon_button(ready, icon, label).clicked() {
            self.playing = !self.playing;
            if self.playing && self.target >= total {
                self.target = 0.0;
            }
        }
        if ui
            .icon_button(ready, icons::RESTART, "Restart")
            .on_hover_text("Go back to the start of the program")
            .clicked()
        {
            self.target = 0.0;
        }
        if ui
            .icon_button(ready, icons::FINISH, "Finish")
            .on_hover_text("Jump to the finished result")
            .clicked()
        {
            self.target = total;
            self.playing = false;
        }
        ui.separator();
        ui.option_group(|ui| {
            for speed in SPEEDS {
                ui.selectable_value(&mut self.speed, speed, format!("{speed:.0}×"))
                    .on_hover_text("Playback speed, as a multiple of the estimated run time");
            }
        });
        ui.separator();
        ui.separator();
        // Rarely changed, so kept in a popover that stays open while the controls are adjusted.
        ui.options_menu(
            "Render",
            "Path, resolution, relief, and smoothing of the stock",
            |ui| self.render_controls(ui),
        );
    }

    /// Draws the controls for how the stock is rendered.
    fn render_controls(&mut self, ui: &mut Ui) {
        ui.checkbox(&mut self.show_path, "Path")
            .on_hover_text("Show the whole tool path: cuts and rapid moves");
        ui.separator();
        ui.label(ui.control_label("Resolution"));
        ui.option_group(|ui| {
            for resolution in Simulation::RESOLUTIONS {
                let chosen = ui
                    .selectable_label(self.resolution == resolution, resolution.to_string())
                    .on_hover_text("Cells along the longer side of the stock. Finer is slower.");
                if chosen.clicked() && self.resolution != resolution {
                    self.resolution = resolution;
                    // Forget the program, so that it is simulated again at the new resolution.
                    self.source = None;
                    self.keep_position = true;
                }
            }
        });
        let relief = ui
            .add(egui::Slider::new(&mut self.relief, 0.5..=5.0).text("Relief"))
            .on_hover_text(
                "Exaggerate the depth of the cuts in the shading. The cuts are unchanged.",
            );
        let smooth = ui
            .checkbox(&mut self.smooth, "Smooth")
            .on_hover_text("Blend the stock between cells, instead of showing each cell sharply");
        if relief.changed() || smooth.changed() {
            // Both change how every cell is drawn, so draw the whole texture again.
            self.texture = None;
        }
    }

    /// Draws the scrub bar, messages, and stock, and advances playback.
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        preview: &mut Preview,
        design: &Design,
        lines: &[String],
        settings: &Settings,
    ) {
        if let Some(error) = &self.error {
            ui.centered_and_justified(|ui| ui.colored_label(ui.visuals().error_fg_color, error));
            return;
        }
        if self.sim.is_none() {
            return;
        }
        self.advance(ui);
        self.status_rows(ui, lines);
        self.upload_texture(ui.ctx());

        let canvas = preview.begin_canvas(ui, Some(design.bounds), "Simulation view");
        let t = canvas.transform;
        if let (Some(sim), Some(texture)) = (&self.sim, &self.texture) {
            let (ox, oy) = sim.grid_origin();
            let (nx, ny) = sim.grid_size();
            let half = sim.cell_size() / 2.0;
            let width = nx as f64 * sim.cell_size();
            let height = ny as f64 * sim.cell_size();
            let rect = Rect::from_two_pos(
                t.to_screen(ox - half, oy - half),
                t.to_screen(ox - half + width, oy - half + height),
            );
            let uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
            // Slightly translucent, so the grid stays readable through the stock.
            canvas
                .painter
                .image(texture.id(), rect, uv, Color32::from_white_alpha(205));
        }
        if self.show_path {
            let shapes = self.path_shapes(&t);
            canvas.painter.extend(shapes.iter().cloned());
        }
        if settings.show_axis {
            canvas.draw_axes();
        }
        if let Some(sim) = &self.sim {
            Self::draw_tool(&canvas.painter, &t, sim);
        }
    }

    fn advance(&mut self, ui: &Ui) {
        let Some(sim) = &mut self.sim else {
            return;
        };
        let total = sim.total_seconds();
        if self.playing {
            let dt = f64::from(ui.input(|i| i.stable_dt)).min(0.1);
            self.target += dt * self.speed;
            if self.target >= total {
                self.target = total;
                self.playing = false;
            }
        }
        let caught_up = sim.advance_to(self.target, Some(FRAME_BUDGET));
        if self.playing || !caught_up {
            ui.ctx().request_repaint();
        }
    }

    fn status_rows(&mut self, ui: &mut Ui, lines: &[String]) {
        let Some(total) = self.sim.as_ref().map(Simulation::total_seconds) else {
            return;
        };
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let label = ui
                        .label(format!(
                            "{} / {}",
                            Self::format_time(self.target),
                            Self::format_time(total)
                        ))
                        .on_hover_text("Estimated at constant feed rates, without acceleration");
                    ui.spacing_mut().slider_width = (ui.available_width() - 16.0).max(40.0);
                    let slider = egui::Slider::new(&mut self.target, 0.0..=total.max(1e-9))
                        .show_value(false);
                    ui.add(slider).labelled_by(label.id);
                });
                let Some(sim) = &self.sim else {
                    return;
                };
                if let Some(line) = sim.current_line()
                    && let Some(text) = lines.get(line - 1)
                {
                    ui.label(RichText::new(format!("Line {line}: {text}")).monospace());
                }
                for cut in sim.rapid_cuts() {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!(
                            "Line {}: a rapid move cuts {:.3} into the stock",
                            cut.line, cut.depth
                        ),
                    );
                }
                for warning in sim.warnings() {
                    ui.colored_label(ui.visuals().warn_fg_color, warning);
                }
            });
    }

    /// Sends changed stock cells to the texture.
    fn upload_texture(&mut self, ctx: &egui::Context) {
        let Some(sim) = &mut self.sim else {
            return;
        };
        let dirty = sim.take_dirty();
        let (nx, ny) = sim.grid_size();
        let relief = self.relief;
        let options = if self.smooth {
            TextureOptions::LINEAR
        } else {
            TextureOptions::NEAREST
        };
        match (&mut self.texture, dirty) {
            (None, _) => {
                let all = CellRect {
                    min_i: 0,
                    min_j: 0,
                    max_i: nx - 1,
                    max_j: ny - 1,
                };
                self.texture = Some(ctx.load_texture(
                    "simulated stock",
                    Self::shade(sim, all, relief),
                    options,
                ));
            }
            (Some(texture), Some(rect)) => {
                texture.set_partial(
                    [rect.min_i, ny - 1 - rect.max_j],
                    Self::shade(sim, rect, relief),
                    options,
                );
            }
            (Some(_), None) => {}
        }
    }

    /// Renders a rectangle of the stock with a light from the upper left, top row first.
    fn shade(sim: &Simulation, rect: CellRect, relief: f32) -> ColorImage {
        const WOOD: [f32; 3] = [222.0, 184.0, 135.0];
        let (nx, ny) = sim.grid_size();
        let (top, bottom) = sim.stock_range();
        let cell = sim.cell_size() as f32;
        let span = (top - bottom).max(1e-6);
        let width = rect.max_i - rect.min_i + 1;
        let height = rect.max_j - rect.min_j + 1;
        let mut pixels = Vec::with_capacity(width * height);
        for j in (rect.min_j..=rect.max_j).rev() {
            for i in rect.min_i..=rect.max_i {
                let z = sim.height(i, j);
                let dzdx = relief
                    * (sim.height((i + 1).min(nx - 1), j) - sim.height(i.saturating_sub(1), j))
                    / (2.0 * cell);
                let dzdy = relief
                    * (sim.height(i, (j + 1).min(ny - 1)) - sim.height(i, j.saturating_sub(1)))
                    / (2.0 * cell);
                // Lambert shading of the surface normal (-dzdx, -dzdy, 1).
                let normal_len = (dzdx * dzdx + dzdy * dzdy + 1.0).sqrt();
                let lambert = (0.45 * dzdx - 0.45 * dzdy + 0.77) / normal_len;
                let depth = ((top - z) / span).clamp(0.0, 1.0);
                let brightness = (lambert / 0.77).clamp(0.25, 1.25) * (1.0 - 0.45 * depth);
                let [r, g, b] = WOOD.map(|c| (c * brightness).clamp(0.0, 255.0) as u8);
                pixels.push(Color32::from_rgb(r, g, b));
            }
        }
        ColorImage::new([width, height], pixels)
    }

    fn path_shapes(&mut self, t: &ViewTransform) -> Arc<Vec<egui::Shape>> {
        if let Some((cached, shapes)) = &self.path_cache
            && cached == t
        {
            return shapes.clone();
        }
        let shapes = Arc::new(
            self.sim
                .as_ref()
                .map_or_else(Vec::new, |sim| Self::build_path_shapes(sim.moves(), t)),
        );
        self.path_cache = Some((*t, shapes.clone()));
        shapes
    }

    fn build_path_shapes(moves: &[SimMove], t: &ViewTransform) -> Vec<egui::Shape> {
        let mut cuts: Vec<CarvePoint> = Vec::new();
        let mut shapes = Vec::new();
        let mut group = 0;
        let rapid = Stroke::new(1.0, Color32::from_rgb(235, 87, 87));
        let cut = Stroke::new(1.0, Color32::from_rgb(30, 80, 170));
        for m in moves {
            if m.rapid {
                group += 1;
                let (a, b) = (
                    t.to_screen(m.from[0], m.from[1]),
                    t.to_screen(m.to[0], m.to[1]),
                );
                if (a - b).length() > 0.1 {
                    shapes.extend(egui::Shape::dashed_line(&[a, b], rapid, 4.0, 4.0));
                }
            } else {
                let point = |p: [f64; 3]| CarvePoint {
                    x: p[0],
                    y: p[1],
                    r: 0.0,
                    loop_num: group,
                };
                cuts.push(point(m.from));
                cuts.push(point(m.to));
            }
        }
        Preview::path_lines(&cuts, t, cut, &mut shapes);
        shapes
    }

    fn draw_tool(painter: &egui::Painter, t: &ViewTransform, sim: &Simulation) {
        let [x, y, z] = sim.tool_position();
        let center = t.to_screen(x, y);
        let color = if z < 0.0 {
            Color32::from_rgb(200, 30, 30)
        } else {
            Color32::from_rgb(20, 120, 60)
        };
        let radius = (sim.tool_radius() as f32 * t.scale).max(4.0);
        painter.circle_stroke(center, radius, Stroke::new(1.5, color));
        painter.circle_filled(center, 2.5, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_formatted_for_people() {
        assert_eq!(SimulatorView::format_time(0.0), "0:00");
        assert_eq!(SimulatorView::format_time(65.4), "1:05");
        assert_eq!(SimulatorView::format_time(3725.0), "1:02:05");
        assert_eq!(SimulatorView::format_time(-1.0), "0:00");
    }

    #[test]
    fn initial_speed_scales_with_program_length() {
        assert_eq!(SimulatorView::comfortable_speed(30.0), 1.0);
        assert_eq!(SimulatorView::comfortable_speed(300.0), 10.0);
        assert_eq!(SimulatorView::comfortable_speed(3000.0), 100.0);
        assert_eq!(SimulatorView::comfortable_speed(1e7), 1000.0);
    }

    fn simulated() -> Simulation {
        use rengrave_core::font::{Font, Stroke as FontStroke};
        use rengrave_core::{CutType, InputType};
        let font = Font::from_drawing(vec![
            FontStroke::new(0.0, 0.0, 0.0, 1.0),
            FontStroke::new(0.0, 1.0, 1.0, 1.0),
            FontStroke::new(1.0, 1.0, 1.0, 0.0),
            FontStroke::new(1.0, 0.0, 0.0, 0.0),
        ]);
        let settings = Settings {
            input_type: InputType::Image,
            cut_type: CutType::VCarve,
            y_scale: 1.0,
            ..Settings::default()
        };
        let design = Design::layout(&font, &settings).unwrap();
        let program: Vec<String> = ["G20 G90", "F60", "G0 X0.5 Y0.5", "G1 Z-0.2", "G1 X0.9"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        Simulation::new(&program, &design, &settings, Simulation::DEFAULT_RESOLUTION).unwrap()
    }

    #[test]
    fn shading_darkens_cuts_and_flips_rows() {
        let mut sim = simulated();
        let (nx, ny) = sim.grid_size();
        let all = CellRect {
            min_i: 0,
            min_j: 0,
            max_i: nx - 1,
            max_j: ny - 1,
        };
        let before = SimulatorView::shade(&sim, all, 1.0);
        assert_eq!(before.size, [nx, ny]);
        let uncut = before.pixels[0];
        assert!(before.pixels.iter().all(|p| *p == uncut));

        sim.run_to_end();
        let after = SimulatorView::shade(&sim, all, 1.0);
        assert_eq!(after.pixels[0], uncut, "the corner stays uncut");

        // The groove lies in the lower half of Y here, so it is in the lower half of the image.
        let (ox, oy) = sim.grid_origin();
        let cell = sim.cell_size();
        let i = ((0.7 - ox) / cell).round() as usize;
        let j = ((0.5 - oy) / cell).round() as usize;
        let groove = after.pixels[(ny - 1 - j) * nx + i];
        assert!(groove.r() < uncut.r(), "{groove:?} vs {uncut:?}");
    }

    #[test]
    fn partial_shading_matches_full_shading() {
        let mut sim = simulated();
        sim.run_to_end();
        let (nx, ny) = sim.grid_size();
        let all = SimulatorView::shade(
            &sim,
            CellRect {
                min_i: 0,
                min_j: 0,
                max_i: nx - 1,
                max_j: ny - 1,
            },
            1.0,
        );
        let rect = CellRect {
            min_i: 3,
            min_j: 5,
            max_i: 40,
            max_j: 90,
        };
        let part = SimulatorView::shade(&sim, rect, 1.0);
        assert_eq!(part.size, [38, 86]);
        // The first pixel is the top left: the highest row of the rectangle.
        let expected = all.pixels[(ny - 1 - rect.max_j) * nx + rect.min_i];
        assert_eq!(part.pixels[0], expected);
    }

    #[test]
    fn path_separates_rapids_from_cuts() {
        let sim = simulated();
        let t = ViewTransform {
            scale: 100.0,
            center: (0.5, 0.5),
            origin: Pos2::new(200.0, 200.0),
        };
        let shapes = SimulatorView::build_path_shapes(sim.moves(), &t);
        assert!(shapes.len() >= 2);
        let empty = SimulatorView::build_path_shapes(&[], &t);
        assert!(empty.is_empty());
    }

    #[test]
    fn relief_exaggerates_the_shading_without_changing_the_stock() {
        let mut sim = simulated();
        sim.run_to_end();
        let (nx, ny) = sim.grid_size();
        let all = CellRect {
            min_i: 0,
            min_j: 0,
            max_i: nx - 1,
            max_j: ny - 1,
        };
        let flat = SimulatorView::shade(&sim, all, 1.0);
        let deep = SimulatorView::shade(&sim, all, 4.0);
        assert_ne!(flat.pixels, deep.pixels);
        // Cells away from any edge look the same, so only the walls of the groove change.
        assert_eq!(flat.pixels[0], deep.pixels[0]);
    }
}

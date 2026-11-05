// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Simulates a G-code program: moves a tool along the interpreted path and removes material from
//! a height-field model of the stock.
//!
//! The simulation reads the program text rather than the calculated toolpath, so it shows what a
//! machine would receive, including simplification, arc fitting, passes, and retracts.
//!
//! Times assume constant feed rates and a fixed rapid rate, without acceleration, so they are
//! estimates. The stock is a grid of heights, so it shows a 3-axis result up to the resolution of
//! that grid.

use crate::design::Design;
use crate::error::{Error, Result};
use crate::geom::Bounds;
use crate::portable_math::PortableMath as _;
use crate::settings::{BitShape, CutType, Settings};
use glam::DVec3;
use std::time::{Duration, Instant};
use warp_gcode::{Interpreter, MotionKind};
use warp_sim::{Footprint, Stock};
use warp_tools::{Tool, ToolShape};

/// Assumed speed of rapid moves, in millimeters per minute.
const RAPID_MM_PER_MIN: f64 = 3000.0;
/// Most grid cells across the radius of the tool; beyond this the grid is coarsened.
const MAX_RADIUS_CELLS: f64 = 64.0;
/// Number of stock snapshots kept for moving backward in time.
const CHECKPOINTS: usize = 16;
/// Longest stretch of a move stamped at once, in grid cells, which keeps time budgets accurate.
const MAX_PIECE_CELLS: f64 = 32.0;

/// A straight piece of the simulated path, in the program's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimMove {
    /// Start position.
    pub from: [f64; 3],
    /// End position.
    pub to: [f64; 3],
    /// Whether this is a rapid `G0` move.
    pub rapid: bool,
    /// One-based line of the program.
    pub line: usize,
    /// Time at which the move begins, in seconds.
    pub start: f64,
    /// Time the move takes, in seconds.
    pub duration: f64,
}

/// A rectangle of grid cells, with inclusive bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    /// First column.
    pub min_i: usize,
    /// First row.
    pub min_j: usize,
    /// Last column.
    pub max_i: usize,
    /// Last row.
    pub max_j: usize,
}

/// A rapid move that cut into the stock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RapidCut {
    /// One-based line of the program.
    pub line: usize,
    /// Deepest material removed by the move.
    pub depth: f64,
}

/// A saved state of the stock, for moving backward in time.
struct Checkpoint {
    time: f64,
    next: usize,
    position: DVec3,
    heights: Vec<f32>,
}

/// A running simulation of one program.
pub struct Simulation {
    moves: Vec<SimMove>,
    stock: Stock,
    footprint: Footprint,
    reach: usize,
    tool_radius: f64,
    total_seconds: f64,
    time: f64,
    next: usize,
    position: DVec3,
    checkpoints: Vec<Checkpoint>,
    dirty: Option<CellRect>,
    rapid_cuts: Vec<RapidCut>,
    warnings: Vec<String>,
    /// Smallest removal by a rapid that counts as cutting, ignoring grid rounding.
    rapid_cut_threshold: f32,
}

impl Simulation {
    /// Grid cells along the longer side of the stock, unless the tool is large for it.
    pub const DEFAULT_RESOLUTION: usize = 1024;

    /// The finest resolution used, since the grid has one more point than cells and a texture
    /// of 2048 points a side is the limit on some graphics hardware.
    pub const MAX_RESOLUTION: usize = 2047;

    /// The resolutions offered to the user, from fastest to finest.
    pub const RESOLUTIONS: [usize; 3] = [512, 1024, 2048];

    /// Interprets a program and prepares untouched stock around `design`.
    ///
    /// `lines` is the program, as produced by [`crate::GcodeProgram::generate`]. Fails if the
    /// program cannot be interpreted, which can happen with a custom header or footer.
    /// `resolution` is the number of grid cells along the longer side of the stock.
    pub fn new(
        lines: &[String],
        design: &Design,
        settings: &Settings,
        resolution: usize,
    ) -> Result<Self> {
        let source = Self::preprocess(lines)?;
        let mut interpreter = Interpreter::default();
        let program = interpreter
            .run_str(&source)
            .map_err(|e| Error::Simulation(e.to_string()))?;
        // The interpreter works in millimeters; report positions in the program's own units.
        let scale = if interpreter.state.inches {
            1.0 / 25.4
        } else {
            1.0
        };
        let chord_mm = (settings.accuracy / scale / 2.0).max(0.005);

        let mut moves = Vec::new();
        let mut clock = 0.0;
        for motion in program.motions.iter().filter(|m| m.is_move()) {
            let rapid = matches!(motion.kind, MotionKind::Rapid);
            let rate = if rapid {
                RAPID_MM_PER_MIN
            } else {
                motion.feed.max(1.0)
            };
            let mut from = motion.from;
            for (to, _) in motion.linearize(chord_mm) {
                let duration = from.distance(to) / rate * 60.0;
                moves.push(SimMove {
                    from: (from * scale).to_array(),
                    to: (to * scale).to_array(),
                    rapid,
                    line: motion.line,
                    start: clock,
                    duration,
                });
                clock += duration;
                from = to;
            }
        }

        let deepest = moves
            .iter()
            .flat_map(|m| [m.from[2], m.to[2]])
            .fold(0.0_f64, f64::min);
        let tool = Self::tool_for(settings, deepest);
        let tool_radius = tool.radius();
        let extent = Self::stock_extent(&moves, design, tool_radius);
        let longest = extent.width().max(extent.height()).max(1e-6);
        let cell = (longest / resolution.clamp(16, Self::MAX_RESOLUTION) as f64)
            .max(tool_radius / MAX_RADIUS_CELLS);
        let bottom = deepest * 1.25 - longest * 0.01;
        let stock = Stock::new(
            DVec3::new(extent.min_x, extent.min_y, bottom),
            DVec3::new(extent.max_x, extent.max_y, 0.0),
            (longest / cell).ceil().max(16.0) as usize,
        )
        .map_err(Error::Simulation)?;
        let footprint = Footprint::new(&tool, stock.cell);
        let reach = (tool_radius / stock.cell).ceil() as usize;

        Ok(Self {
            total_seconds: clock,
            position: Self::start_position(&moves),
            moves,
            footprint,
            reach,
            tool_radius,
            time: 0.0,
            next: 0,
            checkpoints: Vec::new(),
            dirty: Some(CellRect {
                min_i: 0,
                min_j: 0,
                max_i: stock.nx - 1,
                max_j: stock.ny - 1,
            }),
            rapid_cuts: Vec::new(),
            warnings: program.warnings,
            rapid_cut_threshold: (longest * 1e-4) as f32,
            stock,
        })
    }

    fn start_position(moves: &[SimMove]) -> DVec3 {
        moves
            .first()
            .map_or(DVec3::ZERO, |m| DVec3::from_array(m.from))
    }

    /// Prepares program text for the interpreter: removes comments, and replaces `#n` variables
    /// with their values. Line numbers are preserved.
    fn preprocess(lines: &[String]) -> Result<String> {
        let mut variables: Vec<(String, String)> = Vec::new();
        let mut out = String::new();
        for (index, raw) in lines.iter().enumerate() {
            let code = Self::strip_comments(raw);
            let trimmed = code.trim();
            if let Some(rest) = trimmed.strip_prefix('#')
                && let Some((name, value)) = rest.split_once('=')
            {
                let (name, value) = (name.trim(), value.trim());
                variables.retain(|(n, _)| n != name);
                variables.push((name.to_string(), value.to_string()));
                out.push('\n');
                continue;
            }
            let mut rest = trimmed;
            while let Some(at) = rest.find('#') {
                out.push_str(&rest[..at]);
                let after = &rest[at + 1..];
                let digits = after.bytes().take_while(u8::is_ascii_digit).count();
                let name = &after[..digits];
                let (_, value) = variables.iter().find(|(n, _)| n == name).ok_or_else(|| {
                    Error::Simulation(format!("line {}: undefined variable #{name}", index + 1))
                })?;
                out.push_str(value);
                rest = &after[digits..];
            }
            out.push_str(rest);
            out.push('\n');
        }
        Ok(out)
    }

    /// Removes `( … )` comments, which may nest, and `;` comments from a line.
    fn strip_comments(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut depth = 0_u32;
        for ch in line.chars() {
            match ch {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ';' if depth == 0 => break,
                _ if depth == 0 => out.push(ch),
                _ => {}
            }
        }
        out
    }

    /// The tool that the settings describe.
    fn tool_for(settings: &Settings, deepest: f64) -> Tool {
        if settings.cut_type == CutType::Engrave {
            let diameter = settings.stroke_thickness.max(1e-6);
            return Tool::new(1, "engraver", ToolShape::FlatEndMill, diameter);
        }
        match settings.bit_shape {
            BitShape::VBit => {
                // The cone above the surface removes nothing, so model it only as wide as its
                // deepest cut. This keeps stamping fast for wide bits.
                let half = (settings.v_bit_angle / 2.0).to_radians();
                let at_depth = 2.0 * -deepest * half.portable_tan();
                let diameter = at_depth.min(settings.v_bit_dia).max(1e-6);
                let shape = ToolShape::VBit {
                    included_angle: settings.v_bit_angle,
                    tip_diameter: 0.0,
                };
                Tool::new(1, "v-bit", shape, diameter)
            }
            BitShape::Ball => Tool::new(1, "ball", ToolShape::BallEndMill, settings.v_bit_dia),
            BitShape::Flat => Tool::new(1, "straight", ToolShape::FlatEndMill, settings.v_bit_dia),
        }
    }

    /// The area the stock covers: the design and everything cut, plus room for the tool.
    fn stock_extent(moves: &[SimMove], design: &Design, tool_radius: f64) -> Bounds {
        let mut extent = Bounds::EMPTY;
        extent.include(design.bounds.min_x, design.bounds.min_y);
        extent.include(design.bounds.max_x, design.bounds.max_y);
        for m in moves.iter().filter(|m| m.to[2] < 0.0 || m.from[2] < 0.0) {
            extent.include(m.from[0], m.from[1]);
            extent.include(m.to[0], m.to[1]);
        }
        let margin = tool_radius + extent.width().max(extent.height()) * 0.02;
        Bounds {
            min_x: extent.min_x - margin,
            min_y: extent.min_y - margin,
            max_x: extent.max_x + margin,
            max_y: extent.max_y + margin,
        }
    }

    /// Estimated run time of the whole program, in seconds.
    pub fn total_seconds(&self) -> f64 {
        self.total_seconds
    }

    /// The current time in the program, in seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// The straight pieces of the path, in order.
    pub fn moves(&self) -> &[SimMove] {
        &self.moves
    }

    /// The position of the tool tip.
    pub fn tool_position(&self) -> [f64; 3] {
        self.position.to_array()
    }

    /// The radius modeled for the tool, which for a V-bit is limited to its widest cut.
    pub fn tool_radius(&self) -> f64 {
        self.tool_radius
    }

    /// One-based program line being executed, if the program has any motion.
    pub fn current_line(&self) -> Option<usize> {
        let index = self.next.min(self.moves.len().checked_sub(1)?);
        Some(self.moves[index].line)
    }

    /// Rapid moves found so far that cut into the stock, which on a machine could be a crash.
    pub fn rapid_cuts(&self) -> &[RapidCut] {
        &self.rapid_cuts
    }

    /// Notes from interpreting the program, such as unsupported codes it ignored.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Number of grid columns and rows.
    pub fn grid_size(&self) -> (usize, usize) {
        (self.stock.nx, self.stock.ny)
    }

    /// Design coordinates of the center of cell `(0, 0)`.
    pub fn grid_origin(&self) -> (f64, f64) {
        (self.stock.origin.x, self.stock.origin.y)
    }

    /// Width of a grid cell.
    pub fn cell_size(&self) -> f64 {
        self.stock.cell
    }

    /// The Z of the original stock surface and of the bottom of the stock.
    pub fn stock_range(&self) -> (f32, f32) {
        (self.stock.top, self.stock.bottom)
    }

    /// Height of the stock surface at column `i` and row `j`, counted from the minimum corner.
    pub fn height(&self, i: usize, j: usize) -> f32 {
        self.stock.height(i, j)
    }

    /// Height of the stock surface at the cell containing a design position, if it is on the stock.
    pub fn height_at(&self, x: f64, y: f64) -> Option<f32> {
        let i = ((x - self.stock.origin.x) / self.stock.cell).round();
        let j = ((y - self.stock.origin.y) / self.stock.cell).round();
        let on_stock =
            i >= 0.0 && j >= 0.0 && (i as usize) < self.stock.nx && (j as usize) < self.stock.ny;
        on_stock.then(|| self.stock.height(i as usize, j as usize))
    }

    /// Returns the cells that changed since the last call, for updating a display.
    pub fn take_dirty(&mut self) -> Option<CellRect> {
        self.dirty.take()
    }

    /// Advances or rewinds to `target` seconds, working for at most `budget`.
    ///
    /// Returns `true` once the simulation has reached the target. If time runs out, call again to
    /// continue. Without a budget, it runs until the target is reached.
    pub fn advance_to(&mut self, target: f64, budget: Option<Duration>) -> bool {
        let target = target.clamp(0.0, self.total_seconds);
        if target < self.time {
            self.rewind_before(target);
        }
        let deadline = budget.and_then(|b| Instant::now().checked_add(b));
        let interval = self.total_seconds / CHECKPOINTS as f64;
        while self.time < target && self.next < self.moves.len() {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return false;
            }
            self.step_toward(target);
            let last_saved = self.checkpoints.last().map_or(0.0, |c| c.time);
            if self.time >= last_saved + interval && self.time < self.total_seconds {
                self.save_checkpoint();
            }
        }
        true
    }

    /// Runs the whole program.
    pub fn run_to_end(&mut self) {
        self.advance_to(self.total_seconds, None);
    }

    /// Cuts one bounded piece of the current move, stopping at `target` seconds.
    fn step_toward(&mut self, target: f64) {
        let m = self.moves[self.next];
        let (from, to) = (DVec3::from_array(m.from), DVec3::from_array(m.to));
        let length = from.distance(to);

        // How far along the move the tool may get, as a fraction.
        let by_time = if m.duration > 0.0 {
            (target - m.start) / m.duration
        } else {
            1.0
        };
        let by_piece = if length > 0.0 {
            (from.distance(self.position) + MAX_PIECE_CELLS * self.stock.cell) / length
        } else {
            1.0
        };
        let fraction = by_time.min(by_piece).clamp(0.0, 1.0);
        let finished = fraction >= 1.0;
        let reaches_target = by_time <= by_piece;
        let end = if finished {
            to
        } else {
            from.lerp(to, fraction)
        };

        let removed = self.stock.cut_segment(&self.footprint, self.position, end);
        self.mark_dirty(self.position, end);
        if m.rapid
            && removed > self.rapid_cut_threshold
            && self.rapid_cuts.iter().all(|c| c.line != m.line)
        {
            self.rapid_cuts.push(RapidCut {
                line: m.line,
                depth: f64::from(removed),
            });
        }
        self.position = end;
        if finished {
            self.time = m.start + m.duration;
            self.next += 1;
            if self.next == self.moves.len() {
                self.time = self.total_seconds;
            }
        } else if reaches_target {
            // Set exactly: recomputing it could round to just short of the target forever.
            self.time = target;
        } else {
            self.time = m.start + m.duration * fraction;
        }
    }

    fn mark_dirty(&mut self, a: DVec3, b: DVec3) {
        let reach = self.reach as f64 + 1.0;
        let (cell, origin) = (self.stock.cell, self.stock.origin);
        let to_cell = |value: f64, base: f64, count: usize, offset: f64| {
            (((value - base) / cell).round() + offset).clamp(0.0, (count - 1) as f64) as usize
        };
        let (nx, ny) = (self.stock.nx, self.stock.ny);
        let rect = CellRect {
            min_i: to_cell(a.x.min(b.x), origin.x, nx, -reach),
            min_j: to_cell(a.y.min(b.y), origin.y, ny, -reach),
            max_i: to_cell(a.x.max(b.x), origin.x, nx, reach),
            max_j: to_cell(a.y.max(b.y), origin.y, ny, reach),
        };
        self.dirty = Some(match self.dirty {
            Some(d) => CellRect {
                min_i: d.min_i.min(rect.min_i),
                min_j: d.min_j.min(rect.min_j),
                max_i: d.max_i.max(rect.max_i),
                max_j: d.max_j.max(rect.max_j),
            },
            None => rect,
        });
    }

    fn save_checkpoint(&mut self) {
        self.checkpoints.push(Checkpoint {
            time: self.time,
            next: self.next,
            position: self.position,
            heights: self.stock.z.clone(),
        });
    }

    /// Restores the latest state at or before `target`, starting over if there is none.
    fn rewind_before(&mut self, target: f64) {
        self.checkpoints.retain(|c| c.time <= target);
        match self.checkpoints.last() {
            Some(c) => {
                self.stock.z.copy_from_slice(&c.heights);
                self.time = c.time;
                self.next = c.next;
                self.position = c.position;
            }
            None => {
                self.stock.reset();
                self.time = 0.0;
                self.next = 0;
                self.position = Self::start_position(&self.moves);
            }
        }
        self.dirty = Some(CellRect {
            min_i: 0,
            min_j: 0,
            max_i: self.stock.nx - 1,
            max_j: self.stock.ny - 1,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Font, Stroke};
    use crate::gcode::GcodeProgram;
    use crate::job::Job;
    use crate::progress::Progress;
    use crate::settings::{ArcFit, InputType};
    use approx::assert_abs_diff_eq;
    use std::path::Path;

    fn square_font() -> Font {
        Font::from_drawing(vec![
            Stroke::new(0.0, 0.0, 0.0, 1.0),
            Stroke::new(0.0, 1.0, 1.0, 1.0),
            Stroke::new(1.0, 1.0, 1.0, 0.0),
            Stroke::new(1.0, 0.0, 0.0, 0.0),
        ])
    }

    fn square_settings() -> Settings {
        Settings {
            input_type: InputType::Image,
            cut_type: CutType::VCarve,
            y_scale: 1.0,
            v_step_len: 0.01,
            ..Settings::default()
        }
    }

    fn job_with(font: Font, settings: Settings) -> Job {
        Job {
            settings,
            font,
            warnings: Vec::new(),
        }
    }

    /// Generates a program for `settings` and runs it to the end.
    fn simulate(font: Font, settings: Settings) -> (Simulation, Design) {
        let job = job_with(font, settings);
        let (design, carve) = job.calculate(&Progress::none()).unwrap();
        let lines = GcodeProgram::generate(&design, carve.as_ref(), &job.settings).unwrap();
        let mut sim = Simulation::new(
            &lines,
            &design,
            &job.settings,
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        sim.run_to_end();
        (sim, design)
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn plain_design() -> Design {
        let settings = Settings {
            input_type: InputType::Image,
            y_scale: 10.0,
            stroke_thickness: 0.0,
            ..Settings::default()
        };
        Design::layout(&square_font(), &settings).unwrap()
    }

    fn surface(sim: &Simulation) -> Vec<f32> {
        let (nx, ny) = sim.grid_size();
        (0..ny)
            .flat_map(|j| (0..nx).map(move |i| (i, j)))
            .map(|(i, j)| sim.height(i, j))
            .collect()
    }

    #[test]
    fn variables_and_comments_are_resolved() {
        let source = lines("#1 = 0.25  ( Safe Z )\n#2 = -0.1\nG0 Z#1 (x #9)\nG1 Z#2 F5 ; #8");
        let text = Simulation::preprocess(&source).unwrap();
        assert_eq!(text, "\n\nG0 Z0.25\nG1 Z-0.1 F5\n");
    }

    #[test]
    fn undefined_variable_names_its_line() {
        let err = Simulation::preprocess(&lines("G90\nG0 Z#7"))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("line 2") && err.contains("#7"), "{err}");
    }

    #[test]
    fn nested_comments_are_removed() {
        assert_eq!(Simulation::strip_comments("G1 (a (b) c) X1"), "G1  X1");
        assert_eq!(Simulation::strip_comments("(unclosed G1"), "");
    }

    #[test]
    fn bad_programs_are_errors() {
        let result = Simulation::new(
            &lines("G1 X$"),
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        );
        assert!(matches!(result, Err(Error::Simulation(_))));
    }

    #[test]
    fn reference_programs_all_interpret_and_run() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity");
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "ngc") {
                continue;
            }
            count += 1;
            let program = lines(&std::fs::read_to_string(&path).unwrap());
            let settings = Settings {
                cut_type: CutType::VCarve,
                ..Settings::default()
            };
            let mut sim = Simulation::new(
                &program,
                &plain_design(),
                &settings,
                Simulation::DEFAULT_RESOLUTION,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(sim.total_seconds() > 0.0, "{}", path.display());
            sim.run_to_end();
            assert_eq!(sim.time(), sim.total_seconds());
            assert!(
                sim.rapid_cuts().is_empty(),
                "{}: {:?}",
                path.display(),
                sim.rapid_cuts()
            );
            assert!(
                sim.warnings().is_empty(),
                "{}: {:?}",
                path.display(),
                sim.warnings()
            );
        }
        assert_eq!(count, 35);
    }

    #[test]
    fn inch_programs_report_inches() {
        let program = lines("G20 G90\nF10\nG0 X1 Y2\nG1 X2 Y2");
        let sim = Simulation::new(
            &program,
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        let m = sim.moves().last().unwrap();
        assert_abs_diff_eq!(m.to[0], 2.0, epsilon = 1e-9);
        // One inch at ten inches per minute.
        assert_abs_diff_eq!(m.duration, 6.0, epsilon = 1e-9);
    }

    #[test]
    fn metric_programs_report_millimeters() {
        let program = lines("G21 G90\nF60\nG1 X10");
        let sim = Simulation::new(
            &program,
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        assert_abs_diff_eq!(sim.moves()[0].to[0], 10.0, epsilon = 1e-9);
        assert_abs_diff_eq!(sim.total_seconds(), 10.0, epsilon = 1e-9);
    }

    #[test]
    fn v_carve_reaches_full_depth_and_stays_inside_the_outline() {
        let (sim, _) = simulate(square_font(), square_settings());
        let (nx, ny) = sim.grid_size();
        let (ox, oy) = sim.grid_origin();
        let cell = sim.cell_size();
        let mut deepest = 0.0_f32;
        for j in 0..ny {
            for i in 0..nx {
                let h = sim.height(i, j);
                deepest = deepest.min(h);
                let (x, y) = (ox + i as f64 * cell, oy + j as f64 * cell);
                let margin = cell * 2.0;
                if x < -margin || y < -margin || x > 1.0 + margin || y > 1.0 + margin {
                    assert_eq!(h, 0.0, "cut outside the outline at ({x}, {y})");
                }
            }
        }
        assert_abs_diff_eq!(f64::from(deepest), -0.433, epsilon = 2e-3);
    }

    #[test]
    fn surface_matches_cone_geometry_across_a_wall() {
        // Well away from corners, the carved wall rises from the outline at the bit's angle.
        let (sim, _) = simulate(square_font(), square_settings());
        let half: f64 = 30f64.to_radians();
        for inset in [0.05, 0.1, 0.2] {
            let expected = -inset / half.portable_tan();
            let actual = f64::from(sim.height_at(0.5, inset).unwrap());
            assert_abs_diff_eq!(actual, expected, epsilon = 0.012);
        }
    }

    #[test]
    fn surface_agrees_with_calculated_tool_positions() {
        // The depth implied by each calculated position is what the program should have cut.
        let job = job_with(square_font(), square_settings());
        let (design, carve) = job.calculate(&Progress::none()).unwrap();
        let carve = carve.unwrap();
        let program = GcodeProgram::generate(&design, Some(&carve), &job.settings).unwrap();
        let mut sim = Simulation::new(
            &program,
            &design,
            &job.settings,
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        sim.run_to_end();
        let half: f64 = 30f64.to_radians();
        for p in &carve.points {
            let expected = -p.r / half.portable_tan();
            let actual = f64::from(sim.height_at(p.x, p.y).unwrap());
            // At least as deep as the position requires, within grid and simplification error.
            assert!(
                actual <= expected + 0.015,
                "{actual} vs {expected} at {p:?}"
            );
        }
    }

    #[test]
    fn arc_formats_cut_the_same_surface() {
        let circle: Vec<Stroke> = (0..72)
            .map(|i| {
                let angle = |k: i32| f64::from(k) * 5f64.to_radians();
                let (a, b) = (-angle(i), -angle(i + 1));
                Stroke::new(
                    a.portable_cos(),
                    a.portable_sin(),
                    b.portable_cos(),
                    b.portable_sin(),
                )
            })
            .collect();
        let run = |arc_fit| {
            let settings = Settings {
                arc_fit,
                accuracy: 0.002,
                y_scale: 2.0,
                v_bit_dia: 0.2,
                ..square_settings()
            };
            simulate(Font::from_drawing(circle.clone()), settings).0
        };
        let plain = run(ArcFit::None);
        for fitted in [run(ArcFit::Center), run(ArcFit::Radius)] {
            assert_eq!(fitted.grid_size(), plain.grid_size());
            for (a, b) in surface(&fitted).iter().zip(&surface(&plain)) {
                assert!((a - b).abs() < 0.03, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn multipass_ends_at_the_same_surface() {
        let single = simulate(square_font(), square_settings()).0;
        let multi = simulate(
            square_font(),
            Settings {
                v_rough_stock: 0.02,
                v_max_cut: -0.1,
                ..square_settings()
            },
        )
        .0;
        assert!(multi.total_seconds() > single.total_seconds());
        for (a, b) in surface(&multi).iter().zip(&surface(&single)) {
            assert!((a - b).abs() < 0.02, "{a} vs {b}");
        }
    }

    #[test]
    fn ball_and_flat_bits_reach_their_depths() {
        let ball = simulate(
            square_font(),
            Settings {
                bit_shape: BitShape::Ball,
                v_bit_dia: 0.25,
                ..square_settings()
            },
        )
        .0;
        assert_abs_diff_eq!(
            f64::from(ball.height_at(0.5, 0.5).unwrap()),
            0.0,
            epsilon = 2e-3
        );
        let deepest = surface(&ball).into_iter().fold(0.0_f32, f32::min);
        assert_abs_diff_eq!(f64::from(deepest), -0.125, epsilon = 2e-3);

        let flat = simulate(
            square_font(),
            Settings {
                bit_shape: BitShape::Flat,
                v_bit_dia: 0.1,
                v_depth_limit: -0.05,
                y_scale: 2.0,
                ..square_settings()
            },
        )
        .0;
        let deepest = surface(&flat).into_iter().fold(0.0_f32, f32::min);
        assert_abs_diff_eq!(f64::from(deepest), -0.05, epsilon = 1e-4);
    }

    #[test]
    fn engraved_circle_cuts_a_full_ring() {
        let settings = Settings {
            input_type: InputType::Text,
            text: "F".into(),
            text_radius: 3.0,
            add_box: true,
            stroke_thickness: 0.05,
            z_cut: -0.02,
            ..Settings::default()
        };
        let font = square_font();
        let (sim, design) = simulate(font, settings);
        let (c, r) = (design.circle_center, design.circle_radius);
        for degrees in [0.0_f64, 90.0, 180.0, 270.0, 45.0] {
            let (sin, cos) = degrees.to_radians().portable_sin_cos();
            let on_ring = sim.height_at(c.x + r * cos, c.y + r * sin).unwrap();
            assert_abs_diff_eq!(f64::from(on_ring), -0.02, epsilon = 1e-4);
        }
    }

    #[test]
    fn engraving_cuts_a_line_at_constant_depth() {
        let settings = Settings {
            input_type: InputType::Image,
            y_scale: 10.0,
            stroke_thickness: 0.1,
            z_cut: -0.02,
            ..Settings::default()
        };
        let (sim, design) = simulate(square_font(), settings);
        let s = design.segments[0];
        let at = sim
            .height_at((s.x1 + s.x2) / 2.0, (s.y1 + s.y2) / 2.0)
            .unwrap();
        assert_abs_diff_eq!(f64::from(at), -0.02, epsilon = 1e-4);
        let center = design.bounds.center();
        assert_eq!(sim.height_at(center.x, center.y), Some(0.0));
    }

    #[test]
    fn time_advances_the_tool_along_a_move() {
        let program = lines("G21 G90\nF60\nG1 X10");
        let mut sim = Simulation::new(
            &program,
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        assert!(sim.advance_to(5.0, None));
        assert_abs_diff_eq!(sim.time(), 5.0, epsilon = 1e-9);
        assert_abs_diff_eq!(sim.tool_position()[0], 5.0, epsilon = 1e-9);
        assert_eq!(sim.current_line(), Some(3));
    }

    #[test]
    fn inexact_targets_terminate_exactly_on_time() {
        let program = lines("G21 G90\nF61\nG1 X10.3 Y3.7\nG1 X1.1 Y9.9\nG1 X7");
        let mut sim = Simulation::new(
            &program,
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        let total = sim.total_seconds();
        for k in 0..200 {
            let target = total * f64::from(k) / 199.0 * 0.999_999_937;
            assert!(sim.advance_to(target, None));
            assert_eq!(sim.time(), target);
        }
    }

    #[test]
    fn rewinding_reproduces_the_same_state() {
        let (mut sim, _) = simulate(square_font(), square_settings());
        let total = sim.total_seconds();
        let finished = surface(&sim);

        assert!(sim.advance_to(total * 0.4, None));
        assert_abs_diff_eq!(sim.time(), total * 0.4, epsilon = 1e-9);
        let midway = surface(&sim);
        assert_ne!(midway, finished, "rewinding restores material");
        assert!(midway.iter().zip(&finished).all(|(m, f)| m >= f));

        sim.run_to_end();
        for (a, b) in surface(&sim).iter().zip(&finished) {
            assert!((a - b).abs() < 0.02, "{a} vs {b}");
        }
        assert!(sim.advance_to(0.0, None));
        assert!(surface(&sim).iter().all(|h| *h == 0.0));
    }

    #[test]
    fn checkpoints_are_bounded() {
        let (mut sim, _) = simulate(square_font(), square_settings());
        for _ in 0..3 {
            sim.advance_to(0.0, None);
            sim.run_to_end();
            assert!(sim.checkpoints.len() <= CHECKPOINTS);
        }
    }

    #[test]
    fn budget_stops_early_and_resumes() {
        let job = job_with(square_font(), square_settings());
        let (design, carve) = job.calculate(&Progress::none()).unwrap();
        let program = GcodeProgram::generate(&design, carve.as_ref(), &job.settings).unwrap();
        let mut sim = Simulation::new(
            &program,
            &design,
            &job.settings,
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        let total = sim.total_seconds();
        assert!(!sim.advance_to(total, Some(Duration::ZERO)));
        assert_eq!(sim.time(), 0.0);
        let mut calls = 0;
        while !sim.advance_to(total, Some(Duration::from_millis(1))) {
            calls += 1;
            assert!(calls < 100_000);
        }
        assert_eq!(sim.time(), total);
    }

    #[test]
    fn dirty_region_covers_changes_then_clears() {
        let program = lines("G21 G90\nF60\nG0 X2 Y2\nG1 Z-0.5\nG1 X4");
        let settings = Settings {
            cut_type: CutType::VCarve,
            ..Settings::default()
        };
        let mut sim = Simulation::new(
            &program,
            &plain_design(),
            &settings,
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        let (nx, ny) = sim.grid_size();
        let all = sim.take_dirty().unwrap();
        assert_eq!((all.max_i, all.max_j), (nx - 1, ny - 1));
        assert!(sim.take_dirty().is_none());

        sim.run_to_end();
        let dirty = sim.take_dirty().unwrap();
        assert!(
            dirty.max_i < nx - 1 || dirty.min_i > 0,
            "only what changed: {dirty:?}"
        );
        let mut changed = 0;
        for j in 0..ny {
            for i in 0..nx {
                if sim.height(i, j) != 0.0 {
                    changed += 1;
                    assert!((dirty.min_i..=dirty.max_i).contains(&i), "{i} in {dirty:?}");
                    assert!((dirty.min_j..=dirty.max_j).contains(&j), "{j} in {dirty:?}");
                }
            }
        }
        assert!(changed > 0);
    }

    #[test]
    fn rapids_through_stock_are_reported_with_their_line() {
        let program = lines("G21 G90\nF60\nG1 Z-1\nG0 X5\nG1 X6");
        let settings = Settings {
            stroke_thickness: 0.5,
            ..Settings::default()
        };
        let mut sim = Simulation::new(
            &program,
            &plain_design(),
            &settings,
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        sim.run_to_end();
        assert_eq!(sim.rapid_cuts().len(), 1);
        assert_eq!(sim.rapid_cuts()[0].line, 4);
        assert_abs_diff_eq!(sim.rapid_cuts()[0].depth, 1.0, epsilon = 1e-4);
    }

    #[test]
    fn conical_tool_is_limited_to_its_widest_cut() {
        let settings = square_settings();
        let shallow = Simulation::tool_for(&settings, -0.05);
        assert_abs_diff_eq!(
            shallow.radius(),
            0.05 * 30f64.to_radians().portable_tan(),
            epsilon = 1e-9
        );
        let deep = Simulation::tool_for(&settings, -5.0);
        assert_abs_diff_eq!(deep.radius(), 0.25, epsilon = 1e-9);
    }

    #[test]
    fn program_without_motion_is_harmless() {
        let mut sim = Simulation::new(
            &lines("G90\nM2"),
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        assert_eq!(sim.total_seconds(), 0.0);
        assert!(sim.advance_to(10.0, None));
        assert_eq!(sim.current_line(), None);
    }

    #[test]
    fn program_end_stops_interpretation() {
        let program = lines("G21\nF60\nG1 X1\nM2\nG1 X99");
        let sim = Simulation::new(
            &program,
            &plain_design(),
            &Settings::default(),
            Simulation::DEFAULT_RESOLUTION,
        )
        .unwrap();
        assert_eq!(sim.moves().len(), 1);
    }

    #[test]
    fn resolution_sets_the_grid_size() {
        let program = lines("G21 G90\nG0 X0 Y0\nG1 Z-1 F100\nG1 X10");
        let size = |resolution| {
            Simulation::new(&program, &plain_design(), &Settings::default(), resolution)
                .unwrap()
                .grid_size()
        };
        let (coarse, fine) = (size(512), size(2048));
        assert!(
            fine.0 > coarse.0 && fine.1 > coarse.1,
            "{coarse:?} {fine:?}"
        );
    }
}

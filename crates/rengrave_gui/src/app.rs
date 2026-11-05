// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Application state and the top-level window layout.

use crate::dialogs::{FileDialogs, FileKind};
use crate::gcode_view::Token;
use crate::icons;
use crate::preview::{Preview, Scene};
use crate::simulator::SimulatorView;
use crate::style::Look;
use crate::widgets::UiExt;
use crate::worker::{Update, Worker};
use egui::{CentralPanel, Key, KeyboardShortcut, Modifiers, Panel, RichText, Ui};
use rengrave_core::font::extension_upper;
use rengrave_core::{
    CarvePath, CarvePoint, CleanupBit, CleanupOps, CutType, Design, FontCatalog, FontCoverage,
    FontRef, GcodeProgram, InputType, Issue, Justify, Settings, Units,
};
use std::path::Path;
use std::sync::Arc;

/// How long input must pause before a recalculation starts, in seconds.
const DEBOUNCE_SECONDS: f64 = 0.12;

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const COPY_GCODE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const FIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0);

/// The groups of settings in the side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// What to cut, and where.
    Design,
    /// How to cut it.
    Toolpath,
    /// What to write.
    Output,
}

/// What occupies the main area of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    /// The graphical preview.
    Preview,
    /// The text of the G-code program.
    Gcode,
    /// The simulated program, cutting into stock.
    Simulator,
}

/// Severity of the message in the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    /// Nothing noteworthy.
    Info,
    /// Work is in progress or results are out of date.
    Busy,
    /// A result was produced, but something needs attention.
    Warning,
    /// The operation failed.
    Error,
}

/// The message in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Severity.
    pub level: StatusLevel,
    /// Text.
    pub text: String,
}

/// The settings that determine cleanup paths, beyond those of the design itself.
#[derive(Clone, Copy, PartialEq)]
struct CleanupInputs {
    dia: f64,
    step: f64,
    v: f64,
    straight: CleanupOps,
    v_bit: CleanupOps,
}

impl CleanupInputs {
    fn of(settings: &Settings) -> Self {
        Self {
            dia: settings.clean_dia,
            step: settings.clean_step,
            v: settings.clean_v,
            straight: settings.clean_straight,
            v_bit: settings.clean_v_bit,
        }
    }
}

/// The cached text of the G-code program.
struct GcodeCache {
    settings: Settings,
    generation: u64,
    lines: Result<Arc<Vec<String>>, String>,
}

/// The G-code program laid out for the text view.
struct GcodeGalley {
    lines: Arc<Vec<String>>,
    dark: bool,
    text: Arc<str>,
    galley: Arc<egui::Galley>,
}

/// All application state.
pub struct RengraveApp {
    pub(crate) ctx: egui::Context,
    /// The current job. Changes are picked up on the next frame.
    pub settings: Settings,
    pub(crate) issues: Vec<Issue>,
    pub(crate) dialogs: Box<dyn FileDialogs>,
    worker: Worker,

    /// The tab shown in the side panel.
    pub tab: Tab,
    /// The content of the main area.
    pub active_view: ActiveView,
    /// Recalculate V-carve paths automatically after a change.
    pub live_carve: bool,
    /// The laid-out G-code text, kept until the program or the theme changes.
    gcode_galley: Option<GcodeGalley>,
    /// The text selected in the G-code view when its context menu was opened.
    gcode_selection: String,
    /// Interactive preview state.
    pub preview: Preview,
    /// Simulator playback state.
    pub simulator: SimulatorView,
    /// The installed fonts and the user's font library.
    pub(crate) fonts: Arc<FontCatalog>,
    pub(crate) font_filter: String,
    show_about: bool,
    /// Whether the application settings dialog is open.
    pub(crate) show_app_settings: bool,
    /// The page of the application settings dialog.
    pub(crate) settings_page: crate::preferences::SettingsPage,
    /// The file the application settings are kept in, if any.
    pub(crate) app_settings_path: Option<std::path::PathBuf>,

    /// The most recent calculation request.
    generation: u64,
    requested: Option<Settings>,
    /// When the settings last differed from the request, in `egui` time.
    changed_at: Option<f64>,
    busy: bool,
    cleanup_busy: bool,

    design: Option<Arc<Design>>,
    carve: Option<Arc<CarvePath>>,
    clean_straight: Option<Arc<Vec<CarvePoint>>>,
    clean_v_bit: Option<Arc<Vec<CarvePoint>>>,
    /// The inputs of the visible cleanup paths.
    cleanup_inputs: Option<CleanupInputs>,
    /// Identifies the latest cleanup request; older results are discarded.
    cleanup_request: u64,
    /// Changes whenever the results to draw change.
    scene_revision: u64,
    gcode_cache: Option<GcodeCache>,

    error: Option<String>,
    warnings: Vec<String>,
    /// The characters of the font of the latest layout.
    pub(crate) font_coverage: Arc<FontCoverage>,
    /// The latest message that is not derived from the job state.
    notice: Option<Status>,
    /// Keep `notice` through the next recalculation, which it may itself have caused.
    keep_notice: bool,
}

impl RengraveApp {
    /// The settings of a new job.
    fn default_settings() -> Settings {
        let mut settings = Settings {
            cut_type: CutType::VCarve,
            text: "F-engrave\nFan!".into(),
            justify: Justify::Center,
            y_scale: 1.0,
            v_step_len: 0.01,
            ..Settings::default()
        };
        // Set in inches, then converted: a one-inch design in millimeters.
        settings.set_units(Units::Millimeters);
        settings
    }

    /// Starts over with the default settings, discarding the current job.
    pub fn new_job(&mut self) {
        self.settings = Self::default_settings();
        self.preview.fit();
        self.notify(
            StatusLevel::Info,
            "Started a new job with the default settings",
        );
    }

    /// Creates the application, using `dialogs` to ask for files.
    pub fn new(ctx: egui::Context, dialogs: Box<dyn FileDialogs>) -> Self {
        let mut app = Self::with_fonts(ctx, dialogs, Arc::new(FontCatalog::with_system_fonts()));
        if let Some(path) = rengrave_core::AppSettings::default_path() {
            app.use_app_settings(path);
        }
        app
    }

    /// Creates the application with its own catalog of fonts. Tests use this to avoid depending
    /// on the fonts of the machine.
    pub fn with_fonts(
        ctx: egui::Context,
        dialogs: Box<dyn FileDialogs>,
        fonts: Arc<FontCatalog>,
    ) -> Self {
        Look::install(&ctx);
        let notify = ctx.clone();
        Self {
            ctx,
            settings: Self::default_settings(),
            issues: Vec::new(),
            dialogs,
            worker: Worker::spawn(fonts.clone(), move || notify.request_repaint()),
            tab: Tab::Design,
            active_view: ActiveView::Preview,
            live_carve: true,
            gcode_galley: None,
            gcode_selection: String::new(),
            preview: Preview::default(),
            simulator: SimulatorView::default(),
            fonts,
            font_filter: String::new(),
            show_about: false,
            show_app_settings: false,
            settings_page: crate::preferences::SettingsPage::default(),
            app_settings_path: None,
            generation: 0,
            requested: None,
            changed_at: None,
            busy: false,
            cleanup_busy: false,
            design: None,
            carve: None,
            clean_straight: None,
            clean_v_bit: None,
            cleanup_inputs: None,
            cleanup_request: 0,
            scene_revision: 0,
            gcode_cache: None,
            error: None,
            warnings: Vec::new(),
            font_coverage: Arc::default(),
            notice: None,
            keep_notice: false,
        }
    }

    /// Serializes the settings for storage between sessions.
    pub fn settings_text(&self) -> String {
        self.settings.to_settings_lines().join("\n")
    }

    /// Restores settings stored by [`RengraveApp::settings_text`].
    pub fn restore_settings_text(&mut self, text: &str) {
        self.settings.apply_settings_text(text);
    }

    /// The positioned outline, once calculated.
    pub fn design(&self) -> Option<&Design> {
        self.design.as_deref()
    }

    /// The V-carve toolpath, once calculated.
    pub fn carve(&self) -> Option<&CarvePath> {
        self.carve.as_deref()
    }

    /// Cleanup paths for `bit`, once calculated.
    pub fn cleanup_paths(&self, bit: CleanupBit) -> Option<&[CarvePoint]> {
        match bit {
            CleanupBit::Straight => self.clean_straight.as_ref().map(|p| p.as_slice()),
            CleanupBit::VBit => self.clean_v_bit.as_ref().map(|p| p.as_slice()),
        }
    }

    /// Returns `true` when no calculation is running or waiting to run.
    pub fn is_idle(&self) -> bool {
        !self.busy && !self.cleanup_busy && !self.settings_are_pending()
    }

    /// Whether the settings differ from the last calculation.
    fn settings_are_pending(&self) -> bool {
        if !self.issues.is_empty() {
            return false;
        }
        match &self.requested {
            Some(requested) => {
                RengraveApp::toolpath_inputs(requested)
                    != RengraveApp::toolpath_inputs(&self.settings)
            }
            None => true,
        }
    }

    /// Whether the visible toolpath is out of date, or was never calculated.
    pub(crate) fn carve_is_stale(&self) -> bool {
        self.settings.cut_type == CutType::VCarve && self.carve.is_none()
    }

    /// Whether the job has something ready to write as G-code.
    pub fn can_write_gcode(&self) -> bool {
        self.issues.is_empty()
            && self.design.is_some()
            && (self.settings.cut_type != CutType::VCarve || self.carve.is_some())
            && !self.busy
            && !self.settings_are_pending()
    }

    /// Returns `true` if any cleanup paths are ready for `bit`.
    pub(crate) fn has_cleanup(&self, bit: CleanupBit) -> bool {
        self.cleanup_paths(bit).is_some_and(|p| !p.is_empty())
    }

    /// The current status line.
    pub fn status(&self) -> Status {
        let status = |level, text: String| Status { level, text };
        if !self.issues.is_empty() {
            let n = self.issues.len();
            return status(
                StatusLevel::Error,
                format!(
                    "{n} setting{} need{} attention: {}",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "s" } else { "" },
                    self.issues[0].message
                ),
            );
        }
        if let Some(error) = &self.error {
            return status(StatusLevel::Error, error.clone());
        }
        if self.busy {
            let done = self.worker.fraction();
            return status(
                StatusLevel::Busy,
                if done > 0.0 {
                    format!("Calculating V-carve: {:.0}%", done * 100.0)
                } else {
                    "Calculating…".to_string()
                },
            );
        }
        if self.cleanup_busy {
            return status(StatusLevel::Busy, "Calculating cleanup paths…".to_string());
        }
        if let Some(notice) = &self.notice {
            return notice.clone();
        }
        if self.carve_is_stale() && self.design.is_some() {
            return status(
                StatusLevel::Busy,
                "V-carve paths are not calculated. Press F5 to calculate.".to_string(),
            );
        }
        if let Some(design) = &self.design {
            let units = self.settings.units;
            let mut text = format!(
                "Bounding box {:.3} × {:.3} {units}",
                design.bounds.width(),
                design.bounds.height()
            );
            if !design.missing_chars.is_empty() {
                let chars: String = design.missing_chars.iter().collect();
                text.push_str(&format!(". Not in font: {chars}"));
                return status(StatusLevel::Warning, text);
            }
            if let Some(issue) = self.settings.controller_issues().first() {
                text.push_str(&format!(". {issue}"));
                return status(StatusLevel::Warning, text);
            }
            if let Some(warning) = self.warnings.first() {
                text.push_str(&format!(". {warning}"));
                return status(StatusLevel::Warning, text);
            }
            return status(StatusLevel::Info, text);
        }
        status(StatusLevel::Info, "Ready".to_string())
    }

    pub(crate) fn notify(&mut self, level: StatusLevel, text: impl Into<String>) {
        self.notice = Some(Status {
            level,
            text: text.into(),
        });
    }

    /// Discards results that no longer match the settings.
    fn clear_results(&mut self) {
        self.scene_revision += 1;
        self.cleanup_request += 1;
        self.cleanup_busy = false;
        self.carve = None;
        self.clean_straight = None;
        self.clean_v_bit = None;
        self.gcode_cache = None;
    }

    /// Starts a calculation now, including V-carve paths if `carve` is set.
    pub fn recalculate(&mut self, carve: bool) {
        self.start_calculation(carve, false);
    }

    /// Reads the font or image again, in case the file changed, and calculates everything.
    pub fn reload_and_recalculate(&mut self) {
        self.start_calculation(true, true);
    }

    fn start_calculation(&mut self, carve: bool, reload: bool) {
        self.generation += 1;
        self.requested = Some(self.settings.clone());
        self.changed_at = None;
        self.busy = true;
        self.error = None;
        if !std::mem::take(&mut self.keep_notice) {
            self.notice = None;
        }
        self.warnings.clear();
        self.clear_results();
        self.worker
            .calculate(self.generation, self.settings.clone(), carve, reload);
    }

    /// Stops the running calculation.
    pub fn cancel(&mut self) {
        self.worker.cancel();
    }

    /// Starts calculating cleanup paths for the current design.
    pub fn calculate_cleanup(&mut self) {
        let Some(design) = self.design.clone() else {
            return;
        };
        self.clean_straight = None;
        self.clean_v_bit = None;
        self.scene_revision += 1;
        self.cleanup_inputs = Some(CleanupInputs::of(&self.settings));
        self.cleanup_busy = true;
        self.cleanup_request += 1;
        self.worker.cleanup(
            self.generation,
            self.cleanup_request,
            self.settings.clone(),
            design,
        );
    }

    fn apply_update(&mut self, update: Update) {
        let generation = match &update {
            Update::Layout { generation, .. }
            | Update::Carve { generation, .. }
            | Update::Cleanup { generation, .. }
            | Update::Stopped { generation, .. } => *generation,
        };
        if generation != self.generation {
            return;
        }
        match update {
            Update::Layout {
                design,
                warnings,
                coverage,
                ..
            } => {
                self.design = Some(design);
                self.warnings = warnings;
                self.font_coverage = coverage;
                self.gcode_cache = None;
                self.scene_revision += 1;
            }
            Update::Carve { design, carve, .. } => {
                self.carve = Some(carve);
                self.design = Some(design);
                self.gcode_cache = None;
                self.scene_revision += 1;
            }
            Update::Cleanup { request, paths, .. } => {
                if request != self.cleanup_request {
                    return;
                }
                self.cleanup_busy = false;
                match paths {
                    Ok((straight, v_bit)) => {
                        self.clean_straight = Some(straight);
                        self.clean_v_bit = Some(v_bit);
                        self.scene_revision += 1;
                    }
                    Err(error) => self.notify(StatusLevel::Error, error),
                }
            }
            Update::Stopped { error, .. } => {
                self.busy = false;
                if let Some(error) = error {
                    self.error = Some(error);
                    self.design = None;
                }
            }
        }
    }

    /// Reacts to settings changes and finished work; called once per frame.
    fn update_state(&mut self, now: f64) {
        while let Some(update) = self.worker.poll() {
            self.apply_update(update);
        }
        if self
            .cleanup_inputs
            .is_some_and(|inputs| inputs != CleanupInputs::of(&self.settings))
        {
            self.clean_straight = None;
            self.clean_v_bit = None;
            self.cleanup_inputs = None;
            self.cleanup_request += 1;
            self.cleanup_busy = false;
            self.scene_revision += 1;
        }
        self.issues = self.settings.validate();
        if !self.issues.is_empty() {
            self.changed_at = None;
            return;
        }
        if self.settings_are_pending() {
            let changed_at = *self.changed_at.get_or_insert(now);
            if now - changed_at >= DEBOUNCE_SECONDS || self.requested.is_none() {
                self.recalculate(self.live_carve);
            } else {
                let remaining = (DEBOUNCE_SECONDS - (now - changed_at)).max(0.0);
                self.ctx
                    .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
            }
        }
    }

    /// Returns the G-code program for the current settings and results.
    pub fn gcode(&mut self) -> Result<Arc<Vec<String>>, String> {
        if let Some(cache) = &self.gcode_cache
            && cache.generation == self.generation
            && cache.settings == self.settings
        {
            return cache.lines.clone();
        }
        let lines = match &self.design {
            Some(design) => GcodeProgram::generate(design, self.carve.as_deref(), &self.settings)
                .map(Arc::new)
                .map_err(|e| e.to_string()),
            None => Err("No design has been calculated".to_string()),
        };
        self.gcode_cache = Some(GcodeCache {
            settings: self.settings.clone(),
            generation: self.generation,
            lines: lines.clone(),
        });
        lines
    }

    fn base_name(&self) -> String {
        if self.settings.input_type == InputType::Image {
            self.settings.image_file.file_stem().map_or_else(
                || "rengrave".to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        } else {
            let safe: String = self
                .settings
                .text
                .chars()
                .take(24)
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            if safe.is_empty() {
                "rengrave".to_string()
            } else {
                safe
            }
        }
    }

    pub(crate) fn write_lines(&mut self, path: &Path, lines: &[String]) {
        let mut text = lines.join("\n");
        text.push('\n');
        match std::fs::write(path, text) {
            Ok(()) => self.notify(StatusLevel::Info, format!("Saved {}", path.display())),
            Err(e) => self.notify(
                StatusLevel::Error,
                format!("Unable to write {}: {e}", path.display()),
            ),
        }
    }

    /// Asks where to save the G-code program, and writes it.
    pub fn save_gcode(&mut self) {
        if !self.can_write_gcode() {
            self.notify(
                StatusLevel::Warning,
                "Nothing to save yet: toolpaths are not up to date.",
            );
            return;
        }
        let lines = match self.gcode() {
            Ok(lines) => lines,
            Err(e) => {
                self.notify(StatusLevel::Error, e);
                return;
            }
        };
        let name = format!("{}.ngc", self.base_name());
        if let Some(path) = self.dialogs.save_file(FileKind::Gcode, &name) {
            self.write_lines(&path, &lines);
        }
    }

    /// Asks where to save a cleanup program, and writes it.
    pub fn save_cleanup(&mut self, bit: CleanupBit) {
        let Some(points) = self.cleanup_paths(bit).map(<[CarvePoint]>::to_vec) else {
            self.notify(StatusLevel::Warning, "Calculate cleanup paths first.");
            return;
        };
        if points.is_empty() {
            self.notify(StatusLevel::Warning, "No cleanup paths were found.");
            return;
        }
        let lines = match GcodeProgram::generate_cleanup(&points, bit, &self.settings) {
            Ok(lines) => lines,
            Err(e) => {
                self.notify(StatusLevel::Error, e.to_string());
                return;
            }
        };
        let suffix = match bit {
            CleanupBit::Straight => "clean",
            CleanupBit::VBit => "v_clean",
        };
        let name = format!("{}_{suffix}.ngc", self.base_name());
        if let Some(path) = self.dialogs.save_file(FileKind::Gcode, &name) {
            self.write_lines(&path, &lines);
        }
    }

    /// Asks where to save the outline as SVG, and writes it.
    pub fn save_svg(&mut self) {
        let Some(design) = self.design.clone() else {
            self.notify(StatusLevel::Warning, "There is no design to export.");
            return;
        };
        let lines = design.to_svg(&self.settings);
        let name = format!("{}.svg", self.base_name());
        if let Some(path) = self.dialogs.save_file(FileKind::Svg, &name) {
            self.write_lines(&path, &lines);
        }
    }

    /// Asks where to save the outline as DXF, and writes it.
    pub fn save_dxf(&mut self, close_loops: bool) {
        let Some(design) = self.design.clone() else {
            self.notify(StatusLevel::Warning, "There is no design to export.");
            return;
        };
        let lines = design.to_dxf(&self.settings, close_loops);
        let name = format!("{}.dxf", self.base_name());
        if let Some(path) = self.dialogs.save_file(FileKind::Dxf, &name) {
            self.write_lines(&path, &lines);
        }
    }

    /// Asks where to save the settings, and writes them.
    pub fn save_settings(&mut self) {
        let lines = GcodeProgram::settings_file(&self.settings);
        if let Some(path) = self.dialogs.save_file(FileKind::SettingsSave, "config.ngc") {
            self.write_lines(&path, &lines);
        }
    }

    /// The characters of the font page shown, as text.
    pub fn font_page_characters(&self) -> String {
        self.font_coverage.chars_in(self.settings.font_page_codes())
    }

    /// Copies the characters of the font page shown to the clipboard.
    pub(crate) fn copy_font_page_characters(&mut self) {
        let text = self.font_page_characters();
        if text.is_empty() {
            self.notify(
                StatusLevel::Warning,
                "The font has no characters on this page",
            );
            return;
        }
        let count = text.chars().count();
        self.ctx.copy_text(text);
        self.notify(StatusLevel::Info, format!("Copied {count} characters"));
    }

    /// Copies the G-code program to the clipboard.
    pub fn copy_gcode(&mut self) {
        if !self.can_write_gcode() {
            self.notify(
                StatusLevel::Warning,
                "Nothing to copy yet: toolpaths are not up to date.",
            );
            return;
        }
        match self.gcode() {
            Ok(lines) => {
                let mut text = lines.join("\n");
                text.push('\n');
                self.ctx.copy_text(text);
                self.notify(
                    StatusLevel::Info,
                    format!("Copied {} lines of G-code", lines.len()),
                );
            }
            Err(e) => self.notify(StatusLevel::Error, e),
        }
    }

    /// Opens any supported file: settings, a font, or an image, chosen by its extension.
    pub fn open_path(&mut self, path: &Path) {
        match extension_upper(path).as_str() {
            "NGC" | "NC" | "TAP" | "GCODE" | "TXT" | "CFG" => match std::fs::read_to_string(path) {
                Ok(text) => {
                    let problems = self.settings.apply_settings_text(&text);
                    self.keep_notice = true;
                    if let Some(first) = problems.first() {
                        self.notify(
                            StatusLevel::Warning,
                            format!("Ignored {} bad settings, first: {first}", problems.len()),
                        );
                    } else {
                        self.notify(
                            StatusLevel::Info,
                            format!("Loaded settings from {}", path.display()),
                        );
                    }
                }
                Err(e) => self.notify(
                    StatusLevel::Error,
                    format!("Unable to read {}: {e}", path.display()),
                ),
            },
            "CXF" | "TTF" | "OTF" | "TTC" | "OTC" => self.add_font_file(path, true),
            _ => {
                self.settings.input_type = InputType::Image;
                self.settings.image_file = path.to_path_buf();
            }
        }
    }

    /// Asks for a drawing or image to engrave.
    pub fn open_image_dialog(&mut self) {
        let start = self.settings.image_file.parent().map(Path::to_path_buf);
        if let Some(path) = self.dialogs.pick_file(FileKind::Image, start.as_deref()) {
            self.open_path(&path);
        }
    }

    /// Asks for a settings file to load.
    pub fn open_settings_dialog(&mut self) {
        if let Some(path) = self.dialogs.pick_file(FileKind::SettingsOpen, None) {
            self.open_path(&path);
        }
    }

    /// Asks for a font file to add to the library.
    pub(crate) fn add_font_dialog(&mut self, select: bool) {
        if let Some(path) = self.dialogs.pick_file(FileKind::Font, None) {
            self.add_font_file(&path, select);
        }
    }

    /// The characters the font of the latest layout has, by Unicode block.
    pub fn font_coverage(&self) -> &FontCoverage {
        &self.font_coverage
    }

    /// The installed fonts and the user's font library.
    pub fn fonts(&self) -> &Arc<FontCatalog> {
        &self.fonts
    }

    /// Adds a font file to the user's library, and with `select`, uses it for the text.
    pub(crate) fn add_font_file(&mut self, path: &Path, select: bool) {
        match self.fonts.add_user_font(path, None) {
            Ok(font) => {
                if select {
                    self.settings.input_type = InputType::Text;
                    self.settings.font = FontRef::User(font.name);
                }
                self.save_app_settings();
            }
            Err(e) => self.notify(StatusLevel::Error, e.to_string()),
        }
    }

    /// The name of the selected font, for the picker.
    pub(crate) fn font_label(&self) -> String {
        match &self.settings.font {
            FontRef::Default => "Default".to_string(),
            font => font.label().to_string(),
        }
    }

    fn handle_shortcuts(&mut self, ui: &Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.save_gcode();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&COPY_GCODE)) {
            self.copy_gcode();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&NEW)) {
            self.new_job();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&OPEN)) {
            self.open_image_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&FIT)) {
            self.preview.fit();
        }
        let (f5, esc, zoom_in, zoom_out, dropped) = ctx.input(|i| {
            (
                i.key_pressed(Key::F5),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::PageUp),
                i.key_pressed(Key::PageDown),
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect::<Vec<_>>(),
            )
        });
        if f5 && self.issues.is_empty() {
            self.reload_and_recalculate();
        }
        if esc {
            self.cancel();
        }
        if zoom_in {
            self.preview.zoom_by(1.25);
        }
        if zoom_out {
            self.preview.zoom_by(0.8);
        }
        for path in dropped {
            self.open_path(&path);
        }
    }

    /// Draws the whole application into `ui` and handles input for this frame.
    pub fn show(&mut self, ui: &mut Ui) {
        Look::ensure_installed(ui.ctx());
        let now = ui.input(|i| i.time);
        self.update_state(now);
        self.handle_shortcuts(ui);

        Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        Panel::left("inspector")
            .default_size(340.0)
            .size_range(280.0..=520.0)
            .show(ui, |ui| self.inspector(ui));
        let pane_fill = ui.visuals().panel_fill;
        CentralPanel::default()
            .frame(egui::Frame::NONE.fill(pane_fill))
            .show(ui, |ui| self.main_area(ui));

        self.about_window(ui);
        self.app_settings_window(ui);
        if self.busy || self.cleanup_busy {
            // The worker wakes the UI on progress, but keep cancellation responsive too.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new()
            .style(Look::menu_style)
            .config(egui::containers::menu::MenuConfig::new().style(Look::menu_style))
            .ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .menu_item(
                            true,
                            icons::NEW,
                            "New",
                            Some(ui.ctx().format_shortcut(&NEW)),
                        )
                        .clicked()
                    {
                        self.new_job();
                        ui.close();
                    }
                    if ui
                        .menu_item(
                            true,
                            icons::OPEN,
                            "Open image or DXF…",
                            Some(ui.ctx().format_shortcut(&OPEN)),
                        )
                        .clicked()
                    {
                        self.open_image_dialog();
                        ui.close();
                    }
                    if ui
                        .menu_item(true, icons::OPEN, "Open settings…", None)
                        .clicked()
                    {
                        self.open_settings_dialog();
                        ui.close();
                    }
                    if ui
                        .menu_item(true, icons::SETTINGS, "Save settings…", None)
                        .clicked()
                    {
                        self.save_settings();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .menu_item(
                            self.can_write_gcode(),
                            icons::SAVE,
                            "Save G-code…",
                            Some(ui.ctx().format_shortcut(&SAVE)),
                        )
                        .clicked()
                    {
                        self.save_gcode();
                        ui.close();
                    }
                    if ui
                        .menu_item(
                            self.has_cleanup(CleanupBit::Straight),
                            icons::SAVE,
                            "Save straight cleanup G-code…",
                            None,
                        )
                        .clicked()
                    {
                        self.save_cleanup(CleanupBit::Straight);
                        ui.close();
                    }
                    if ui
                        .menu_item(
                            self.has_cleanup(CleanupBit::VBit),
                            icons::SAVE,
                            "Save V-bit cleanup G-code…",
                            None,
                        )
                        .clicked()
                    {
                        self.save_cleanup(CleanupBit::VBit);
                        ui.close();
                    }
                    if ui
                        .menu_item(
                            self.can_write_gcode(),
                            icons::COPY,
                            "Copy G-code",
                            Some(ui.ctx().format_shortcut(&COPY_GCODE)),
                        )
                        .clicked()
                    {
                        self.copy_gcode();
                        ui.close();
                    }
                    ui.separator();
                    let export = ui.menu_button((Look::icon(icons::EXPORT), "Export"), |ui| {
                        if ui.menu_item(true, icons::EXPORT, "SVG…", None).clicked() {
                            self.save_svg();
                            ui.close();
                        }
                        if ui.menu_item(true, icons::EXPORT, "DXF…", None).clicked() {
                            self.save_dxf(false);
                            ui.close();
                        }
                        if ui
                            .menu_item(true, icons::EXPORT, "DXF with closed loops…", None)
                            .clicked()
                        {
                            self.save_dxf(true);
                            ui.close();
                        }
                    });
                    export.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, "Export")
                    });
                    ui.separator();
                    if ui.menu_item(true, icons::QUIT, "Quit", None).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .menu_item(true, icons::SETTINGS, "Application settings…", None)
                        .clicked()
                    {
                        self.show_app_settings(crate::preferences::SettingsPage::default());
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .menu_item(
                            true,
                            icons::FIT,
                            "Fit design",
                            Some(ui.ctx().format_shortcut(&FIT)),
                        )
                        .clicked()
                    {
                        self.preview.fit();
                        ui.close();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui
                        .menu_item(true, icons::INFO, "About Rengrave", None)
                        .clicked()
                    {
                        self.show_about = true;
                        ui.close();
                    }
                    ui.hyperlink_to(
                        "F-Engrave website",
                        "https://www.scorchworks.com/Fengrave/fengrave.html",
                    );
                });
            });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let status = self.status();
            let color = match status.level {
                StatusLevel::Info => ui.visuals().text_color(),
                StatusLevel::Busy => ui.visuals().warn_fg_color,
                StatusLevel::Warning => ui.visuals().warn_fg_color,
                StatusLevel::Error => ui.visuals().error_fg_color,
            };
            if self.busy {
                ui.spinner();
                if ui
                    .small_button("Stop")
                    .on_hover_text("Stop the calculation (Esc)")
                    .clicked()
                {
                    self.cancel();
                }
            }
            match status.level {
                StatusLevel::Warning => {
                    ui.label(Look::icon(icons::WARNING).color(color));
                }
                StatusLevel::Error => {
                    ui.label(Look::icon(icons::ERROR).color(color));
                }
                StatusLevel::Info | StatusLevel::Busy => {}
            }
            ui.label(RichText::new(status.text).color(color));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut units = self.settings.units;
                ui.option_group(|ui| {
                    for (choice, name) in [(Units::Millimeters, "mm"), (Units::Inches, "in")] {
                        ui.selectable_value(&mut units, choice, name)
                            .on_hover_text("Units for all lengths. Values convert automatically.");
                    }
                });
                self.settings.set_units(units);
                ui.separator();
                if let Some((x, y)) = self.preview.hover_position
                    && self.active_view != ActiveView::Gcode
                {
                    let d = self.settings.units.coord_decimals();
                    ui.monospace(format!("X {x:.d$}  Y {y:.d$}"));
                }
            });
        });
    }

    fn main_area(&mut self, ui: &mut Ui) {
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 0,
            })
            .show(ui, |ui| {
                let mut view = self.active_view;
                ui.tab_bar_with(
                    &mut view,
                    &[
                        (
                            ActiveView::Preview,
                            "Preview",
                            "The design and its toolpath",
                        ),
                        (
                            ActiveView::Simulator,
                            "Simulate",
                            "Play the G-code program and see what it cuts",
                        ),
                        (
                            ActiveView::Gcode,
                            "G-code",
                            "The text of the G-code program",
                        ),
                    ],
                    |ui| self.carve_controls(ui),
                );
                self.active_view = view;
            });
        // The tools belong to the pane, so that the tabs connect to it directly. The G-code view
        // has only a Copy button, which floats over the text instead of taking a row.
        if self.active_view != ActiveView::Gcode {
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| match self.active_view {
                        ActiveView::Simulator => {
                            self.fit_button(ui);
                            self.simulator.toolbar(ui);
                        }
                        _ => self.preview_toolbar(ui),
                    });
                });
        }
        let view_corner = ui.available_rect_before_wrap().right_top();
        match self.active_view {
            ActiveView::Preview => {
                let scene = Scene {
                    design: self.design.clone(),
                    carve: self.carve.clone(),
                    clean_straight: self.clean_straight.clone(),
                    clean_v_bit: self.clean_v_bit.clone(),
                };
                self.preview
                    .show(ui, &scene, &self.settings, self.scene_revision);
            }
            ActiveView::Gcode => {
                self.gcode_view(ui);
                self.copy_overlay(ui, view_corner);
            }
            ActiveView::Simulator => self.simulator_view(ui),
        }
    }

    /// A Copy button floating at the top right of the G-code text, clear of its scroll bar.
    fn copy_overlay(&mut self, ui: &mut Ui, corner: egui::Pos2) {
        egui::Area::new(egui::Id::unique("gcode_copy"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(corner + egui::vec2(-24.0, 8.0))
            .show(ui.ctx(), |ui| {
                if ui
                    .icon_button(self.can_write_gcode(), icons::COPY, "Copy")
                    .clicked()
                {
                    self.copy_gcode();
                }
            });
    }

    fn simulator_view(&mut self, ui: &mut Ui) {
        if self.settings_are_pending() || self.busy {
            ui.centered_and_justified(|ui| ui.label("Waiting for toolpaths…"));
            return;
        }
        if self.carve_is_stale() {
            ui.centered_and_justified(|ui| ui.label("V-carve paths are not calculated."));
            return;
        }
        let Some(design) = self.design.clone() else {
            return;
        };
        match self.gcode() {
            Ok(lines) => {
                self.simulator.load(&lines, &design, &self.settings);
                self.simulator
                    .show(ui, &mut self.preview, &design, &lines, &self.settings);
            }
            Err(e) => {
                ui.centered_and_justified(|ui| ui.colored_label(ui.visuals().error_fg_color, e));
            }
        }
    }

    fn fit_button(&mut self, ui: &mut Ui) {
        if ui
            .icon_button(!self.preview.is_fitted(), icons::FIT, "Fit")
            .on_hover_text("Fit the design in the window (double-click the preview)")
            .clicked()
        {
            self.preview.fit();
        }
    }

    fn preview_toolbar(&mut self, ui: &mut Ui) {
        self.fit_button(ui);
        ui.separator();
        ui.options_menu("Layers", "What the preview shows", |ui| {
            if self.settings.cut_type == CutType::VCarve {
                ui.checkbox(&mut self.settings.show_v_area, "Carved area")
                    .on_hover_text("Show the area cut by the V-bit");
                ui.checkbox(&mut self.settings.show_v_path, "Tool path")
                    .on_hover_text("Show the path of the tool");
            }
            ui.checkbox(&mut self.preview.layers.grid, "Grid")
                .on_hover_text("Show grid lines and axis labels behind the design");
            ui.checkbox(&mut self.settings.show_axis, "Origin axes")
                .on_hover_text("Show arrows along the X and Y axes at the work origin");
            ui.checkbox(&mut self.settings.show_box, "Bounding box")
                .on_hover_text("Shade the smallest rectangle that holds the design");
            ui.checkbox(&mut self.settings.show_thick, "Line thickness")
                .on_hover_text(
                    "Draw engraved outlines at the width of the bit instead of a hairline",
                );
            ui.checkbox(&mut self.preview.layers.clean_straight, "Straight cleanup")
                .on_hover_text("Show the paths of the straight-bit cleanup, once calculated");
            ui.checkbox(&mut self.preview.layers.clean_v_bit, "V-bit cleanup")
                .on_hover_text("Show the paths of the V-bit cleanup, once calculated");
        });
    }

    fn gcode_view(&mut self, ui: &mut Ui) {
        if self.settings_are_pending() || self.busy {
            ui.centered_and_justified(|ui| ui.label("Waiting for toolpaths…"));
            return;
        }
        if self.carve_is_stale() {
            ui.centered_and_justified(|ui| ui.label("V-carve paths are not calculated."));
            return;
        }
        match self.gcode() {
            Ok(lines) => {
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let dark = ui.visuals().dark_mode;
                let plain = ui.visuals().text_color();
                let current = self
                    .gcode_galley
                    .as_ref()
                    .is_some_and(|c| Arc::ptr_eq(&c.lines, &lines) && c.dark == dark);
                if !current {
                    let mut text = String::new();
                    let mut job = egui::text::LayoutJob::default();
                    for line in lines.iter() {
                        let highlighted = Token::highlight(line, &font, dark, plain);
                        for section in &highlighted.sections {
                            job.append(
                                &highlighted.text
                                    [section.byte_range.start.0..section.byte_range.end.0],
                                0.0,
                                section.format.clone(),
                            );
                        }
                        job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), plain));
                        text.push_str(line);
                        text.push('\n');
                    }
                    job.wrap.max_width = f32::INFINITY;
                    self.gcode_galley = Some(GcodeGalley {
                        lines: lines.clone(),
                        dark,
                        text: text.into(),
                        galley: ui.painter().layout_job(job),
                    });
                }
                let Some(GcodeGalley { text, galley, .. }) = &self.gcode_galley else {
                    return;
                };
                let (text, galley) = (text.clone(), galley.clone());
                let id = egui::Id::unique("gcode_text");
                // egui moves the cursor on a right-click, so keep the selection through it.
                let kept = ui
                    .input(|i| i.pointer.secondary_pressed())
                    .then(|| egui::TextEdit::load_state(ui.ctx(), id))
                    .flatten()
                    .and_then(|state| state.cursor.char_range());
                if ui.input(|i| i.pointer.secondary_pressed()) {
                    // The menu click clears the cursor, so remember what the menu will copy.
                    self.gcode_selection = kept
                        .filter(|range| !range.is_empty())
                        .map(|range| range.slice_str(&text).to_owned())
                        .unwrap_or_default();
                }
                // Line numbers are painted in a gutter beside the text. They are not part of the
                // text, so they cannot be selected or copied.
                let digits = lines.len().max(1).to_string().len().max(3);
                let gutter_width = ui
                    .painter()
                    .layout_no_wrap("0".repeat(digits), font.clone(), plain)
                    .size()
                    .x
                    + 16.0;
                let full = ui.available_rect_before_wrap();
                let (gutter, rest) = full.split_left_right_at_x(full.left() + gutter_width);
                let gutter_painter = ui.painter().with_clip_rect(gutter);
                let number_color = ui.visuals().weak_text_color();
                gutter_painter.rect_filled(gutter, 0.0, ui.visuals().faint_bg_color);
                gutter_painter.vline(
                    gutter.right(),
                    gutter.y_range(),
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
                let line_count = lines.len();
                ui.scope_builder(egui::UiBuilder::new().max_rect(rest), |ui| {
                    let width = ui.available_width().max(galley.size().x + 16.0);
                    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                        // A read-only buffer: it can be selected and copied, but not edited.
                        let mut buffer: &str = &text;
                        let mut layouter =
                            |_: &Ui, _: &dyn egui::TextBuffer, _: f32| galley.clone();
                        let output = egui::TextEdit::multiline(&mut buffer)
                            .id(id)
                            .layouter(&mut layouter)
                            .desired_width(width)
                            .frame(egui::Frame::NONE)
                            .show(ui);
                        if let Some(range) = kept
                            && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id)
                        {
                            state.cursor.set_char_range(Some(range));
                            state.store(ui.ctx(), id);
                        }
                        // Each line is one row of the galley, so the numbers follow the text.
                        for (index, row) in output.galley.rows.iter().take(line_count).enumerate() {
                            let y = output.galley_pos.y + row.pos.y;
                            if y + row.height() < gutter.top() || y > gutter.bottom() {
                                continue;
                            }
                            gutter_painter.text(
                                egui::pos2(gutter.right() - 8.0, y),
                                egui::Align2::RIGHT_TOP,
                                index + 1,
                                font.clone(),
                                number_color,
                            );
                        }
                        output
                            .response
                            .context_menu(|ui| self.gcode_context_menu(ui));
                    });
                });
            }
            Err(e) => {
                ui.centered_and_justified(|ui| ui.colored_label(ui.visuals().error_fg_color, e));
            }
        }
    }

    fn gcode_context_menu(&mut self, ui: &mut Ui) {
        if ui
            .menu_item(!self.gcode_selection.is_empty(), icons::COPY, "Copy", None)
            .clicked()
        {
            ui.ctx().copy_text(self.gcode_selection.clone());
            ui.close();
        }
        if ui
            .menu_item(self.can_write_gcode(), icons::COPY, "Copy all", None)
            .clicked()
        {
            self.copy_gcode();
            ui.close();
        }
    }

    fn about_window(&mut self, ui: &mut Ui) {
        let mut open = self.show_about;
        egui::Window::new("About Rengrave")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(format!("Rengrave {}", env!("CARGO_PKG_VERSION")));
                ui.label("A Rust port of F-Engrave 1.78 by Scorch (scorchworks.com).");
                ui.label("Licensed under the GNU GPL, version 3 or later.");
            });
        self.show_about = open;
    }
}

impl RengraveApp {
    /// Returns a copy with fields that do not affect toolpaths cleared, for detecting real changes.
    fn toolpath_inputs(settings: &Settings) -> Settings {
        let d = Settings::default();
        Settings {
            show_axis: d.show_axis,
            show_box: d.show_box,
            show_thick: d.show_thick,
            show_v_path: d.show_v_path,
            show_v_area: d.show_v_area,
            feed: d.feed,
            plunge: d.plunge,
            z_safe: d.z_safe,
            z_cut: d.z_cut,
            arc_fit: d.arc_fit,
            g_pre: d.g_pre,
            g_post: d.g_post,
            disable_variables: d.disable_variables,
            no_comments: d.no_comments,
            v_rough_stock: d.v_rough_stock,
            v_max_cut: d.v_max_cut,
            clean_dia: d.clean_dia,
            clean_step: d.clean_step,
            clean_v: d.clean_v,
            clean_straight: d.clean_straight,
            clean_v_bit: d.clean_v_bit,
            ..settings.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_gcode_only_fields_do_not_affect_toolpaths() {
        let base = Settings::default();
        let cosmetic = Settings {
            show_axis: false,
            feed: 99.0,
            g_post: "M30".into(),
            ..base.clone()
        };
        assert!(RengraveApp::toolpath_inputs(&base) == RengraveApp::toolpath_inputs(&cosmetic));
        let real = Settings {
            y_scale: 3.0,
            ..base.clone()
        };
        assert!(RengraveApp::toolpath_inputs(&base) != RengraveApp::toolpath_inputs(&real));
    }
}

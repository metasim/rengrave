// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Smoke and interaction tests of the whole application under `egui_kittest`.
//!
//! The harness steps the real `RengraveApp`, finding widgets through the accessibility tree the
//! same way a screen reader would, and also verifies that every input widget has a name.

use egui::Key;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, HarnessBuilder};
use rengrave_core::{
    ArcFit, BitShape, CleanupBit, CutType, InputType, Justify, Origin, Settings, Units,
};
use rengrave_gui::{ActiveView, FileDialogs, FileKind, RengraveApp, StatusLevel, Tab};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Dialog answers shared between a test and the application it runs.
#[derive(Clone, Default)]
struct Answers {
    inner: Arc<Mutex<AnswerState>>,
}

#[derive(Default)]
struct AnswerState {
    open: Option<PathBuf>,
    save: Option<PathBuf>,
    last_suggestion: Option<String>,
}

impl Answers {
    fn answer_save(&self, path: &Path) {
        self.inner.lock().unwrap().save = Some(path.to_path_buf());
    }

    fn answer_open(&self, path: &Path) {
        self.inner.lock().unwrap().open = Some(path.to_path_buf());
    }

    fn last_suggestion(&self) -> Option<String> {
        self.inner.lock().unwrap().last_suggestion.clone()
    }
}

impl FileDialogs for Answers {
    fn pick_file(&mut self, _kind: FileKind, _start: Option<&Path>) -> Option<PathBuf> {
        self.inner.lock().unwrap().open.take()
    }

    fn pick_folder(&mut self, _start: Option<&Path>) -> Option<PathBuf> {
        self.inner.lock().unwrap().open.take()
    }

    fn save_file(&mut self, _kind: FileKind, suggested: &str) -> Option<PathBuf> {
        let mut state = self.inner.lock().unwrap();
        state.last_suggestion = Some(suggested.to_string());
        state.save.take()
    }
}

fn builder() -> HarnessBuilder<RengraveApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 900.0))
}

fn harness_with(settings: impl FnOnce(&mut Settings)) -> (Harness<'static, RengraveApp>, Answers) {
    let answers = Answers::default();
    let dialogs = Box::new(answers.clone());
    let mut slot = Some(RengraveApp::new(egui::Context::default(), dialogs));
    if let Some(app) = slot.as_mut() {
        // Keep calculations small so that tests stay quick.
        app.settings.text = "Ab".into();
        app.settings.v_step_len = 0.03;
        settings(&mut app.settings);
    }
    let app = slot.take().expect("just created");
    let mut harness = builder().build_ui_state(|ui, app: &mut RengraveApp| app.show(ui), app);
    settle(&mut harness);
    (harness, answers)
}

fn harness() -> (Harness<'static, RengraveApp>, Answers) {
    harness_with(|_| {})
}

/// Steps until the background calculation finishes, as the event loop would.
fn settle(harness: &mut Harness<'_, RengraveApp>) {
    let start = Instant::now();
    loop {
        harness.step();
        if harness.state().is_idle() {
            // One more frame so anything that just finished is drawn.
            harness.step();
            if harness.state().is_idle() {
                return;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "calculation did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn click(harness: &mut Harness<'_, RengraveApp>, label: &str) {
    // The settings panel scrolls, so make sure the widget is on screen first.
    harness.get_by_label(label).scroll_to_me();
    // Scrolling is animated, so wait for the widget to stop moving.
    let mut rect = harness.get_by_label(label).rect();
    let mut still = 0;
    for _ in 0..60 {
        harness.step();
        let now = harness.get_by_label(label).rect();
        still = if now == rect { still + 1 } else { 0 };
        if still == 3 {
            break;
        }
        rect = now;
    }
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rengrave_gui_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rengrave_core/tests/parity")
        .join(name)
}

// ---- Smoke tests ------------------------------------------------------------------------------

#[test]
fn starts_and_calculates_default_job() {
    let (harness, _) = harness();
    let app = harness.state();
    assert!(app.design().is_some());
    assert!(
        app.carve().is_some(),
        "V-carve is the default and runs live"
    );
    assert!(app.can_write_gcode());
    assert_eq!(app.status().level, StatusLevel::Info);
}

#[test]
fn every_tab_renders_in_every_mode() {
    for input in [InputType::Text, InputType::Image] {
        for cut in [CutType::VCarve, CutType::Engrave] {
            for bit in [BitShape::VBit, BitShape::Ball, BitShape::Flat] {
                for units in [Units::Inches, Units::Millimeters] {
                    let (mut harness, _) = harness_with(|s| {
                        s.input_type = input;
                        s.image_file = fixture("drawing.dxf");
                        s.cut_type = cut;
                        s.bit_shape = bit;
                        s.set_units(units);
                    });
                    for tab in [Tab::Design, Tab::Toolpath, Tab::Output] {
                        harness.state_mut().tab = tab;
                        // `run_steps` also runs the accessibility check on every input widget.
                        harness.run_steps(2);
                    }
                    harness.state_mut().active_view = ActiveView::Gcode;
                    harness.run_steps(2);
                    harness.state_mut().active_view = ActiveView::Simulator;
                    harness.run_steps(2);
                    assert!(harness.state().simulator.simulation().is_some());
                    harness.check_accessibility();
                }
            }
        }
    }
}

#[test]
fn survives_narrow_and_tiny_windows() {
    let (mut harness, _) = harness();
    for size in [(760.0, 480.0), (320.0, 200.0), (2000.0, 1200.0)] {
        harness.set_size(egui::vec2(size.0, size.1));
        harness.run_steps(3);
    }
}

#[test]
fn tabs_are_exposed_as_tabs_and_report_the_selected_one() {
    use egui::accesskit::Role;
    let (mut harness, _) = harness();
    for name in [
        "Design", "Toolpath", "Output", "Preview", "Simulate", "G-code",
    ] {
        assert!(
            harness.query_by_role_and_label(Role::Tab, name).is_some(),
            "{name}"
        );
    }
    let is_selected = |h: &Harness<'_, RengraveApp>, name: &str| {
        h.get_by_role_and_label(Role::Tab, name)
            .accesskit_node()
            .toggled()
            == Some(egui::accesskit::Toggled::True)
    };
    assert!(is_selected(&harness, "Design"));
    assert!(!is_selected(&harness, "Output"));
    assert!(is_selected(&harness, "Preview"));

    click(&mut harness, "Output");
    click(&mut harness, "G-code");
    assert!(!is_selected(&harness, "Design"));
    assert!(is_selected(&harness, "Output"));
    assert!(is_selected(&harness, "G-code"));
    assert_eq!(harness.state().tab, Tab::Output);
    assert_eq!(harness.state().active_view, ActiveView::Gcode);
}

#[test]
fn tools_sit_below_the_tabs_inside_the_pane() {
    let (harness, _) = harness();
    let tabs = harness.get_by_label("Preview").rect();
    let tool = harness.get_by_label("Grid").rect();
    let canvas = harness.get_by_label("Design preview").rect();
    assert!(tool.top() >= tabs.bottom(), "{tool:?} under {tabs:?}");
    assert!(canvas.top() >= tool.bottom(), "{canvas:?} under {tool:?}");
}

#[test]
fn panel_layout_is_stable_across_frames() {
    let (mut harness, _) = harness();
    let first = harness.get_by_label("Design preview").rect();
    harness.run_steps(30);
    assert_eq!(harness.get_by_label("Design preview").rect(), first);
    for tab in ["Toolpath", "Output", "Design"] {
        click(&mut harness, tab);
        let settled = harness.get_by_label("Design preview").rect();
        harness.run_steps(30);
        assert_eq!(
            harness.get_by_label("Design preview").rect(),
            settled,
            "{tab}"
        );
    }
}

#[test]
fn all_menus_open() {
    let (mut harness, _) = harness();
    for (menu, item) in [
        ("File", "Export SVG…"),
        ("View", "Fit design"),
        ("Help", "About rengrave"),
    ] {
        click(&mut harness, menu);
        assert!(
            harness.query_by_label_contains(item).is_some(),
            "{item} in {menu}"
        );
        harness.key_press(Key::Escape);
        harness.run_steps(2);
    }
}

#[test]
fn about_window_opens() {
    let (mut harness, _) = harness();
    click(&mut harness, "Help");
    click(&mut harness, "About rengrave");
    assert!(harness.query_by_label_contains("GNU GPL").is_some());
}

// ---- Design tab -------------------------------------------------------------------------------

#[test]
fn typing_text_updates_the_design() {
    let (mut harness, _) = harness();
    let before = harness.state().design().unwrap().bounds.width();
    let text =
        harness.get_by_role_and_label(egui::accesskit::Role::MultilineTextInput, "Text to engrave");
    text.focus();
    harness.run_steps(1);
    harness
        .get_by_role_and_label(egui::accesskit::Role::MultilineTextInput, "Text to engrave")
        .type_text("cdefgh");
    harness.run_steps(1);
    settle(&mut harness);
    let app = harness.state();
    assert!(
        app.settings.text.ends_with("cdefgh"),
        "{}",
        app.settings.text
    );
    assert!(app.design().unwrap().bounds.width() > before);
}

#[test]
fn changing_height_rescales_the_design() {
    let (mut harness, _) = harness();
    harness.get_by_label("Height").set_value_accesskit(0.5);
    harness.run_steps(2);
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.y_scale, 0.5);
    assert!((app.design().unwrap().bounds.height() - 0.5).abs() < 1e-6);
}

#[test]
fn origin_grid_moves_the_origin() {
    let (mut harness, _) = harness();
    click(&mut harness, "Middle center");
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.origin, Origin::MidCenter);
    let b = app.design().unwrap().bounds;
    assert!((b.min_x + b.max_x).abs() < 1e-6 && (b.min_y + b.max_y).abs() < 1e-6);
}

#[test]
fn justify_control_applies() {
    let (mut harness, _) = harness_with(|s| s.text = "Ab\nA".into());
    click(&mut harness, "Right");
    settle(&mut harness);
    assert_eq!(harness.state().settings.justify, Justify::Right);
}

#[test]
fn mirror_checkbox_toggles() {
    let (mut harness, _) = harness();
    click(&mut harness, "Mirror");
    settle(&mut harness);
    let app = harness.state();
    assert!(app.settings.mirror);
    assert!(app.design().unwrap().bounds.max_x <= 1e-9);
}

#[test]
fn box_option_reveals_gap_and_surrounds_design() {
    let (mut harness, _) = harness();
    assert!(harness.query_by_label("Gap").is_none());
    click(&mut harness, "Add box");
    settle(&mut harness);
    assert!(harness.query_by_label("Gap").is_some());
    assert!(harness.state().settings.add_box);
}

#[test]
fn selecting_image_input_without_file_reports_an_error() {
    let (mut harness, _) = harness();
    click(&mut harness, "Image");
    settle(&mut harness);
    let status = harness.state().status();
    assert_eq!(status.level, StatusLevel::Error);
    assert!(status.text.contains("no image file"), "{}", status.text);
    assert!(!harness.state().can_write_gcode());
}

#[test]
fn opening_a_dxf_through_the_dialog_loads_it() {
    let (mut harness, answers) = harness();
    click(&mut harness, "Image");
    answers.answer_open(&fixture("drawing.dxf"));
    click(&mut harness, "Open…");
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.input_type, InputType::Image);
    assert!(!app.design().unwrap().segments.is_empty());
    assert!(harness.query_by_label("drawing.dxf").is_some());
}

#[test]
fn raster_image_can_be_traced() {
    let dir = temp_dir("raster");
    let path = dir.join("square.pbm");
    write_pbm(&path);
    let (mut harness, _) = harness_with(|s| {
        s.input_type = InputType::Image;
        s.image_file = path.clone();
        s.cut_type = CutType::Engrave;
    });
    settle(&mut harness);
    assert!(!harness.state().design().unwrap().segments.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Writes a 16-by-16 plain PBM with a black square in the middle.
fn write_pbm(path: &Path) {
    let mut text = String::from("P1\n16 16\n");
    for y in 0..16 {
        for x in 0..16 {
            let on = (4..12).contains(&x) && (4..12).contains(&y);
            text.push_str(if on { "1 " } else { "0 " });
        }
        text.push('\n');
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn font_can_be_chosen_from_the_list() {
    let (mut harness, _) =
        harness_with(|s| s.font_dir = fixture("tiny.cxf").parent().unwrap().into());
    click(&mut harness, "Font");
    click(&mut harness, "Hack (built in)");
    settle(&mut harness);
    assert_eq!(harness.state().settings.font_file, "Hack");
    harness.get_by_label("Font").click();
    harness.run_steps(2);
    click(&mut harness, "tiny.cxf");
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.font_file, "tiny.cxf");
    assert_eq!(
        app.status().level,
        StatusLevel::Warning,
        "b is not in the fixture font"
    );
}

#[test]
fn folder_button_rescans_fonts() {
    let (mut harness, answers) = harness();
    answers.answer_open(fixture("tiny.cxf").parent().unwrap());
    click(&mut harness, "Folder…");
    click(&mut harness, "Font");
    assert!(harness.query_by_label("tiny.cxf").is_some());
}

#[test]
fn circle_section_applies_radius() {
    let (mut harness, _) = harness();
    click(&mut harness, "Text on a circle");
    harness.get_by_label("Radius").set_value_accesskit(3.0);
    harness.run_steps(2);
    settle(&mut harness);
    assert_eq!(harness.state().settings.text_radius, 3.0);
    click(&mut harness, "Bottom");
    settle(&mut harness);
    assert!(!harness.state().settings.upper);
}

// ---- Validation -------------------------------------------------------------------------------

#[test]
fn invalid_value_blocks_output_and_explains_why() {
    let (mut harness, _) = harness();
    harness.get_by_label("Height").set_value_accesskit(0.0);
    harness.run_steps(3);
    let app = harness.state();
    assert_eq!(app.status().level, StatusLevel::Error);
    assert!(app.status().text.contains("Height"));
    assert!(!app.can_write_gcode());
    assert!(
        harness
            .query_by_label("Height should be greater than 0")
            .is_some()
    );

    // Fixing the value recovers.
    harness.get_by_label("Height").set_value_accesskit(0.75);
    harness.run_steps(2);
    settle(&mut harness);
    assert!(harness.state().can_write_gcode());
}

#[test]
fn missing_font_reports_an_error_then_recovers() {
    let (mut harness, _) = harness_with(|s| s.font_file = "missing.ttf".into());
    assert_eq!(harness.state().status().level, StatusLevel::Error);
    harness.state_mut().settings.font_file = "Hack".into();
    harness.run_steps(1);
    settle(&mut harness);
    assert_eq!(harness.state().status().level, StatusLevel::Info);
}

// ---- Toolpath tab -----------------------------------------------------------------------------

#[test]
fn switching_to_engrave_hides_bit_settings() {
    let (mut harness, _) = harness();
    click(&mut harness, "Toolpath");
    assert!(harness.query_by_label("Diameter").is_some());
    click(&mut harness, "Engrave");
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.cut_type, CutType::Engrave);
    assert!(app.carve().is_none());
    assert!(harness.query_by_label("Diameter").is_none());
    assert!(harness.query_by_label("Engrave depth").is_some());
}

#[test]
fn bit_shape_controls_follow_the_selection() {
    let (mut harness, _) = harness();
    click(&mut harness, "Toolpath");
    assert!(harness.query_by_label("Angle").is_some());
    click(&mut harness, "Ball nose");
    settle(&mut harness);
    assert_eq!(harness.state().settings.bit_shape, BitShape::Ball);
    assert!(harness.query_by_label("Angle").is_none());
}

#[test]
fn deepest_cut_is_shown_from_bit_geometry() {
    let (mut harness, _) = harness();
    click(&mut harness, "Toolpath");
    assert!(harness.query_by_label_contains("-0.4330").is_some());
}

#[test]
fn finish_stock_enables_pass_depth() {
    let (mut harness, _) = harness();
    click(&mut harness, "Toolpath");
    click(&mut harness, "Multi-pass");
    let depth = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Depth per pass");
    assert!(depth.accesskit_node().is_disabled());
    harness
        .get_by_label("Finish stock")
        .set_value_accesskit(0.01);
    harness.run_steps(2);
    let depth = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Depth per pass");
    assert!(!depth.accesskit_node().is_disabled());
}

#[test]
fn turning_off_live_carve_waits_for_calculate() {
    let (mut harness, _) = harness();
    click(&mut harness, "Live V-carve");
    harness.get_by_label("Height").set_value_accesskit(0.5);
    harness.run_steps(2);
    settle(&mut harness);
    let app = harness.state();
    assert!(app.design().is_some());
    assert!(app.carve().is_none());
    assert!(!app.can_write_gcode());
    assert!(app.status().text.contains("F5"));

    click(&mut harness, "Calculate");
    settle(&mut harness);
    assert!(harness.state().carve().is_some());
    assert!(harness.state().can_write_gcode());
}

#[test]
fn f5_recalculates_and_escape_cancels() {
    let (mut harness, _) = harness();
    click(&mut harness, "Live V-carve");
    harness.get_by_label("Height").set_value_accesskit(0.5);
    harness.run_steps(2);
    settle(&mut harness);
    assert!(harness.state().carve().is_none());

    harness.key_press(Key::F5);
    harness.run_steps(1);
    settle(&mut harness);
    assert!(harness.state().carve().is_some());
}

#[test]
fn cancel_stops_a_slow_calculation_and_leaves_outline() {
    let (mut harness, _) = harness_with(|s| {
        s.text = "The quick brown fox".into();
    });
    {
        let app = harness.state_mut();
        app.settings.v_step_len = 0.001;
        app.settings.y_scale = 3.0;
        app.recalculate(true);
    }
    harness.step();
    harness.state_mut().cancel();
    settle(&mut harness);
    let app = harness.state();
    assert!(app.carve().is_none(), "canceled before finishing");
    assert!(!app.can_write_gcode());
    assert_ne!(app.status().level, StatusLevel::Error);
}

#[test]
fn cleanup_paths_calculate_and_save() {
    let dir = temp_dir("cleanup");
    let path = dir.join("clean.ngc");
    let (mut harness, answers) = harness_with(|s| {
        s.text = "O".into();
        s.y_scale = 3.0;
        s.v_bit_dia = 0.05;
        s.clean_dia = 0.01;
        s.v_step_len = 0.05;
    });
    click(&mut harness, "Toolpath");
    click(&mut harness, "Cleanup");
    click(&mut harness, "Calculate cleanup");
    settle(&mut harness);
    assert!(
        harness
            .state()
            .cleanup_paths(CleanupBit::Straight)
            .is_some()
    );
    assert!(
        !harness
            .state()
            .cleanup_paths(CleanupBit::Straight)
            .unwrap()
            .is_empty()
    );

    answers.answer_save(&path);
    click(&mut harness, "Save straight…");
    assert!(std::fs::read_to_string(&path).unwrap().contains("G1 X"));
    assert!(answers.last_suggestion().unwrap().ends_with("_clean.ngc"));

    // A changed cleanup setting makes the old paths stale.
    harness
        .get_by_label("Straight bit")
        .set_value_accesskit(0.02);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .cleanup_paths(CleanupBit::Straight)
            .is_none()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn feed_change_does_not_discard_toolpaths() {
    let (mut harness, _) = harness();
    click(&mut harness, "Toolpath");
    harness.get_by_label("Feed rate").set_value_accesskit(12.0);
    harness.run_steps(3);
    let app = harness.state_mut();
    assert!(app.carve().is_some());
    assert!(app.is_idle());
    let gcode = app.gcode().unwrap();
    assert!(gcode.contains(&"F12.00".to_string()));
}

// ---- Output and files -------------------------------------------------------------------------

#[test]
fn save_button_writes_program_matching_core() {
    let dir = temp_dir("save");
    let path = dir.join("out.ngc");
    let (mut harness, answers) = harness();
    answers.answer_save(&path);
    click(&mut harness, "Save G-code…");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.starts_with("G90\n"));
    assert!(harness.state().status().text.contains("Saved"));
    assert_eq!(answers.last_suggestion().unwrap(), "Ab.ngc");

    let expected = {
        let job = rengrave_core::Job::load(harness.state().settings.clone()).unwrap();
        job.generate_gcode(&rengrave_core::Progress::none())
            .unwrap()
            .join("\n")
            + "\n"
    };
    assert_eq!(saved, expected);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn canceling_the_save_dialog_writes_nothing() {
    let (mut harness, _) = harness();
    click(&mut harness, "Save G-code…");
    assert!(!harness.state().status().text.contains("Saved"));
}

#[test]
fn ctrl_s_saves() {
    let dir = temp_dir("ctrl_s");
    let path = dir.join("out.ngc");
    let (mut harness, answers) = harness();
    answers.answer_save(&path);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::S);
    harness.run_steps(2);
    assert!(path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn write_failure_is_reported() {
    let (mut harness, answers) = harness();
    answers.answer_save(Path::new("/nonexistent-dir/out.ngc"));
    click(&mut harness, "Save G-code…");
    let status = harness.state().status();
    assert_eq!(status.level, StatusLevel::Error);
    assert!(status.text.contains("Unable to write"));
}

#[test]
fn exports_write_drawings() {
    let dir = temp_dir("export");
    let (mut harness, answers) = harness();
    click(&mut harness, "Output");
    for (button, name, needle) in [
        ("SVG…", "o.svg", "</svg>"),
        ("DXF…", "o.dxf", "AcDbLine"),
        ("DXF, closed…", "c.dxf", "AcDbLine"),
        ("Settings…", "c.ngc", "fengrave_set"),
    ] {
        let path = dir.join(name);
        answers.answer_save(&path);
        click(&mut harness, button);
        assert!(
            std::fs::read_to_string(&path).unwrap().contains(needle),
            "{button}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn settings_round_trip_through_files() {
    let dir = temp_dir("settings");
    let path = dir.join("job.ngc");
    let (mut harness, answers) = harness_with(|s| {
        s.text = "Zz".into();
        s.y_scale = 1.5;
        s.arc_fit = ArcFit::Radius;
    });
    answers.answer_save(&path);
    harness.state_mut().save_settings();

    let (mut other, other_answers) = harness_with(|_| {});
    other_answers.answer_open(&path);
    click(&mut other, "File");
    click(&mut other, "Open settings…");
    settle(&mut other);
    let app = other.state();
    assert_eq!(app.settings.text, "Zz");
    assert_eq!(app.settings.y_scale, 1.5);
    assert_eq!(app.settings.arc_fit, ArcFit::Radius);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn file_menu_saves_gcode() {
    let dir = temp_dir("menu_save");
    let path = dir.join("menu.ngc");
    let (mut harness, answers) = harness();
    answers.answer_save(&path);
    click(&mut harness, "File");
    let items: Vec<_> = harness.get_all_by_label_contains("Save G-code…").collect();
    assert_eq!(items.len(), 2, "the panel button and the menu item");
    items
        .iter()
        .find(|node| {
            node.accesskit_node()
                .label()
                .is_some_and(|l| l != "Save G-code…")
        })
        .expect("the menu item shows its shortcut")
        .click();
    harness.run_steps(2);
    assert!(path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[derive(Debug)]
struct DroppedPath(PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("unused".to_string())
    }
}

#[test]
fn dropping_a_file_opens_it() {
    let (mut harness, _) = harness();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(DroppedPath(fixture("drawing.dxf"))));
    harness.run_steps(1);
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.input_type, InputType::Image);
    assert!(app.design().is_some());
}

#[test]
fn opened_file_is_routed_by_type() {
    let (mut harness, _) = harness();
    harness.state_mut().open_path(&fixture("tiny.cxf"));
    assert_eq!(harness.state().settings.font_file, "tiny.cxf");
    harness.state_mut().open_path(&fixture("drawing.dxf"));
    assert_eq!(harness.state().settings.input_type, InputType::Image);
    harness.run_steps(1);
    settle(&mut harness);
    assert!(harness.state().design().is_some());
}

#[test]
fn arc_format_selector_changes_program() {
    let (mut harness, _) = harness();
    click(&mut harness, "Output");
    click(&mut harness, "Center");
    harness.run_steps(2);
    let app = harness.state_mut();
    assert_eq!(app.settings.arc_fit, ArcFit::Center);
    assert!(app.gcode().unwrap().contains(&"G91.1".to_string()));
}

#[test]
fn header_field_edits_program() {
    let (mut harness, _) = harness();
    click(&mut harness, "Output");
    harness
        .get_by_label("Header")
        .set_text_value_accesskit("G17 M3 S999");
    harness.run_steps(2);
    let gcode = harness.state_mut().gcode().unwrap();
    assert!(gcode.contains(&"G17 M3 S999".to_string()));
}

#[test]
fn inverted_variable_checkbox_controls_parameters() {
    let (mut harness, _) = harness();
    click(&mut harness, "Output");
    click(&mut harness, "Use #1 and #2 variables");
    harness.run_steps(1);
    assert!(!harness.state().settings.disable_variables);
    let gcode = harness.state_mut().gcode().unwrap();
    assert!(gcode.iter().any(|l| l.starts_with("#1 = ")));
}

#[test]
fn units_switch_converts_values() {
    let (mut harness, _) = harness();
    click(&mut harness, "mm");
    settle(&mut harness);
    let app = harness.state();
    assert_eq!(app.settings.units, Units::Millimeters);
    assert_eq!(app.settings.y_scale, 25.4);
    assert!(app.status().text.contains("mm"));
}

#[test]
fn gcode_view_shows_program_text() {
    let (mut harness, _) = harness();
    click(&mut harness, "G-code");
    harness.run_steps(2);
    assert_eq!(harness.state().active_view, ActiveView::Gcode);
    assert!(harness.query_by_label("G90").is_some());
    harness.get_by_label("Copy").click();
    harness.run_steps(2);
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t.starts_with("G90")))
        || harness.state().status().text.contains("Copied");
    assert!(copied);
}

#[test]
fn gcode_view_highlights_syntax_and_keeps_the_text() {
    let (mut harness, _) = harness_with(|s| s.no_comments = false);
    // Only visible rows are laid out, so make room for the settings block and the moves.
    harness.set_size(egui::vec2(1200.0, 3000.0));
    click(&mut harness, "G-code");
    harness.run_steps(2);

    // The accessible text is unchanged, so highlighting does not affect screen readers.
    assert!(harness.query_by_label("G90").is_some());

    let colors_of = |needle: &str| -> Vec<egui::Color32> {
        harness
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with(needle) => Some(
                    text.galley
                        .job
                        .sections
                        .iter()
                        .map(|s| s.format.color)
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no line starting with {needle}"))
    };
    let mut rapid = colors_of("G0 Z");
    rapid.dedup();
    assert!(
        rapid.len() >= 2,
        "the code and its operands differ: {rapid:?}"
    );

    let comment = colors_of("( Code generated");
    assert_eq!(comment.len(), 1, "a comment is a single span");
    assert_ne!(comment[0], rapid[0]);
}

#[test]
fn header_unsupported_by_fluidnc_is_flagged_and_fixable() {
    let (mut harness, _) = harness();
    assert!(harness.state().settings.controller_issues().is_empty());

    harness.state_mut().settings.g_pre = "G17 G64 P0.001 M3 S3000".into();
    harness.run_steps(2);
    let status = harness.state().status();
    assert_eq!(status.level, StatusLevel::Warning);
    assert!(status.text.contains("G64"), "{}", status.text);

    click(&mut harness, "Output");
    click(&mut harness, "Remove blending codes");
    assert_eq!(harness.state().settings.g_pre, "G17 M3 S3000");
    assert_eq!(harness.state().status().level, StatusLevel::Info);
}

#[test]
fn rejected_codes_with_no_safe_removal_are_only_reported() {
    let (mut harness, _) = harness();
    harness.state_mut().settings.g_post = "G81 X1 Z-1|M2".into();
    click(&mut harness, "Output");
    harness.run_steps(2);
    assert!(harness.state().status().text.contains("G81"));
    // Once in the status bar, and once beside the field.
    assert_eq!(harness.get_all_by_label_contains("uses G81").count(), 2);
    assert!(harness.query_by_label("Remove blending codes").is_none());
}

#[test]
fn reset_buttons_restore_each_field_alone() {
    let defaults = Settings::default();
    let (mut harness, _) = harness_with(|s| {
        s.g_pre = "G17 M3 S9000".into();
        s.g_post = "M5|M30".into();
    });
    click(&mut harness, "Output");

    click(&mut harness, "Reset header");
    assert_eq!(harness.state().settings.g_pre, defaults.g_pre);
    assert_eq!(harness.state().settings.g_post, "M5|M30");

    click(&mut harness, "Reset footer");
    assert_eq!(harness.state().settings.g_post, defaults.g_post);
}

#[test]
fn reset_buttons_are_disabled_at_the_default() {
    let (mut harness, _) = harness();
    click(&mut harness, "Output");
    assert!(
        harness
            .get_by_label("Reset header")
            .accesskit_node()
            .is_disabled()
    );
    harness.state_mut().settings.g_pre = "G17".into();
    harness.run_steps(2);
    assert!(
        !harness
            .get_by_label("Reset header")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_label("Reset footer")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn reset_header_clears_the_controller_warning() {
    let (mut harness, _) = harness_with(|s| s.g_pre = "G17 G64 P0.001 M3 S3000".into());
    assert_eq!(harness.state().status().level, StatusLevel::Warning);
    click(&mut harness, "Output");
    click(&mut harness, "Reset header");
    assert_eq!(harness.state().status().level, StatusLevel::Info);
}

// ---- Simulator --------------------------------------------------------------------------------

fn simulator_harness(settings: impl FnOnce(&mut Settings)) -> Harness<'static, RengraveApp> {
    let (mut harness, _) = harness_with(settings);
    click(&mut harness, "Simulate");
    harness.run_steps(2);
    harness
}

/// Steps until the stock has been cut up to the playback position.
fn catch_up(harness: &mut Harness<'_, RengraveApp>) {
    let start = Instant::now();
    harness.step();
    while !harness.state().simulator.has_caught_up() {
        harness.step();
        assert!(start.elapsed() < Duration::from_secs(60));
    }
}

#[test]
fn simulator_starts_paused_on_untouched_stock() {
    let harness = simulator_harness(|_| {});
    let view = &harness.state().simulator;
    let sim = view.simulation().expect("the program interprets");
    assert!(sim.total_seconds() > 0.0);
    assert!(!view.is_playing());
    assert_eq!(view.position(), 0.0);
    let (nx, ny) = sim.grid_size();
    assert_eq!(sim.height(nx / 2, ny / 2), 0.0);
    assert!(harness.query_by_label("Simulation view").is_some());
}

#[test]
fn play_advances_and_pause_stops() {
    let mut harness = simulator_harness(|_| {});
    click(&mut harness, "Play");
    assert!(harness.state().simulator.is_playing());
    harness.run_steps(3);
    let playing_at = harness.state().simulator.position();
    assert!(playing_at > 0.0);

    click(&mut harness, "Pause");
    assert!(!harness.state().simulator.is_playing());
    let paused_at = harness.state().simulator.position();
    harness.run_steps(3);
    assert_eq!(harness.state().simulator.position(), paused_at);
}

#[test]
fn finish_shows_the_carved_result_and_restart_restores_stock() {
    let mut harness = simulator_harness(|_| {});
    click(&mut harness, "Finish");
    catch_up(&mut harness);
    let carved_depth = {
        let sim = harness.state().simulator.simulation().unwrap();
        assert_eq!(sim.time(), sim.total_seconds());
        let (nx, ny) = sim.grid_size();
        (0..ny)
            .flat_map(|j| (0..nx).map(move |i| (i, j)))
            .map(|(i, j)| sim.height(i, j))
            .fold(0.0_f32, f32::min)
    };
    assert!(carved_depth < -0.05, "{carved_depth}");

    click(&mut harness, "Restart");
    catch_up(&mut harness);
    let sim = harness.state().simulator.simulation().unwrap();
    let (nx, ny) = sim.grid_size();
    assert_eq!(sim.time(), 0.0);
    assert!((0..ny).all(|j| (0..nx).all(|i| sim.height(i, j) == 0.0)));
}

#[test]
fn play_after_the_end_starts_over() {
    let mut harness = simulator_harness(|_| {});
    click(&mut harness, "Finish");
    catch_up(&mut harness);
    click(&mut harness, "Play");
    assert!(harness.state().simulator.is_playing());
    let total = harness
        .state()
        .simulator
        .simulation()
        .unwrap()
        .total_seconds();
    assert!(harness.state().simulator.position() < total);
}

#[test]
fn scrub_bar_moves_playback_both_ways() {
    let mut harness = simulator_harness(|_| {});
    let total = harness
        .state()
        .simulator
        .simulation()
        .unwrap()
        .total_seconds();
    let bar = harness.get_by_role(egui::accesskit::Role::Slider);
    bar.set_value_accesskit(total * 0.5);
    harness.run_steps(1);
    catch_up(&mut harness);
    let middle = harness.state().simulator.simulation().unwrap().time();
    assert!((middle - total * 0.5).abs() < total * 0.01, "{middle}");

    let bar = harness.get_by_role(egui::accesskit::Role::Slider);
    bar.set_value_accesskit(total * 0.25);
    harness.run_steps(1);
    catch_up(&mut harness);
    let earlier = harness.state().simulator.simulation().unwrap().time();
    assert!((earlier - total * 0.25).abs() < total * 0.01, "{earlier}");
}

#[test]
fn speed_choice_applies() {
    let mut harness = simulator_harness(|_| {});
    click(&mut harness, "1000×");
    assert_eq!(harness.state().simulator.speed(), 1000.0);
}

#[test]
fn changed_program_restarts_the_simulation() {
    let mut harness = simulator_harness(|_| {});
    click(&mut harness, "Finish");
    catch_up(&mut harness);
    let before = harness
        .state()
        .simulator
        .simulation()
        .unwrap()
        .total_seconds();

    // A slower feed changes only the program, not the toolpath.
    harness.state_mut().settings.feed = 1.0;
    harness.run_steps(3);
    let view = &harness.state().simulator;
    assert_eq!(view.position(), 0.0);
    assert!(view.simulation().unwrap().total_seconds() > before * 2.0);
}

#[test]
fn simulator_waits_while_live_carve_is_off() {
    let mut harness = simulator_harness(|_| {});
    click(&mut harness, "Live V-carve");
    harness.state_mut().settings.y_scale = 0.5;
    harness.run_steps(1);
    settle(&mut harness);
    assert!(
        harness
            .query_by_label("V-carve paths are not calculated.")
            .is_some()
    );
}

#[test]
fn uninterpretable_program_shows_why() {
    let mut harness = simulator_harness(|_| {});
    harness.state_mut().settings.g_post = "G1 X$".into();
    harness.run_steps(3);
    let view = &harness.state().simulator;
    assert!(view.simulation().is_none());
    let error = view
        .error()
        .expect("the footer cannot be parsed")
        .to_string();
    assert!(harness.query_by_label(&error).is_some(), "{error}");
}

#[test]
fn engraving_can_be_simulated() {
    let mut harness = simulator_harness(|s| s.cut_type = CutType::Engrave);
    click(&mut harness, "Finish");
    catch_up(&mut harness);
    let sim = harness.state().simulator.simulation().unwrap();
    let design = harness.state().design().unwrap();
    let s = design.segments[0];
    let at = sim
        .height_at((s.x1 + s.x2) / 2.0, (s.y1 + s.y2) / 2.0)
        .unwrap();
    assert!(
        (f64::from(at) - harness.state().settings.z_cut).abs() < 1e-3,
        "{at}"
    );
}

#[test]
fn simulator_shares_pan_and_zoom_with_the_preview() {
    let (mut harness, _) = harness();
    harness.state_mut().preview.zoom_by(2.0);
    harness.run_steps(2);
    let view = harness.state().preview.transform.unwrap();
    click(&mut harness, "Simulate");
    harness.run_steps(2);
    assert_eq!(harness.state().preview.transform.unwrap().scale, view.scale);
    click(&mut harness, "Path");
    harness.run_steps(2);
}

// ---- Preview ----------------------------------------------------------------------------------

#[test]
fn dragging_pans_and_fit_restores() {
    let (mut harness, _) = harness();
    let canvas = harness.get_by_label("Design preview").rect();
    let before = harness.state().preview.transform.unwrap();
    assert!(harness.state().preview.is_fitted());

    let start = canvas.center();
    harness.hover_at(start);
    harness.run_steps(1);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(start + egui::vec2(50.0, 30.0));
    harness.run_steps(1);
    harness.drop_at(start + egui::vec2(50.0, 30.0));
    harness.run_steps(2);

    let after = harness.state().preview.transform.unwrap();
    assert!(!harness.state().preview.is_fitted());
    assert!(
        after.center.0 < before.center.0,
        "dragging right moves the view left"
    );
    assert!(
        after.center.1 > before.center.1,
        "dragging down moves the view up"
    );

    click(&mut harness, "Fit");
    harness.run_steps(1);
    assert!(harness.state().preview.is_fitted());
    assert_eq!(
        harness.state().preview.transform.unwrap().center,
        before.center
    );
}

#[test]
fn scrolling_zooms_about_the_pointer() {
    let (mut harness, _) = harness();
    let canvas = harness.get_by_label("Design preview").rect();
    let at = canvas.center() + egui::vec2(60.0, -40.0);
    harness.hover_at(at);
    harness.run_steps(2);
    let before = harness.state().preview.transform.unwrap();
    let anchor = before.to_design(at);
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 100.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    });
    harness.run_steps(12);
    let after = harness.state().preview.transform.unwrap();
    assert!(after.scale > before.scale);
    let moved = after.to_design(at);
    assert!((moved.0 - anchor.0).abs() < 0.01 && (moved.1 - anchor.1).abs() < 0.01);
}

#[test]
fn page_keys_zoom() {
    let (mut harness, _) = harness();
    let before = harness.state().preview.transform.unwrap().scale;
    harness.key_press(Key::PageUp);
    harness.run_steps(2);
    let zoomed = harness.state().preview.transform.unwrap().scale;
    assert!(zoomed > before);
    harness.key_press(Key::PageDown);
    harness.run_steps(2);
    assert!(harness.state().preview.transform.unwrap().scale < zoomed);
}

#[test]
fn hover_reports_design_coordinates() {
    let (mut harness, _) = harness();
    let canvas = harness.get_by_label("Design preview").rect();
    harness.hover_at(canvas.center());
    harness.run_steps(2);
    let position = harness
        .state()
        .preview
        .hover_position
        .expect("hovering the canvas");
    let t = harness.state().preview.transform.unwrap();
    assert!((position.0 - t.center.0).abs() < 1e-3);
    assert!(harness.query_by_label_contains("  Y ").is_some());
}

#[test]
fn layer_toggles_are_available_and_work() {
    let (mut harness, _) = harness();
    click(&mut harness, "Area");
    assert!(!harness.state().settings.show_v_area);
    click(&mut harness, "Path");
    assert!(!harness.state().settings.show_v_path);
    click(&mut harness, "Grid");
    assert!(!harness.state().preview.layers.grid);
    // These are display settings, so they must not trigger recalculation.
    assert!(harness.state().carve().is_some());
    assert!(harness.state().is_idle());
}

#[test]
fn preview_redraws_new_layout_in_a_panned_view() {
    let (mut harness, _) = harness_with(|s| s.cut_type = CutType::Engrave);
    // A custom view keeps the transform fixed, so only the content can change the drawing.
    harness.state_mut().preview.zoom_by(1.1);
    harness.run_steps(2);
    let lines = |h: &Harness<'_, RengraveApp>| -> Vec<[egui::Pos2; 2]> {
        h.output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::LineSegment { points, .. } => Some(*points),
                _ => None,
            })
            .collect()
    };
    let before = lines(&harness);
    harness.state_mut().settings.text = "bA".into();
    harness.run_steps(1);
    settle(&mut harness);
    assert_ne!(lines(&harness), before, "the new text must be drawn");
}

#[test]
fn gcode_is_unavailable_until_a_running_layout_finishes() {
    let (mut harness, _) = harness_with(|s| s.cut_type = CutType::Engrave);
    let old = harness.state_mut().gcode().unwrap();
    harness.state_mut().settings.text = "Different".into();
    harness.state_mut().recalculate(false);
    assert!(!harness.state().can_write_gcode());
    // Asking anyway must not leave the old program cached for the new design.
    let _ = harness.state_mut().gcode();
    settle(&mut harness);
    assert!(harness.state().can_write_gcode());
    assert_ne!(harness.state_mut().gcode().unwrap(), old);
}

#[test]
fn f5_rereads_a_changed_source_file() {
    let dir = temp_dir("reload");
    let path = dir.join("d.dxf");
    let line = |x: f64| {
        format!(
            "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n{x}\n21\n1\n0\nENDSEC\n0\nEOF\n"
        )
    };
    std::fs::write(&path, line(1.0)).unwrap();
    let (mut harness, _) = harness_with(|s| {
        s.input_type = InputType::Image;
        s.image_file = path.clone();
        s.cut_type = CutType::Engrave;
        s.stroke_thickness = 0.0;
        s.y_scale = 1.0;
    });
    assert!((harness.state().design().unwrap().bounds.width() - 1.0).abs() < 1e-9);

    std::fs::write(&path, line(3.0)).unwrap();
    harness.key_press(Key::F5);
    harness.run_steps(1);
    settle(&mut harness);
    assert!((harness.state().design().unwrap().bounds.width() - 3.0).abs() < 1e-9);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cleanup_result_is_discarded_when_its_inputs_changed() {
    let (mut harness, _) = harness_with(|s| {
        s.text = "O".into();
        s.y_scale = 3.0;
        s.v_bit_dia = 0.05;
        s.clean_dia = 0.01;
        s.v_step_len = 0.05;
    });
    harness.state_mut().calculate_cleanup();
    harness.state_mut().settings.clean_dia = 0.02;
    settle(&mut harness);
    // Give a late result time to arrive; it must be ignored.
    std::thread::sleep(Duration::from_millis(200));
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .cleanup_paths(CleanupBit::Straight)
            .is_none()
    );
}

#[test]
fn load_warnings_survive_the_recalculation_they_cause() {
    let dir = temp_dir("load_warning");
    let path = dir.join("job.ngc");
    std::fs::write(
        &path,
        "(fengrave_set YSCALE 0.5 )\n(fengrave_set nonsense 1 )\n",
    )
    .unwrap();
    let (mut harness, _) = harness();
    harness.state_mut().open_path(&path);
    harness.run_steps(1);
    settle(&mut harness);
    let status = harness.state().status();
    assert_eq!(status.level, StatusLevel::Warning);
    assert!(status.text.contains("Ignored 1"), "{}", status.text);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hidden_settings_do_not_block_output() {
    let (mut harness, _) = harness_with(|s| {
        s.box_gap = 0.0;
        s.clean_dia = 0.0;
        s.cut_type = CutType::Engrave;
    });
    assert!(harness.state().can_write_gcode());
    harness.state_mut().settings.cut_type = CutType::VCarve;
    harness.run_steps(2);
    assert_eq!(harness.state().status().level, StatusLevel::Error);
    harness.state_mut().settings.clean_dia = 0.25;
    harness.run_steps(1);
    settle(&mut harness);
    assert!(harness.state().can_write_gcode());
}

#[test]
fn invalid_bit_angle_draws_and_blocks_output() {
    let (mut harness, _) = harness();
    click(&mut harness, "Toolpath");
    harness.get_by_label("Angle").set_value_accesskit(0.0);
    harness.run_steps(3);
    assert_eq!(harness.state().status().level, StatusLevel::Error);
    assert!(!harness.state().can_write_gcode());
    harness.get_by_label("Angle").set_value_accesskit(60.0);
    harness.run_steps(2);
    settle(&mut harness);
    assert!(harness.state().can_write_gcode());
}

#[test]
fn large_design_draws_without_panicking() {
    let (mut harness, _) = harness_with(|s| {
        s.text = "Lorem ipsum\ndolor sit amet".into();
        s.y_scale = 0.5;
        s.v_step_len = 0.005;
    });
    assert!(harness.state().carve().unwrap().points.len() > 1000);
    harness.run_steps(2);
}

#[test]
fn persistence_text_restores_settings() {
    let (harness, _) = harness_with(|s| s.text = "persist".into());
    let text = harness.state().settings_text();
    let answers = Answers::default();
    let mut restored = RengraveApp::new(egui::Context::default(), Box::new(answers));
    restored.restore_settings_text(&text);
    assert_eq!(restored.settings.text, "persist");
}

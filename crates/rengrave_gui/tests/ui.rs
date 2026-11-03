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

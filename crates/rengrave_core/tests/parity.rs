//! Compares output with reference G-code produced by the original F-Engrave 1.78.
//!
//! The fixtures in `tests/parity` are written by `tools/parity/generate.py`, which runs the
//! original program's batch mode. Every program must match line for line. Trailing whitespace
//! is ignored because the original appends a space to header lines read from settings files.

use rengrave_core::font::Font;
use rengrave_core::{InputType, Job, Progress, Settings};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity")
}

fn check_case(name: &str) {
    let dir = fixture_dir();
    let cfg = std::fs::read_to_string(dir.join(format!("{name}.cfg"))).unwrap();
    let expected = std::fs::read_to_string(dir.join(format!("{name}.ngc"))).unwrap();
    let text = std::fs::read_to_string(dir.join(format!("{name}.text"))).unwrap();

    let mut settings = Settings::default();
    let problems = settings.apply_settings_text(&cfg);
    assert!(problems.is_empty(), "{name}: {problems:?}");
    settings.text = text;
    settings.font_dir = dir.clone();
    if settings.input_type == InputType::Image {
        settings.image_file = dir.join(&settings.image_file);
    }
    let job = Job::load(settings).unwrap_or_else(|e| panic!("{name}: {e}"));
    let actual = job
        .generate_gcode(&Progress::none())
        .unwrap_or_else(|e| panic!("{name}: {e}"));

    let expected: Vec<&str> = expected.lines().map(str::trim_end).collect();
    for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(a, e, "{name}: first difference at line {}", i + 1);
    }
    assert_eq!(actual.len(), expected.len(), "{name}: program length");
}

macro_rules! parity_cases {
    ($($name:ident),+ $(,)?) => {
        $(#[test] fn $name() { check_case(stringify!($name)); })+
    };
}

parity_cases!(
    engrave_default,
    engrave_mm,
    engrave_plunge,
    engrave_variables,
    engrave_circle,
    engrave_inside_bottom,
    engrave_rotate_mirror,
    engrave_box,
    engrave_arcs,
    engrave_radius_arcs,
    engrave_max_all,
    engrave_header_footer,
    vcarve_default,
    vcarve_mm,
    vcarve_chr_scope,
    vcarve_depth_limit,
    vcarve_arcs,
    vcarve_ball,
    vcarve_ball_limit,
    vcarve_flat,
    vcarve_flat_multipass,
    vcarve_multipass,
    vcarve_flop,
    vcarve_box,
    vcarve_mirror,
    vcarve_flip_box,
    vcarve_circle,
    vcarve_inlay,
    vcarve_tight_corners,
    image_engrave,
    image_percent,
    image_vcarve,
    image_vcarve_flop,
    image_vcarve_box,
    image_rotate,
);

#[test]
fn fixture_font_parses() {
    let text = std::fs::read_to_string(fixture_dir().join("tiny.cxf")).unwrap();
    assert_eq!(Font::parse_cxf(&text, 5.0).glyphs.len(), 4);
}

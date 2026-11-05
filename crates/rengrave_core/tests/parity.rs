// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Compares output with reference G-code produced by the original F-Engrave 1.78.
//!
//! The fixtures in `tests/parity` were written by `tools/parity/generate.py`, which runs the
//! original program's batch mode. Eight V-carve cases (`image_rotate`, `image_vcarve_box`,
//! `image_vcarve_flop`, `vcarve_circle`, `vcarve_flat`, `vcarve_flat_multipass`, `vcarve_flop`,
//! and `vcarve_inlay`) were later regenerated from this program, because its math comes from the
//! `libm` crate (see `portable_math`) and no longer matches the operating system's math library
//! that the original used; their paths differ from the original's in a few moves. Every program
//! must match line for line, on every platform. Trailing whitespace is ignored because the
//! original appends a space to header lines read from settings files, and one deliberate
//! difference, for FluidNC, is undone before comparing.

use rengrave_core::font::Font;
use rengrave_core::{FontCatalog, FontRef, InputType, Job, Progress, Settings};
use std::path::{Path, PathBuf};

const ORIGINAL_HEADER: &str = "G17 G64 P0.001 M3 S3000";

/// Undoes the one deliberate difference from the original's G-code: a full circle now names its
/// end point, as FluidNC and GRBL require.
fn without_circle_end_point(line: &str) -> String {
    match line.strip_prefix("G2 X") {
        Some(rest) if rest.contains(" I") => {
            let offsets = &rest[rest.find(" I").unwrap_or(0)..];
            format!("G2{offsets}")
        }
        _ => line.to_string(),
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity")
}

fn check_case(name: &str) {
    let dir = fixture_dir();
    let cfg = std::fs::read_to_string(dir.join(format!("{name}.cfg"))).unwrap();
    let expected = std::fs::read_to_string(dir.join(format!("{name}.ngc"))).unwrap();
    let text = std::fs::read_to_string(dir.join(format!("{name}.text"))).unwrap();

    // The reference programs use the original's default header, which suits LinuxCNC.
    let mut settings = Settings {
        g_pre: ORIGINAL_HEADER.into(),
        ..Settings::default()
    };
    let problems = settings.apply_settings_text(&cfg);
    assert!(problems.is_empty(), "{name}: {problems:?}");
    settings.text = text;
    // The reference programs read `tiny.cxf` from the font directory named in the settings file.
    let fonts = FontCatalog::without_system_fonts();
    fonts
        .add_user_font(&dir.join("tiny.cxf"), Some("tiny"))
        .unwrap();
    settings.font = FontRef::User("tiny".into());
    if settings.input_type == InputType::Image {
        settings.image_file = dir.join(&settings.image_file);
    }
    let job = Job::load(settings, &fonts).unwrap_or_else(|e| panic!("{name}: {e}"));
    let actual = job
        .generate_gcode(&Progress::none())
        .unwrap_or_else(|e| panic!("{name}: {e}"));

    let expected: Vec<&str> = expected.lines().map(str::trim_end).collect();
    for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
        let a = if name == "engrave_circle" {
            without_circle_end_point(a)
        } else {
            a.clone()
        };
        assert_eq!(&a, e, "{name}: first difference at line {}", i + 1);
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

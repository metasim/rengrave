// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! End-to-end tests of the `rengrave` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

/// The application settings file the tests use, which does not exist unless a test writes it, so
/// the tests never read the settings of the person running them.
fn app_settings_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rengrave_cli_{}_app_settings.yaml",
        std::process::id()
    ))
}

fn rengrave(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rengrave-cli"))
        .args(args)
        .env("RENGRAVE_SETTINGS", app_settings_path())
        .output()
        .expect("binary runs")
}

/// A font file for tests, so that they do not depend on the fonts of the machine.
fn test_font() -> String {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let path =
            std::env::temp_dir().join(format!("rengrave_cli_test_{}.ttf", std::process::id()));
        std::fs::write(&path, epaint_default_fonts::HACK_REGULAR).unwrap();
        path
    })
    .display()
    .to_string()
}

/// Runs the binary with the test font, `args` last.
fn rengrave_with_font(args: &[&str]) -> Output {
    let font = test_font();
    let mut full = vec![args[0], "--font-file", &font];
    full.extend_from_slice(&args[1..]);
    rengrave(&full)
}

fn stdout_of_plain(args: &[&str]) -> String {
    let out = rengrave(args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn stdout_of(args: &[&str]) -> String {
    let out = rengrave_with_font(args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rengrave_cli_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rengrave_core/tests/parity")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn text_v_carve_runs() {
    let program = stdout_of(&[
        "gcode", "-t", "Hi", "--mode", "v-carve", "--height", "1", "--step", "0.02",
    ]);
    assert!(program.starts_with("G90\nG20\n"));
    assert!(program.contains("\nG1 X"));
    assert!(program.trim_end().ends_with("M2"));
}

#[test]
fn output_file_is_written_and_stdout_is_silent() {
    let dir = temp_dir("output");
    let path = dir.join("o.ngc");
    let out = rengrave_with_font(&["gcode", "-t", "A", "-o", path.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(std::fs::read_to_string(&path).unwrap().contains("G20"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cxf_font_and_settings_file_match_the_original_program() {
    let expected = std::fs::read_to_string(fixture("vcarve_box.ngc")).unwrap();
    let out = rengrave(&[
        "gcode",
        "-c",
        &fixture("vcarve_box.cfg"),
        "--font-file",
        &fixture("tiny.cxf"),
        "-t",
        "AB",
        "--set",
        "gpre=G17 G64 P0.001 M3 S3000",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let actual = String::from_utf8(out.stdout).unwrap();
    let trimmed: Vec<&str> = expected.lines().map(str::trim_end).collect();
    let actual: Vec<String> = actual.lines().map(str::to_string).collect();
    for (i, (a, e)) in actual.iter().zip(&trimmed).enumerate() {
        assert_eq!(a, e, "first difference at line {}", i + 1);
    }
    assert_eq!(actual.len(), trimmed.len(), "program length");
}

#[test]
fn settings_written_by_one_run_reproduce_the_job() {
    let dir = temp_dir("settings");
    let path = dir.join("job.cfg");
    let args = [
        "-t", "Rt", "--mode", "v-carve", "--units", "mm", "--height", "20", "--step", "0.5",
    ];
    let mut write = vec!["settings", "-o", path.to_str().unwrap()];
    write.extend_from_slice(&args);
    assert!(rengrave_with_font(&write).status.success());

    let mut direct = vec!["gcode"];
    direct.extend_from_slice(&args);
    let from_file = stdout_of(&["gcode", "-c", path.to_str().unwrap()]);
    assert!(std::fs::read_to_string(&path).unwrap().contains("\"user:"));
    assert_eq!(stdout_of(&direct), from_file);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn image_dxf_exports_svg_and_dxf() {
    let image = fixture("drawing.dxf");
    let svg = stdout_of(&["svg", "-i", &image]);
    assert!(svg.contains("<svg") && svg.trim_end().ends_with("</svg>"));
    let dxf = stdout_of(&["dxf", "-i", &image, "--close-loops"]);
    assert!(dxf.contains("AcDbLine") && dxf.trim_end().ends_with("EOF"));
}

#[test]
fn cleanup_operation_writes_program() {
    let program = stdout_of(&[
        "cleanup",
        "-t",
        "O",
        "--mode",
        "v-carve",
        "--height",
        "3",
        "--bit-dia",
        "0.05",
        "--step",
        "0.05",
        "--set",
        "clean_dia=0.01",
    ]);
    assert!(program.contains("\nG1 X"));
}

#[test]
fn cleanup_of_engraving_is_an_error() {
    let out = rengrave_with_font(&["cleanup", "-t", "O"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cleanup"));
}

#[test]
fn bad_input_fails_with_a_message() {
    let out = rengrave(&["gcode", "--font-file", "/nonexistent/font.ttf"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error:"));
    let out = rengrave_with_font(&["gcode", "--height", "0"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Height"));
}

#[test]
fn non_finite_values_are_rejected() {
    for args in [["--z-safe", "nan"], ["--height", "inf"]] {
        let mut full = vec!["gcode", "-t", "A", "--mode", "v-carve"];
        full.extend_from_slice(&args);
        let out = rengrave_with_font(&full);
        assert!(!out.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).starts_with("error:"));
    }
}

#[test]
fn header_unsupported_by_fluidnc_warns_on_stderr() {
    let out = rengrave_with_font(&["gcode", "-t", "A", "--set", "gpre=G17 G64 P0.001 M3 S3000"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("G64"));

    // The defaults need no warning.
    let out = rengrave_with_font(&["gcode", "-t", "A"]);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("G64"));
    assert!(String::from_utf8_lossy(&out.stdout).contains(
        "
G17 M3 S3000
"
    ));
}

#[test]
fn missing_characters_warn_on_stderr() {
    let out = rengrave(&["gcode", "--font-file", &fixture("tiny.cxf"), "-t", "AZ"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Z"));
}

#[test]
fn fonts_describes_a_font_file() {
    let listing = stdout_of_plain(&["fonts", &fixture("tiny.cxf")]);
    assert!(listing.contains("tiny.cxf"));
    assert!(listing.contains("characters"));
}

#[test]
fn fonts_in_the_shared_settings_file_are_usable() {
    let dir = temp_dir("shared");
    let settings = dir.join("settings.yaml");
    std::fs::write(
        &settings,
        format!(
            "user_fonts:\n  - name: Mine\n    path: {}\n",
            fixture("tiny.cxf")
        ),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rengrave-cli"))
            .args(args)
            .env("RENGRAVE_SETTINGS", &settings)
            .output()
            .expect("binary runs")
    };
    let out = run(&["gcode", "-f", "user:Mine", "-t", "AB"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("G1 X"));
    let listing = run(&["fonts"]);
    assert!(String::from_utf8_lossy(&listing.stdout).contains("user:Mine"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_broken_shared_settings_file_warns_but_does_not_stop_the_job() {
    let dir = temp_dir("broken");
    let settings = dir.join("settings.yaml");
    std::fs::write(&settings, "user_fonts: [oops\n").unwrap();
    let font = test_font();
    let out = Command::new(env!("CARGO_BIN_EXE_rengrave-cli"))
        .args(["gcode", "--font-file", &font, "-t", "A"])
        .env("RENGRAVE_SETTINGS", &settings)
        .output()
        .expect("binary runs");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("settings.yaml"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unknown_font_prefix_is_rejected() {
    let out = rengrave(&["gcode", "-f", "Hack"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--font"));
}

#[test]
fn font_page_engraves_a_block_of_the_font() {
    let latin = stdout_of(&["gcode", "--font-page", "latin"]);
    let basic = stdout_of(&["gcode", "--font-page", "basic latin", "--page", "1"]);
    assert!(latin.contains("G1"));
    assert!(latin.len() > basic.len(), "Latin-1 adds characters");
    assert!(basic.contains("G1"));
}

#[test]
fn font_page_rejects_an_unknown_block_and_a_page_without_one() {
    let out = rengrave_with_font(&["gcode", "--font-page", "Klingon"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown Unicode block"));
    let out = rengrave_with_font(&["gcode", "--page", "2"]);
    assert!(!out.status.success());
}

#[test]
fn verbose_reports_progress_on_stderr_only() {
    let out = rengrave_with_font(&[
        "gcode", "-t", "A", "--mode", "v-carve", "--step", "0.05", "-v",
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("100%"));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("calculating"));
}

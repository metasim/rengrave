// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! End-to-end tests of the `rengrave` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn rengrave(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rengrave-cli"))
        .args(args)
        .output()
        .expect("binary runs")
}

fn stdout_of(args: &[&str]) -> String {
    let out = rengrave(args);
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
fn text_v_carve_runs_with_no_setup() {
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
    let out = rengrave(&["gcode", "-t", "A", "-o", path.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(std::fs::read_to_string(&path).unwrap().contains("G20"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cxf_font_and_settings_file_match_the_original_program() {
    let expected = std::fs::read_to_string(fixture("vcarve_box.ngc")).unwrap();
    let actual = stdout_of(&[
        "gcode",
        "-c",
        &fixture("vcarve_box.cfg"),
        "-f",
        &fixture("tiny.cxf"),
        "-t",
        "AB",
        "--set",
        "gpre=G17 G64 P0.001 M3 S3000",
    ]);
    let trimmed: Vec<&str> = expected.lines().map(str::trim_end).collect();
    assert_eq!(actual.lines().collect::<Vec<_>>(), trimmed);
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
    assert!(rengrave(&write).status.success());

    let mut direct = vec!["gcode"];
    direct.extend_from_slice(&args);
    let from_file = stdout_of(&["gcode", "-c", path.to_str().unwrap()]);
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
    let out = rengrave(&["cleanup", "-t", "O"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cleanup"));
}

#[test]
fn bad_input_fails_with_a_message() {
    let out = rengrave(&["gcode", "-f", "/nonexistent/font.ttf"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error:"));
    let out = rengrave(&["gcode", "--height", "0"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Height"));
}

#[test]
fn non_finite_values_are_rejected() {
    for args in [["--z-safe", "nan"], ["--height", "inf"]] {
        let mut full = vec!["gcode", "-t", "A", "--mode", "v-carve"];
        full.extend_from_slice(&args);
        let out = rengrave(&full);
        assert!(!out.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).starts_with("error:"));
    }
}

#[test]
fn header_unsupported_by_fluidnc_warns_on_stderr() {
    let out = rengrave(&["gcode", "-t", "A", "--set", "gpre=G17 G64 P0.001 M3 S3000"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("G64"));

    // The defaults need no warning.
    let out = rengrave(&["gcode", "-t", "A"]);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("G64"));
    assert!(String::from_utf8_lossy(&out.stdout).contains(
        "
G17 M3 S3000
"
    ));
}

#[test]
fn missing_characters_warn_on_stderr() {
    let out = rengrave(&["gcode", "-f", &fixture("tiny.cxf"), "-t", "AZ"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Z"));
}

#[test]
fn fonts_lists_builtin_and_directory() {
    let dir = Path::new(&fixture("tiny.cxf"))
        .parent()
        .unwrap()
        .display()
        .to_string();
    let listing = stdout_of(&["fonts", &dir]);
    assert!(listing.contains("Ubuntu Light"));
    assert!(listing.contains("tiny.cxf"));
}

#[test]
fn verbose_reports_progress_on_stderr_only() {
    let out = rengrave(&[
        "gcode", "-t", "A", "--mode", "v-carve", "--step", "0.05", "-v",
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("100%"));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("calculating"));
}

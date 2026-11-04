// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Renders the documentation screenshots. Needs a GPU, so it only runs with the `screenshots`
//! feature: `cargo test -p rengrave_gui --features screenshots --test screenshots`.
//!
//! Images are written to the directory named by `RENGRAVE_SCREENSHOT_DIR`, or `docs/images`.
#![cfg(feature = "screenshots")]

use egui_kittest::Harness;
use rengrave_gui::{ActiveView, NativeDialogs, RengraveApp, Tab};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn settle(harness: &mut Harness<'_, RengraveApp>) {
    let start = Instant::now();
    while !harness.state().is_idle() {
        harness.step();
        assert!(start.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.run_steps(3);
}

fn save(harness: &mut Harness<'_, RengraveApp>, name: &str) {
    let dir = std::env::var_os("RENGRAVE_SCREENSHOT_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/images"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let image = harness.render().expect("rendering needs a GPU");
    image.save(dir.join(name)).unwrap();
}

/// Waits for the simulator to reach `fraction` of the program, and saves it.
fn save_simulator(harness: &mut Harness<'_, RengraveApp>, name: &str, fraction: f64) {
    harness.state_mut().tab = Tab::Toolpath;
    harness.state_mut().active_view = ActiveView::Simulator;
    harness.run_steps(3);
    let total = harness
        .state()
        .simulator
        .simulation()
        .expect("the program interprets")
        .total_seconds();
    harness.state_mut().simulator.seek(total * fraction);
    let start = Instant::now();
    harness.step();
    while !harness.state().simulator.has_caught_up() {
        harness.step();
        assert!(start.elapsed() < Duration::from_secs(120));
    }
    harness.run_steps(2);
    save(harness, name);
}

fn harness(app: RengraveApp, dark: bool) -> Harness<'static, RengraveApp> {
    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    Harness::builder()
        .with_size(egui::vec2(1180.0, 760.0))
        .with_theme(theme)
        .wgpu()
        .build_ui_state(|ui, app: &mut RengraveApp| app.show(ui), app)
}

/// Renders the default text, plus the other tabs when `all_tabs` is set.
fn shoot(dark: bool, suffix: &str, all_tabs: bool) {
    let app = RengraveApp::new(egui::Context::default(), Box::new(NativeDialogs));
    let mut harness = harness(app, dark);
    settle(&mut harness);
    save(&mut harness, &format!("design{suffix}.png"));
    if !all_tabs {
        return;
    }

    harness.state_mut().tab = Tab::Toolpath;
    harness.run_steps(3);
    save(&mut harness, "toolpath.png");

    harness.state_mut().tab = Tab::Output;
    harness.state_mut().active_view = ActiveView::Gcode;
    harness.run_steps(3);
    save(&mut harness, "output.png");

    save_simulator(&mut harness, "simulator.png", 0.6);
}

/// Renders the image example.
fn shoot_image() {
    let mut app = RengraveApp::new(egui::Context::default(), Box::new(NativeDialogs));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/angry-unicorn.png");
    app.open_path(&path);
    let mut harness = harness(app, true);
    settle(&mut harness);
    save(&mut harness, "image.png");
    save_simulator(&mut harness, "image_simulator.png", 0.6);
}

#[test]
fn render_screenshots() {
    shoot(true, "", true);
    shoot(false, "_light", false);
    shoot_image();
}

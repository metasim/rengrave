// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Launches the rengrave desktop application.

use rengrave_gui::{NativeDialogs, RengraveApp};
use std::path::PathBuf;

/// Key under which settings persist between sessions.
const STORAGE_KEY: &str = "settings";

/// The application icon, set when the window opens. A bundled app also has the icon in its
/// `.icns` file, but an application started from the command line has only this one.
const ICON_PNG: &[u8] = include_bytes!("../assets/icon/Rengrave.png");

/// Forwards the application to `eframe`, persisting settings between runs.
struct Window(RengraveApp);

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.0.settings_text());
    }
}

fn main() -> eframe::Result {
    let file: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rengrave")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(ICON_PNG)
                    .expect("the embedded application icon is a valid PNG"),
            )
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Rengrave",
        native,
        Box::new(move |cc| {
            let mut app = RengraveApp::new(cc.egui_ctx.clone(), Box::new(NativeDialogs));
            if let Some(stored) = cc.storage.and_then(|s| s.get_string(STORAGE_KEY)) {
                app.restore_settings_text(&stored);
            }
            if let Some(path) = file {
                app.open_path(&path);
            }
            Ok(Box::new(Window(app)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(ICON_PNG).unwrap();
        assert_eq!((icon.width, icon.height), (512, 512));
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! The application settings dialog, and keeping the settings file up to date.
//!
//! These are the settings of the program, shared with the command-line tool through one file,
//! not the settings of a job. Today the dialog has one page, the user's font library.

use crate::app::{RengraveApp, StatusLevel};
use crate::icons;
use crate::widgets::UiExt;
use egui::Ui;
use rengrave_core::AppSettings;
use std::path::PathBuf;

/// A page of the application settings dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    /// The user's library of font files.
    #[default]
    Fonts,
}

impl SettingsPage {
    const ALL: [Self; 1] = [Self::Fonts];

    const fn title(self) -> &'static str {
        match self {
            Self::Fonts => "Fonts",
        }
    }
}

impl RengraveApp {
    /// Reads the application settings from `path`, uses the user's fonts in them, and keeps the
    /// file up to date from now on. A file that cannot be read is reported and left alone.
    pub fn use_app_settings(&mut self, path: PathBuf) {
        match AppSettings::load_from(&path) {
            Ok(settings) => settings.apply_to(&self.fonts),
            Err(e) => self.notify(StatusLevel::Warning, e.to_string()),
        }
        self.app_settings_path = Some(path);
    }

    /// Writes the application settings, if a file is in use.
    pub(crate) fn save_app_settings(&mut self) {
        let Some(path) = self.app_settings_path.clone() else {
            return;
        };
        if let Err(e) = AppSettings::from_catalog(&self.fonts).save_to(&path) {
            self.notify(StatusLevel::Error, e.to_string());
        }
    }

    /// Opens the application settings dialog on `page`.
    pub fn show_app_settings(&mut self, page: SettingsPage) {
        self.settings_page = page;
        self.show_app_settings = true;
    }

    pub(crate) fn app_settings_window(&mut self, ui: &mut Ui) {
        let mut open = self.show_app_settings;
        egui::Window::new("Application settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .show(ui.ctx(), |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(90.0);
                        for page in SettingsPage::ALL {
                            ui.selectable_value(&mut self.settings_page, page, page.title());
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| match self.settings_page {
                        SettingsPage::Fonts => self.fonts_page(ui),
                    });
                });
            });
        self.show_app_settings = open;
    }

    fn fonts_page(&mut self, ui: &mut Ui) {
        ui.label(ui.control_label("Your fonts"));
        ui.weak("Font files you add appear under User fonts in the font list. Installed fonts are always available under System fonts.");
        ui.add_space(4.0);
        let fonts = self.fonts.user_fonts();
        let mut remove = None;
        if fonts.is_empty() {
            ui.weak("No fonts added yet.");
        }
        egui::Grid::new("user_fonts")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for font in &fonts {
                    ui.label(&font.name);
                    let missing = !font.path.is_file();
                    let location = font.path.display().to_string();
                    let text = if missing {
                        format!("{location} (file not found)")
                    } else {
                        location
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
                    if ui
                        .button("Remove")
                        .on_hover_text(format!("Remove {} from your fonts", font.name))
                        .clicked()
                    {
                        remove = Some(font.name.clone());
                    }
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        if ui
            .icon_button(true, icons::OPEN, "Add font file…")
            .clicked()
        {
            self.add_font_dialog(false);
        }
        if let Some(name) = remove {
            self.fonts.remove_user_font(&name);
            self.save_app_settings();
        }
    }
}

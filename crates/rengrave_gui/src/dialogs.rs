// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! File selection, behind a trait so tests can run without native dialogs.

use std::path::{Path, PathBuf};

/// What a file dialog is being opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A DXF drawing or raster image to engrave.
    Image,
    /// A font file.
    Font,
    /// A settings file or G-code file to read settings from.
    SettingsOpen,
    /// A settings file to write.
    SettingsSave,
    /// A G-code program to write.
    Gcode,
    /// An SVG drawing to write.
    Svg,
    /// A DXF drawing to write.
    Dxf,
}

impl FileKind {
    const fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Image => (
                "Drawings and images",
                &[
                    "dxf", "png", "bmp", "pbm", "pgm", "ppm", "pnm", "jpg", "jpeg", "gif", "tif",
                    "tiff",
                ],
            ),
            Self::Font => ("Fonts", &["cxf", "ttf", "otf"]),
            Self::SettingsOpen => ("Settings or G-code", &["ngc", "nc", "tap", "gcode", "txt"]),
            Self::SettingsSave | Self::Gcode => ("G-code", &["ngc", "nc", "tap", "gcode"]),
            Self::Svg => ("SVG", &["svg"]),
            Self::Dxf => ("DXF", &["dxf"]),
        }
    }
}

/// Asks the user to pick files.
pub trait FileDialogs {
    /// Chooses an existing file.
    fn pick_file(&mut self, kind: FileKind, start_dir: Option<&Path>) -> Option<PathBuf>;
    /// Chooses a directory.
    fn pick_folder(&mut self, start_dir: Option<&Path>) -> Option<PathBuf>;
    /// Chooses where to write a file.
    fn save_file(&mut self, kind: FileKind, suggested: &str) -> Option<PathBuf>;
}

/// Operating system file dialogs.
#[derive(Default)]
pub struct NativeDialogs;

impl FileDialogs for NativeDialogs {
    fn pick_file(&mut self, kind: FileKind, start_dir: Option<&Path>) -> Option<PathBuf> {
        let (name, extensions) = kind.filter();
        let mut dialog = rfd::FileDialog::new().add_filter(name, extensions);
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file()
    }

    fn pick_folder(&mut self, start_dir: Option<&Path>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_folder()
    }

    fn save_file(&mut self, kind: FileKind, suggested: &str) -> Option<PathBuf> {
        let (name, extensions) = kind.filter();
        rfd::FileDialog::new()
            .add_filter(name, extensions)
            .set_file_name(suggested)
            .save_file()
    }
}

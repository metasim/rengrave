// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Settings of the program itself, as opposed to the settings of a job.
//!
//! They live in one YAML file in the platform's configuration directory, which the desktop
//! application and the command-line tool both read. Today they hold the user's font library.

use crate::catalog::{FontCatalog, UserFont};
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The name of the folder that holds the program's files, shared with the desktop application's
/// own storage.
const APP_DIR: &str = "Rengrave";
/// The name of the settings file.
const FILE_NAME: &str = "settings.yaml";
/// An environment variable that names the settings file to use instead of the standard one.
pub const PATH_VARIABLE: &str = "RENGRAVE_SETTINGS";

/// Settings that belong to the user rather than to a job.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// The font files the user has added.
    pub user_fonts: Vec<UserFont>,
}

impl AppSettings {
    /// Where the settings file is: the file named by `RENGRAVE_SETTINGS` if set, otherwise
    /// `settings.yaml` in the platform's configuration directory. `None` if the platform has no
    /// home directory.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os(PATH_VARIABLE).filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(path));
        }
        directories::ProjectDirs::from("", "", APP_DIR)
            .map(|dirs| dirs.config_dir().join(FILE_NAME))
    }

    /// Reads the settings from `path`. A file that does not exist gives the defaults.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        // A file with nothing but comments reads as the defaults.
        if text
            .lines()
            .all(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
        {
            return Ok(Self::default());
        }
        serde_norway::from_str(&text).map_err(|e| Error::AppSettings {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// Writes the settings to `path`, creating its folder. The file is replaced in one step, so
    /// a failure does not leave half a file for the other program to read.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let io = |source| Error::Io {
            path: path.to_path_buf(),
            source,
        };
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let text = serde_norway::to_string(self).map_err(|e| Error::AppSettings {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, text).map_err(io)?;
        std::fs::rename(&temporary, path).map_err(io)
    }

    /// Puts the user's font library into `catalog`.
    pub fn apply_to(&self, catalog: &FontCatalog) {
        catalog.set_user_fonts(self.user_fonts.clone());
    }

    /// The settings that describe the state of `catalog`.
    pub fn from_catalog(catalog: &FontCatalog) -> Self {
        Self {
            user_fonts: catalog.user_fonts(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rengrave_app_{}_{name}", std::process::id()))
    }

    #[test]
    fn missing_file_gives_defaults() {
        let settings = AppSettings::load_from(&temp_file("missing/settings.yaml")).unwrap();
        assert_eq!(settings, AppSettings::default());
    }

    #[test]
    fn settings_round_trip_through_a_file() {
        let path = temp_file("round/settings.yaml");
        let settings = AppSettings {
            user_fonts: vec![
                UserFont {
                    name: "Tiny".into(),
                    path: PathBuf::from("/fonts/tiny.cxf"),
                    face: 0,
                },
                UserFont {
                    name: "Collection: second".into(),
                    path: PathBuf::from("/fonts/c.ttc"),
                    face: 1,
                },
            ],
        };
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        assert!(!path.with_extension("yaml.tmp").exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_face_defaults_to_zero_and_unknown_keys_are_ignored() {
        let path = temp_file("face.yaml");
        std::fs::write(
            &path,
            "# comment\nuser_fonts:\n  - name: A\n    path: /a.ttf\nfuture_setting: 3\n",
        )
        .unwrap();
        let settings = AppSettings::load_from(&path).unwrap();
        assert_eq!(settings.user_fonts[0].face, 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn comment_only_and_empty_files_give_defaults() {
        let path = temp_file("empty.yaml");
        std::fs::write(&path, "# nothing yet\n\n").unwrap();
        assert_eq!(
            AppSettings::load_from(&path).unwrap(),
            AppSettings::default()
        );
        std::fs::write(&path, "").unwrap();
        assert_eq!(
            AppSettings::load_from(&path).unwrap(),
            AppSettings::default()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn malformed_file_is_an_error_naming_the_file() {
        let path = temp_file("bad.yaml");
        std::fs::write(&path, "user_fonts: [unclosed\n").unwrap();
        let err = AppSettings::load_from(&path).unwrap_err();
        assert!(matches!(err, Error::AppSettings { .. }), "{err}");
        assert!(err.to_string().contains("bad.yaml"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn catalog_round_trip() {
        let catalog = FontCatalog::without_system_fonts();
        let fonts = vec![UserFont {
            name: "X".into(),
            path: PathBuf::from("/x.ttf"),
            face: 0,
        }];
        let settings = AppSettings {
            user_fonts: fonts.clone(),
        };
        settings.apply_to(&catalog);
        assert_eq!(catalog.user_fonts(), fonts);
        assert_eq!(AppSettings::from_catalog(&catalog), settings);
    }
}

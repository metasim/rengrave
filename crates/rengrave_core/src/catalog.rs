// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Where fonts come from: the fonts installed on the machine and a library of font files added
//! by the user.
//!
//! A job names its font with a [`FontRef`]. A [`FontCatalog`] resolves the reference to a
//! [`Font`] of strokes.

use crate::error::{Error, Result};
use crate::font::{Font, extension_upper};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Which font a job uses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FontRef {
    /// The system's default sans-serif font.
    #[default]
    Default,
    /// A font family installed on the machine, by its family name.
    System(String),
    /// A font in the user's library, by its library name.
    User(String),
}

impl FontRef {
    const SYSTEM_PREFIX: &'static str = "system:";
    const USER_PREFIX: &'static str = "user:";

    /// A short name for display, without the library prefix.
    pub fn label(&self) -> &str {
        match self {
            Self::Default => "Default",
            Self::System(name) | Self::User(name) => name,
        }
    }
}

impl fmt::Display for FontRef {
    /// Writes `system:Family`, `user:Name`, or nothing for the default.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => Ok(()),
            Self::System(name) => write!(f, "{}{name}", Self::SYSTEM_PREFIX),
            Self::User(name) => write!(f, "{}{name}", Self::USER_PREFIX),
        }
    }
}

impl FromStr for FontRef {
    type Err = String;

    /// Reads the form written by [`Display`](fmt::Display). Empty text is the default font.
    fn from_str(text: &str) -> std::result::Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            Ok(Self::Default)
        } else if let Some(name) = text.strip_prefix(Self::SYSTEM_PREFIX) {
            Ok(Self::System(name.trim().to_string()))
        } else if let Some(name) = text.strip_prefix(Self::USER_PREFIX) {
            Ok(Self::User(name.trim().to_string()))
        } else {
            Err(format!(
                "font {text:?} must be empty, \"system:<family>\", or \"user:<name>\""
            ))
        }
    }
}

/// A font file the user added to their library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFont {
    /// The name the library lists the font under.
    pub name: String,
    /// The `.cxf`, `.ttf`, `.otf`, `.ttc`, or `.otc` file.
    pub path: PathBuf,
    /// The font within a collection file; zero for a single font.
    #[serde(default)]
    pub face: u32,
}

/// The font files the program can read, by extension in upper case.
const FONT_EXTENSIONS: [&str; 5] = ["CXF", "TTF", "OTF", "TTC", "OTC"];

/// The installed fonts and the user's font library.
///
/// The catalog is safe to share between threads; lookups take turns.
pub struct FontCatalog {
    user: Mutex<Vec<UserFont>>,
    #[cfg(feature = "system-fonts")]
    system: Mutex<SystemSlot>,
    /// Set when the scan for installed fonts has ended, so that callers can ask without waiting.
    #[cfg(feature = "system-fonts")]
    scanned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The state of the installed fonts.
#[cfg(feature = "system-fonts")]
enum SystemSlot {
    /// Installed fonts are not used.
    Off,
    /// A thread is looking for them.
    Scanning(std::thread::JoinHandle<SystemFonts>),
    /// They are known.
    Ready(Box<SystemFonts>),
}

#[cfg(feature = "system-fonts")]
struct SystemFonts {
    collection: fontique::Collection,
    cache: fontique::SourceCache,
}

#[cfg(feature = "system-fonts")]
impl SystemFonts {
    fn scan() -> Self {
        Self {
            collection: fontique::Collection::new(fontique::CollectionOptions {
                shared: false,
                system_fonts: true,
            }),
            cache: fontique::SourceCache::new(fontique::SourceCacheOptions::default()),
        }
    }

    /// Family names for display, sorted without regard to case. Names that start with a dot are
    /// the operating system's private interface fonts and are left out.
    fn families(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self
            .collection
            .family_names()
            .filter(|n| !n.starts_with('.') && !n.trim().is_empty())
            .map(str::to_string)
            .collect();
        names.sort_by_cached_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// The regular face of the named family: its bytes and its index within the file.
    fn regular_face(&mut self, family: &str) -> Option<(fontique::Blob<u8>, u32)> {
        let info = self.collection.family_by_name(family)?;
        Self::face_of(&mut self.cache, &info)
    }

    /// The regular face of the first generic sans-serif family that has a readable font.
    fn default_face(&mut self) -> Option<(fontique::Blob<u8>, u32)> {
        let ids: Vec<_> = self
            .collection
            .generic_families(fontique::GenericFamily::SansSerif)
            .collect();
        ids.into_iter().find_map(|id| {
            let info = self.collection.family(id)?;
            Self::face_of(&mut self.cache, &info)
        })
    }

    fn face_of(
        cache: &mut fontique::SourceCache,
        family: &fontique::FamilyInfo,
    ) -> Option<(fontique::Blob<u8>, u32)> {
        let face = family
            .match_font(
                fontique::FontWidth::NORMAL,
                fontique::FontStyle::Normal,
                fontique::FontWeight::NORMAL,
                false,
            )
            .or_else(|| family.default_font())?;
        Some((face.load(Some(cache))?, face.index()))
    }
}

#[cfg(feature = "system-fonts")]
impl SystemSlot {
    /// The installed fonts, waiting for the scan to end if it has not.
    fn ready(&mut self) -> Option<&mut SystemFonts> {
        if matches!(self, Self::Scanning(_)) {
            let Self::Scanning(handle) = std::mem::replace(self, Self::Off) else {
                unreachable!("checked above");
            };
            *self = handle
                .join()
                .map_or(Self::Off, |fonts| Self::Ready(Box::new(fonts)));
        }
        match self {
            Self::Ready(fonts) => Some(fonts.as_mut()),
            _ => None,
        }
    }
}

impl Default for FontCatalog {
    /// A catalog with no system fonts and an empty library.
    fn default() -> Self {
        Self::without_system_fonts()
    }
}

impl FontCatalog {
    /// A catalog with only the user's library. System fonts are never looked up, which suits
    /// tests and callers that supply their own fonts.
    pub fn without_system_fonts() -> Self {
        Self {
            user: Mutex::new(Vec::new()),
            #[cfg(feature = "system-fonts")]
            system: Mutex::new(SystemSlot::Off),
            #[cfg(feature = "system-fonts")]
            scanned: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    /// A catalog that includes the fonts installed on this machine, found on another thread.
    ///
    /// Finding them can take a few hundred milliseconds. Loading an installed font, or listing
    /// the families, waits for the search to end; [`FontCatalog::is_scanning`] asks without
    /// waiting.
    #[cfg(feature = "system-fonts")]
    pub fn with_system_fonts() -> Self {
        use std::sync::atomic::Ordering;
        let catalog = Self::without_system_fonts();
        catalog.scanned.store(false, Ordering::Release);
        let scanned = catalog.scanned.clone();
        let handle = std::thread::spawn(move || {
            let fonts = SystemFonts::scan();
            scanned.store(true, Ordering::Release);
            fonts
        });
        *catalog.system_slot() = SystemSlot::Scanning(handle);
        catalog
    }

    /// Returns `true` while the search for installed fonts is still running.
    pub fn is_scanning(&self) -> bool {
        #[cfg(feature = "system-fonts")]
        return !self.scanned.load(std::sync::atomic::Ordering::Acquire);
        #[cfg(not(feature = "system-fonts"))]
        false
    }

    fn users(&self) -> MutexGuard<'_, Vec<UserFont>> {
        self.user.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(feature = "system-fonts")]
    fn system_slot(&self) -> MutexGuard<'_, SystemSlot> {
        self.system.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The installed font families, sorted by name; empty if system fonts are not included.
    /// Waits for the search for installed fonts to end.
    pub fn system_families(&self) -> Vec<String> {
        #[cfg(feature = "system-fonts")]
        if let Some(system) = self.system_slot().ready() {
            return system.families();
        }
        Vec::new()
    }

    /// The fonts in the user's library, in the order they were added.
    pub fn user_fonts(&self) -> Vec<UserFont> {
        self.users().clone()
    }

    /// Replaces the user's library, for example with the one read from the application settings.
    pub fn set_user_fonts(&self, fonts: Vec<UserFont>) {
        *self.users() = fonts;
    }

    /// Checks a font file and adds it to the library, returning the entry.
    ///
    /// The entry is named `name`, or after the file if `name` is `None`. A name already in the
    /// library gets a number appended. Fails if the file is not a readable font.
    pub fn add_user_font(&self, path: &Path, name: Option<&str>) -> Result<UserFont> {
        if !FONT_EXTENSIONS.contains(&extension_upper(path).as_str()) {
            return Err(Error::Font(format!(
                "unsupported font file type: {}",
                path.display()
            )));
        }
        let font = Font::load_file(path, 0, 45.0, &Font::codes_for_text(""))?;
        if font.is_empty() {
            return Err(Error::Font(format!(
                "no characters in font: {}",
                path.display()
            )));
        }
        let base = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Font".to_string());
        let mut users = self.users();
        let taken = |candidate: &str| users.iter().any(|f| f.name == candidate);
        let unique = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base} ({n})")
                }
            })
            .find(|candidate| !taken(candidate))
            .unwrap_or(base);
        let entry = UserFont {
            name: unique,
            path: path.to_path_buf(),
            face: 0,
        };
        users.push(entry.clone());
        Ok(entry)
    }

    /// Removes the named font from the library; returns `false` if there was none.
    pub fn remove_user_font(&self, name: &str) -> bool {
        let mut users = self.users();
        let before = users.len();
        users.retain(|f| f.name != name);
        users.len() != before
    }

    /// Returns `true` if `font` can be loaded: the file or family exists. Waits for the search
    /// for installed fonts to end if `font` is one of them.
    pub fn contains(&self, font: &FontRef) -> bool {
        match font {
            FontRef::Default => true,
            FontRef::User(name) => self
                .users()
                .iter()
                .any(|f| &f.name == name && f.path.is_file()),
            FontRef::System(family) => self.system_families().iter().any(|f| f == family),
        }
    }

    /// Loads the strokes for the code points in `wanted`, flattening curves to within `seg_arc`
    /// degrees. CXF fonts are read whole. Waits for the search for installed fonts to end if
    /// `font` is one of them.
    pub fn load(&self, font: &FontRef, seg_arc: f64, wanted: &[u32]) -> Result<Font> {
        match font {
            FontRef::User(name) => {
                let entry = self
                    .users()
                    .iter()
                    .find(|f| &f.name == name)
                    .cloned()
                    .ok_or_else(|| Error::Font(format!("font {name:?} is not in your library")))?;
                Font::load_file(&entry.path, entry.face, seg_arc, wanted)
            }
            FontRef::System(family) => self.load_system(Some(family), seg_arc, wanted),
            FontRef::Default => self.load_system(None, seg_arc, wanted),
        }
    }

    #[cfg(feature = "system-fonts")]
    fn load_system(&self, family: Option<&str>, seg_arc: f64, wanted: &[u32]) -> Result<Font> {
        let (blob, face) = {
            let mut slot = self.system_slot();
            let system = slot.ready().ok_or_else(Self::no_system_fonts)?;
            match family {
                Some(name) => system
                    .regular_face(name)
                    .ok_or_else(|| Error::Font(format!("font {name:?} is not installed")))?,
                None => system.default_face().ok_or_else(Self::no_system_fonts)?,
            }
        };
        Font::from_outline_data(Arc::new(blob), face, seg_arc, wanted)
    }

    #[cfg(not(feature = "system-fonts"))]
    fn load_system(&self, _family: Option<&str>, _seg_arc: f64, _wanted: &[u32]) -> Result<Font> {
        Err(Self::no_system_fonts())
    }

    fn no_system_fonts() -> Error {
        Error::Font(
            "no system font is available; install a font, or choose one from a file".to_string(),
        )
    }
}

/// A catalog holding one real typeface, so tests do not depend on the fonts of the machine.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::OnceLock;

    /// The library name of the test font.
    pub(crate) const FONT_NAME: &str = "Test Font";

    /// Writes Hack, which egui ships, to a file once per process.
    fn font_file() -> &'static Path {
        static FILE: OnceLock<PathBuf> = OnceLock::new();
        FILE.get_or_init(|| {
            let path =
                std::env::temp_dir().join(format!("rengrave-test-font-{}.ttf", std::process::id()));
            std::fs::write(&path, epaint_default_fonts::HACK_REGULAR).unwrap();
            path
        })
    }

    /// A catalog whose library contains the test font and nothing else.
    pub(crate) fn catalog() -> FontCatalog {
        let catalog = FontCatalog::without_system_fonts();
        catalog.add_user_font(font_file(), Some(FONT_NAME)).unwrap();
        catalog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_file(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/parity")
            .join(name)
    }

    #[test]
    fn font_refs_round_trip_through_text() {
        for font in [
            FontRef::Default,
            FontRef::System("Helvetica Neue".into()),
            FontRef::User("My Font".into()),
        ] {
            assert_eq!(font.to_string().parse::<FontRef>(), Ok(font));
        }
        assert!("Hack".parse::<FontRef>().is_err());
        assert_eq!("  ".parse::<FontRef>(), Ok(FontRef::Default));
    }

    #[test]
    fn user_fonts_are_added_named_and_removed() {
        let catalog = FontCatalog::without_system_fonts();
        let first = catalog
            .add_user_font(&fixture_file("tiny.cxf"), None)
            .unwrap();
        assert_eq!(first.name, "tiny");
        let second = catalog
            .add_user_font(&fixture_file("tiny.cxf"), None)
            .unwrap();
        assert_eq!(second.name, "tiny (2)");
        assert_eq!(catalog.user_fonts().len(), 2);
        assert!(catalog.contains(&FontRef::User("tiny".into())));
        assert!(catalog.remove_user_font("tiny"));
        assert!(!catalog.remove_user_font("tiny"));
        assert!(!catalog.contains(&FontRef::User("tiny".into())));
    }

    #[test]
    fn a_user_font_loads_through_the_catalog() {
        let catalog = FontCatalog::without_system_fonts();
        catalog
            .add_user_font(&fixture_file("tiny.cxf"), Some("Tiny"))
            .unwrap();
        let font = catalog
            .load(
                &FontRef::User("Tiny".into()),
                5.0,
                &Font::codes_for_text(""),
            )
            .unwrap();
        assert!(!font.is_empty());
    }

    #[test]
    fn unreadable_files_are_not_added() {
        let catalog = FontCatalog::without_system_fonts();
        assert!(
            catalog
                .add_user_font(&fixture_file("drawing.dxf"), None)
                .is_err()
        );
        assert!(
            catalog
                .add_user_font(&fixture_file("missing.ttf"), None)
                .is_err()
        );
        assert!(catalog.user_fonts().is_empty());
    }

    #[test]
    fn a_missing_library_entry_is_an_error() {
        let catalog = FontCatalog::without_system_fonts();
        let err = catalog
            .load(&FontRef::User("nope".into()), 5.0, &[65])
            .unwrap_err();
        assert!(err.to_string().contains("not in your library"), "{err}");
    }

    #[test]
    fn system_fonts_are_unavailable_without_a_scan() {
        let catalog = FontCatalog::without_system_fonts();
        assert!(catalog.system_families().is_empty());
        assert!(catalog.load(&FontRef::Default, 5.0, &[65]).is_err());
    }

    /// Loads every installed family to learn how many fail to open. It reports the count with
    /// `--nocapture` and fails only if the default font is unusable on a machine that has fonts.
    #[cfg(feature = "system-fonts")]
    #[test]
    fn installed_fonts_are_readable() {
        let catalog = FontCatalog::with_system_fonts();
        let families = catalog.system_families();
        let wanted = Font::codes_for_text("");
        let failures: Vec<(String, String)> = families
            .iter()
            .filter_map(|name| {
                catalog
                    .load(&FontRef::System(name.clone()), 30.0, &wanted)
                    .err()
                    .map(|e| (name.clone(), e.to_string()))
            })
            .collect();
        println!(
            "{} families, {} could not be read",
            families.len(),
            failures.len()
        );
        for (name, error) in &failures {
            println!("  {name}: {error}");
        }
        if !families.is_empty() {
            catalog.load(&FontRef::Default, 30.0, &wanted).unwrap();
        }
    }
}

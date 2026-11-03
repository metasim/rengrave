//! The high-level interface: settings plus loaded source glyphs.

use crate::cleanup::CleanupPaths;
use crate::design::Design;
use crate::error::{Error, Result};
use crate::font::{BuiltinFont, Font};
use crate::gcode::GcodeProgram;
use crate::progress::Progress;
use crate::settings::{CleanupBit, CutType, InputType, Settings};
use crate::trace::ImportedDrawing;
use crate::vcarve::CarvePath;
use std::path::PathBuf;

/// Settings together with the font or drawing they refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// The settings, which may be changed freely; see [`Job::needs_reload`].
    pub settings: Settings,
    /// The loaded glyphs.
    pub font: Font,
    /// Non-fatal observations from loading the source.
    pub warnings: Vec<String>,
}

impl Job {
    /// Loads the source named by `settings`.
    pub fn load(settings: Settings) -> Result<Self> {
        let mut warnings = Vec::new();
        let font = match settings.input_type {
            InputType::Text => Self::load_font(&settings)?,
            InputType::Image => {
                if settings.image_file.as_os_str().is_empty() {
                    return Err(Error::Import("no image file selected".into()));
                }
                let imported = ImportedDrawing::load(&settings.image_file, &settings)?;
                warnings = imported.warnings;
                Font::from_drawing(imported.strokes)
            }
        };
        Ok(Self {
            settings,
            font,
            warnings,
        })
    }

    fn load_font(settings: &Settings) -> Result<Font> {
        let name = settings.font_file.trim();
        if name.is_empty() {
            return BuiltinFont::UbuntuLight.load(settings.seg_arc, settings.extended_chars);
        }
        if let Some(builtin) = BuiltinFont::from_name(name) {
            return builtin.load(settings.seg_arc, settings.extended_chars);
        }
        let path = Self::font_path(settings);
        Font::load_file(&path, settings.seg_arc, settings.extended_chars)
    }

    /// The path of a font file named by `settings`. Absolute names ignore the font directory.
    pub fn font_path(settings: &Settings) -> PathBuf {
        settings.font_dir.join(settings.font_file.trim())
    }

    /// Returns `true` if changing from `old` to `new` settings requires reloading the source.
    pub fn needs_reload(old: &Settings, new: &Settings) -> bool {
        old.input_type != new.input_type
            || old.font_dir != new.font_dir
            || old.font_file != new.font_file
            || old.image_file != new.image_file
            || old.seg_arc != new.seg_arc
            || old.extended_chars != new.extended_chars
            || old.tracer != new.tracer
            || old.bmp_turn_policy != new.bmp_turn_policy
            || old.bmp_turd_size != new.bmp_turd_size
            || old.bmp_alpha_max != new.bmp_alpha_max
            || old.bmp_opt_tolerance != new.bmp_opt_tolerance
            || old.bmp_long_curve != new.bmp_long_curve
    }

    /// Positions the outlines. See [`Design::layout`].
    pub fn layout(&self) -> Result<Design> {
        Design::layout(&self.font, &self.settings)
    }

    /// Runs layout and any V-carve calculation, returning the design and the toolpath.
    ///
    /// The toolpath is `None` for engraving jobs.
    pub fn calculate(&self, progress: &Progress) -> Result<(Design, Option<CarvePath>)> {
        let mut design = self.layout()?;
        let carve = if self.settings.cut_type == CutType::VCarve {
            Some(CarvePath::calculate(&mut design, &self.settings, progress)?)
        } else {
            None
        };
        Ok((design, carve))
    }

    /// Runs the whole job, returning the lines of the G-code program.
    pub fn generate_gcode(&self, progress: &Progress) -> Result<Vec<String>> {
        let (design, carve) = self.calculate(progress)?;
        GcodeProgram::generate(&design, carve.as_ref(), &self.settings)
    }

    /// Runs the job and a cleanup calculation, returning the cleanup G-code program.
    pub fn generate_cleanup_gcode(
        &self,
        bit: CleanupBit,
        progress: &Progress,
    ) -> Result<Vec<String>> {
        let (design, _) = self.calculate(progress)?;
        let paths = CleanupPaths::calculate(&design, &self.settings, bit)?;
        if paths.points.is_empty() {
            return Err(Error::NothingToCut(
                "No cleanup paths were found for the current settings".into(),
            ));
        }
        GcodeProgram::generate_cleanup(&paths.points, bit, &self.settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_job_loads_builtin_font() {
        let job = Job::load(Settings::default()).unwrap();
        assert!(job.font.glyph('F').is_some());
    }

    #[test]
    fn missing_font_file_is_an_error() {
        let settings = Settings {
            font_file: "nope.ttf".into(),
            ..Settings::default()
        };
        assert!(matches!(Job::load(settings), Err(Error::Io { .. })));
    }

    #[test]
    fn image_input_without_file_is_an_error() {
        let settings = Settings {
            input_type: InputType::Image,
            ..Settings::default()
        };
        assert!(matches!(Job::load(settings), Err(Error::Import(_))));
    }

    #[test]
    fn reload_detection() {
        let base = Settings::default();
        assert!(!Job::needs_reload(
            &base,
            &Settings {
                y_scale: 9.0,
                ..base.clone()
            }
        ));
        assert!(Job::needs_reload(
            &base,
            &Settings {
                seg_arc: 9.0,
                ..base.clone()
            }
        ));
        assert!(Job::needs_reload(
            &base,
            &Settings {
                font_file: "Hack".into(),
                ..base.clone()
            }
        ));
    }

    #[test]
    fn engrave_job_produces_program() {
        let job = Job::load(Settings::default()).unwrap();
        let program = job.generate_gcode(&Progress::none()).unwrap();
        assert!(program.iter().any(|l| l.starts_with("G1 X")));
    }

    #[test]
    fn v_carve_of_text_stays_within_depth() {
        let settings = Settings {
            text: "Rust".into(),
            cut_type: CutType::VCarve,
            y_scale: 1.0,
            v_step_len: 0.02,
            ..Settings::default()
        };
        let job = Job::load(settings).unwrap();
        let (design, carve) = job.calculate(&Progress::none()).unwrap();
        let carve = carve.unwrap();
        assert!(!carve.points.is_empty());
        let rbit = job.settings.effective_bit_dia() / 2.0;
        for p in &carve.points {
            assert!(p.r >= 0.0 && p.r <= rbit + 1e-12);
            assert!(p.x >= design.bounds.min_x - 1e-9 && p.x <= design.bounds.max_x + 1e-9);
        }
    }

    #[test]
    fn cleanup_requires_paths() {
        let settings = Settings {
            text: "I".into(),
            ..Settings::default()
        };
        let job = Job::load(settings).unwrap();
        // An engraving job has nothing to clean.
        assert!(matches!(
            job.generate_cleanup_gcode(CleanupBit::Straight, &Progress::none()),
            Err(Error::NothingToCut(_))
        ));
    }

    #[test]
    fn cleanup_program_for_large_letter() {
        let settings = Settings {
            text: "O".into(),
            cut_type: CutType::VCarve,
            y_scale: 3.0,
            v_bit_dia: 0.05,
            clean_dia: 0.01,
            v_step_len: 0.05,
            ..Settings::default()
        };
        let job = Job::load(settings).unwrap();
        let program = job
            .generate_cleanup_gcode(CleanupBit::Straight, &Progress::none())
            .unwrap();
        assert!(program.iter().any(|l| l.starts_with("G1 X")));
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Toolpath engine for engraving and V-carving text and images.
//!
//! `rengrave_core` is a Rust port of the calculations in
//! [F-Engrave](https://www.scorchworks.com/Fengrave/fengrave.html). It has no user interface;
//! the `rengrave_cli` and `rengrave_gui` crates are front ends to it.
//!
//! A job passes through these stages:
//!
//! 1. Load the source glyphs: [`Job::load`] reads the font or image named by the [`Settings`].
//! 2. Position the outlines: [`Job::layout`] produces a [`Design`].
//! 3. Calculate depths, if V-carving: [`CarvePath::calculate`].
//! 4. Write the result: [`GcodeProgram::generate`], [`Design::to_svg`], or
//!    [`Design::to_dxf`]. [`CleanupPaths`] and [`GcodeProgram::generate_cleanup`] produce the
//!    optional secondary operations.
//!
//! [`Job::generate_gcode`] runs stages 2 through 4 in one call:
//!
//! ```no_run
//! use rengrave_core::{CutType, FontCatalog, Job, Progress, Settings};
//!
//! let settings = Settings {
//!     text: "Hi".into(),
//!     cut_type: CutType::VCarve,
//!     ..Settings::default()
//! };
//! let fonts = FontCatalog::with_system_fonts();
//! let job = Job::load(settings, &fonts)?;
//! let gcode = job.generate_gcode(&Progress::none())?;
//! assert!(gcode.iter().any(|line| line.starts_with("G1 ")));
//! # Ok::<(), rengrave_core::Error>(())
//! ```

#![warn(missing_docs)]

pub mod app_settings;
pub mod catalog;
pub mod cleanup;
pub mod design;
pub mod dxf;
mod error;
mod export;
pub mod font;
pub mod gcode;
pub mod geom;
mod job;
mod progress;
pub mod settings;
pub mod shaping;
pub mod simulate;
pub mod sort;
pub mod trace;
pub mod unicode_blocks;
pub mod vcarve;

pub use app_settings::AppSettings;
pub use catalog::{FontCatalog, FontRef, UserFont};
pub use cleanup::CleanupPaths;
pub use design::Design;
pub use error::{Error, Result};
pub use font::{Font, FontCoverage};
pub use gcode::GcodeProgram;
pub use job::Job;
pub use progress::Progress;
pub use settings::{
    ArcFit, BitShape, CheckScope, CleanupBit, CleanupOps, CutType, HeightCalc, InputType, Issue,
    Justify, Origin, Settings, Tracer, TurnPolicy, Units,
};
pub use simulate::Simulation;
pub use vcarve::{CarvePath, CarvePoint};

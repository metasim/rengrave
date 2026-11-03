//! The rengrave desktop application, built with [egui](https://www.egui.rs).
//!
//! [`RengraveApp`] holds all application state and draws itself into any [`egui::Ui`], so it can
//! run in a native window (`rengrave-gui`) or headless under `egui_kittest`.

#![warn(missing_docs)]

mod app;
pub mod dialogs;
mod gcode_view;
mod inspector;
pub mod preview;
mod widgets;
pub mod worker;

pub use app::{ActiveView, RengraveApp, Status, StatusLevel, Tab};
pub use dialogs::{FileDialogs, FileKind, NativeDialogs};

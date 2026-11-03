# rengrave_gui

The `rengrave` desktop application, built with [egui](https://www.egui.rs). See the
[project README](../../README.md) for usage and the [interface review](../../docs/UX_REVIEW.md) for
the reasoning behind its design.

`RengraveApp::show` draws the application into any `egui::Ui`, which lets the tests in `tests/ui.rs`
drive it headless with `egui_kittest`.

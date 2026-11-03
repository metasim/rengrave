// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Named [Lucide](https://lucide.dev) glyphs, as code points of the bundled icon font.
//!
//! Draw them with [`crate::style::Look::icon`], or through `UiExt::icon_button`.

// Code points are from `codepoints.json` of lucide-static 1.67.0, whose `lucide.ttf` is the font
// that is bundled.

/// A play triangle.
pub const PLAY: char = '\u{e13c}';
/// Two vertical bars.
pub const PAUSE: char = '\u{e12e}';
/// A counter-clockwise arrow.
pub const RESTART: char = '\u{e148}';
/// A triangle with a bar, for jumping to the end.
pub const FINISH: char = '\u{e160}';
/// Corner brackets that grow outward.
pub const FIT: char = '\u{e112}';
/// A sheet with a plus.
pub const NEW: char = '\u{e0c9}';
/// An open folder.
pub const OPEN: char = '\u{e247}';
/// A floppy disk.
pub const SAVE: char = '\u{e14d}';
/// Two overlapping sheets.
pub const COPY: char = '\u{e09e}';
/// A curved arrow back.
pub const RESET: char = '\u{e2a1}';
/// A gear.
pub const SETTINGS: char = '\u{e154}';
/// A sheet with an arrow pointing down.
pub const EXPORT: char = '\u{e318}';
/// An arrow leaving a doorway.
pub const QUIT: char = '\u{e10e}';
/// A circled "i".
pub const INFO: char = '\u{e0f9}';
/// A warning triangle.
pub const WARNING: char = '\u{e193}';
/// A circled cross.
pub const ERROR: char = '\u{e084}';
/// A refresh symbol.
pub const CALCULATE: char = '\u{e145}';

# Decision log

The port was built during an unattended session. Each entry records a question that would
normally have gone to the project owner, the decision taken to keep moving, and how to
revisit it. Later entries are appended as the work progresses.

## D1. License: GPL-3.0-or-later

F-Engrave is GPLv3-or-later and this code is a derivative work, so the workspace is licensed
the same way. `LICENSE` is the GPL text from the F-Engrave distribution. *Not revisitable
without a clean-room rewrite.*

## D2. Crate layout and branch

Three crates as suggested, under `crates/`: `rengrave_core`, `rengrave_cli` (binary
`rengrave-cli`), and `rengrave_gui` (binary `rengrave`). The stub root package created by
`cargo new` was replaced by a virtual workspace. The binaries were first named `rengrave` and
`rengrave-gui`; they were renamed so that the application has the plain name. See D17 for the
repository's branches. The source archive `F-Engrave-1.78_src.zip` is ignored rather than committed, since it is reference
material available from the upstream site.

## D3. Polygon clipping: `clipper2-rust`

The original uses `pyclipper` (Clipper 1) for cleanup paths. `clipper2-rust` is a pure-Rust
port of the successor library, which keeps the build free of a C++ toolchain. Vertex-level
output differs from Clipper 1 by design, so cleanup paths are verified by geometric
invariants rather than byte-for-byte comparison. *Alternative: the `clipper2` FFI crate.*

## D4. TrueType support without a helper program

The original shells out to `ttf2cxf_stream` (C++/FreeType). The port reads fonts in-process
with `ttf-parser`, scales glyphs by the CXF convention that `A` is 9 units tall, and flattens
curves adaptively. *Corrected by D15:* this first ported the helper's flattening, which was a
licensing mistake.

## D5. Fonts: installed and user-added, none bundled

*Supersedes the original D5, which embedded Ubuntu Light and Hack so that the program worked with
no setup.* The program now ships no engraving font. Text uses fonts from two libraries:

- **System fonts:** every family installed on the machine, found with `fontique` (the engine
  `eframe` already uses for fallback fonts) through CoreText, DirectWrite, or fontconfig.
- **User fonts:** CXF, TTF, OTF, and TTC files the user adds, listed by name.

A job names its font as `system:<family>` or `user:<name>`; no name means the system's default
sans-serif. Glyph outlines are read with `skrifa`, which egui also uses, and only Latin-1 and the
characters in the text are loaded, so large fonts stay cheap. Output can differ between machines
that have different fonts. Settings files no longer carry F-Engrave's `fontdir` and `fontfile`
keys; the reader ignores them. See `docs/PLAN_SYSTEM_FONTS.md`.

The user library is application state, not job state. It is kept in `settings.yaml` in the
platform's configuration directory (found with `directories`, the crate `eframe`'s own tests use to
check its storage path), written with `serde` and `serde_norway`. The desktop application and
`rengrave-cli` share the file; `RENGRAVE_SETTINGS` overrides its location. The YAML crate's parser
is a translation of libyaml and contains `unsafe` code, which was accepted for a small,
user-controlled file. Job settings keep their own text format.

## D6. Bitmap tracing: native tracer, with potrace optional

The original requires the external `potrace` program. To keep "runnable with no helper
programs" true, the port includes a small native tracer. It follows pixel boundaries, then
simplifies and smooths them. It is simpler than potrace, so when `potrace` is installed it
can be selected (`Tracer::Potrace`) and its DXF output goes through the same reader as the
original.

## D7. Verification against the original program

The original's batch mode (`-b`) runs headless once `tkinter` and `pyclipper` are stubbed.
Reference G-code generated that way is committed under
`crates/rengrave_core/tests/parity/` and the port must reproduce it line for line. The stub
harness lives in `tools/parity/`.

## D8. Deliberate differences from the original's behavior

Parity tests pin the original's G-code wherever behavior is well defined. The following are
clear defects, so the port deliberately does not reproduce them:

- CXF: a stroke on the last line of a file was dropped. It is kept.
- DXF: `SOLID` entities raised an error that aborted the whole import. They are drawn.
- DXF: `CIRCLE` overshot its start point for tolerances that do not divide 360. It closes exactly.
- DXF: one damaged entity lost the whole drawing. Now it is skipped and reported.
- DXF: `LWPOLYLINE` treated any nonzero flag as closed, so `PLINEGEN` (128) closed open polylines.
  Only bit 1 closes one now. Bulges also stay aligned when vertices carry width codes.
- DXF: unclamped splines were evaluated outside their domain. They are evaluated over it now, and
  invalid knot vectors are rejected.
- A V-bit angle of 0 raised an exception during G-code generation. It now fails validation, as
  does 180, and any `NaN` or infinite value.
- Validation stopped all output for settings unused by the job, such as a zero box gap with no box.
  Only settings that apply are checked now.
- Text: center/right justification did nothing in batch mode, because the last line was only
  measured if followed by a newline. All lines are measured in all modes.
- Fonts: cubic curves were reduced to one chord, and CFF (`.otf`) winding was not normalized.
- A full engraved circle did not name its end point, which FluidNC rejects. See D13.
- Python exceptions such as a zero-length chord in arc fitting are handled as "no arc".
- Settings keys match by exact name rather than by substring, so one key cannot capture another's
  value.

The G-code comment header names rengrave instead of F-Engrave, and `gpre`/`gpost` values read
from a settings file no longer gain a trailing space.

## D9. Interface: a redesign rather than a copy

The review in [UX_REVIEW.md](UX_REVIEW.md) led to one window with tabs in place of the original's
modal dialogs, automatic recalculation in place of the Recalculate button, and a background
calculation in place of the modal progress window. The **Auto** toggle (first named "Live V-carve")
turns recalculation on or off, and **Refresh** recalculates by hand, so it is disabled while Auto
is on. *Open question for the owner:* whether **Auto** should default to on. It is on, because the engine is fast enough for typical designs and
canceling is immediate.

## D10. Defaults

`Settings::default()` keeps the original's defaults (engraving, inches, left-justified), except for
what D13 changes for FluidNC, so settings files and the command line behave as existing users
expect. The desktop application starts on a V-carve in millimeters, because V-carving is the
program's main purpose. The text is "F-engrave" over "Fan!", center-justified, so the first view
shows more than one line. The height is one inch, converted to 25.4 mm. **File ▸ New** returns to
these defaults. The application saves its settings when it closes and restores them on the next
start, so the defaults apply only to a first run or after File ▸ New.

## D11. Features not ported

- The `Plot During V-Carve Calculation` option, which is superseded by the live preview.
- Running as a LinuxCNC AXIS filter (`Write to Axis`).
- `(fengrave_set v_pplot ...)` is accepted but ignored.

## D12. Repository state at the start

The repository had no remote or commits at the start, so there was no pull request. The first work
was on the branch `feature/rust-port`, and the unborn `master` was never committed to. This is
superseded by D17.

## D13. FluidNC is the target controller

The owner's machine runs FluidNC, so the defaults target it, in the library, the command line, and
the desktop application alike. Program output was reviewed against FluidNC's `GCode.cpp` at commit
`c82d217`. It has not been tested on a controller.

- **Default header:** now `G17 M3 S3000`, without `G64 P0.001`. FluidNC has no `G64` and returns
  `GcodeUnsupportedCommand`. The original's header puts `M3` on that line, so as read from the source
  the whole line is rejected and the spindle does not start. Reference tests still use the original
  header, and LinuxCNC users can set it.
- **Custom headers and footers:** `Settings::controller_issues` compares their `G` and `M` codes with
  an allowlist taken from FluidNC's source, so unknown codes are flagged by default. This matters
  because the simulator's interpreter accepts some codes FluidNC rejects, so a clean simulation does
  not show that a program will run. Older settings files keep the header they stored, and get the
  same warning. Lines with expressions or named parameters are not checked. The list is fixed at
  that FluidNC commit, and some accepted codes also depend on the machine's configuration, such as
  `M4` and coolant.
- **Removing codes:** only `G64` and `G61.1` are removed automatically. Removing another code could
  change what the rest of its line does, such as turning `G81 X1 Z-1` into a move, so those are
  reported for the user to edit.
- **Engraved circle around text:** was `G2 I… J…`, with no end point, which FluidNC rejects with
  `GcodeNoAxisWords`. It now names the end point, the same as the start, which still cuts a full
  circle on LinuxCNC. This is the one deliberate difference from the original's G-code.
- **Unchanged because FluidNC supports them:** `G90`, `G91.1`, `G20`/`G21`, `G17`, `M3`, `M5`, `M2`,
  inline `F`, parenthesized comments, and `#1`/`#2` variables.
- **Known risk with arc fitting, which is off by default:** FluidNC rejects center-format arcs when
  the start and end radii differ by more than 0.005 mm and 0.1% of the radius. At four decimals in
  inches, rounding could in the worst case reach about 0.005 mm on very small arcs. Output in
  millimeters is not close to this limit. Prefer millimeters or `Lines` if such an error appears.

*Open question for the owner:* the spindle speed `S3000` and the 5 in/min feed (127 mm/min in the
desktop application) are still the original's defaults, and the command line still uses inches.
FluidNC accepts them, but they may not suit the machine.

## D14. Toolpath simulator

Research found no embeddable simulator widget in Rust, but [WarpNine](https://github.com/jonahhawk/warpnine)
has a usable interpreter and height-field stock model, so rengrave builds on those rather than
writing its own. `gsim-rs` is Fanuc-only and not embeddable; `chipbreaker-gcode`'s engine is too slow
to animate; and `fluidSender`'s `fluid-sim` emulates the protocol, with no material removal.

- **Vendored, not a git dependency.** The project had 4 commits and no releases, so builds should
  not depend on it staying up. The owner approved vendoring. The sources stay unmodified.
- **Reads the emitted G-code,** not the internal toolpath, so it shows what a machine receives, and
  gives engraving and V-carving one code path. A test confirms the surface agrees with the internal
  tool positions.
- **Top-down shaded view first.** A **Render** menu in the toolbar sets the grid resolution (the grid is at most 2047
  cells on a side, to fit the largest texture), the relief shading, and smoothing. *Open question:*
  a 3D view could be added later.
- **Constant-feed timing,** with an assumed rapid rate, and no acceleration. Run times are labeled
  as estimates. *Open question:* realistic timing would need FluidNC's acceleration settings, and
  `swarf-motion` could provide it.
- **The main program only.** *Open question:* cleanup operations could be simulated too.
- **Does not model the controller.** The interpreter silently accepts `G64`, which FluidNC rejects,
  so the separate check from D13 is still needed.

## D15. Per-file licensing, and the GPL-2.0-only helper

The owner asked for copyright to be tracked per file. See [COPYRIGHT.md](../COPYRIGHT.md).

Doing so found a real conflict. `ttf2cxf_stream` is GPL **version 2 only**, unlike the rest of
F-Engrave. F-Engrave stays clear of it by running a separate process, but `font.rs` had ported its
adaptive curve flattening into this GPL-3.0-or-later program, and those licenses cannot be combined
in one binary. The flattening has been replaced with a different algorithm (recursive subdivision
by chord turning angle), so the helper's code is no longer used.

*Open question for the owner:* the replacement was written by someone who had read the original, so
it is independent in expression but not clean-room. Consider asking a lawyer if this matters for
how rengrave will be distributed. The earlier sources F-Engrave credits (cxf2cnc.py, DXF2GCODE,
LinuxCNC's Author.py, and others) were taken on trust from its GPL-3.0-or-later distribution, and
their licenses were not checked individually.

## D16. Interface style: neutral chrome, blue focus, bundled fonts

The owner asked for the desktop application to look closer to another of their programs. Only a
written style specification was used; none of that program's code or assets was consulted.
`crates/rengrave_gui/src/style.rs` installs the fonts and a `Visuals` for each theme when
`RengraveApp::new` runs. It also runs from `show` for any context that has not been styled yet,
because `egui_kittest` draws into a different context than the one given to `new`.

- **Neutral chrome, brand color for focus only.** Panels and widgets are gray. Blue marks
  links, hovered, active, and open widgets, selection, and control labels and section headings.
  At the owner's request, selected controls use the same blue as the preview's carved area,
  `rgb(40, 96, 150)` in dark mode and `rgb(150, 190, 230)` in light mode, from one shared constant.
  Dark mode text uses a lighter tint, `rgb(125, 180, 235)`, because the fill's blue is too dim to
  read on a dark panel; a test enforces 4.5:1 contrast. Warnings and errors use `#FF6600` in dark
  mode and `#B34700` in light mode.
- **Fonts.** Lato Regular leads the proportional family, with egui's fonts behind it as fallbacks.
  Lato Bold is its own family, because `strong()` changes color and not weight. Lucide is another
  family, reached through `icons.rs`. Monospace stays Hack, and text sizes are egui's defaults.
  The files add about 2.3 MB to the binary; Lucide is not subset.
- **Fonts and licenses.** Lato 2.015 is under OFL-1.1, and Lucide (lucide-static 1.67.0) under ISC,
  with MIT for icons Lucide took from Feather. Texts are in `LICENSES/` and beside the fonts, and
  `COPYRIGHT.md` and `REUSE.toml` record them.
- **Stable sizes.** `expansion` is 0 and widget strokes are always 1 pt, so only colors change on
  hover and press.
- **Helpers on `UiExt`:** `option_group` (a bordered frame around segmented options), `compact`
  (4x1 padding, radius 2), `control_label`, `section_heading`, `icon_button`, and `menu_item`.
  Buttons keep their plain text as the accessible name, so the glyph does not leak into it.
- **Warnings and errors are now the same orange.** The status bar tells them apart with an icon (a
  triangle or a circled cross), and the Busy state has the spinner. Beside a field the message and
  its orange label are as before, so an invalid field is no longer distinguished from a warning by
  hue alone. *Open question:* if that proves unclear, give errors a separate hue.
- **Test helper.** The larger button padding pushes some settings below the visible part of the
  scrolling panel in the 1200x900 test window, so the `click` helper in `tests/ui.rs` now scrolls
  to a widget first. The assertions are unchanged.
- **Simulate toolbar wraps** onto a second line when the window is narrow, because the padded
  buttons no longer fit in 760 pixels.

## D17. Application name, bundling, and branches

- **Name.** The window title, the About box, and the macOS application menu say "Rengrave". The
  binaries are `rengrave-cli` and `rengrave` (see D2).
- **macOS bundle.** `cargo bundle --release`, run in `crates/rengrave_gui`, builds `Rengrave.app`
  from `[package.metadata.bundle]`, with the identifier `net.metasim.rengrave`. The bundle has the
  icon from `assets/icon` (also set at runtime, so a plain `cargo run` shows it) and is not signed
  or notarized, so Gatekeeper will ask before it opens. *Open question:*
  signing if the app is shared.
- **Maintenance tasks.** Repository tasks are small Rust programs in the `xtask` crate, run as
  `cargo xtask <task>` through an alias in `.cargo/config.toml`, not shell scripts. Each task is its
  own executable, `xtask-<task>`; `xtask` only forwards to it, so tasks parse their own arguments
  with `clap` and run commands with `xshell`.
- **Universal binaries.** Cargo builds one target at a time, so `cargo xtask build-universal` builds
  `aarch64-apple-darwin` and `x86_64-apple-darwin`, joins the executables with `lipo`, and puts the
  universal executable into the arm64 bundle that `cargo-bundle` made, then signs it ad hoc. It
  does not notarize. `cargo xtask build-dmg` wraps the universal app in a disk image. The minimum is macOS 13 (Ventura), set through `MACOSX_DEPLOYMENT_TARGET` and
  `minimum_system_version`.
- **Branches.** The repository now has a remote, `origin`. `main` holds only the initial commit and
  receives work through pull requests, with `develop` as the integration branch. Nothing from the
  port has reached `main`.

## D18. Desktop interface changes after D16

- **Origin controls.** The work origin is a labeled 3x3 grid, with the Design origin and Circle
  center options below it, all in one thinly stroked panel. The earlier layout did not show what the
  choice was among.
- **Popovers for rarely used controls.** The Preview's **Layers** popover and the Simulate view's
  **Render** popover hold the display options as checkboxes, with the Simulate resolution and relief.
  They replace the Area, Path, Grid, and Smooth toggle buttons and the checkboxes that were in the
  View menu, which now has only Fit design. A popover stays open while its controls are used. The
  display options are not saved between sessions, except for the layer settings stored with the job.
- **G-code view.** The G-code is a read-only text field, so selection, `Ctrl`+`C`, double and triple
  click, select-all, and the context menu come from egui and not from custom code. A first version
  used a custom selection, and was dropped because egui clears a label's selection on any mouse
  press, including the right click that opens the menu. The **Copy** button floats over the text
  rather than taking a row of its own. There is no separate Save G-code button; saving is in the File
  menu.
- **Export.** The SVG, DXF, and DXF-with-closed-loops outputs are grouped under **File ▸ Export**.
  Saving settings and G-code stay directly on the File menu.
- **Image file name.** The name is a read-only field beside the **Open…** button, and the hint
  about dropping a file is that button's tooltip.
- **Bit group.** The bit choice and its profile are in one centered box. The profile drawing is
  filled and hatched to show the cut.

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
`rengrave`), and `rengrave_gui` (binary `rengrave-gui`). The stub root package created by
`cargo new` was replaced by a virtual workspace. Work is on `feature/rust-port`; the
repository had no commits, and no remote is configured, so no PR has been opened. The source
archive `F-Engrave-1.78_src.zip` is ignored rather than committed, since it is reference
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

## D5. Fonts out of the box

The distribution ships no fonts, and without one the program cannot produce anything. The
port embeds the Ubuntu Light and Hack typefaces through the `epaint_default_fonts` crate
(already in the GUI's dependency tree) so that `rengrave gcode --text Hello` works with zero
setup. CXF stroke fonts and TTF/OTF files are loaded the same way as in the original.

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
calculation in place of the modal progress window. *Open question for the owner:* whether **Live
V-carve** should default to on. It is on, because the engine is fast enough for typical designs and
canceling is immediate.

## D10. Defaults

`Settings::default()` keeps the original's defaults (engraving, inches), except for what D13 changes
for FluidNC, so settings files and the command line behave as existing users expect. The desktop application starts on a one-inch V-carve,
because V-carving is the program's main purpose. Settings persist between sessions.

## D11. Features not ported

- The `Plot During V-Carve Calculation` option, which is superseded by the live preview.
- Running as a LinuxCNC AXIS filter (`Write to Axis`).
- `(fengrave_set v_pplot ...)` is accepted but ignored.

## D12. Repository state

The repository had no remote or commits at the start, so there is no pull request. All work is on
the branch `feature/rust-port`. The unborn `master` was never committed to, and the owner's default
branch name, `main`, has not been created.

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

*Open question for the owner:* the spindle speed `S3000` and the inch units and 5 in/min feed are
still the original's defaults. FluidNC accepts them, but they may not suit the machine.

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
- **Top-down shaded view first.** *Open question:* a 3D view could be added later.
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

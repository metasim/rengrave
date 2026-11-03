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
with `ttf-parser` and reproduces the helper's scaling (`A` is 9 units tall) and adaptive
curve flattening. Cubic curves, which the helper replaced with a straight chord, are
flattened properly.

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
- Text: center/right justification did nothing in batch mode, because the last line was only
  measured if followed by a newline. All lines are measured in all modes.
- Fonts: cubic curves were reduced to one chord, and CFF (`.otf`) winding was not normalized.
- Python exceptions such as a zero-length chord in arc fitting are handled as "no arc".
- Multiple adjacent identical settings keys parse by exact name, not substring match, so a value
  for `v_bit_dia` cannot be captured by another key that happens to contain the same text.

The G-code comment header names rengrave instead of F-Engrave, and `gpre`/`gpost` values read
from a settings file no longer gain a trailing space.

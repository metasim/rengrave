# rengrave

Generate engraving and V-carve G-code from text and images, from the command line or a desktop
application.

rengrave is a Rust port of [F-Engrave](https://www.scorchworks.com/Fengrave/fengrave.html) 1.78
by Scorch. The toolpath engine reproduces the original's G-code line for line across a suite of
reference programs. It runs with no helper programs and needs no font files.

![The rengrave desktop application showing a V-carve of the text "Rengrave V-Carve"](docs/images/design.png)

## Features

- **V-carving** with V-bits, ball-nose bits, and straight bits, including multi-pass roughing,
  depth limits, and prismatic cuts for inlays
- **Engraving** of outlines at constant depth
- **Text** in CXF stroke fonts, TrueType and OpenType fonts, or the two built-in typefaces, with
  justification, spacing, rotation, mirroring, and placement on a circle
- **Images**: DXF drawings, plus PNG, BMP, PBM/PGM/PPM, JPEG, GIF, and TIFF bitmaps through a
  built-in tracer or `potrace`
- **Cleanup operations** that clear flat regions with a straight bit and tidy edges with a V-bit
- **Output** as G-code with optional arc fitting (`I`/`J` or `R`), plus SVG and DXF outlines
- **Settings files** compatible with F-Engrave's `fengrave_set` format, including settings
  embedded in its G-code

## Getting started

Building requires a [Rust toolchain](https://rustup.rs) (edition 2024). On Linux, the desktop
application also needs the [usual `eframe` system
libraries](https://github.com/emilk/egui/tree/main/crates/eframe#usage).

```bash
cargo build --release
```

This produces two programs in `target/release`: `rengrave` and `rengrave-gui`.

### Desktop application

```bash
cargo run --release --bin rengrave-gui
```

The window shows the design, its carved area, and the tool path, recalculating after each change.
Choose the text and font on the **Design** tab and the bit on the **Toolpath** tab, then press
**Save G-code…**.

| Action | Control |
| --- | --- |
| Pan | Drag the preview |
| Zoom | Scroll or pinch; `Page Up` / `Page Down` |
| Fit design | Double-click the preview; `Ctrl`+`0` |
| Calculate now | `F5` |
| Stop calculating | `Esc` |
| Save G-code | `Ctrl`+`S` |
| Copy G-code | `Ctrl`+`G` |
| Open image | `Ctrl`+`O`, or drop a file on the window |

A file path on the command line (`rengrave-gui job.ngc`) opens that settings file, font, or image.

### Command line

V-carve some text with a built-in font, in millimeters:

```bash
rengrave gcode --text "Hello" --mode v-carve --units mm --height 25 --bit-dia 12.7 --output hello.ngc
```

V-carve a drawing, centered on the origin:

```bash
rengrave gcode --image logo.dxf --mode v-carve --height 2 --origin mid-center --output logo.ngc
```

Use a font file, and two lines of centered text (`|` starts a new line):

```bash
rengrave gcode --font /path/to/font.ttf --text "One|Two" --justify center --mode v-carve --output two.ngc
```

Write the job's settings, then reuse them:

```bash
rengrave settings --text "Hello" --mode v-carve --output job.cfg
```

```bash
rengrave gcode --settings job.cfg --output hello.ngc
```

The commands are:

| Command | Writes |
| --- | --- |
| `gcode` | The G-code program |
| `cleanup` | The G-code for a cleanup operation (`--tool straight` or `--tool v-bit`) |
| `svg` | The outline as SVG |
| `dxf` | The outline as DXF (`--close-loops` stitches open chains) |
| `settings` | A settings file |
| `fonts` | A list of built-in fonts, and of those in a directory |

Output goes to standard output unless `--output` is given. `rengrave gcode --help` lists every
option. Any setting without its own option can be set by its settings-file name, as in
`--set v_stp_crner=210`.

> **Check before cutting.** Lengths default to inches. Preview a new program in a G-code viewer
> and confirm the units, origin, and depths suit your machine. The default header
> (`G17 G64 P0.001 M3 S3000`) starts the spindle and targets LinuxCNC. Other controllers, such as
> GRBL, may need a different header (`--set "gpre=G17 M3 S10000"`).

## Project layout

| Path | Contents |
| --- | --- |
| [`crates/rengrave_core`](crates/rengrave_core) | The toolpath engine, as a library |
| [`crates/rengrave_cli`](crates/rengrave_cli) | The `rengrave` command |
| [`crates/rengrave_gui`](crates/rengrave_gui) | The `rengrave-gui` application, built with [egui](https://www.egui.rs) |
| [`docs/PORTING.md`](docs/PORTING.md) | How the code maps to the original, and how it was verified |
| [`docs/UX_REVIEW.md`](docs/UX_REVIEW.md) | The design review behind the application's interface |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Decisions made during the port, for review |
| [`tools/parity`](tools/parity) | Regenerates reference programs from the original |

## Using the engine

`rengrave_core` can be used on its own:

```rust
use rengrave_core::{CutType, Job, Progress, Settings};

let settings = Settings {
    text: "Hi".into(),
    cut_type: CutType::VCarve,
    ..Settings::default()
};
let job = Job::load(settings)?;
let gcode: Vec<String> = job.generate_gcode(&Progress::none())?;
# Ok::<(), rengrave_core::Error>(())
```

## Testing

```bash
cargo test --workspace
```

The suite includes unit tests of the computations, line-for-line comparison with the original's
output, end-to-end runs of the command, and tests that drive the whole desktop application through
[`egui_kittest`](https://crates.io/crates/egui_kittest). Screenshots need a GPU, so they run
separately:

```bash
cargo test -p rengrave_gui --features screenshots --test screenshots
```

## License

Licensed under the [GNU General Public License, version 3 or later](LICENSE), as is F-Engrave, of
which this is a derivative work. F-Engrave is copyright Scorch. The built-in typefaces come from
the [`epaint_default_fonts`](https://crates.io/crates/epaint_default_fonts) crate: Ubuntu Light is
under the Ubuntu Font License, and Hack under its own permissive license.

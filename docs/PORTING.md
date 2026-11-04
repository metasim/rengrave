# Porting notes

This document maps `rengrave_core` to the original `f-engrave.py` (version 1.78), and describes how
the port was verified.

## Code map

| Original | Port | Notes |
| --- | --- | --- |
| `parse` | `font::Font::parse_cxf` | |
| `ttf2cxf_stream` (C++ helper) | `font::Font::from_outline_font` | Replaced, not ported: it is GPL-2.0-only. See [COPYRIGHT.md](../COPYRIGHT.md) |
| `parse_dxf`, `DXF_CLASS` | `dxf::DxfDrawing::parse` | |
| `NURBSClass`, `BSplineClass` | `dxf::Nurbs` | Evaluates positions only; derivatives were never used |
| `potrace` (external program) | `trace::Bitmap` | Native tracer, with `Tracer::Potrace` as an option |
| `Read_font_file`, `Read_image_file` | `Job::load` | |
| Tk variables and `Entry_*_Check` | `Settings`, `Settings::validate` | Typed values |
| `calc_vbit_dia`, `calc_depth_limit` | `Settings::effective_bit_dia`, `Settings::max_cut_depth` | |
| `Scale_Linear_Inputs` | `Settings::set_units` | |
| `Open_G_Code_File`, the settings block of `WriteGCode` | `Settings::apply_settings_text`, `Settings::to_settings_lines` | Names match exactly, not by substring |
| `DoIt`, `Rotn` | `Design::layout` | |
| `sort_for_v_carve` | `Segment::sort_into_loops` | |
| `Sort_Paths` | `PathRange::order_loops` | |
| `V_Carve_It`, `find_max_circle` | `CarvePath::calculate`, `Carver` | Grid partitions are stored as indices |
| `Clean_Path_Calc`, `DetectIntersect` | `CleanupPaths::calculate` | With `clipper2-rust` |
| `Gcode`, `douglas` | `gcode::MotionWriter`, `gcode::Fitted::simplify` | Iterative rather than recursive |
| `WriteGCode`, `WRITE_CLEAN_UP` | `GcodeProgram::generate`, `GcodeProgram::generate_cleanup` | |
| `WriteSVG`, `WriteDXF` | `Design::to_svg`, `Design::to_dxf` | |
| `Application` (Tk) | `rengrave_gui` | Redesigned; see [UX_REVIEW.md](UX_REVIEW.md) |
| `-b` batch mode | `rengrave_cli` | |

## Verification

### Comparison with the original

`crates/rengrave_core/tests/parity.rs` compares complete G-code programs with reference output from
the original program. The 35 reference programs exercise:

- Engraving: metric and inch units, plunge rates, `#` variables, boxes and circles, text on circles
  (all placements), rotation, mirroring, origins, arc fitting in both formats, custom headers
- V-carving: V-bits, ball-nose and flat bits, depth limits, multi-pass, flipped normals, boxes,
  mirroring, circular text, prismatic inlays, character-scoped limits, custom corner angles, arc
  fitting
- DXF input: engraving, percent sizing, V-carving with loop sorting, including open chains,
  rotation, and boxes

Every line matches, given two allowances. Trailing whitespace is ignored, because the original
appends a space to header lines read from a settings file. And the engraved circle now names its end
point, as FluidNC requires, which the test undoes before comparing. The tests also set the original's
header, since rengrave's default header targets FluidNC instead. See D13 in
[DECISIONS.md](DECISIONS.md).

To regenerate the references, run the program from the source archive with the stubbed GUI modules
in `tools/parity`:

```bash
python3 tools/parity/generate.py /path/to/F-Engrave-1.78_src/f-engrave.py
```

### Not compared with the original

These run only in the original's GUI, or need software unavailable in its batch mode, so they are
checked by geometric properties and unit tests instead:

- **Cleanup paths.** Clipper 2's vertices differ from Clipper 1's by design. Tests check insets from
  walls and islands, step-over spacing, closure, and that nothing cuts into the design. The
  surrounding logic follows the original closely, but this is the least-proven area of the port;
  check cleanup paths in a viewer before cutting.
- **TrueType layout.** The reference programs use a CXF font, because the original needs its C++
  helper for TrueType. Tests verify scale, closure, and contour direction of `skrifa` outlines.
- **Bitmap tracing.** The native tracer is new code, and it is not the same algorithm as `potrace`.
  Its results are smooth but less refined. `potrace` integration has not been run against a real
  `potrace` binary, which was not available during development.
- **SVG and DXF export.** Compared by structure, and by reading exports back in.
- **Actual cutting.** No program from this port has been run on a machine.

### Other tests

- Unit tests for each algorithm, including the circle-fitting cases of `find_max_circle`, and
  properties such as "a carve circle never crosses the outline" and "simplified paths stay within
  tolerance".
- `crates/rengrave_cli/tests/cli.rs` runs the command end to end.
- `crates/rengrave_gui/tests/ui.rs` drives the whole application through `egui_kittest`: every tab in
  every mode, editing, validation, saving and exporting, dialogs, keyboard shortcuts, panning and
  zooming, cancellation, and the accessibility check on every input.

## Deliberate differences

Where the original has a clear defect, the port does not reproduce it. See entry D8 of
[DECISIONS.md](DECISIONS.md).

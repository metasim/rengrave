# Vendored crates

`warp-gcode`, `warp-sim`, and `warp-tools` come from
[WarpNine](https://github.com/jonahhawk/warpnine) by Jonah Hawk, at commit `30a50b3158d9cd0a1d2b3ec00bd5c03a30883cd1`, under the MIT
license in [LICENSE-warpnine](LICENSE-warpnine). rengrave uses them for the toolpath simulator.

The `src` directories are unmodified, so they can be compared with upstream or updated by copying.
Only the manifests differ: they replace workspace inheritance with explicit values, silence clippy
for code maintained elsewhere, and turn off doctests.

`rustfmt.toml` here keeps `cargo fmt` from reformatting them, and `tests/licensing.rs` in
`rengrave_core` fails if they change.

The reasons for vendoring are in entry D14 of [DECISIONS.md](../docs/DECISIONS.md).

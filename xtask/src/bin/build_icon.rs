// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Renders the application icon source, an SVG, into a macOS icon set and `Rengrave.icns`.
//!
//! Needs `rsvg-convert` (`brew install librsvg`) and `iconutil`, which comes with macOS. The
//! output is next to the SVG, in `crates/rengrave_gui/assets/icon`: `Rengrave.iconset/`,
//! `Rengrave.icns` for the bundle, and `Rengrave.png`, which the application embeds as its window
//! icon.

use anyhow::{Result, bail};
use clap::Parser;
use tracing::info;
use xshell::cmd;
use xtask::Project;

/// The pixel size and file name of each image `iconutil` expects.
const SIZES: [(u32, &str); 10] = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512@2x.png"),
];

#[derive(Parser)]
#[command(
    name = "cargo xtask build-icon",
    about = "Render the application icon (Rengrave.icns) from its SVG source"
)]
struct Args {}

fn main() -> Result<()> {
    Project::init_logging();
    Args::parse();
    Project::require_macos("build-icon")?;
    let project = Project::locate()?;
    let sh = project.shell()?;
    for tool in ["rsvg-convert", "iconutil"] {
        if cmd!(sh, "which {tool}")
            .quiet()
            .ignore_stdout()
            .run()
            .is_err()
        {
            bail!("{tool} is not installed");
        }
    }
    let dir = project.path("crates/rengrave_gui/assets/icon");
    let svg = dir.join("rengrave-icon.svg");
    let set = dir.join("Rengrave.iconset");
    let icns = dir.join("Rengrave.icns");
    sh.remove_path(&set)?;
    sh.create_dir(&set)?;
    for (pixels, name) in SIZES {
        let px = pixels.to_string();
        let file = set.join(name);
        cmd!(
            sh,
            "rsvg-convert --width {px} --height {px} {svg} --output {file}"
        )
        .run()?;
    }
    cmd!(sh, "iconutil --convert icns --output {icns} {set}").run()?;
    info!("Wrote {}", icns.display());
    // The window icon of the application, which is embedded in the executable.
    let png = dir.join("Rengrave.png");
    sh.copy_file(set.join("icon_512x512.png"), &png)?;
    info!("Wrote {}", png.display());
    Ok(())
}

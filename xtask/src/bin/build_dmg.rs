// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Builds `Rengrave.dmg`, a disk image holding the universal `Rengrave.app` and a link to the
//! Applications folder, so it can be dragged there.
//!
//! The image is written to `target/universal/release/Rengrave.dmg`. The app inside is signed
//! ad hoc, so other Macs show a Gatekeeper warning until it is signed with a Developer ID and
//! notarized.

use anyhow::{Context, Result};
use clap::Parser;
use xshell::cmd;
use xtask::{Project, Universal};

#[derive(Parser)]
#[command(
    name = "cargo xtask build-dmg",
    about = "Build a universal Rengrave.dmg disk image"
)]
struct Args {
    #[command(flatten)]
    universal: Universal,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let project = Project::locate()?;
    args.universal.build_binaries(&project)?;
    let app = args.universal.build_app(&project)?;

    let sh = project.shell()?;
    let out = Universal::out_dir(&project);
    let staging = out.join("dmg");
    let dmg = out.join("Rengrave.dmg");
    sh.remove_path(&staging)?;
    sh.create_dir(&staging)?;
    cmd!(sh, "cp -R {app} {staging}").run()?;
    let link = staging.join("Applications");
    cmd!(sh, "ln -s /Applications {link}").run()?;
    sh.remove_path(&dmg)?;
    cmd!(
        sh,
        "hdiutil create -volname Rengrave -srcfolder {staging} -format UDZO -ov {dmg}"
    )
    .run()
    .context("hdiutil could not create the disk image")?;
    sh.remove_path(&staging)?;
    println!("Wrote {}", dmg.display());
    Ok(())
}

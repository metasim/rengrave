// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Builds the release package on macOS: universal binaries, `Rengrave.app`, and `Rengrave.dmg`,
//! all signed with a Developer ID, notarized, stapled, and verified.
//!
//! The results are in `target/universal/release`. Unlike `build-dmg`, a signing identity is
//! required, so a package is never produced that other Macs would reject.

use anyhow::{Result, bail};
use clap::Parser;
use xtask::{Project, Signing, Universal};

#[derive(Parser)]
#[command(
    name = "cargo xtask package",
    about = "Build, sign, and notarize the universal Rengrave.app and Rengrave.dmg (macOS)"
)]
struct Args {
    #[command(flatten)]
    universal: Universal,
    #[command(flatten)]
    signing: Signing,
}

fn main() -> Result<()> {
    let args = Args::parse();
    Project::require_macos("package")?;
    if !args.signing.enabled() {
        bail!("no signing identity; pass --identity or set RENGRAVE_SIGNING_IDENTITY");
    }
    let project = Project::locate()?;
    args.signing.preflight(&project)?;

    args.universal.build_binaries(&project)?;
    let app = args.universal.build_app(&project)?;
    args.signing.sign_app(&project, &app)?;

    let dmg = Universal::build_dmg(&project, &app)?;
    args.signing.sign_dmg(&project, &dmg)?;
    Signing::verify(&project, &dmg)?;

    println!("\nPackaged {} and {}", app.display(), dmg.display());
    Ok(())
}

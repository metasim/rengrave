// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Builds `Rengrave.dmg`, a disk image holding the universal `Rengrave.app` and a link to the
//! Applications folder, so it can be dragged there.
//!
//! The image is written to `target/universal/release/Rengrave.dmg`. By default the app inside is
//! signed ad hoc, so other Macs show a Gatekeeper warning. With `--identity`, the app and the image
//! are signed with that Developer ID, notarized, stapled, and verified.

use anyhow::Result;
use clap::Parser;
use xtask::{Project, Signing, Universal};

#[derive(Parser)]
#[command(
    name = "cargo xtask build-dmg",
    about = "Build a universal Rengrave.dmg disk image"
)]
struct Args {
    #[command(flatten)]
    universal: Universal,
    #[command(flatten)]
    signing: Signing,
}

fn main() -> Result<()> {
    Project::init_logging();
    let args = Args::parse();
    let project = Project::locate()?;
    if args.signing.enabled() {
        args.signing.preflight(&project)?;
    }
    args.universal.build_binaries(&project)?;
    let app = args.universal.build_app(&project)?;
    if args.signing.enabled() {
        args.signing.sign_app(&project, &app)?;
    }
    let dmg = Universal::build_dmg(&project, &app)?;
    if args.signing.enabled() {
        args.signing.sign_dmg(&project, &dmg)?;
        Signing::verify(&project, &dmg)?;
    }
    Ok(())
}

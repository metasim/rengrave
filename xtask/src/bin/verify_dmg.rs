// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Checks that a `.dmg` is signed, notarized, and stapled, and that the app inside passes
//! Gatekeeper. Nothing is signed or changed.

use anyhow::{Result, bail};
use clap::Parser;
use std::path::PathBuf;
use xtask::{Project, Signing, Universal};

#[derive(Parser)]
#[command(
    name = "cargo xtask verify-dmg",
    about = "Verify the signature and notarization of Rengrave.dmg"
)]
struct Args {
    /// The disk image to check. Defaults to the one `build-dmg` writes.
    dmg: Option<PathBuf>,
}

fn main() -> Result<()> {
    Project::init_logging();
    let args = Args::parse();
    Project::require_macos("verify-dmg")?;
    let project = Project::locate()?;
    let dmg = args.dmg.unwrap_or_else(|| Universal::dmg_path(&project));
    if !dmg.exists() {
        bail!(
            "{} does not exist; run `cargo xtask build-dmg` first",
            dmg.display()
        );
    }
    Signing::verify(&project, &dmg)
}

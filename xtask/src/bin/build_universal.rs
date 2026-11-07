// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Builds universal (Apple silicon and Intel) macOS binaries and `Rengrave.app`.
//!
//! The results are in `target/universal/release`: `rengrave`, `rengrave-cli`, and
//! `bundle/Rengrave.app`. Cargo builds one target at a time, so this builds both and joins them
//! with `lipo`.

use anyhow::Result;
use clap::Parser;
use xtask::{Project, Universal};

#[derive(Parser)]
#[command(
    name = "cargo xtask build-universal",
    about = "Build universal (Apple silicon and Intel) macOS binaries and Rengrave.app"
)]
struct Args {
    #[command(flatten)]
    universal: Universal,
    /// Build only the binaries, not Rengrave.app, which needs cargo-bundle
    #[arg(long)]
    no_bundle: bool,
}

fn main() -> Result<()> {
    Project::init_logging();
    let args = Args::parse();
    let project = Project::locate()?;
    args.universal.build_binaries(&project)?;
    if !args.no_bundle {
        args.universal.build_app(&project)?;
    }
    Ok(())
}

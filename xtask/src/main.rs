// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! `cargo xtask <task>`: maintenance tasks for the repository.
//!
//! Each task is its own executable, `xtask-<task>`, built from this package. This program only
//! finds the executable and runs it with the remaining arguments, so `cargo xtask build-dmg
//! --help` shows the help of `build-dmg`. A new task needs a `[[bin]]` named `xtask-<task>`; it
//! works without an entry below, which only makes it appear in `cargo xtask --help`.

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::ffi::OsString;
use xshell::{Shell, cmd};

#[derive(Parser)]
#[command(name = "cargo xtask", version, about)]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

/// The arguments of a task are not interpreted here, including `--help`.
#[derive(Subcommand)]
enum Task {
    /// Build universal (Apple silicon and Intel) macOS binaries and Rengrave.app
    #[command(disable_help_flag = true)]
    BuildUniversal(Forwarded),
    /// Build a universal Rengrave.dmg disk image
    #[command(disable_help_flag = true)]
    BuildDmg(Forwarded),
    /// Render the application icon (Rengrave.icns) from its SVG source
    #[command(disable_help_flag = true)]
    BuildIcon(Forwarded),
    /// Any other `xtask-<task>` executable of this package
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(clap::Args)]
struct Forwarded {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<OsString>,
}

impl Task {
    /// The task's name and the arguments to give its executable.
    fn name_and_args(self) -> Result<(String, Vec<OsString>)> {
        Ok(match self {
            Self::BuildUniversal(f) => ("build-universal".into(), f.args),
            Self::BuildDmg(f) => ("build-dmg".into(), f.args),
            Self::BuildIcon(f) => ("build-icon".into(), f.args),
            Self::External(mut all) => {
                let name = all.remove(0).to_string_lossy().into_owned();
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                if !valid {
                    bail!("{name:?} is not a task name; run `cargo xtask --help` to list them");
                }
                (name, all)
            }
        })
    }
}

fn main() -> Result<()> {
    let (name, args) = Cli::parse().task.name_and_args()?;
    let sh = Shell::new()?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let bin = format!("xtask-{name}");
    // Running through cargo builds the task first when its source has changed.
    cmd!(sh, "{cargo} run --quiet --package xtask --bin {bin} --")
        .args(args)
        .quiet()
        .run()
        .map_err(|e| anyhow::anyhow!("task {name} failed: {e}"))
}

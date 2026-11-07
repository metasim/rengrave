// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Shared by the tasks: where the repository is, and the steps of the macOS release build.

mod signing;

pub use signing::Signing;

use anyhow::{Context, Result, bail};
use clap::Args;
use std::path::{Path, PathBuf};
use tracing::info;
use tracing_subscriber::EnvFilter;
use xshell::{Shell, cmd};

/// The repository the tasks run in.
pub struct Project {
    root: PathBuf,
}

impl Project {
    /// The repository that contains this package.
    pub fn locate() -> Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("the xtask package is not inside a repository")?
            .to_path_buf();
        Ok(Self { root })
    }

    /// A shell that starts in the repository root.
    pub fn shell(&self) -> Result<Shell> {
        let sh = Shell::new()?;
        sh.change_dir(&self.root);
        Ok(sh)
    }

    /// A path relative to the repository root.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Sends `tracing` output to the console in color. Every task calls this first. `RUST_LOG`
    /// overrides the default `info` level.
    pub fn init_logging() {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .without_time()
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .init();
    }

    /// Fails unless running on macOS, where `lipo`, `codesign`, and `hdiutil` exist.
    pub fn require_macos(task: &str) -> Result<()> {
        if !cfg!(target_os = "macos") {
            bail!("{task} can only run on macOS");
        }
        Ok(())
    }
}

/// Options for the universal build.
#[derive(Args, Debug, Clone)]
pub struct Universal {
    /// The oldest macOS the binaries support. 13 is Ventura.
    #[arg(long, env = "MACOSX_DEPLOYMENT_TARGET", default_value = "13.0")]
    pub deployment_target: String,
}

impl Universal {
    const ARM: &'static str = "aarch64-apple-darwin";
    const INTEL: &'static str = "x86_64-apple-darwin";
    const TARGETS: [&'static str; 2] = [Self::ARM, Self::INTEL];

    /// Where the universal results go.
    pub fn out_dir(project: &Project) -> PathBuf {
        project.path("target/universal/release")
    }

    /// Where the universal application bundle goes.
    pub fn app_path(project: &Project) -> PathBuf {
        Self::out_dir(project).join("bundle/Rengrave.app")
    }

    /// Where the disk image goes.
    pub fn dmg_path(project: &Project) -> PathBuf {
        Self::out_dir(project).join("Rengrave.dmg")
    }

    /// Builds both architectures of `rengrave` and `rengrave-cli` and joins each pair.
    pub fn build_binaries(&self, project: &Project) -> Result<()> {
        Project::require_macos("the universal build")?;
        let sh = project.shell()?;
        // Read by the compiler and the linker, so every dependency is built for this version.
        sh.set_var("MACOSX_DEPLOYMENT_TARGET", &self.deployment_target);

        let installed = cmd!(sh, "rustup target list --installed").read()?;
        for target in Self::TARGETS {
            if !installed.lines().any(|l| l.trim() == target) {
                info!("Installing the Rust target {target}");
                cmd!(sh, "rustup target add {target}").run()?;
            }
        }
        for target in Self::TARGETS {
            info!("Building for {target}");
            cmd!(
                sh,
                "cargo build --release --locked --target {target} -p rengrave_gui -p rengrave_cli"
            )
            .run()?;
        }

        let out = Self::out_dir(project);
        sh.create_dir(&out)?;
        for bin in ["rengrave", "rengrave-cli"] {
            let output = out.join(bin);
            let arm = format!("target/{}/release/{bin}", Self::ARM);
            let intel = format!("target/{}/release/{bin}", Self::INTEL);
            cmd!(sh, "lipo -create -output {output} {arm} {intel}").run()?;
            info!("Wrote {}: {}", output.display(), Self::archs(&sh, &output)?);
        }
        Ok(())
    }

    /// Makes the universal `Rengrave.app` from the binaries of [`Self::build_binaries`] and
    /// returns its path.
    pub fn build_app(&self, project: &Project) -> Result<PathBuf> {
        let sh = project.shell()?;
        sh.set_var("MACOSX_DEPLOYMENT_TARGET", &self.deployment_target);
        if cmd!(sh, "cargo bundle --version")
            .quiet()
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .is_err()
        {
            bail!("cargo-bundle is not installed; run `cargo install cargo-bundle`");
        }
        // Bundle the arm64 build for the layout and Info.plist, then swap in the universal
        // executable.
        {
            let _gui = sh.push_dir(project.path("crates/rengrave_gui"));
            let arm = Self::ARM;
            cmd!(sh, "cargo bundle --release --format osx --target {arm}").run()?;
        }
        let built = project.path(&format!(
            "target/{}/release/bundle/osx/Rengrave.app",
            Self::ARM
        ));
        let app = Self::app_path(project);
        sh.remove_path(&app)?;
        sh.create_dir(app.parent().context("the app has a parent directory")?)?;
        cmd!(sh, "cp -R {built} {app}").run()?;
        let executable = app.join("Contents/MacOS/rengrave");
        sh.copy_file(Self::out_dir(project).join("rengrave"), &executable)?;
        // Replacing the executable invalidates its signature; an ad-hoc one lets it run locally.
        cmd!(sh, "codesign --force --deep --sign - {app}").run()?;
        info!(
            "Wrote {}: {}",
            app.display(),
            Self::archs(&sh, &executable)?
        );
        Ok(app)
    }

    /// Makes `Rengrave.dmg` from `app` and a link to the Applications folder, and returns its path.
    pub fn build_dmg(project: &Project, app: &Path) -> Result<PathBuf> {
        let sh = project.shell()?;
        let staging = Self::out_dir(project).join("dmg");
        let dmg = Self::dmg_path(project);
        sh.remove_path(&staging)?;
        sh.create_dir(&staging)?;
        // `ditto` keeps the attributes and stapled ticket that `cp -R` can lose.
        let staged_app = staging.join("Rengrave.app");
        cmd!(sh, "ditto {app} {staged_app}").run()?;
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
        info!("Wrote {}", dmg.display());
        Ok(dmg)
    }

    fn archs(sh: &Shell, file: &Path) -> Result<String> {
        Ok(cmd!(sh, "lipo -archs {file}").read()?)
    }
}

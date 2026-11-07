// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Developer ID signing, notarization, and verification of the macOS release artifacts.

use crate::Project;
use anyhow::{Context, Result, bail};
use clap::Args;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use xshell::{Cmd, Shell, cmd};

/// Mach-O magic numbers: 32/64-bit thin (either byte order) and fat/universal.
const MACHO_MAGICS: [[u8; 4]; 5] = [
    [0xCF, 0xFA, 0xED, 0xFE],
    [0xCE, 0xFA, 0xED, 0xFE],
    [0xFE, 0xED, 0xFA, 0xCF],
    [0xFE, 0xED, 0xFA, 0xCE],
    [0xCA, 0xFE, 0xBA, 0xBE],
];

/// Directory suffixes that are signable bundles in their own right.
const NESTED_BUNDLE_SUFFIXES: [&str; 5] = [".framework", ".app", ".xpc", ".appex", ".bundle"];

/// Options for signing with a Developer ID certificate and notarizing with Apple.
///
/// Without `--identity` nothing is signed beyond the ad-hoc signature, and nothing is notarized.
#[derive(Args, Debug, Clone)]
pub struct Signing {
    /// Keychain identity to sign with, e.g. "Developer ID Application: Name (TEAMID)". Giving it
    /// also notarizes and staples the app and the disk image.
    #[arg(long, env = "RENGRAVE_SIGNING_IDENTITY")]
    pub identity: Option<String>,

    /// `notarytool` keychain profile holding the App Store Connect credentials.
    #[arg(
        long,
        env = "RENGRAVE_NOTARY_PROFILE",
        default_value = "notary-profile"
    )]
    pub keychain_profile: String,

    /// Entitlements plist to apply to the app. Request only what the app needs.
    #[arg(long)]
    pub entitlements: Option<PathBuf>,
}

/// A mounted disk image, detached on drop.
struct Mount(PathBuf);

impl Mount {
    /// Attaches `dmg` read-only and unlisted at `mountpoint`.
    fn attach(sh: &Shell, dmg: &Path, mountpoint: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&mountpoint)?;
        cmd!(
            sh,
            "hdiutil attach -readonly -nobrowse -quiet -mountpoint {mountpoint} {dmg}"
        )
        .run()
        .with_context(|| format!("could not mount {}", dmg.display()))?;
        Ok(Self(mountpoint))
    }

    /// The `.app` at the top level of the image.
    fn app(&self) -> Result<PathBuf> {
        for entry in std::fs::read_dir(&self.0)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "app") {
                return Ok(path);
            }
        }
        bail!("no .app inside the .dmg")
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        let _ = std::process::Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.0)
            .status();
    }
}

impl Signing {
    /// Whether to sign and notarize.
    pub fn enabled(&self) -> bool {
        self.identity.is_some()
    }

    fn identity(&self) -> Result<&str> {
        self.identity
            .as_deref()
            .context("no signing identity; pass --identity")
    }

    fn work_dir(project: &Project) -> Result<PathBuf> {
        let dir = project.path("target/signing");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Fails unless the identity is in the keychain and the notary profile works, so a missing
    /// credential is found before the long build rather than after it.
    pub fn preflight(&self, project: &Project) -> Result<()> {
        let sh = project.shell()?;
        let identity = self.identity()?;
        let listing = cmd!(sh, "security find-identity -v -p codesigning").read()?;
        if !listing.contains(identity) {
            bail!("signing identity `{identity}` is not in your keychain:\n{listing}");
        }
        let profile = &self.keychain_profile;
        let check = cmd!(sh, "xcrun notarytool history --keychain-profile {profile}")
            .ignore_status()
            .output()?;
        if !check.status.success() {
            bail!(
                "no usable notarytool profile `{profile}`. Create it once with:\n\n  \
                 xcrun notarytool store-credentials {profile} \
                 --apple-id <your Apple ID> --team-id <your team ID>\n\n\
                 (it prompts for an app-specific password; see \
                 https://support.apple.com/102654)\n\n{}",
                String::from_utf8_lossy(&check.stderr)
            );
        }
        Ok(())
    }

    /// Signs `app` inside-out with the hardened runtime, notarizes it, and staples the ticket.
    pub fn sign_app(&self, project: &Project, app: &Path) -> Result<()> {
        let sh = project.shell()?;
        println!("Signing {} (inside-out, hardened runtime)", app.display());
        let main_exe = Self::main_executable(&sh, app)?;
        let mut nested = Vec::new();
        Self::collect_nested(app, app, &main_exe, &mut nested)?;
        nested.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        for path in &nested {
            self.codesign(&sh, path, None)?;
        }
        self.codesign(&sh, app, self.entitlements.as_deref())?;
        cmd!(sh, "codesign --verify --deep --strict --verbose=2 {app}").run()?;

        println!("Notarizing {}", app.display());
        let zip = Self::work_dir(project)?.join("Rengrave-app.zip");
        cmd!(sh, "ditto -c -k --keepParent {app} {zip}").run()?;
        self.notarize(&sh, &zip)?;
        cmd!(sh, "xcrun stapler staple {app}").run()?;
        Ok(())
    }

    /// Signs `dmg`, notarizes it, and staples the ticket.
    pub fn sign_dmg(&self, project: &Project, dmg: &Path) -> Result<()> {
        let sh = project.shell()?;
        let identity = self.identity()?;
        println!("Signing {}", dmg.display());
        cmd!(sh, "codesign --force --timestamp --sign {identity} {dmg}").run()?;
        println!("Notarizing {}", dmg.display());
        self.notarize(&sh, dmg)?;
        cmd!(sh, "xcrun stapler staple {dmg}").run()?;
        Ok(())
    }

    fn codesign(&self, sh: &Shell, path: &Path, entitlements: Option<&Path>) -> Result<()> {
        let identity = self.identity()?;
        let entitlements = entitlements.map(|e| ["--entitlements".as_ref(), e.as_os_str()]);
        let entitlements = entitlements.iter().flatten();
        cmd!(
            sh,
            "codesign --force --options runtime --timestamp {entitlements...} --sign {identity} {path}"
        )
        .run()
        .with_context(|| format!("signing {}", path.display()))
    }

    /// Submits `file` and waits; on anything but `Accepted`, prints Apple's log and fails.
    fn notarize(&self, sh: &Shell, file: &Path) -> Result<()> {
        let profile = &self.keychain_profile;
        let out = cmd!(
            sh,
            "xcrun notarytool submit {file} --keychain-profile {profile} --wait"
        )
        .ignore_status()
        .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        println!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
        if out.status.success() && stdout.contains("status: Accepted") {
            return Ok(());
        }
        if let Some(id) = stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix("id: "))
            .map(str::to_owned)
        {
            let _ = cmd!(sh, "xcrun notarytool log {id} --keychain-profile {profile}").run();
        }
        bail!("notarization of {} did not succeed", file.display())
    }

    /// The bundle's `CFBundleExecutable`, via `plutil`.
    fn main_executable(sh: &Shell, app: &Path) -> Result<PathBuf> {
        let plist = app.join("Contents/Info.plist");
        let name = cmd!(sh, "plutil -extract CFBundleExecutable raw -o - {plist}").read()?;
        Ok(app.join("Contents/MacOS").join(name.trim()))
    }

    /// Collects the code inside `dir` that needs its own signature, other than `main_exe`.
    fn collect_nested(
        root: &Path,
        dir: &Path,
        main_exe: &Path,
        out: &mut Vec<PathBuf>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let meta = path.symlink_metadata()?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                Self::collect_nested(root, &path, main_exe, out)?;
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if path != root && NESTED_BUNDLE_SUFFIXES.iter().any(|s| name.ends_with(s)) {
                    out.push(path);
                }
            } else if path != main_exe && Self::is_macho(&path) {
                out.push(path);
            }
        }
        Ok(())
    }

    fn is_macho(path: &Path) -> bool {
        let mut magic = [0u8; 4];
        File::open(path)
            .and_then(|mut f| f.read_exact(&mut magic))
            .is_ok_and(|()| MACHO_MAGICS.contains(&magic))
    }

    /// Verifies `dmg` and the `.app` inside it (stapled ticket, Gatekeeper, signature), printing
    /// each result. Runs every check even after a failure, then errors if any failed.
    pub fn verify(project: &Project, dmg: &Path) -> Result<()> {
        let sh = project.shell()?;
        println!("Checking {}\n", dmg.display());
        let mut failures = Vec::new();

        Self::step(
            &mut failures,
            "dmg: notarization ticket stapled",
            cmd!(sh, "xcrun stapler validate {dmg}"),
            &[],
            &[],
        );
        Self::step(
            &mut failures,
            "dmg: Gatekeeper assessment",
            cmd!(
                sh,
                "spctl -a -vvv -t open --context context:primary-signature {dmg}"
            ),
            &[],
            &["accepted", "Notarized Developer ID"],
        );
        Self::step(
            &mut failures,
            "dmg: signature",
            cmd!(sh, "codesign -dvv {dmg}"),
            &["Authority", "TeamIdentifier", "Timestamp"],
            &["Developer ID Application", "Timestamp"],
        );

        let mountpoint = Self::work_dir(project)?.join("check-mnt");
        match Mount::attach(&sh, dmg, mountpoint).and_then(|m| m.app().map(|app| (m, app))) {
            Ok((_mount, app)) => {
                Self::step(
                    &mut failures,
                    "app: notarization ticket stapled",
                    cmd!(sh, "xcrun stapler validate {app}"),
                    &[],
                    &[],
                );
                Self::step(
                    &mut failures,
                    "app: Gatekeeper assessment",
                    cmd!(sh, "spctl -a -vvv -t exec {app}"),
                    &[],
                    &["accepted", "Notarized Developer ID"],
                );
                Self::step(
                    &mut failures,
                    "app: signature and hardened runtime",
                    cmd!(sh, "codesign -dvv {app}"),
                    &["Authority", "TeamIdentifier", "Timestamp", "flags"],
                    &["Developer ID Application", "Timestamp", "(runtime)"],
                );
                Self::step(
                    &mut failures,
                    "app: signature intact",
                    cmd!(sh, "codesign --verify --deep --strict {app}"),
                    &[],
                    &[],
                );
            }
            Err(e) => failures.push(format!("app: {e:#}")),
        }

        if failures.is_empty() {
            println!("All checks passed.");
            Ok(())
        } else {
            bail!(
                "{} check(s) failed:\n  {}",
                failures.len(),
                failures.join("\n  ")
            )
        }
    }

    /// Runs one check command and prints its output, keeping only lines that contain one of
    /// `show` (all lines if empty). Fails if the command fails or its full output lacks any of
    /// `require`.
    fn step(
        failures: &mut Vec<String>,
        label: &str,
        command: Cmd<'_>,
        show: &[&str],
        require: &[&str],
    ) {
        println!("--- {label}");
        let out = match command.ignore_status().output() {
            Ok(out) => out,
            Err(e) => {
                println!("  could not run: {e}\n");
                failures.push(label.to_owned());
                return;
            }
        };
        // `spctl` and `codesign` write their findings to stderr.
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for line in text.lines() {
            if show.is_empty() || show.iter().any(|s| line.contains(s)) {
                println!("  {line}");
            }
        }
        let missing: Vec<_> = require.iter().filter(|r| !text.contains(**r)).collect();
        if out.status.success() && missing.is_empty() {
            println!("  PASS\n");
        } else {
            println!(
                "  FAIL (exit {:?}, missing {missing:?})\n",
                out.status.code()
            );
            failures.push(label.to_owned());
        }
    }
}

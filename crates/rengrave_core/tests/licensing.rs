// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Keeps the per-file copyright notices and the inventory in `COPYRIGHT.md` in step.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    let root = root().canonicalize().unwrap();
    let path = path.canonicalize().unwrap();
    path.strip_prefix(&root).unwrap().display().to_string()
}

#[test]
fn every_source_file_states_its_license_and_copyright() {
    let mut files = Vec::new();
    rust_files(&root().join("crates"), &mut files);
    assert!(files.len() > 25);
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let head: Vec<&str> = text.lines().take(3).collect();
        assert_eq!(
            head[0],
            "// SPDX-License-Identifier: GPL-3.0-or-later",
            "{}",
            file.display()
        );
        assert!(
            text.contains("SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch"),
            "{}",
            file.display()
        );
    }
}

#[test]
fn inventory_lists_exactly_the_files_credited_to_f_engrave() {
    let inventory = std::fs::read_to_string(root().join("COPYRIGHT.md")).unwrap();
    let mut files = Vec::new();
    rust_files(&root().join("crates"), &mut files);
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        // Only look at the header, since this test's own source mentions the name.
        let header: String = text
            .lines()
            .take_while(|l| l.starts_with("//") && !l.starts_with("//!"))
            .collect();
        let name = relative(&file);
        let credited = header.contains("Scorch");
        let listed = inventory.contains(&format!("| `{name}` |"));
        assert_eq!(credited, listed, "{name}: header and COPYRIGHT.md disagree");
        assert_eq!(credited, header.contains("Ported from"), "{name}");
    }
}

#[test]
fn vendored_crates_keep_their_own_license() {
    let vendor = root().join("vendor");
    let license = std::fs::read_to_string(vendor.join("LICENSE-warpnine")).unwrap();
    assert!(license.starts_with("MIT License") && license.contains("Jonah Hawk"));
    for name in ["warp-gcode", "warp-sim", "warp-tools"] {
        let manifest = std::fs::read_to_string(vendor.join(name).join("Cargo.toml")).unwrap();
        assert!(manifest.contains("license = \"MIT\""), "{name}");
        let lib = std::fs::read_to_string(vendor.join(name).join("src/lib.rs")).unwrap();
        assert!(
            !lib.contains("SPDX"),
            "{name} must stay identical to upstream"
        );
    }
    let reuse = std::fs::read_to_string(root().join("REUSE.toml")).unwrap();
    assert!(reuse.contains("vendor/**") && reuse.contains("MIT"));
}

/// FNV-1a, which is simple enough to compute anywhere for comparing with upstream.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn vendored_sources_are_unmodified() {
    // Update these only when deliberately copying a new upstream commit into `vendor/`.
    const EXPECTED: [(&str, u64); 5] = [
        ("warp-gcode/src/interp.rs", 0x73aaae34b9d22f36),
        ("warp-gcode/src/lib.rs", 0x6907019c076e0681),
        ("warp-gcode/src/parse.rs", 0x5832909b9d3b075e),
        ("warp-sim/src/lib.rs", 0x276e7635634dbc99),
        ("warp-tools/src/lib.rs", 0x87e318fd7016404b),
    ];
    for (file, expected) in EXPECTED {
        let bytes = std::fs::read(root().join("vendor").join(file)).unwrap();
        assert_eq!(fnv1a(&bytes), expected, "{file} differs from upstream");
    }
}

#[test]
fn gpl_2_only_helper_is_not_part_of_the_program() {
    let cargo = std::fs::read_to_string(root().join("Cargo.lock")).unwrap();
    assert!(!cargo.contains("freetype"));
    let font = std::fs::read_to_string(root().join("crates/rengrave_core/src/font.rs")).unwrap();
    assert!(font.contains("independent of the GPL-2.0-only ttf2cxf_stream helper"));
}

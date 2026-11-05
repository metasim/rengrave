#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
#
# Builds universal (arm64 + x86_64) macOS binaries and, unless --no-bundle is given, a universal
# Rengrave.app. Cargo builds one target at a time, so this builds both and joins them with lipo.
#
# Output, under target/universal/release:
#   rengrave, rengrave-cli       universal binaries
#   bundle/Rengrave.app          universal application bundle (needs cargo-bundle)
#
# Set MACOSX_DEPLOYMENT_TARGET to choose the oldest macOS the binaries support.

set -euo pipefail

cd "$(dirname "$0")/.."

ARM=aarch64-apple-darwin
INTEL=x86_64-apple-darwin
OUT=target/universal/release
BUNDLE=true

for arg in "$@"; do
    case "$arg" in
        --no-bundle) BUNDLE=false ;;
        -h | --help)
            sed -n '4,13p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

if [[ "$(uname -s)" != Darwin ]]; then
    echo "universal macOS binaries can only be built on macOS" >&2
    exit 1
fi

for target in "$ARM" "$INTEL"; do
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "Installing the Rust target $target"
        rustup target add "$target"
    fi
done

for target in "$ARM" "$INTEL"; do
    echo "Building for $target"
    cargo build --release --locked --target "$target" -p rengrave_gui -p rengrave_cli
done

mkdir -p "$OUT"
for bin in rengrave rengrave-cli; do
    lipo -create -output "$OUT/$bin" \
        "target/$ARM/release/$bin" "target/$INTEL/release/$bin"
    echo "Wrote $OUT/$bin: $(lipo -archs "$OUT/$bin")"
done

if [[ "$BUNDLE" == true ]]; then
    if ! command -v cargo-bundle > /dev/null; then
        echo "cargo-bundle is not installed; run 'cargo install cargo-bundle', or use --no-bundle" >&2
        exit 1
    fi
    # Bundle the arm64 build for the layout and Info.plist, then swap in the universal binary.
    (cd crates/rengrave_gui && cargo bundle --release --format osx --target "$ARM")
    app="target/$ARM/release/bundle/osx/Rengrave.app"
    rm -rf "$OUT/bundle/Rengrave.app"
    mkdir -p "$OUT/bundle"
    cp -R "$app" "$OUT/bundle/Rengrave.app"
    cp "$OUT/rengrave" "$OUT/bundle/Rengrave.app/Contents/MacOS/rengrave"
    # Replacing the executable invalidates its signature; an ad-hoc one lets it run locally.
    codesign --force --deep --sign - "$OUT/bundle/Rengrave.app"
    echo "Wrote $OUT/bundle/Rengrave.app: $(lipo -archs "$OUT/bundle/Rengrave.app/Contents/MacOS/rengrave")"
fi

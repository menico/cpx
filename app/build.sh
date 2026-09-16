#!/usr/bin/env bash
# Assemble cpx.app from the Swift build and the Rust command.
#
#   app/build.sh <version> <path-to-cpx-binary> [--universal]
#
# The app drives the `cpx` command, so the command ships inside the bundle
# next to the app's own executable and is found there first.
set -euo pipefail

VERSION="${1:?version}"
CLI="${2:?path to the cpx binary}"
UNIVERSAL="${3:-}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLI="$(cd "$(dirname "$CLI")" && pwd)/$(basename "$CLI")"
OUT="$HERE/../target/app"
BUNDLE="$OUT/cpx.app"

cd "$HERE"
if [[ "$UNIVERSAL" == "--universal" ]]; then
  swift build -c release --arch arm64 --arch x86_64 >&2
  BIN="$(swift build -c release --arch arm64 --arch x86_64 --show-bin-path)"
else
  swift build -c release >&2
  BIN="$(swift build -c release --show-bin-path)"
fi

rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp "$BIN/cpx-app" "$BUNDLE/Contents/MacOS/cpx-app"
cp "$CLI" "$BUNDLE/Contents/MacOS/cpx"
chmod 755 "$BUNDLE/Contents/MacOS/cpx-app" "$BUNDLE/Contents/MacOS/cpx"
cp "$HERE/Sources/CpxApp/Resources/cpx.icns" "$BUNDLE/Contents/Resources/cpx.icns"
# SwiftPM puts resources in a bundle of its own; ship it so they resolve.
if [[ -d "$BIN/cpx_CpxApp.bundle" ]]; then
  cp -R "$BIN/cpx_CpxApp.bundle" "$BUNDLE/Contents/Resources/"
fi
sed "s/__VERSION__/$VERSION/g" "$HERE/Bundle/Info.plist" > "$BUNDLE/Contents/Info.plist"
printf 'APPL????' > "$BUNDLE/Contents/PkgInfo"

# Ad-hoc signing is enough for the machine that built it. A build meant for
# anyone else needs a Developer ID and notarisation.
codesign --force --deep --sign - "$BUNDLE" >&2
echo "$BUNDLE"

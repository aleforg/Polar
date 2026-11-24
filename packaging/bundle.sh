#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

profile=${1:-release}
case "$profile" in
    debug) cargo build --locked ;;
    release) cargo build --locked --release ;;
    *) printf 'Usage: %s [debug|release]\n' "$0" >&2; exit 2 ;;
esac

staging=$(mktemp -d "$PWD/target/polar.XXXXXX")
trap 'rm -rf "$staging"' EXIT
bundle="$staging/Polar.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp packaging/Info.plist "$bundle/Contents/Info.plist"
cp vendor/mathjax/LICENSE "$bundle/Contents/Resources/MathJax-LICENSE"
cp "target/$profile/polar" "$bundle/Contents/MacOS/polar"

magick -background none packaging/icon.svg "PNG32:$staging/icon.png"
python3 - "$staging/icon.png" "$bundle/Contents/Resources/Polar.icns" <<'PYICON'
from pathlib import Path
from struct import pack
from sys import argv

image = Path(argv[1]).read_bytes()
record = pack(">4sI", b"ic10", 8 + len(image)) + image
Path(argv[2]).write_bytes(pack(">4sI", b"icns", 8 + len(record)) + record)
PYICON

sdk_version=$(xcrun --sdk macosx --show-sdk-version)
if [ "${sdk_version%%.*}" -ge 27 ]; then
    helper="$bundle/Contents/Helpers/polar-note-revision"
    mkdir -p "$bundle/Contents/Helpers"
    xcrun --sdk macosx swiftc -O -parse-as-library -swift-version 6 \
        -target arm64-apple-macosx26.4 -module-cache-path "$staging/swift-cache" \
        packaging/NoteRevision.swift -o "$helper"
    codesign --force --sign - "$helper"
    codesign --verify --strict "$helper"
else
    printf '%s\n' 'Note revision requires the macOS 27 SDK to build its helper.' >&2
fi

codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
rm -rf target/Polar.app
mv "$bundle" target/Polar.app
printf '%s\n' "$PWD/target/Polar.app"

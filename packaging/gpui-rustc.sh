#!/bin/sh
set -eu

compiler=$1
shift
if [ "${1:-}" = --crate-name ] && [ "${2:-}" = gpui_macos ]; then
    # shortcut: GPUI 0.3.11 uses this unstable std method, remove when upgrading to a stable-compatible release.
    RUSTC_BOOTSTRAP=gpui_macos exec "$compiler" "$@" '-Zcrate-attr=feature(result_option_map_or_default)'
fi
exec "$compiler" "$@"

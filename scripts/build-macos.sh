#!/bin/sh
# Native build on Apple Silicon or Intel. Pass --release for an optimized release.
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "Run this script on macOS." >&2
    exit 1
fi
cargo build --locked -p skate-game --bin skate3rust --no-default-features "$@"

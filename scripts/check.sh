#!/bin/sh
# The checks CI runs, in the same order, for running them locally first.
# Building cosmic-ext-dimmer needs libxkbcommon-dev and libwayland-dev
# (see docs/workspace.md); everything else is pure Rust.
set -eu

cd "$(dirname "$0")/.."

# cosmic-applets keeps rustfmt's defaults (it has no rustfmt.toml); so do we.
cargo fmt --all -- --check

# cosmic-applets CI: clippy over all targets and features. Its `just check`
# adds clippy::pedantic, which the workspace lints enable; -D warnings makes
# any of them fail the run.
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

cargo test --locked --workspace

# Desktop entries use Categories=COSMIC; as the stock applets do, which only
# desktop-file-utils 0.28+ registers. CI validates with 0.28; say so plainly
# here instead of passing silently on an older validator.
dfu=$(desktop-file-validate --version 2>/dev/null | awk '{print $NF}') || dfu=
case "$dfu" in
    0.2[89]* | 0.[3-9]* | [1-9]*)
        find crates -name '*.desktop' -exec desktop-file-validate {} + ;;
    *)
        echo "SKIPPED desktop entry validation: needs desktop-file-utils >= 0.28" \
            "(found: ${dfu:-none}); CI's desktop-files job runs it." >&2 ;;
esac

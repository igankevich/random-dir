#!/bin/sh

. ./ci/preamble.sh

git config --global --add safe.directory "$PWD"
cargo fmt --workspace --check
cargo clippy --workspace --all-targets --all-features --quiet -- -D warnings

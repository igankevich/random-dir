#!/bin/sh

. ./ci/preamble.sh

git config --global --add safe.directory "$PWD"
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --quiet -- -D warnings

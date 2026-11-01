#!/usr/bin/env sh
set -eu

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --quiet -p searvorn-cli -- --root . stat Cargo.toml >/dev/null
cargo run --quiet -p searvorn-bench -- 1 >/dev/null

#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../../.."
closure_logs=${1:-docs/validation/2026-10-31/performance-closure}
mkdir -p "$closure_logs"
cargo fmt --all -- --check > "$closure_logs/fmt.log" 2>&1
cargo check --workspace --all-targets --all-features --locked > "$closure_logs/check.log" 2>&1
cargo test --workspace --all-features --locked > "$closure_logs/tests.log" 2>&1
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings > "$closure_logs/clippy.log" 2>&1
: > "$closure_logs/features.log"
for features in '' eventhub scheduler runtime config-toml observability; do
    cargo check -p modorch --no-default-features --features "$features" --locked >> "$closure_logs/features.log" 2>&1
done
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked > "$closure_logs/rustdoc.log" 2>&1
: > "$closure_logs/examples.log"
for example in minimal-app stateful-initiator multi-module; do
    cargo run -p "$example" --locked >> "$closure_logs/examples.log" 2>&1
done
cargo run -p graceful-http --locked -- --once >> "$closure_logs/examples.log" 2>&1
: > "$closure_logs/load-smoke.log"
for mode in serial partitioned concurrent dynamic; do
    cargo run -p runtime-load --locked -- --mode "$mode" --messages 100 --producers 8 --capacity 8 >> "$closure_logs/load-smoke.log" 2>&1
done
cargo run --manifest-path tests/fixtures/external-app/Cargo.toml --locked > "$closure_logs/external.log" 2>&1
cargo fmt --manifest-path tests/fixtures/external-app/Cargo.toml -- --check >> "$closure_logs/external.log" 2>&1
cargo +1.85.0 check --workspace --all-targets --all-features --locked > "$closure_logs/msrv.log" 2>&1

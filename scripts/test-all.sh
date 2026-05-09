#!/usr/bin/env bash
set -euo pipefail
echo "== fmt ==" && cargo fmt --all -- --check
echo "== clippy ==" && cargo clippy --all-targets --all-features -- -D warnings
echo "== tests ==" && cargo test --all-features
echo "== release tests ==" && cargo test --release --all-features
echo "== audit ==" && cargo audit
echo "== deny ==" && cargo deny check
echo "== docs ==" && cargo doc --no-deps --all-features
echo "== release build ==" && cargo build --release --locked
echo "All checks passed."

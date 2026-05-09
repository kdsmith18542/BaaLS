#!/usr/bin/env pwsh
$ErrorActionPreference = "Stop"
Write-Host "== fmt =="
cargo fmt --all -- --check
if ($?) { Write-Host "== clippy ==" } else { exit 1 }
cargo clippy --all-targets --all-features -- -D warnings
if ($?) { Write-Host "== tests ==" } else { exit 1 }
cargo test --all-features
if ($?) { Write-Host "== release tests ==" } else { exit 1 }
cargo test --release --all-features
if ($?) { Write-Host "== audit ==" } else { exit 1 }
cargo audit
if ($?) { Write-Host "== deny ==" } else { exit 1 }
cargo deny check
if ($?) { Write-Host "== docs ==" } else { exit 1 }
cargo doc --no-deps --all-features
if ($?) { Write-Host "== release build ==" } else { exit 1 }
cargo build --release --locked
Write-Host "All checks passed."

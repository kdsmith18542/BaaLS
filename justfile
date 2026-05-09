check:
    cargo check --all-targets

check-all:
    cargo check --all-targets --all-features

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --all-features

test-all:
    cargo test --all-features

test-release:
    cargo test --release --all-features

audit:
    cargo audit

deny:
    cargo deny check

bench:
    cargo bench

docs:
    cargo doc --no-deps --all-features

ci: fmt-check clippy test audit deny

build:
    cargo build

release:
    cargo build --release --locked

run:
    cargo run -- node start --data-dir ./data

clean-data:
    rm -rf ./data

miri:
    cargo +nightly miri test --lib

outdated:
    cargo outdated

machete:
    cargo machete

hygiene: outdated machete

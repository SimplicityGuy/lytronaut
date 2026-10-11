default:
    @just --list

setup:
    rustup component add rustfmt clippy
    rustup target add wasm32-unknown-unknown
    cargo fetch --locked

# Beadhive runs this bare from clean checkouts; enter the pinned environment here.
check:
    mise exec -- just fmt lint test build wasm-build dependencies licenses

fmt:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    cargo clippy --locked -p lytronaut-client --target wasm32-unknown-unknown -- -D warnings

test:
    cargo test --locked --workspace --all-targets --all-features
    cargo test --locked --workspace --doc

build:
    cargo build --locked --workspace --all-targets --all-features

wasm-build:
    cargo check --locked -p lytronaut-format --target wasm32-unknown-unknown
    cargo build --locked -p lytronaut-client --target wasm32-unknown-unknown
    test "$(wasm-bindgen --version)" = "wasm-bindgen 0.2.129"
    wasm-bindgen --target web --out-dir dist "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/debug/lytronaut_client.wasm"

dependencies:
    cargo run --locked -p xtask -- boundaries

licenses:
    cargo run --locked -p xtask -- licenses

licenses-update:
    cargo run --locked -p xtask -- licenses --write

serve:
    cargo run --locked -p lytronaut-service

format:
    cargo fmt --all

security-scan:
    gitleaks git --redact
    gitleaks dir --redact --config .gitleaks.toml .

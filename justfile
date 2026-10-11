default:
    @just --list

setup:
    rustup component add rustfmt clippy
    cargo fetch --locked

check: fmt lint test build

fmt:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

test:
    cargo test --locked --all-targets --all-features
    cargo test --locked --doc

build:
    cargo build --locked --all-targets --all-features

format:
    cargo fmt --all

security-scan:
    gitleaks git --redact
    gitleaks dir --redact --config .gitleaks.toml .

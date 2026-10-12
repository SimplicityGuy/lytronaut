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

# Explicit disposable PostgreSQL only; the default SQLite test recipe stays independent.
# Requires a restricted role, CONNECT+CREATE on its dedicated DB, and the synthetic
# public sentinel described in docs/design/006-database-boundary.md.
postgres-test:
    python3 scripts/postgres-test.py

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

serve: wasm-build
    cargo run --locked -p lytronaut-service

format:
    cargo fmt --all

security-scan:
    gitleaks git --redact
    gitleaks dir --redact --config .gitleaks.toml .

# Synthetic-only actual service+Rust/WASM browser integration.
browser-test:
    npm ci --ignore-scripts --prefix scripts/browser
    bash scripts/browser-test.sh

# Actual platform codec acceptance using independently generated AVC pixels.
video-test:
    npm ci --ignore-scripts --prefix scripts/browser
    node scripts/video-test.cjs

# Explicit local Linux image builds; never publish to a registry.
container-build architecture image relay_image revision:
    docker build --platform "linux/{{architecture}}" --tag "{{image}}" --build-arg "SOURCE_REVISION={{revision}}" --progress=plain .
    docker build --platform "linux/{{architecture}}" --tag "{{relay_image}}" --file scripts/Dockerfile.smoke-relay scripts

# Freeze the products from the same cached build before the runtime proof.
container-artifacts architecture image output:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{output}}"
    docker build --platform "linux/{{architecture}}" --target validation-artifacts --output "type=local,dest={{output}}/programs" .
    docker image save "{{image}}" | gzip -1 > "{{output}}/image.tar.gz"

container-smoke architecture image relay_image revision output:
    mkdir -p "{{output}}"
    LYTRONAUT_TEST_REVISION="{{revision}}" LYTRONAUT_SMOKE_RELAY_IMAGE="{{relay_image}}" python3 scripts/container-smoke.py "{{image}}" "linux/{{architecture}}" > "{{output}}/runtime.json"

# Requires an actual native Linux runner and checks saved image/layer identity.
container-evidence architecture revision output:
    LYTRONAUT_TEST_REVISION="{{revision}}" python3 scripts/container-evidence.py "{{output}}" "{{architecture}}"

container-bundle output archive:
    mkdir -p "{{output}}"
    tar -C "{{output}}" -czf "{{archive}}" .
    python3 -c 'import pathlib,sys; assert pathlib.Path(sys.argv[1]).stat().st_size <= 128*1024**2, "evidence archive exceeds 128 MiB"' "{{archive}}"

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
    env -u LYTRONAUT_VIDEO_PRIVATE_ROOT bash scripts/browser-test.sh
    node scripts/live-photo-test.cjs

# Actual platform codec acceptance using independently generated AVC pixels.
video-test:
    npm ci --ignore-scripts --prefix scripts/browser
    env -u LYTRONAUT_VIDEO_PRIVATE_ROOT node scripts/video-test.cjs

# Public synthetic portable format/contracts/API tests; no private source data.
live-photo-test:
    cargo test --locked -p lytronaut-format --test live_photo
    cargo test --locked -p lytronaut-contracts live_photo
    cargo test --locked -p lytronaut-service --test api live_photo

# Produce independently authored RGB resources and one complete ZIP in output.
[positional-arguments]
live-photo-native output:
    cargo run --locked -p lytronaut-format --example live_photo -- "$1"

# Actual Rust/WASM codec, maximum geometry, cancellation and resource cleanup.
live-photo-browser-test: wasm-build
    npm ci --ignore-scripts --prefix scripts/browser
    node scripts/live-photo-test.cjs

# Separate feature artifact, shared by owner and reference-host GPU recipes.
live-photo-owner-build:
    just build
    cargo test --locked -p lytronaut-client --features test-harness retained_observations::tests
    cargo test --locked -p lytronaut-format --features test-harness capacity_observations::metadata_tests
    cargo clippy --locked -p lytronaut-client --target wasm32-unknown-unknown --features test-harness -- -D warnings
    cargo build --locked -p lytronaut-client --target wasm32-unknown-unknown --features test-harness
    test "$(wasm-bindgen --version)" = "wasm-bindgen 0.2.129"
    wasm-bindgen --target web --out-dir dist-owner "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/debug/lytronaut_client.wasm"
    cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/debug/lytronaut_client.wasm" dist-owner/application.wasm
    git rev-parse HEAD > dist-owner/source-revision

live-photo-owner-test:
    rm -f owner-evidence.tar.gz owner-evidence.tar.gz.partial
    just live-photo-owner-build
    npm ci --ignore-scripts --prefix scripts/browser
    mkdir -p owner-evidence
    env -u LYTRONAUT_VIDEO_PRIVATE_ROOT LYTRONAUT_TEST_REVISION="$(git rev-parse HEAD)" LYTRONAUT_CLIENT=dist-owner LYTRONAUT_OWNER_TEST=1 LYTRONAUT_OWNER_EVIDENCE=owner-evidence bash scripts/live-photo-owner-evidence.sh

# Reference-host actual GPU promises/remounts; reuse the current feature build.
# Fails when no real GPU is available. Not a Linux software/GPU substitution.
live-photo-gpu-owner-test:
    test -z "${LYTRONAUT_VIDEO_PRIVATE_ROOT:-}${LYTRONAUT_PRIVATE_SEALED_ROOT:-}"
    test "${LYTRONAUT_PRIVATE_TEST:-0}" != 1
    test "$(cat dist-owner/source-revision)" = "$(git rev-parse HEAD)"
    mkdir -p gpu-owner-evidence
    env -u LYTRONAUT_VIDEO_PRIVATE_ROOT -u LYTRONAUT_OWNER_TEST LYTRONAUT_TEST_REVISION="$(git rev-parse HEAD)" LYTRONAUT_CLIENT=dist-owner LYTRONAUT_GPU_TEST=1 LYTRONAUT_GPU_LIFETIME_TEST=1 LYTRONAUT_GPU_EVIDENCE=gpu-owner-evidence bash scripts/browser-test.sh

# Public real-encoder experiment only; reuse the current test-harness build.
live-photo-jpeg-stress-test:
    mkdir -p owner-evidence
    rm -f owner-evidence/jpeg-stress.json
    env -u LYTRONAUT_VIDEO_PRIVATE_ROOT LYTRONAUT_TEST_REVISION="$(git rev-parse HEAD)" LYTRONAUT_CLIENT=dist-owner LYTRONAUT_OWNER_EVIDENCE=owner-evidence node scripts/live-photo-jpeg-stress-test.cjs

# macOS resource-loader check only; does not open Photos or prove GUI import.
[positional-arguments]
live-photo-apple output:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ $(uname -s) != Darwin ]]; then
        echo 'Apple resource verification requires macOS; use live-photo-test and live-photo-browser-test here.' >&2
        exit 2
    fi
    just live-photo-native "$1"
    mkdir -p "$1/swift-cache"
    swiftc -module-cache-path "$1/swift-cache" scripts/verify-live-photo.swift -o "$1/apple-verify"
    "$1/apple-verify" "$1"

# Opt-in, aggregate-only existing AVC/SPS proof against a sealed read-only copy.
# Not part of check, browser-test, public Live Photo recipes, or CI.
[positional-arguments]
video-private-test sealed_root: wasm-build
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -r "$1/app.db" || ! -d "$1/originals" ]]; then
        echo 'An authorized prepared sealed catalog copy is required.' >&2
        exit 2
    fi
    npm ci --ignore-scripts --prefix scripts/browser
    LYTRONAUT_VIDEO_PRIVATE_ROOT="$1" node scripts/video-test.cjs

# Opt-in pair proof using frozen production programs; no build/install or CI.
[positional-arguments]
live-photo-private-test sealed_root:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -r "$1/app.db" || ! -d "$1/originals" || -z "${LYTRONAUT_CLIENT:-}" || ! "${LYTRONAUT_TEST_REVISION:-}" =~ ^[0-9a-f]{40}$ || ! "${LYTRONAUT_TEST_WASM_SHA256:-}" =~ ^[0-9a-f]{64}$ || ! "${LYTRONAUT_TEST_GLUE_SHA256:-}" =~ ^[0-9a-f]{64}$ ]]; then
        echo 'An authorized sealed copy and frozen production directory/revision/WASM+glue manifest hashes are required.' >&2
        exit 2
    fi
    if [[ ! -r "$LYTRONAUT_CLIENT/lytronaut_client_bg.wasm" || ! -r "$LYTRONAUT_CLIENT/lytronaut_client.js" || ! -d scripts/browser/node_modules/puppeteer-core ]]; then
        echo 'Prepare and freeze production WASM/glue and locked browser tooling before private verification.' >&2
        exit 2
    fi
    env -u LYTRONAUT_VIDEO_PRIVATE_ROOT LYTRONAUT_PRIVATE_SEALED_ROOT="$1" node scripts/live-photo-private-test.cjs

# Explicit local Linux image builds; never publish to a registry.
container-build architecture image relay_image revision:
    docker build --platform "linux/{{ architecture }}" --tag "{{ image }}" --build-arg "SOURCE_REVISION={{ revision }}" --progress=plain .
    docker build --platform "linux/{{ architecture }}" --tag "{{ relay_image }}" --file scripts/Dockerfile.smoke-relay scripts

# Freeze the products from the same cached build before the runtime proof.
container-artifacts architecture image output:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{ output }}"
    docker build --platform "linux/{{ architecture }}" --target validation-artifacts --output "type=local,dest={{ output }}/programs" .
    docker image save "{{ image }}" | gzip -1 > "{{ output }}/image.tar.gz"

container-smoke architecture image relay_image revision output:
    mkdir -p "{{ output }}"
    LYTRONAUT_TEST_REVISION="{{ revision }}" LYTRONAUT_SMOKE_RELAY_IMAGE="{{ relay_image }}" python3 scripts/container-smoke.py "{{ image }}" "linux/{{ architecture }}" > "{{ output }}/runtime.json"

# Requires an actual native Linux runner and checks saved image/layer identity.
container-evidence architecture revision output:
    LYTRONAUT_TEST_REVISION="{{ revision }}" python3 scripts/container-evidence.py "{{ output }}" "{{ architecture }}"

container-bundle output archive:
    mkdir -p "{{ output }}"
    tar -C "{{ output }}" -czf "{{ archive }}" .
    python3 -c 'import pathlib,sys; assert pathlib.Path(sys.argv[1]).stat().st_size <= 128*1024**2, "evidence archive exceeds 128 MiB"' "{{ archive }}"

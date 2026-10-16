#!/usr/bin/env bash
# First-party line coverage: native (stable + cargo-llvm-cov) and the actual
# Rust/WASM client instrumented with a dated nightly, driven by the real browser
# suites, then merged and gated by `cargo run -p xtask -- coverage`.
#
#   scripts/coverage.sh toolchain   install/verify coverage-only toolchains
#   scripts/coverage.sh wasm        build instrumented WASM clients (nightly)
#   scripts/coverage.sh native      instrumented native tests + binaries (stable)
#   scripts/coverage.sh browser     browser suites against both instrumented builds
#   scripts/coverage.sh report      export LCOV, merge, report and enforce floors
#
# Synthetic fixtures only; never point this at a private library.
set -euo pipefail

# Dated nightly for WASM instrumentation only (-Zno-profiler-runtime needs
# nightly). Its LLVM must equal the stable product toolchain's LLVM and the
# runtime bundled by minicov 0.3.9 (LLVM 23.1, raw profile v11).
readonly NIGHTLY=nightly-2026-10-01
readonly LLVM_COV_VERSION=0.9.1
readonly WASM_BINDGEN_VERSION=0.2.129
readonly WASM_TARGET=wasm32-unknown-unknown
# WASM builds: name and client features.
readonly WASM_BUILDS=("client:coverage" "harness:coverage,test-harness")

root=$(git rev-parse --show-toplevel)
cd "$root"
cov="$root/target/coverage"
meta="$cov/meta"

die() { echo "coverage: $*" >&2; exit 1; }
llvm_version() { rustc "$@" -vV | sed -n 's/^LLVM version: //p'; }
host_tools() { echo "$(rustc "$@" --print sysroot)/lib/rustlib/$(rustc "$@" -vV | sed -n 's/^host: //p')/bin"; }
find_llvm_ar() {
    if command -v llvm-ar >/dev/null; then command -v llvm-ar; return; fi
    local candidate
    candidate=$(compgen -G '/usr/bin/llvm-ar-*' | sort -V | tail -1 || true)
    [[ -n $candidate ]] || die 'llvm-ar is required to archive the minicov runtime for wasm32'
    echo "$candidate"
}
record() { mkdir -p "$meta"; printf '%s=%s\n' "$1" "$2" >> "$meta/$3.txt"; }

toolchain() {
    rm -rf "$meta" && mkdir -p "$meta"
    rustup component add llvm-tools-preview
    rustup toolchain install "$NIGHTLY" --profile minimal --component llvm-tools-preview --target "$WASM_TARGET"
    local stable nightly
    stable=$(llvm_version)
    nightly=$(llvm_version "+$NIGHTLY")
    [[ -n $stable && $stable == "$nightly" ]] || die "nightly LLVM $nightly != stable LLVM $stable"
    [[ $(cargo llvm-cov --version) == "cargo-llvm-cov $LLVM_COV_VERSION" ]] || die "cargo-llvm-cov $LLVM_COV_VERSION required (MISE_ENV=coverage)"
    [[ $(wasm-bindgen --version) == "wasm-bindgen $WASM_BINDGEN_VERSION" ]] || die "wasm-bindgen $WASM_BINDGEN_VERSION required"
    clang --print-targets | grep -q wasm32 || die 'clang with a wasm32 backend is required for the minicov runtime'
    find_llvm_ar >/dev/null
    record rustc_stable "$(rustc -vV | tr '\n' ' ')" tools
    record rustc_nightly "$(rustc "+$NIGHTLY" -vV | tr '\n' ' ')" tools
    record llvm_cov_stable "$("$(host_tools)/llvm-cov" --version | grep -m1 -i version | xargs)" tools
    record llvm_cov_nightly "$("$(host_tools "+$NIGHTLY")/llvm-cov" --version | grep -m1 -i version | xargs)" tools
    record cargo_llvm_cov "$(cargo llvm-cov --version)" tools
    record wasm_bindgen "$(wasm-bindgen --version)" tools
    record clang "$(clang --version | head -1)" tools
    record llvm_ar "$(find_llvm_ar)" tools
    record node "$(node --version)" tools
    record minicov "0.3.9 (Cargo.lock)" tools
    record runner_image "${ImageOS:-local}/${ImageVersion:-unknown} ${RUNNER_OS:-$(uname -sm)}" tools
}

# Coverage needs LLVM mapping sections, not DWARF; skipping debuginfo keeps the
# instrumented targets small enough for hosted runners.
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0

wasm() {
    local target="$cov/wasm-target" wrapper="$root/scripts/coverage-rustc.sh"
    CC_wasm32_unknown_unknown=${CC_wasm32_unknown_unknown:-clang}
    AR_wasm32_unknown_unknown=$(find_llvm_ar)
    export CC_wasm32_unknown_unknown AR_wasm32_unknown_unknown
    for build in "${WASM_BUILDS[@]}"; do
        local name=${build%%:*} features=${build#*:}
        local out="$cov/wasm/$name"
        rm -rf "$out" && mkdir -p "$out/dist"
        # RUSTFLAGS replaces .cargo/config.toml's target rustflags, so the web-sys
        # cfg is repeated. The wrapper instruments only first-party crates.
        env CARGO_TARGET_DIR="$target" CARGO_INCREMENTAL=0 RUSTC_WRAPPER="$wrapper" \
            RUSTFLAGS='-Zno-profiler-runtime -Clink-args=--no-gc-sections --cfg=web_sys_unstable_apis' \
            cargo "+$NIGHTLY" build --locked -p lytronaut-client --target "$WASM_TARGET" --features "$features"
        local raw="$target/$WASM_TARGET/debug/lytronaut_client.wasm"
        python3 -I "$root/scripts/coverage-sections.py" "$raw"
        cp "$raw" "$out/application.wasm"
        wasm-bindgen --target web --out-dir "$out/dist" "$raw"
        cp "$raw" "$out/dist/application.wasm"
        git rev-parse HEAD > "$out/dist/source-revision"
        record "wasm_${name}_features" "$features" builds
        record "wasm_${name}_application_sha256" "$(sha256sum "$out/application.wasm" | cut -d' ' -f1)" builds
    done
}

native() {
    export CARGO_TARGET_DIR="$cov/native-target" CARGO_INCREMENTAL=0
    mkdir -p "$cov"
    { cargo llvm-cov show-env --sh; echo "export CARGO_TARGET_DIR='$CARGO_TARGET_DIR' CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0"; } > "$cov/native-env.sh"
    # shellcheck source=/dev/null
    source "$cov/native-env.sh"
    cargo llvm-cov clean --workspace
    cargo test --locked --workspace --all-targets --all-features
    # Instrumented service binary and synthetic fixture example for the browser suites.
    cargo build --locked --workspace --all-targets --all-features
    # Actual PostgreSQL backend paths against a disposable pinned server.
    LYTRONAUT_TEST_PREFIX=${LYTRONAUT_TEST_PREFIX:-lytronaut-coverage} \
        python3 scripts/postgres_fixture.py -- python3 scripts/postgres-test.py
}

browser() {
    [[ -f $cov/native-env.sh ]] || die 'run "scripts/coverage.sh native" first'
    # shellcheck source=/dev/null
    source "$cov/native-env.sh"
    npm ci --ignore-scripts --prefix scripts/browser
    # Every suite runs even after an earlier failure so one CI pass yields complete
    # evidence; the step still fails if any suite failed.
    local failed=()
    run() {
        local name=$1; shift
        [[ -f $cov/wasm/$name/dist/lytronaut_client_bg.wasm ]] || die "run \"scripts/coverage.sh wasm\" first ($name)"
        if ! env -u LYTRONAUT_VIDEO_PRIVATE_ROOT -u LYTRONAUT_PRIVATE_SEALED_ROOT -u LYTRONAUT_PRIVATE_TEST \
            LYTRONAUT_CLIENT="$cov/wasm/$name/dist" LYTRONAUT_COVERAGE_DIR="$cov/wasm/$name/profraw" "$@"; then
            failed+=("$name: $*")
        fi
    }
    run client bash scripts/browser-test.sh
    run client node scripts/live-photo-test.cjs
    run client node scripts/video-test.cjs
    rm -rf "$cov/owner-evidence" && mkdir -p "$cov/owner-evidence"
    run harness env LYTRONAUT_TEST_REVISION="$(git rev-parse HEAD)" LYTRONAUT_OWNER_TEST=1 \
        LYTRONAUT_OWNER_EVIDENCE="$cov/owner-evidence" bash scripts/live-photo-owner-evidence.sh
    if ((${#failed[@]})); then
        printf 'coverage: browser suite failed: %s\n' "${failed[@]}" >&2
        exit 1
    fi
}

report() {
    [[ -f $cov/native-env.sh ]] || die 'run "scripts/coverage.sh native" first'
    (
        # shellcheck source=/dev/null
        source "$cov/native-env.sh"
        cargo llvm-cov report --lcov --output-path "$cov/native.lcov"
    )
    local tools args=()
    tools=$(host_tools "+$NIGHTLY")
    for build in "${WASM_BUILDS[@]}"; do
        local name=${build%%:*} dir="$cov/wasm/${build%%:*}"
        local profiles=("$dir"/profraw/*.profraw)
        if [[ ! -e ${profiles[0]} ]]; then
            # Still report native/union numbers; the gate fails on the missing build.
            echo "coverage: no WASM profiles for $name" >&2
            : > "$dir/coverage.lcov"
            record "wasm_${name}_profiles" 0 builds
            args+=(--wasm "$name=$dir")
            continue
        fi
        "$tools/llvm-profdata" merge -sparse -failure-mode=all "${profiles[@]}" -o "$dir/merged.profdata" 2> "$dir/merge.stderr"
        "$tools/llvm-cov" export -format=lcov -instr-profile "$dir/merged.profdata" -object "$dir/application.wasm" > "$dir/coverage.lcov" 2> "$dir/export.stderr"
        if grep -iE 'warning|error|mismatch|malformed' "$dir/merge.stderr" "$dir/export.stderr"; then
            die "llvm tools reported problems for $name"
        fi
        record "wasm_${name}_profiles" "${#profiles[@]}" builds
        # Covered instrumented lines per browser script (profiles are named
        # <script>-<pid>-<seq>-<reason>.profraw), checked against recorded floors.
        local script
        for script in $(for f in "${profiles[@]}"; do basename "$f" | sed -E 's/-[0-9]+-[0-9]{4}-.*$//'; done | sort -u); do
            "$tools/llvm-profdata" merge -sparse -failure-mode=all "$dir"/profraw/"$script"-[0-9]*.profraw -o "$dir/script.profdata" 2>> "$dir/merge.stderr"
            record "wasm_${name}_script_${script}_lines" "$("$tools/llvm-cov" export -summary-only -instr-profile "$dir/script.profdata" -object "$dir/application.wasm" 2>> "$dir/export.stderr" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"][0]["totals"]["lines"]["covered"])')" builds
            # Kept as evidence so two runs can be compared line by line per script.
            mkdir -p "$dir/scripts"
            "$tools/llvm-cov" export -format=lcov -instr-profile "$dir/script.profdata" -object "$dir/application.wasm" > "$dir/scripts/$script.lcov" 2>> "$dir/export.stderr"
        done
        # Per-script exports append diagnostics too; inspect after every export.
        if grep -iE 'warning|error|mismatch|malformed' "$dir/merge.stderr" "$dir/export.stderr"; then
            die "llvm tools reported problems for $name per-script profiles"
        fi
        rm -f "$dir/script.profdata"
        args+=(--wasm "$name=$dir")
    done
    record git_head "$(git rev-parse HEAD)" builds
    record chrome "$("${LYTRONAUT_CHROME:-google-chrome}" --version 2>/dev/null || echo unknown)" tools
    cargo run --locked -q -p xtask -- coverage --native "$cov/native.lcov" "${args[@]}" \
        --meta "$meta/tools.txt" --meta "$meta/builds.txt" --out "$cov/report"
}

case ${1:-} in
    toolchain|wasm|native|browser|report) "$1" ;;
    *) die "usage: $0 toolchain|wasm|native|browser|report" ;;
esac

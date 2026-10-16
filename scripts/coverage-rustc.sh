#!/usr/bin/env bash
# Coverage tooling: RUSTC_WRAPPER for the instrumented WASM client build.
# Adds LLVM coverage instrumentation to first-party application crates only;
# dependencies (leptos, wgpu, web-sys, ...) compile unchanged, mirroring what
# cargo-llvm-cov does for the native workspace.
set -euo pipefail
rustc=$1
shift
crate=''
previous=''
for argument in "$@"; do
    if [[ $previous == --crate-name ]]; then
        crate=$argument
        break
    fi
    previous=$argument
done
case $crate in
    lytronaut_client | lytronaut_format | lytronaut_contracts) exec "$rustc" "$@" -Cinstrument-coverage ;;
    *) exec "$rustc" "$@" ;;
esac

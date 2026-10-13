# First-party line coverage

`just coverage` measures line coverage of the first-party Rust application code
in three ways and fails unless each reaches the floors in
`crates/xtask/src/coverage.rs` (95%):

| Gate | Denominator | Execution |
|---|---|---|
| native | `lytronaut-contracts`, `-format`, `-storage`, `-service` | workspace tests, PostgreSQL parity, and the instrumented service binary driven by the browser suites |
| WASM | `lytronaut-client` (all browser-only code) | the actual instrumented client mounted in headless Chrome by the browser suites |
| union | every application line any target compiles | a line counts as covered if any target executed it |

The `coverage` GitHub workflow runs the same recipes on every pull request and
`main` push and uploads `coverage-<commit>` with the JSON/Markdown report, LCOV
files, tool versions and WASM hashes. Synthetic fixtures only.

## Recipes

```sh
just coverage            # everything below, inside MISE_ENV=coverage
just coverage-toolchain  # stable llvm-tools + dated nightly; verifies LLVM versions match
just coverage-wasm       # instrumented client builds: `coverage`, `coverage,test-harness`
just coverage-native     # instrumented tests, binaries and PostgreSQL parity (Docker)
just coverage-browser    # browser suites against the instrumented service and clients
just coverage-report     # LCOV export, merge, report, floors
```

Requirements: Chrome (`LYTRONAUT_CHROME`), Docker for the disposable
PostgreSQL server, clang with a wasm32 backend and `llvm-ar` (for the minicov
runtime). Output lives in `target/coverage/`; the report is
`target/coverage/report/coverage.md`.

## How the WASM client is measured

The product toolchain stays Rust 1.99.0 (`rust-toolchain.toml`). Only the
coverage build of the client uses the dated nightly pinned in
`scripts/coverage.sh`, because `-Zno-profiler-runtime` is nightly-only; its LLVM
must equal the stable toolchain's and the runtime bundled by `minicov` 0.3.9
(LLVM 23.1, raw profile v11). `scripts/coverage-rustc.sh` instruments only the
first-party crates.

The off-by-default `coverage` feature of `lytronaut-client` links the separate
tooling crate `lytronaut-coverage`, which publishes
`globalThis.__lytronautCoverage` with `dump()` and `signature()`. `just
dependencies` rejects that crate and `minicov` in the default client graph, so
production and test-harness artifacts never contain it.

`scripts/coverage.cjs` wraps every Puppeteer browser the suites launch. Before
`page.goto`, `page.reload`, `page.close` and `browser.close` it dumps the
counters of each page that loaded the client and writes a `.profraw`. A page
that loaded the client but has no hook, an empty profile, a profile whose
recomputed module signature differs, or a main-frame navigation without a
preceding dump fails the suite. `browser-test.cjs` awaits the service's
graceful SIGINT exit so the native profile is written.

## Browser suites in the coverage run

`just coverage-browser` runs every suite even after one fails, then fails the
step if any did, so one CI pass yields complete evidence. It runs:

- the synthetic browser suite (including `scripts/sidebar-test.cjs` and the
  fault-injection flows in `scripts/flows/`);
- the same suite in GPU mode against Chrome's SwiftShader software WebGPU
  adapter (`--enable-unsafe-webgpu`, `LYTRONAUT_GPU_ADAPTER=google/swiftshader`),
  with the injected adapter, device, pipeline and device-loss faults;
- the Live Photo and video codec suites;
- the owner suite against the `coverage,test-harness` build.

The SwiftShader run is real WebGPU API execution for coverage only. It is never
a physical-GPU or hardware result; physical-GPU evidence comes only from the
separate reference-host runs.

## What is excluded

Only code that cannot run in a product build, each range itemised in the report:

- items gated on `cfg(test)` or marked `#[test]`, including whole test modules
  declared under `#[cfg(test)]`;
- items gated on the opt-in `test-harness` feature (synthetic lifecycle
  observations), including the whole `capacity_observations` and
  `retained_observations` modules.

Tooling is listed separately and never counted: `xtask`, `lytronaut-coverage`,
and `tests/` and `examples/` sources. Every `crates/*/src` file must be reachable
from its crate's module tree, and a file with function bodies must have
instrumented lines in each target that compiles it, so no module can drop out of
the denominator. Files with declarations only (no function bodies) are listed.

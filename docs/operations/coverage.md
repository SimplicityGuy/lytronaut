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
- the Live Photo and video codec suites;
- the owner suite against the `coverage,test-harness` build.

The GPU suite is not part of this run. On the CI runner, Chrome 154's SwiftShader
software adapter renders offscreen but loses the device on canvas presentation in
headless mode. A headful browser on Xvfb with Vulkan SwiftShader presents
intermittently: acquisition sometimes exceeds the client's 700 ms bound and falls
back to CPU, and when it presents, the exact-pixel comparison does not match.

The GPU suite can still be run by hand against the software adapter:

```sh
LYTRONAUT_GPU_TEST=1 LYTRONAUT_GPU_ADAPTER=google/swiftshader LYTRONAUT_CHROME_HEADLESS=0 \
  LYTRONAUT_CHROME_ARGS='--enable-unsafe-webgpu --enable-features=Vulkan --use-vulkan=swiftshader --use-webgpu-adapter=swiftshader --enable-unsafe-swiftshader' \
  xvfb-run -a bash scripts/browser-test.sh
```

That is WebGPU API execution, never a physical-GPU result. Physical-GPU evidence
comes only from the reference-host runs.

## GPU-specific exception

The owner approved an exception on 2026-10-13: GPU-specific code may stay below
95%. It is listed by function in `GPU_EXEMPTIONS` in `crates/xtask/src/coverage.rs`:
the whole WebGPU module `crates/lytronaut-client/src/gpu.rs`; `gpu_identity`,
`present_gpu` and `watch_gpu` in `browser.rs`; and three GPU-only blocks inside
`change_focus` and `change_video`. Each block is named by its exact opening line,
which must occur once in that function. These lines are removed from both
the numerator and the denominator of the WASM(client) and union gates. The native
gate is unchanged. The report still measures them on their own row
("GPU-specific (exempt)"), prints every exempt range, and prints the WASM and union
totals without the exemption.

Guard rails make the exception reviewable rather than silent:

- Only `gpu.rs`, `browser.rs` and `video.rs` may be named, and only `gpu.rs` may be
  exempted whole (it must use `wgpu`).
- Each named function or block must exist exactly once and reference GPU code.
- Exempt source is capped at `GPU_EXEMPT_SOURCE_LINE_CEILING` lines, so growth
  needs a reviewed change.

Shared non-GPU logic such as `model.rs` and generic browser flows is never
exempt. GPU rendering is verified instead by the physical-GPU reference-host runs, which
include the injected adapter, device, pipeline and device-loss faults. A
software-adapter run (above) is optional evidence, and its results are never
presented as physical-GPU results.

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

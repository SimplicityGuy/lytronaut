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

The product toolchain stays Rust 1.99.1 (`rust-toolchain.toml`). Only the
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

### Keeping the WASM measurement repeatable

Line coverage of the WASM client only counts what the browser suites actually
ran, so a suite that stops early silently lowers it. That is what made early CI
results swing by up to 4.7 points: when one flow failed (a changed status message,
or a test that assumed a capture was on the first page), the rest of
`browser-test` was skipped, and the run reported fewer covered lines without
any coverage-specific error. The following guards make incomplete execution fail visibly:

- Flows locate captures through the real search control instead of assuming a
  page layout, and every page waits for in-flight work to settle before its
  counters are dumped.
- `EXPECTED_DRIVERS` in `crates/xtask/src/coverage.rs` lists the scripts that
  must write profiles for each instrumented build; a missing one fails the gate.
- `crates/xtask/coverage-script-baseline.txt` is a reviewed baseline of the
  WASM(client) lines each browser script covers on its own, counted like the
  gate (client sources only, minus test/harness exclusions and the GPU
  exemption). The gate fails, naming the script, if a script covers more than
  max(3 lines, 0.25%) fewer lines than recorded: that is what a suite ending
  early looks like. A smaller drop or any growth is printed as a note, so
  one-line timing jitter does not fail the gate. Each script's own LCOV
  (`wasm/<build>/scripts/<script>.lcov`) is kept in the evidence so two runs can
  be compared line by line. Update the baseline in the change that changes the
  suites.
- Tests never race the client's own requests for coverage: where a newer
  listing must supersede an in-flight one, the suite holds that request so the
  supersede always happens.

Three historical runs over two commits reported identical aggregate WASM client
totals, but two failed the old per-script gate. They do not establish three
passing repetitions of one commit; the exact revisions and conclusions are in
[the validation report](../validation/mvp-validation.md#repeatability). The
native total can differ by two lines: `lytronaut-storage/src/desktop.rs`, where the second of
two concurrent imports of one library (`tokio::join!` in the PostgreSQL desktop
test) finds the first already installed only if both passed the early
fingerprint check before either committed. That race is the behaviour under
test; it moves the native total by 0.02 points, far inside the margin above the
95% floor.

### GPU suite

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

- Only `gpu.rs` and `browser.rs` may be named, and only `gpu.rs` may be
  exempted whole (it must use `wgpu`).
- Each named function or block must exist exactly once. Its exact source range
  must match the reviewed SHA-256 digest; a GPU name in a comment, string or
  unrelated function cannot satisfy the guard.
- Exempt source is capped at `GPU_EXEMPT_SOURCE_LINE_CEILING` lines, so growth
  fails even before a new baseline can be considered. Changes to a digest require
  review of the entire scope; expanding an exception requires explicit owner
  approval. Updating a digest alone is not that approval.

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

# Rust library browser

`mise exec -- just serve` builds the Rust/WASM client and matching wasm-bindgen
0.2.129 glue, then starts the native service at http://127.0.0.1:3000. Application
logic is Rust with Leptos CSR 0.8.22; CSS and the tiny module initialization in
owned HTML are presentation/glue. SQLite, native JPEG decoding and service crates
stay outside the WASM dependency graph.

Use **First run? Set up administrator**, read the private data directory's
`bootstrap.secret` locally, create the administrator, then sign in. Credentials
and CSRF stay in browser memory; the HttpOnly cookie is managed by the service.
Only an opaque import job ID is retained in sessionStorage for status recovery.
The configured Origin/Host and CSRF policy remain authoritative. The server
loads only the fixed trusted generated JS/WASM filenames from `LYTRONAUT_CLIENT`
(default `dist`) and embeds application-owned HTML/CSS. Uploaded libraries never
become public static assets; unknown paths remain authenticated.

`LYTRONAUT_DATA` defaults to `data`. `LYTRONAUT_DATABASE` optionally selects a
single filename inside that private directory (default `catalog.db`); the owner
CLI and service share this policy. `LYTRONAUT_LISTEN` defaults to
`127.0.0.1:3000`. An isolated local port requires a matching `LYTRONAUT_ORIGIN`.
Changing the listener does not trust proxy headers or relax Origin/Host admission.
External deployment must retain the documented HTTPS/proxy boundary.

Directory selection sends browser File objects one at a time through XHR,
without buffering originals into WASM. ZIP selection reads at most 65,577 tail
bytes, a 56-byte ZIP64 record when present, and at most 32 MiB central metadata
for expanded-byte/file admission. Classic and ZIP64 declarations are independent
client preflight; the server still verifies all hostile framing, file types,
paths and actual quotas. Upload progress distinguishes transferred bytes from
committed intake/status. Cancellation aborts XHR and requests durable cancellation.
Selection preflight clears viewer imagery and holds the shared 64 MiB working
lease, preventing ZIP metadata copies from overlapping retained previews. It checks
directory path metadata incrementally against 32 MiB without retaining expanded
hexadecimal paths. A scoped busy guard prevents simultaneous selections and
releases controls on every preflight failure. Recover status after restart; reselect input to create a new bounded intake after
interruption, with server content deduplication. Retry catalog adaptation seals
or retries semantic readiness through the idempotent finish endpoint.

Libraries and albums have bounded load-more controls. Capture/file pages hold
24 items. Search is literal, URI encoded, and capped at 128 UTF-8 bytes. Album
order is the server's retained picture_index order; ratings are explicitly
imported ratings. Truncation flags appear as `[truncated]`. Favorites and
unsupported focus/aperture/render/export controls are not invented. The inspector
shows preserved originals, missing companion roles, static preview and honest
legacy-edit/color/optional-renderer status. Preserved-only/quarantined libraries
retain paginated original links. Browser attachment downloads stream originals;
the WASM client never buffers a 512 MiB original.

Two grid workers and the stage share a Rust admission controller. At most two
preview requests transfer, and GET 429 retry is bounded to two retries with short
backoff. Manifest/detail bodies are streamed with a 256 KiB cap; chunks are at
most 8 MiB and require exact manifest length. Shared contracts enforce 48 MiB
encoded and 2048² geometry limits; whole asset length and SHA-256 are verified
before displaying any pixels. Viewer working admission conservatively counts
four encoded copies, the largest current chunk, two RGBA allocations and a 128²
canvas. It reserves at most 64 MiB working space and leaves 32 MiB for persistent
DOM imagery. Stage preview admission is at most 28 MiB by that conservative
formula. Grid images are decoded and downsized under Rust control to 128² JPEGs,
with data URLs capped at 128 KiB each. Original image URLs are revoked immediately
after downsizing; stage URLs are revoked on replacement/logout/unmount. Old
requests are aborted and generation/session/selection checks discard late results.
This is conservative application allocation accounting, not a measured bound on
browser/driver process RSS or browser decoder internal allocations.

## Behavioral and accessibility evidence

[Synthetic screenshot](../validation/assets/lytronaut-95r-browser.png) and
[measured browser results](../validation/assets/lytronaut-95r-browser.json)
contain independently generated content only.

`mise exec -- just setup wasm-build build browser-test` runs the actual binary,
setup/login, synthetic directory and multi-file ZIP import, capture pagination,
ordered album comparisons, search/sort, previews and logout/unauthorized replay in
an isolated Chrome profile. Public test inputs are independently authored schema
records and a Rust-encoded abstract scene. Puppeteer-core 25.13.0 is exact locked,
MIT/Apache/BSD/ISC test tooling, excluded from product assets. The npm inventory
is in package-lock.json and weekly Dependabot covers it. The GitHub `browser`
job runs this same synthetic suite without service secrets or private data.

The suite includes classic ZIP and small forced-ZIP64 uploads, cancelled
XHR/durable recovery, unknown-schema originals,
delayed album responses across logout, corrupted preview transfer, canvas failure
and URL cleanup/revisit. It compares the downloaded preserved thumbnail bytes
after corrupted preview transfer and drains all created Blob URLs before the
first browser restart, as well as after the zoom/accessibility session. GPU and VideoDecoder absence are injected before app
initialization; this tests baseline capability absence, not physical GPU absence.

Semantic labels, visible focus, native buttons/inputs/selects/progress and a
polite atomic status live region expose meaningful browser accessibility nodes.
Stage arrow keys and named previous/next controls navigate captures. Forced-colors
and reduced-motion media rules are tested through CDP emulation. Actual 200%
browser zoom uses the isolated Chromium profile's default zoom preference,
then requires doubled DPR and halved CSS viewport; viewport/page scaling is not
substituted. Responsive layouts also check horizontal overflow. The tested text
palette `#eef3f8`/`#171b22`, secondary `#b8c4d2`/`#222933`, cyan
`#52d5eb`/`#252c36`, and control border `#7d8b9b`/`#252c36` exceed
4.5:1 text and 3:1 control contrast. Computed main text contrast measured
15.47:1 and button border/background contrast 4.05:1 in Chrome 154.0.8037.98. Accessibility-tree roles/names and live-region
semantics are measured; full manual VoiceOver/NVDA certification, touch input,
onscreen GPU presentation, Windows ACLs and cross-browser behavior are unmeasured.

Private validation reuses the owner-authorized sealed scratch catalog and prints
only aggregate counts. Private captures, source names/IDs/hashes, credentials and
screenshots are never test artifacts or Git inputs. The final browser run covers
109 thumbnails and three ordered albums with 12, 39 and 58 memberships.
Final debug-build service readiness took 115.54 seconds for the retained
approximately 2 GiB / 728-file scratch corpus, including integrity recovery; the
harness requires `/healthz` readiness within 300 seconds. Release-build startup
latency is unmeasured.
Native tests reproduce SQLite read-to-write upgrade failure under another writer:
API mapping and schema migrations reserve their writer with SQLx-managed
`BEGIN IMMEDIATE`. Concurrent HTTP progress polling during publication remains
successful without product polling sleeps or hidden retries.

## Application coverage gate

Normal product builds remain stable Rust 1.99.0. This bead does not claim 95%
application line coverage. Native tests exercise model policy but do not compile
`cfg(wasm32)` browser.rs; JavaScript coverage of generated glue also does not
measure Rust application lines. Final acceptance bead lytronaut-cjq owns the
source-inventory-audited native + WASM report and enforcing GitHub threshold.

The official [wasm-bindgen coverage guide](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/coverage.html)
requires experimental nightly instrumentation, `cfg(wasm_bindgen_unstable_test_coverage)`,
`-Cinstrument-coverage -Zno-profiler-runtime` and matching LLVM/Clang. LLVM IR
provides symbols because llvm-cov cannot extract them from the resulting WASM.
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) supports this target and
native external-test profiles. A candidate **coverage-only** pinned toolchain is
`nightly-2026-10-10`; it has not been installed or verified here. Final acceptance
must select and record a working pin with its matching LLVM tools, separately
from stable product tools, then instrument actual browser Rust execution:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 \
CARGO_TARGET_DIR=/private/tmp/lytronaut-wasm-coverage/target \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='-Cinstrument-coverage -Zno-profiler-runtime -Clink-args=--no-gc-sections --cfg=wasm_bindgen_unstable_test_coverage' \
cargo +nightly-2026-10-10 llvm-cov test --target wasm32-unknown-unknown -p lytronaut-client
```

This is a research candidate, not a passed coverage command. It requires actual
wasm-bindgen Rust test instrumentation and browser profiles; the existing CDP
behavior suite does not emit profraw on its stable build. The final denominator
must include browser.rs and all first-party application source, audit test/helper
lines separately and fail when application files lack instrumented evidence.
No coverage-driven application exclusions or unsupported 95% claim are acceptable.

### Discrete JPEG focus and explicit orientation

The Rust/WASM viewer now selects the deterministic nearest **stored** JPEG plane,
with a native labelled range, keyboard values, accessible plane/lambda text and
actual source dimensions (360 × 360 for the validated private LFE 11 stacks).
There is no calibrated focus synthesis. A separate explicit EXIF 1–8 selector
changes verified derivative pixels; the default is identity because processed
recipes may be baked. Imported sidecar/embedded/catalog orientation stays
preserved. The interoperability source records 109/109 numerically matching
sidecar-v4/embedded-view fields, but this establishes correspondence rather than
rendering parity. Unknown or conflicting edits are not combined. Synthetic
embedded orientation 8 versus sidecar 6 verifies default identity and explicit
viewer precedence. Catalog 0 is rejected as EXIF.

Source bytes use the authenticated native CPU flow documented in `api.md`.
Each response carries a typed selection, geometry, generated-byte hash and lossy
provenance. The Rust client checks these before actual JPEG decoding/display.
Stage requests share the existing two-transfer browser admission; each focus
operation reserves 20 MiB within the 64 MiB transient viewer allowance, leaving
32 MiB for persistent stage/grid within the common 96 MiB CPU/WASM budget.
Changing focus, capture, library or session aborts obsolete requests; generation
checks discard late results. Temporary decode objects are released, superseded
Blob URLs are revoked, and logout clears the displayed derivative. There is no
unbounded plane cache. Failed derivatives retain the previous verified frame and
preserved downloads; the output label describes the frame actually displayed.

The independent synthetic browser suite now exercises all public stored planes,
SHA256/typed extents and actual Chrome decode, rectangular colored-corner EXIF
rotation, baked/sidecar disagreement, keyboard Home/End, corrupt derivative
retention, held-request cancellation and static fallback without GPU/video.
Existing import/recovery, ZIP/ZIP64, preview corruption, original-byte equality,
pre-restart URL/canvas cleanup, sorting, actual 200% DPR/CSS zoom, responsive,
forced-colors, reduced-motion and AX checks remain required. AX inspection does
not certify manual VoiceOver behavior. Native tests cover all eight EXIF rotation/
reflection coordinate maps, inverse geometry, hostile metadata and resource
limits, exact source preservation, authentication and response permit lifecycle.

Latency measurement is input event through verified Rust/WASM derivative decode,
stage image decode and the next animation-frame callback: **decoded frame ready**,
not a claim that physical display presentation has completed. The release CPU
100 ms p95 gate uses 24 repeated interactions on declared hardware after grid
images finish loading. Aggregate-only corpus checks decode every stored plane.
Debug build timing does not establish this performance target. Final first-party
native plus WASM Rust line coverage instrumentation and its failing 95% floor
remain the acceptance bead's experimental tooling gate; these tests do not claim
95% measured application coverage.

The development `just serve` and canonical `wasm-build` use debug profiles. A
public synthetic diagnostic run measured 111 ms p95 HTTP/decode and 153.6 ms p95
input-to-frame-ready with debug native/WASM builds. An initial private WIP run
with release native but debug WASM passed all 58/420 native/Chrome decodes and
missed the 100 ms UI gate; detailed UI percentiles were not retained because
that gate failed before its report was written. These results do not meet the
performance target and are retained as development limitations. Grid contention
was a candidate contributor, not a measured cause.

Reproduce the optimized profiles separately; the generated client is still Rust
WASM plus the pinned wasm-bindgen glue:

```sh
mise exec -- cargo build --locked --release -p lytronaut-service
mise exec -- cargo build --locked --release -p lytronaut-client --target wasm32-unknown-unknown
mise exec -- wasm-bindgen --target web --out-dir dist/release \
  "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/lytronaut_client.wasm"
LYTRONAUT_CLIENT=dist/release \
  "${CARGO_TARGET_DIR:-target}/release/lytronaut-service"
```

The aggregate-only private harness can point at an explicitly approved sealed
scratch catalog using `LYTRONAUT_PRIVATE_TEST=1`, `LYTRONAUT_TEST_ROOT`, the
release `LYTRONAUT_BINARY`, `LYTRONAUT_CLIENT`, an isolated loopback
`LYTRONAUT_TEST_ORIGIN`, and `LYTRONAUT_RELEASE_BENCHMARK=1`. Set
`LYTRONAUT_TEST_REVISION` to the actual checked-out commit and
`LYTRONAUT_BROWSER_REPORT` to a new protected report path. The harness records
both the first 24 interactions and 24 subsequent interactions after all grid
images decode; each records its observed pending-grid count, p50/p95/max, and
must independently satisfy the unchanged 100 ms p95 target. A first batch is
load-contended evidence only if its pending-grid count is nonzero. Separate
native/WASM artifact hashes and OS/browser/hardware bind the report to the tested
builds. No private paths, source hashes, identifiers or pixels belong in reports.

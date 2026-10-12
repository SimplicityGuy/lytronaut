# Rust/wgpu uploaded derivative validation

Bead **lytronaut-us7 — Integrate optional Rust wgpu derivative presentation with safe fallback**.
Public fixtures are independently Rust-encoded abstract colored scenes; no private
capture, source identifier, calibration or original library was used.

The full synthetic suite passed on MacBookPro18,1, Apple M1 Pro (16 GPU cores),
32 GiB RAM, macOS 26.6.2, Chrome 154.0.8037.98. GPUAdapterInfo reports `apple`,
`metal-3`; maximum adapter 2D texture dimension is 16384. Chrome uses an isolated
owned profile and no feature flags. Actual wgpu 30.0.1 code runs in Rust/WASM,
uploads decoded verified identity derivatives and presents a real canvas.

All eight EXIF transforms compare every compositor screenshot pixel against an
independent test-only CPU coordinate oracle of the decoded identity JPEG.
Maximum channel error and RMSE are **zero**, including non-primary midtones.
The visible output is 360 × 240 or 240 × 360. The integer scaling test at the
256 cap is 170 × 256, also exact. Native allocation tests reject hostile geometry
and check aspect/cap/budgets. Browser injection rejects an oversized typed
derivative while retaining the identical prior JPEG URL.

The warm protocol uses 35 actual focus interactions, discards the first five,
and records 30 submit-to-double-rAF post-frame callbacks. Headed Chromium tracing
captures `blink.user_timing`, `cc`, `benchmark`, `viz`, `gpu`, timeline/frame events
and screenshots. Rust emits a unique `lytronaut-gpu-submit-<time>` marker directly
before actual `queue.submit`/`queue.present`. Presentation events are retained as
unmatched when trace data does not identify the submitted canvas frame. Picking
the first unrelated presentation event would produce invalid latency evidence.
Neither queue completion, mapped readback nor one prepaint rAF is substituted for
presentation. No physical scanout/photon timing is measured.

The initial frozen WIP run measured post-frame callback p95 **21.2 ms** at the
256 cap. Its trace had 46 submit markers, 26 FramePresented, 182 FrameDisplayed
and 366 DrawAndSwap events, with **zero attributable canvas-presentation samples**.
The **16.7 ms compositor submit-to-present target is unverified**, and the
observed frame-ready proxy exceeds it. Default automatic GPU quality is reduced to
512 from the 1024 interactive ceiling, with an explicit 256 choice. No guaranteed
frame rate is advertised. Final exact-code evidence is recorded in the linked
report when the branch artifact has been frozen and rerun.

The WIP failure suite observed CPU fallback after device destruction in 92 ms,
oversized metadata in 11 ms, missing API/adapter/device rejection/pipeline failure
in 151/130/139/149 ms, and late device acquisition in 955 ms. The acquisition
bound was subsequently reduced to 700 ms for more scheduling margin. Created
and explicitly destroyed failure/late devices matched (4/4). Twelve sequential
capture switches passed. No page errors/unhandled rejections or asset 429s;
all Blob URLs drained at logout. Last valid frames, controls and original links
remained usable. The same complete run passed import/ZIP/ZIP64/cancellation,
orientation/integrity/stale-response checks, 30 thumbnails, three ordered albums,
keyboard, actual 200% browser zoom/DPR, responsive/forced-colors/reduced-motion
and accessibility tree checks. Manual screen-reader certification is unmeasured.

Reproduce from an exclusive artifact build, immediately freeze native binary,
WASM and generated glue before another branch can overwrite the shared target:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 mise exec -- just wasm-build build
# Freeze the matching service, synthetic_jpeg example and dist client directory.
# Generate a public scratch with synthetic_jpeg and scripts/synthetic-library.py.
LYTRONAUT_TEST_ROOT=<public-scratch> LYTRONAUT_BINARY=<frozen-service> \
LYTRONAUT_CLIENT=<frozen-client> LYTRONAUT_TEST_ORIGIN=http://127.0.0.1:3021 \
LYTRONAUT_GPU_TEST=1 LYTRONAUT_GPU_TRACE=1 \
mise exec -- node scripts/browser-test.cjs
```

The existing temporary login harness is used for this branch's proof; deployment
access-control changes are owned by the integration bead. It is not a requirement
for final built-in user accounts. Actual application-line native/WASM coverage and
a fail-closed 95% GitHub floor remain the final acceptance bead's gate.

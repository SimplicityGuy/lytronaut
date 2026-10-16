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

The final frozen application source is `35eb53c0f60ae3a4d47ec94192729073582b1e8b`.
The [public aggregate report](assets/lytronaut-us7-gpu.json) records artifact hashes,
hardware, browser run 47886 and the previous exact-code run 67559. Later evidence
and harness commits leave the Rust application and frozen artifacts unchanged.
The [orientation-six screenshot](assets/lytronaut-us7-gpu-orientation6.png) is a
public synthetic canvas output. Native and WASM artifacts use debug profiles.

At the 256 cap, run 47886 measured double-rAF post-frame callback p95 **23.9 ms**;
run 67559 measured **34.1 ms**. These are frame-ready proxies, not actual compositor
presentation latency. The latest trace has 52 submit markers, 107 FramePresented,
315 FrameDisplayed and 636 DrawAndSwap events, with **zero attributable canvas
presentation samples**. The **16.7 ms compositor submit-to-present target is
unverified**. Both observed proxies exceed 16.7 ms. Automatic GPU quality starts
at 512, reduced from the 1024 interactive ceiling; the resolution control also
permits 256. No 60 fps or guaranteed presentation claim is made.

Automatic renderer selection requires no mode, toggle or retry button. The suite
asserts those controls are absent and missing API, adapter failure, device rejection,
pipeline failure and late device acquisition fall back without user action.
Measured acquisition-to-fallback times are 15.2/9.6/15.1/16.1/718.6 ms respectively.
The timeout is 700 ms; late-created devices are explicitly destroyed after resolution.
Pipeline and late-acquisition fault devices each record one created and one destroyed.
Device loss falls back in 98 ms. Oversized metadata retains the prior JPEG URL in
10 ms. Twelve capture switches create seven devices, destroy six, and retain only
the active device; logout destroys the seventh. No page errors or unhandled
rejections occur, controls and the last valid frame remain usable.

Actual WASM linear memory is **2,162,688 bytes** before and after the warm interaction
sequence. This measures the WASM buffer, separately from conservative counted
application allocations: **55,205,376 CPU bytes** and **1,728,016 GPU bytes** for the
full synthetic frame (1,213,456 GPU bytes at the 256 cap). Neither number is browser
RSS or measured driver residency. The allocation policy includes decoded pixels,
canvas intermediate, staging copies, encoded derivative leases and persistent
browser budget; the active focus lease is 24 MiB within shared admission limits.

The complete run also passes import/ZIP/ZIP64/cancellation, orientation/integrity/
stale-response checks, 30 thumbnails, three ordered albums, keyboard, actual 200%
browser zoom/DPR, responsive/forced-colors/reduced-motion and accessibility tree
checks. No asset 429s occur and all Blob URLs drain at logout. Manual screen-reader
certification is unmeasured.

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

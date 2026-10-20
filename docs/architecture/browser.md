# Rust library browser

`mise exec -- just serve` builds the Rust/WASM client and matching wasm-bindgen
0.2.129 glue, then starts the native service at http://127.0.0.1:3000. Application
logic is Rust with Leptos CSR 0.8.22; CSS and the tiny module initialization in
owned HTML are presentation/glue. SQLite, native JPEG decoding and service crates
stay outside the WASM dependency graph.

Open the library workspace directly; access control belongs to deployment.
Only an opaque import job ID is retained in sessionStorage for status recovery.
The configured Origin/Host and mutation-intent policy remain authoritative. The server
loads only the fixed trusted generated JS/WASM filenames from `LYTRONAUT_CLIENT`
(default `dist`) and embeds application-owned HTML/CSS. Uploaded libraries never
become public static assets; unknown paths remain deployment-protected.

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
committed intake/status. While a ZIP request remains open after transmission,
one serial status request at a bounded cadence reports preservation and catalog
preparation; only `catalog_complete` refreshes the library list. Cancellation,
page disposal and replacement abort the in-flight request and its status poll.
Cancellation aborts XHR and requests durable cancellation.
Selection preflight clears viewer imagery and holds the shared 64 MiB working
lease, preventing ZIP metadata copies from overlapping retained previews. It checks
directory path metadata incrementally against 32 MiB without retaining expanded
hexadecimal paths. A scoped busy guard prevents simultaneous selections and
releases controls on every preflight failure. Recover status after restart; reselect input to create a new bounded intake after
interruption, with server content deduplication. Retry catalog adaptation seals
or retries semantic readiness through the idempotent finish endpoint.

Chrome 155.0.8059.82 on macOS 27.0.1 can select a recognized `.lytrolibrary`
package through the dedicated normal file input and supplies its contents as a
ZIP. Lytronaut validates that archive as bytes and never trusts its name or MIME
type. This native package behavior is platform integration rather than a
cross-browser guarantee; the ordinary directory and explicit ZIP controls remain
available.

Libraries and albums have bounded load-more controls. Capture/file pages hold
24 items. Search is literal, URI encoded, and capped at 128 UTF-8 bytes. Album
order is the server's retained picture_index order; ratings are explicitly
imported ratings. Truncation flags appear as `[truncated]`. Favorites and
unsupported aperture/render controls are not invented. The inspector
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
after downsizing; stage URLs are revoked on replacement or unmount. Old
requests are aborted and generation/selection checks discard late results.
This is conservative application allocation accounting, not a measured bound on
browser/driver process RSS or browser decoder internal allocations.

## Behavioral and accessibility evidence

[Synthetic screenshot](../validation/assets/browser-validation.png) and
[measured browser results](../validation/assets/browser-validation.json)
contain independently generated content only.

`mise exec -- just setup wasm-build build browser-test` runs the actual binary,
direct entry, synthetic directory and multi-file ZIP import, capture pagination,
ordered album comparisons, search/sort, previews and direct entry and foreign-origin rejection in
an isolated Chrome profile. Public test inputs are independently authored schema
records and a Rust-encoded abstract scene. Puppeteer-core 25.13.0 is exact locked,
MIT/Apache/BSD/ISC test tooling, excluded from product assets. The npm inventory
is in package-lock.json and weekly Dependabot covers it. The GitHub `browser`
job runs this same synthetic suite without service secrets or private data.

The suite includes classic ZIP and small forced-ZIP64 uploads, cancelled
XHR/durable recovery, unknown-schema originals,
delayed album responses across library changes, corrupted preview transfer, canvas failure
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
One historical debug-build service readiness measurement took 115.54 seconds for the then-retained
approximately 2 GiB / 728-file scratch corpus, including integrity recovery; the
harness requires `/healthz` readiness within 300 seconds. A later release run
measured 7.4 seconds while rehashing every sealed original; it remains a
source-bound observation, not a universal startup target.
Native tests reproduce SQLite read-to-write upgrade failure under another writer:
API mapping and schema migrations reserve their writer with SQLx-managed
`BEGIN IMMEDIATE`. Concurrent HTTP progress polling during publication remains
successful without product polling sleeps or hidden retries.

## Application coverage gate

Production builds remain on stable Rust 1.99.1. The coverage recipe uses its
pinned matching nightly only for actual instrumented Rust/WASM execution in
Chrome. Exact-main coverage passed at native 96.14%, client 96.00%, and union
96.19%, with the unchanged itemized GPU exception. See
[coverage operation](../operations/coverage.md) and
[final-main status](../validation/final-main-status.md).

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

Source bytes use the deployment-protected native CPU flow documented in `api.md`.
Each response carries a typed selection, geometry, generated-byte hash and lossy
provenance. The Rust client checks these before actual JPEG decoding/display.
Stage requests share the existing two-transfer browser admission; each focus
operation reserves 20 MiB within the 64 MiB transient viewer allowance, leaving
32 MiB for persistent stage/grid within the common 96 MiB CPU/WASM budget.
Changing focus, capture or library aborts obsolete requests; generation
checks discard late results. Temporary decode objects are released, superseded
Blob URLs are revoked, and changing library clears the displayed derivative. There is no
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
limits, exact source preservation, deployment access and response permit lifecycle.

Latency measurement is input event through verified Rust/WASM derivative decode,
stage image decode and the next animation-frame callback: **decoded frame ready**,
not a claim that physical display presentation has completed. The release CPU
100 ms p95 gate uses 24 repeated interactions on declared hardware after grid
images finish loading. Aggregate-only corpus checks decode every stored plane.
Debug build timing does not establish this performance target. The final
source-bound A/B/B/A run passed the CPU and genuine grid-overlap gates; both
baseline grid windows were unavailable and no quiet-host comparison is claimed.

The production FIFO admission queue allows two active operations under the
unchanged 64 MiB transient ceiling and at most ten pending tickets. Releases and
cancellations synchronously wake waiters; RAII removes cancelled tickets without
letting newer thumbnail work starve an older selected-view request.

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


Verified PNG/JPEG export selects the displayed verified stored JPEG plane and its
explicit orientation, or static JPEG pixels with identity. Download the image and
separate Lytronaut provenance record. Flattening loses refocus ability; unknown
color and preserved-only imported edits stay explicit. The backup control streams
an originals/catalog bundle; restore uploads it into a separate isolated store.
Current libraries are unchanged. Job status can be recovered after refresh, and
uploads/pending jobs can be cancelled. These flows do not buffer archives in WASM.

Apple Live Photo export runs in the Rust/WASM client from verified archived video
and selected-key still data. The final private run verified 320/320 pairs across
109 streams, including pre-abort, active cancellation, and recovery; see the
[source-bound report](../validation/private-corpus-validation.md). Its pixel comparator
scope and platform/browser limits remain explicit there.

### Optional archived browser H.264 focus

The viewer keeps the JPEG/static baseline and offers **Try browser H.264
refocus** for a preserved standard `RefocusStack`. Each attempt validates the
bounded original manifest, exact downloaded length and whole-container SHA256,
then all LFP section digests and descriptors. It admits at most 8 MiB, 256
sections, 256 KiB metadata and 64 frames. Only the recognized standard refocus
descriptor is accepted; perspective and imported recipes remain preserved-only.

The independent Rust Annex-B inspector derives codec, coded geometry, chroma
crop units, whole-frame MBAFF geometry, SPS/PPS/slice references, AUD access
units and consistent IDR boundaries. Initial SPS/PPS bytes are retained during
software retry. WebCodecs configuration omits AVCC `description`. A support
query is followed by actual decode, complete frame-count/geometry verification,
flush and a selected-frame JPEG encode plus independent image decode. Failure
retries once with software preference; cancellation does not retry. No failed
attempt replaces the previously verified frame. Frame/capture/library
generation guards discard stale completions. Every VideoFrame is closed and
every temporary canvas is reset on success or error, with no decoded plane cache.

One codec operation holds the shared 64 MiB transient lease, so grid/JPEG work
waits. The conservative allowance covers four encoded copies (32 MiB), an 8 MiB
current chunk, two visible RGBA allocations (16 MiB), and metadata/generated
JPEG slack. Video output is limited to 2 megapixels, with its JPEG data URL
capped at 2 MiB; previous/current stage imagery and the grid remain within the
32 MiB persistent reserve. At most two encoded chunks are queued and no output
frames are retained after their callback. Coded storage remains bounded by
2048², with at most one macroblock of extra platform padding above normative
SPS dimensions. Visible rectangle, crop origins and display extents must match
the actual per-stream SPS and metadata. These are application admission bounds,
not a browser/driver RSS measurement or guarantee about platform codec internals.

H.264 lambdas are finite/unique in archived order; they are not sorted, because
sorting metadata separately would lose encoded frame associations. Slider bounds
are true extrema; nearest selection chooses the lower archived index on ties,
including robust comparison of hostile finite values whose differences overflow.
The default is explicit EXIF identity. The video orientation selector remains
disabled, and imported orientation/aperture/perspective/color/animation are never
replayed. While browser-video pixels are displayed, flattened PNG/JPEG controls
are disabled with an explanation and their handler rejects injected/stale calls.
Select a verified static/JPEG preview before exporting pixels; original backup
remains available. Successful video adoption hides the old GPU canvas and drops
its renderer/device. Failed video attempts preserve verified GPU imagery and
rearm its loss watcher; later device loss still restores the verified image.
Only owned Blob URLs are revoked when replacing or clearing the stage.

The [integrated private proof](../validation/video-validation.md) passed all
109 standard blocks / 654 frames and both five-frame LFE representatives on the
declared Chrome/macOS/M1 Pro configuration. All blocks required software retry;
there is no hardware codec claim. The UI still requires a successful actual
decode for each asset and has no assumed cross-browser capability flag. Untested
platforms are unverified and any absent/failed codec leaves previews usable.

`mise exec -- just video-test` runs the real Rust decoder on the independent
MIT colored AVC fixture plus public codec faults. `just browser-test` additionally
imports that fixture through the actual service and tests video slider pixels,
SHA-corrupt original retention, capture-switch cancellation, absence fallback and
retry beside the existing JPEG/browser suite. The SHA-pinned browser CI job
runs this same public fixture suite, with no private data. Coverage-only explicit
Rust flags must retain `--cfg=web_sys_unstable_apis`; generated JavaScript coverage
does not substitute for actual Rust/WASM line instrumentation.

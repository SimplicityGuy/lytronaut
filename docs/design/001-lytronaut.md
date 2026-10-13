# Lytronaut: evidence-led scope and implementation roadmap

Status: **Approved implementation epic lytronaut-re8; kickoff approved; delivery active**.
Decision bead: **lytronaut-zxk — Scope verdict and next implementation plan**.
Evidence date: 2026-10-11. Research epic: **lytronaut-ck2 — Lytronaut: prove
library compatibility and Rust rendering architecture**.
Name: **Lytronaut**. Repository: `SimplicityGuy/lytronaut` (private).
Tagline: **an explorer for your light-field library**.

## Decision and scope

Implement an MIT-licensed, self-hostable Rust application that uploads an offline
Desktop library, preserves every original byte and library association, browses
verified derivatives and supports explicitly labelled discrete focus selection.
The first supported semantic adapter is **Desktop schema 23, original F01**.
Support both original Lytro and Illum over time through independently verified
variants; this decision does not advertise Illum support or calibrated synthesis.

The research molecule is complete: all four spike reports and this scope
decision have been reviewed and landed. `plans/implementation.yaml` is filed as implementation epic `lytronaut-re8`.
The operator approved filing, kickoff, delivery, review and landing of its 13
tasks. Foundation bead `lytronaut-922` establishes the compiled workspace and
contracts; product capabilities remain gated by each downstream acceptance.
Release, image publication and private data publication remain disabled.

The baseline is useful without a GPU, camera calibration or browser video codec:
all 109 private reference captures have Rust-decodable thumbnails. The 58 LFE-11
captures additionally have 420 decoded JPEG focus planes. Original files remain
downloadable even if a feature is unsupported. Preserve unknown formats without
running them through a guessed adapter. No cloud service or external telemetry
is required. Camera USB control, native mobile apps, cloud sync, new depth
estimation, metric stereo reconstruction and lost-data recovery are outside this MVP.

## Reviewed evidence

| Research bead | Report | Finding that changes the plan |
|---|---|---|
| lytronaut-7w9 — Desktop fidelity and authorized corpus | [Library corpus](../spikes/lytronaut-7w9-library-corpus.md) | 109 pictures, 3 albums, 109 ordered memberships, 2 bins, 3 events; all companions reconcile; 728 files/2,001,830,074 bytes unchanged across SHA-256 checks; legacy picture hashes empty |
| lytronaut-4li — Native decoding and calibration | [Decoder/calibration](../spikes/lytronaut-4li-decode-calibration.md) | All 109 actual camera models F01; 3280 × 3280 packed 12-bit; 1,172,665,600 samples roundtrip exactly; local calibration exists but lens selection/quality is unverified |
| lytronaut-w6z — Processed stacks, depth and edits | [Interoperability](../spikes/lytronaut-w6z-processed-interop.md) | All 109 thumbnails + 420 JPEG planes decoded in native Rust; 654 standard H.264 frames verified only by external research reference; two five-frame browser representatives pass only with software preference |
| lytronaut-0mz — Rust WebUI and GPU budgets | [GPU/CPU and design](../spikes/lytronaut-0mz-rust-gpu.md) | Rust dependencies compile; physical direct-WebGPU synthetic compute is measured, but full Rust/wgpu presentation remains unverified; concrete graphite/cyan concept and keyboard checks available |

The catalog has LFE 10/0 for 51 captures and LFE 11/0 for 58, 109 raw/depth/stack/view
companions, 58 optional low-quality stacks, 109 JPEG thumbnails, 109 version-4 edit
sidecars and 2 opaque animation sidecars. All 385 processed containers’ 1,307
section digests verified. Standard stacks contain 109 H.264 blocks/654 frames;
eight additional perspective blocks contain 296 frames. The 58 low-quality stacks
contain 420 JPEG planes, 360 × 360. Thumbnail dimensions vary: 85 at 300 × 300 and 24
at 1080 × 1080. View-stack companions contain JSON, not image planes. Depth and
confidence are 328 × 328 little-endian f32 here, not the 330 × 330 historical example.
They are relative focus data, not metres; confidence slightly exceeds 1 in one
floating-point range, so preserve originals and use justified tolerance.

The inspected library is private and authorized for local validation only.
No captures, calibration bundles, names, dates, source UUIDs/serials, row values or
individual content hashes may enter Git or public CI. The durable manifest is
`fixtures/private/library-corpus-manifest.json` in the operator's main clone,
mode 0600 and Git-ignored. Disposable probes under `/private/tmp` are temporary;
the spike reports retain commands, algorithms and hashes. Public automated tests
use independently authored synthetic fixtures with provenance/license records.
Private corpus validation is reported separately, never represented as public CI.

## Per-feature GO / NO-GO

GO below means sufficient evidence to implement under the stated acceptance,
not that the product already works. Conditional features must fail visibly into
the supported baseline when their runtime/release gate fails.

| Capability | Verdict | Boundary |
|---|---|---|
| Schema 23 import, albums/order/events/metadata and original preservation | GO | Stage immutable offline snapshot; preserve unknown fields/files/status codes; synthetic nonzero ratings/flags/captions require tests |
| Static JPEG browsing and original download | GO | All 109 thumbnails decoded natively; authenticate every asset/download; bound decoded dimensions |
| F01 container/reference validation and lossless 12-bit extraction | GO | All 109 verified; sensor samples are not photographs; unknown variants preserved-only |
| LFE 11 JPEG-stack focus selection | GO | 58 stacks / 420 planes; explicit discrete nearest-plane controls using validated lambdas, no continuous optical refocus claim |
| Standard derivative EXIF orientation | GO with transform tests | Sidecar codes 1/8; preserve catalog code 0 independently and avoid double-applying baked transforms |
| Browser H.264 standard refocus | Conditional GO | Chrome 154 two five-frame representatives measured with prefer-software; full 654-frame gate and integrated Rust/WASM runtime required; actual decode failure falls back to JPEG/static |
| Native pure-Rust H.264 decoder | NO-GO | No proven published decoder for this corpus; no FFmpeg/C++ runtime added |
| Optional Rust/wgpu derivative presentation | Conditional GO | Dependency compilation and direct-WebGPU compute evidence; require real Rust/WASM texture-to-canvas correctness, budget, loss and presentation tests |
| GPU kernel performance guarantee / cross-browser parity | NO-GO as current claim | Headless compute is not submit-to-present timing; Safari/Firefox/mobile/Windows/Linux clients unmeasured |
| Preserve depth/confidence and recipe artifacts | GO | Preserve raw bytes/types; bounded finite-map inspection; no metric depth inference |
| Depth picking, perspective/stereo/tilt, aperture synthesis | NO-GO | Coordinate/mapping/occlusion semantics and output quality unverified |
| Imported legacy edit appearance parity | NO-GO | Matching stored values do not establish processing transfer functions; preserve unknown edits rather than silently render guesses |
| Flattened verified derivative export / original bundle backup | GO to implement | Explicit plane/orientation/provenance and loss of refocus ability; byte-exact originals and album metadata survive restore. Bundles have no original size or count limit; this release's logical catalog section holds about 33,800 captures (measured, docs/design/007) and a larger catalog fails at backup time without a bundle |
| Calibrated F01 synthesis | NO-GO | Four local data.C bundles have 195 valid sections, 191 references, 63 sensor payloads but 62 MOD records; 46-character trailers unverified; 31 lens-setting pairs have zero exact matches with 60 capture pairs |
| Illum or other active Desktop schema semantic support | NO-GO | Zero authorized local samples, paired calibration or alternate active-schema fixtures |
| Linux amd64/arm64 self-hosted operation | GO to implement, unverified now | CPU-only nonroot builds and actual startup/import/backup tests on both architectures required before claiming support |

## Chosen architecture

**Rust throughout application logic.** Native independent container/metadata
handling and pure-Rust JPEG decoding; Rust HTTP service and persistent jobs;
Leptos Rust/WASM client. CSS, WGSL and generated browser glue are permitted web
assets. Browser platform codec/DOM APIs are called through Rust bindings. No
Python/C#/FFmpeg processing service or C++ H.264 decoder is introduced. Research
scripts and the local GPL-enabled FFmpeg reference are not runtime dependencies.

Use target-separated workspace crates for format/metadata, shared contracts,
storage/catalog/jobs, native service/diagnostics and client/viewer. Native targets
must not depend on browser or GPU APIs. Browser targets must not pull in SQLite
or native codecs. Preserve a deterministic CPU/image baseline; GPU is optional.

| Layer | Selection and validation status |
|---|---|
| Toolchain | Existing mise Rust 1.99.0/just 1.58.0; preserve lockfiles and update pins deliberately |
| Service | Axum 0.8.9/Tokio; exact Axum native API compilation passed; persistent HTTP/auth behavior remains implementation work |
| Catalog/jobs | SQLite with SQLx; selected persistence direction, exact SQLx/Tokio versions and host/WASM separation must pass foundation compile/license audit; migrations/rollback tests required |
| Browser DOM | Leptos 0.8.22 `csr`, no required SSR; static assets served by Axum |
| JPEG | image 0.25.9 with jpeg feature / zune-jpeg 0.5.15; actual native decoding 529/529 images; WASM decode path still needs acceptance |
| Browser bridge | wasm-bindgen 0.2.129, web-sys/js-sys 0.3.106, wasm-bindgen-futures 0.4.79; match CLI to resolved crate; WebCodecs features/unstable cfg compile-tested separately |
| Optional canvas renderer | wgpu 30.0.1, WASM default features disabled with webgpu/wgsl; real integration gate, CPU/static fallback |
| Optional browser video | WebCodecs Annex-B via Rust; derive SPS codec/keyframe/access-unit boundaries, close VideoFrames and respect visible crop; no native codec fallback promised |

Pin the actual implementation lockfile after compilation and a complete
transitive license audit. Version evidence and direct dependency licenses are
linked in the spike reports; no future dependency graph is claimed audited yet.
Retain required MIT/Apache or other permitted notices. Validate ordinary storage
and authentication dependencies before acceptance; do not assume platform
compilation merely because their API is familiar.

### Preservation, authentication and capability rules

First-run setup is local and explicitly authenticated before upload/asset access.
Use secure credential hashing, sessions/CSRF, bounded requests and documented
reverse-proxy trust; historical album `public=1` is metadata, never permission to
publish. Originals are immutable content-addressed files on an operator volume,
with separate staging, SQLite catalog and disposable derivative cache. Hash and
verify before exposing transactional catalog records. Uploaded SQLite is data:
read-only/query-only staged offline snapshot, no extension loading/attachments,
no execution of source triggers or imported code. Reject unhandled WAL/journals.

Use independent SHA-256+length and byte verification for dedup; all observed
`picture.hash` fields are empty. Preserve logical albums/capture associations
separately from physical-byte reuse. Conflicting IDs/new bytes create explicit
versions or quarantine, never overwrite. Unknown source files and fields remain
in the preservation manifest. Durable jobs recover after interruption without
committed rows pointing to incomplete originals. Proposed defaults: 20 GiB actual
uncompressed upload, 100,000 files, 512 MiB/file, 4 KiB path, depth 32, with streaming
limits and disk reservation; these are targets, not measured large-library claims.

Expose separate capability states for original preserved, preview available,
JPEG stack available, browser codec validated, sensor decoded and calibrated
synthesis unavailable. Never turn missing calibration/codec into data loss or a
misleading control. Do not double-apply complete imported recipes to processed
pixels. Only explicit plane selection and tested standard orientation are initial
rendered transforms; other legacy edit fields remain preserved-only.

### Viewer and design

Use the [synthetic design concept](../spikes/assets/lytronaut-0mz-concept.png):
graphite shell, cyan links/focus, subtle beveled header/gradients, compact controls,
album rail, filmstrip, large photo stage and small-radius panels. Real application
screens must retain keyboard navigation, labelled numeric ranges, visible focus,
readable contrast, 200% zoom, screen-reader/forced-colors checks and reduced motion.
The concept screenshot is a design artifact, not a running product.

Static/CPU image display is authoritative. Optional GPU presentation of verified
derivatives may perform bounded orientation/scaling; it must not invent
light-field geometry. Select GPU only after device/pipeline/limits success,
release resources on loss and retain usable controls/last frame with fallback.
At most 2 asset requests, 8 MiB/chunk, 256 KiB manifest, 48 MiB encoded viewer asset;
≤1024² interactive previews, 2048² hard representation cap then downsample/tile;
≤64 MiB explicit GPU and ≤96 MiB counted CPU/WASM working allocations. These proposed
caps must be enforced/tested; measured total browser RSS/driver residency is separate.

The synthetic GPU benchmark measured 1024² completion median 1.2 ms / p95 2.2 ms,
versus Rust/WASM CPU 6.2/6.5 ms, with exact integer agreement. It excludes real textures,
optical/color processing and onscreen presentation. Target actual warm GPU
submit-to-present p95 ≤16.7 ms on declared reference hardware, CPU/precomputed
interaction p95 ≤100 ms at ≤512², and fallback indication ≤1 s after observed loss.
Actual texture orientation/scaling must match CPU fixtures exactly for integer
operations; justified float color tolerance ≤1 eight-bit code / RMSE ≤0.5 may apply only
when a defined float pipeline exists. Failed budgets reduce optional quality;
they must never make baseline browsing unavailable.

## Supported implementation phases and measurable acceptance

The complete runnable dependency DAG is [plans/implementation.yaml](../../plans/implementation.yaml).
It links provenance to research epic `lytronaut-ck2`; implementation epic
`lytronaut-re8` is now filed and approved for delivery.

1. **Foundations and safe intake.** Separate native and WASM dependencies, validate
   licenses and contracts, then add authentication before uploads. Implement
   bounded ZIP and directory upload, immutable storage, durable jobs and a
   schema-23 catalog adapter. Acceptance includes traversal, symlink, archive
   bomb, collision, quota and WAL rejection. Inject failures around writes,
   fsync, rename and catalog commits. Repeated imports must deduplicate safely;
   altered files create explicit conflicts. Source hashes stay unchanged.
2. **Usable library browsing.** Add bounded format/JPEG parsing, authenticated
   asset APIs and the graphite/cyan Rust client. Validate all 109 pictures,
   three albums and 109 ordered memberships, including events, metadata and
   unknown files. Decode every thumbnail. Test Unicode and nonzero status fields
   with synthetic fixtures, keyboard access, contrast, zoom and reduced motion.
   The browser must work without GPU or video codecs.
3. **Verified derivative interactions.** Support the 58 JPEG stacks and their
   420 planes using explicit nearest-lambda selection and tested orientation.
   Optional Rust/wgpu presentation and browser H.264 have separate runtime gates.
   Validate actual Rust/WASM flows, visible crop, bounded memory, frame cleanup
   and failure recovery. Decode all 654 standard H.264 frames in a private
   browser report before claiming corpus-wide compatibility. The 296 perspective
   frames remain preserved-only. New calibrated synthesis is deferred.
4. **Export and CPU-only self-hosting.** Add verified flattened JPEG/PNG export,
   original bundles and catalog backup/restore. Build non-root Linux amd64/arm64
   containers with persistent volumes, readiness and graceful shutdown. Check
   original hashes, albums and unknown fields through restoration. Run startup,
   import, restart and restore smoke tests on both architectures without GPU
   devices. Document memory limits and offline operation after installation.
5. **Acceptance and handoff.** Run meaningful public synthetic CI and a separate
   private corpus report. Review transitive licenses, notices, authentication,
   redacted diagnostics, quotas and recovery. Provide operator guides and a
   verified support matrix. CI may build containers; release and image publishing
   remain disabled pending separate authorization.

## Future evidence prerequisites, not implementation promises

Calibrated F01 synthesis needs the MOD wrapper identified, complete paired
sensor metadata, independent camera/lens selection or interpolation, and measured
microlens grid, flat-field and color quality against permitted reference exports.
Proposed held-out center residual targets are median ≤0.25 sensor pixel and
p95 ≤0.5 pixel; a proposed central white-field coefficient of variation target is
≤5%. These are provisional scientific targets, not measured product quality.
Confirm that they predict visible artifacts before adopting them as guarantees.
Metric rectification additionally needs checkerboard captures.

Illum needs authorized LFR/RAW/processed/Desktop samples, matching pairing
archives or XRAW, verified 10-bit packing and CFA, and measured calibrated
outputs. Other Desktop schemas need versioned adapters and fixtures. Depth
picking needs verified coordinates and confidence interpretation. Perspective,
aperture and stereo require geometry, occlusion and rendering references.
Imported edit parity needs independently measured transfer functions and
transform precedence. Browser expansion requires actual platform runtime tests.
These deferred capabilities have no implementation beads in the approved MVP.

## Reference evidence and MIT boundary

| Reference | Research contribution | Reuse boundary |
|---|---|---|
| [lytro-py](https://github.com/yamada-sexta/lytro-py) | Extracted RAW/TXT, 12-bit packing and calibration | No root license observed at reviewed revision; no port |
| [lytro-lumi](https://github.com/hypothete/lytro-lumi) | Experimental browser GPU viewer | Camera-specific preprocessing; no root license observed; no port |
| [Jaramilloh](https://github.com/Jaramilloh/Process-Light-Fields-Captured-With-Lytro-Camera) | Original calibration/rectification | CC-BY-4.0 project; MATLAB toolbox separate terms; no copied code/fixtures |
| [plenopticam](https://github.com/hahnec/plenopticam) | Illum/calibration/refocus research | GPL-3.0 reference only; no translated code |
| [Lytro.Net](https://github.com/mscappini/Lytro.Net) | Older container/metadata exploration | No root license observed; no port |

Reviewed HEAD prefixes respectively dcd1c106, d5f1eafc, aae3d3c2, 24bb5237, 55dc84ba.
The README retains their Acknowledgement links. Attribution is not reuse
permission. Independent original application code is MIT; third-party code,
assets and private samples retain their own rights. Review licenses/provenance
at the exact future dependency/fixture revision before redistribution.

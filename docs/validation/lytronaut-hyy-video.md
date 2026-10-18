# Archived browser AVC validation

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

Historical implementation proof: `29edb81dae0a598b9ab4d97046994c209a6756ca`, tree
`3ab7de5b41f63690bb2e0cbbb2053294a055a8fa`.
[Aggregate evidence](assets/lytronaut-hyy-video.json) contains only counts,
environment and generated program artifact hashes. Private captures, metadata
identities, original hashes, source paths, pixels and screenshots are absent.
This report binds that exact source snapshot. The final integrated submission
requires a separate proof bound to its final commit and generated artifacts.

The **actual integrated Rust/WASM decoder** decoded all **109 standard blocks /
654 frames** on Chrome 154.0.8037.98, macOS 26.6.2 (25G83), MacBookPro18,1,
Apple M1 Pro. A five-frame representative of each LFE 10 and LFE 11 variant
decoded twice, including its corpus pass. The support/configure/decode/flush,
software retry, visible geometry, frame lifecycle and selected-frame canvas/JPEG
flow are Rust application code called by `scripts/video-test.cjs`. This is
separate from the earlier disposable JavaScript research probe. Stable Rust
1.99.0 and wasm-bindgen 0.2.129 built an isolated **debug** WASM artifact; this
functional result makes no latency or hardware acceleration claim.

All 109 streams required successful `prefer-software` retry after the initial
`no-preference` attempt failed. Software preference is a browser request, not
proof of its internal implementation. All **664 decoder outputs were closed**;
the input decode queue never exceeded **two**, and no uncaught page errors
occurred. No perspective blocks were offered or decoded by this product path.
Other browsers, OS versions, hardware and Chrome versions remain unverified.

Normative coded geometry comes from each SPS, including whole-frame MBAFF map
height and chroma crop units. The observed private sequence geometry is
1088 × 1088, while Chrome frame storage is 1088 × 1090; visible/display geometry
is 1080 × 1080. The independent public fixture also demonstrates decoder storage
padding: 32 × 32 SPS geometry, 32 × 34 browser storage, 32 × 32 visible output.
Code never embeds either representative geometry. It permits at most one
macroblock of additional storage padding within the common 2048² coded limit,
and requires exact SPS-derived visible/display extents and crop origins.
Whole field pictures, unsupported chroma/depth and changing/mismatched parameter
sets remain unavailable. AVCC `description` is deliberately absent; original
Annex-B SPS/PPS and AUD access-unit boundaries are retained.

All 109 metadata arrays have finite, unique lambdas. **83** are strictly
increasing and **26** are nonmonotonic. H.264 metadata/AU order is preserved;
selection uses true extrema and nearest lambda, with ties choosing the lower
archived index. The independent nonmonotonic colored-frame fixture verifies
selection of the correct blue plane. JPEG's 58 stacks retain their existing
strictly increasing lambda contract. Stored parameters do not establish Desktop
pixel parity or calibrated synthesis.

Before and after the codec run, full SHA256, size, mode and mtime checks verified
all **728 logical originals / 638 unique sealed blobs**, representing
**2,001,830,074 logical bytes**, unchanged and read-only. This is a sealed-copy
integrity check, distinct from the coordinator's audit of the source library.
The test reads only the authorized sealed copy and uses an isolated loopback
fixture server; it never opens the original library or a writable application
store. Its Chrome profile and server are closed after each run.

Public unit/property/adversarial tests use independently authored MIT syntax and
colored-frame fixtures. The actual WASM codec suite covers absent VideoDecoder,
unsupported and synchronously throwing support queries, cancelled pending
support/flush, actual no-preference decode failure and software retry, late
decoder error, inconsistent frame geometry, missing canvas context, cancellation
before/during decode, corrupt framing, bounded queue, output SHA256/actual JPEG
geometry and recovery. Native tests cover SPS/PPS/slice associations, mixed
IDR/non-IDR rejection, MBAFF crop units versus whole field pictures, prefix and
parameter fuzz, byte/cardinality budgets, nonfinite/duplicate lambdas, integer
nearest-selection properties and finite subtraction overflow.

The existing canonical and HTTP viewer gates are required before submission.
This evidence does **not** claim 95% first-party native plus actual WASM line
coverage. The final acceptance bead owns that source-inclusive instrumentation
and its enforcing threshold, without application/error/security exclusions.

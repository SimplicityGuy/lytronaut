# Export and lossless restore validation

Implementation commit: `67545924a8e42477e2eb02d628d53fbe5ea4362e`, tree
`4274b3cb993baca8091402c3dc8477c2d44895d2`. The first evidence and test-report
commit leaves all application source objects unchanged. The [aggregate evidence](assets/lytronaut-u8i-export.json)
records exact release program artifact hashes, reference hardware and results.
Shared Cargo builds were serialized; native service, private harness, synthetic
fixture generator and release WASM/glue were frozen before releasing the build slot.

All 82 native tests, native/WASM Clippy/build, dependency boundaries and the
267-package permissive license inventory passed the canonical check. Tests use
independent synthetic colored pixels for all eight EXIF transforms and check PNG
exact pixels, JPEG integrity, separate provenance, preserved original bytes,
unsupported operations, authentication and request-origin failures. Export decodes
the stored JPEG directly for PNG and never claims to replay imported adjustments.

Restore tests preserve unknown source SQLite tables, fields and scalar types as
opaque original bytes. They reject malicious checksummed app databases containing
triggers, views, generated tables, revived auth rows, unfinished jobs or mismatched
associations; they also reject truncation, trailing bytes and destination conflicts.
Actual HTTP tests cover disconnected uploads, DELETE during a stalled upload,
bounded cancellation during copying, completed restore deletion, unrelated-file
preservation and retained-storage accounting returning to zero.

The protected full roundtrip verified all 728 logical original files,
2,001,830,074 bytes and 638 unique blobs. All source digests, sizes and modification
times remained unchanged. All 109 captures, three albums and 109 ordered
memberships, typed original records, metadata and stable API IDs matched exactly.
The original schema-23 database and unknown database/file bytes survive unchanged.
Only the isolated application catalog snapshot is scrubbed of authentication and
unfinished transient jobs. Restore never overwrites the source or live database.
The coordinator inspected protected aggregate and program provenance evidence
before authorizing removal of the owned temporary archive and restored copy.
No private pixels, source identifiers, paths, rows or digest inventory are published.

The complete actual Chrome/Rust-WASM synthetic suite passed against the frozen
release artifacts: flattened PNG geometry/hash/colored corners and recipe,
flattened JPEG, original archive download, file upload and isolated restore, then
all existing focus/integrity/cancellation/import/session faults, navigation, AX,
responsive layout, reduced motion, forced colors, real 200% zoom and URL cleanup.
There were zero page errors and zero asset429 responses. The unchanged focus
100 ms gate passed: 83.4 ms p95 with four grid previews pending, 83.6 ms warm p95.
Reference: Apple M1 Pro, 32 GiB, macOS 26.6.2, Chrome 154.0.8037.98; both native
and WASM release profiles. Two earlier release runs stopped at the timing gate
before reaching export checks; one overlapped private hashing/copying, the other
preceded scratch cleanup. Those failures are retained separately. An intervening
timing-disabled functional run passed but is not performance acceptance evidence.

Before submission, the current GPU integration branch was merged at
`6747441c89983be9a5b801143b1c5dfbc0c35d47` (tree
`f56e98f1502b14448af71332f4347b6b1c58fffc`). The service, bundle, storage, decoder
and contract source objects remain identical to the private roundtrip revision.
The combined Rust/WASM client and 298-package license inventory were rebuilt
and frozen. Actual WebGPU regression passed exact pixels for EXIF1–8 and scaling,
automatic acquisition/device/pipeline/loss fallbacks, explicit device cleanup,
stable WASM memory, and the complete flattened export/recipe/archive/restore and
existing functional browser assertions with zero errors or asset429 responses.

Combined CPU fallback timing acceptance did not pass: two release runs reported
grid/warm p95 150.1/150.7 ms and 104.7/134.0 ms. These failures and the exact
combined program hashes are retained in the aggregate evidence. The combined GPU
functional run disables the CPU timing assertion explicitly; it cannot count as
CPU performance acceptance. GPU post-frame callbacks also do not establish the
16.7 ms physical submit-to-present gate. The final integration acceptance must
resolve these timing gates using isolated release native and release WASM runs.

This bead does not certify 95% native or actual WASM line coverage, calibrated
synthesis, imported adjustment replay, browser video export, Live Photo or manual
VoiceOver. The final acceptance bead owns coverage and combined release timing.
The video integration must disable flattened export while verified video is
displayed, requiring an explicit switch to JPEG/static pixels; archive remains
available. Deployment-level access control replaces legacy login in its separate
authorized bead, which must update the exact app-schema snapshot preflight.

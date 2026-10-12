# Discrete JPEG focus validation

Implementation revision: `cef102f4198c96b36f6507e8a24ca8db8e15467d`.
The later evidence-only commit changes documentation, with identical application
source objects listed in the [aggregate report](assets/lytronaut-m5h-focus.json).
Release artifacts were rebuilt from this exact checked-out implementation before
measurement; hashes below identify program artifacts, never private source files.

Reference: MacBookPro18,1, Apple M1 Pro, 32 GiB RAM; macOS 26.6.2 (25G83);
Chrome 154.0.8037.98; Rust 1.99.0; wasm-bindgen 0.2.129. Both native service
and Rust/WASM client used release profiles. One owned service and isolated Chrome
ran without concurrent task builds/browser tests. The generated client directory
was passed with `LYTRONAUT_CLIENT`; reproducible release commands are in
[the browser architecture guide](../architecture/browser.md).

All 109 captures, three albums and 109 memberships (12/39/58) remained usable.
Authenticated native generation plus Chrome decoding verified all 58 JPEG stacks
and all 420 stored planes, finite strictly increasing lambdas, typed selection/
geometry, generated response SHA256 and decoded output extents. The largest
source stack was 739,984 bytes, below the enforced 8 MiB cap. No private images,
paths, source identities, database rows or source hashes are in this evidence.

| Measurement | Samples | Pending grid previews at start | p50 | p95 | Max |
|---|---:|---:|---:|---:|---:|
| First interactions during observed grid load | 24 | 13 | 66.7 ms | 83.8 ms | 85.9 ms |
| Warmed interactions after decoded grid | 24 | 0 | 66.6 ms | 77.1 ms | 77.7 ms |

Both unchanged p95 ≤100 ms UI gates passed at 360 × 360 source geometry.
The measurement ends after verified Rust/WASM decoding, stage image decoding
and the next animation-frame callback: frame ready, without certifying physical
presentation. Native HTTP plus Chrome decode p95 was 6.8 ms across all 420
planes. Development profiles are slower: the final debug synthetic run recorded
154.2/152.2 ms p95 first/warm UI batches, and is functional evidence rather than
performance acceptance. The earlier mixed native-release/WASM-debug WIP miss is
retained in the architecture guide.

The complete final synthetic browser fault suite ran against the implementation
revision: three independent colored-corner planes, baked orientation 8 versus
sidecar 6 default identity, explicit orientation 6 visible corner rotation and
rectangular output, keyboard Home/End, corrupt derivative retaining the verified
frame, held focus cancellation on capture change, static fallback, all original
preview/import/recovery/ZIP/ZIP64/session/sort faults and exact-original checks.
Actual 200% DPR/CSS zoom, responsive layout, forced colors, reduced motion, AX,
pre-restart canvas/URL cleanup and logout URL cleanup passed with zero page
errors and zero asset429 responses. The source fixture now has one focus stack
and conflicting edit sidecar; native preserved-only file count assertions changed
from seven to nine accordingly. Corrupt focus capability is now explicitly
`unavailable(corrupt_asset)` rather than the prior unimplemented `disabled` state.

All 75 native tests, native/WASM Clippy/build, boundaries and 263-package licenses
passed in the working-tree canonical gate. The submission separately records a
pristine exact-submission-SHA gate. Native tests include EXIF1–8 coordinate/reflection/
inverse geometry, nonfinite/clamped/tie selection, typed metadata adversaries,
small working/output encoder limits and response-permit EOF/drop. A mutation
experiment changed EXIF6 to the incorrect transpose: the independent colored-
corner oracle failed; restoring the implementation passed. Source archives,
recipes and original bytes are never rewritten by derivative generation.
Post-run every unique sealed original blob was SHA256-verified against its
existing ingested manifest and remained read-only: 728 logical files,
2,001,830,074 logical bytes, 638 unique blobs / 2,000,902,860 physical bytes.
A separate coordinator audit rechecked the original library against its private
baseline: all 728 regular files matched SHA256, size and mtime, with zero
symlinks, read errors or mismatches. This is distinct from the sealed-copy check.

Memory admission is conservative: 8 MiB encoded source, 64 MiB codec/transform/
encoder working cap, 8 MiB metadata/slack and 16 MiB transfer reserve within
96 MiB CPU; client transient leases stay within 64 MiB plus 32 MiB persistent
reserve. Producer metadata and manifests are limited to 256 KiB, each derivative
is limited to 2 MiB, shared transfers to two, and CPU validation is serialized.
The capped encoder admits RGBA/RGB/scratch/output growth before allocating and
caps each writer operation; EXIF1 reuses its validated decoded pixel allocation.
Plane pixels are not cached. The future GPU budget remains 64 MiB.

LFE10 without verified JPEG planes remains static until the downstream browser
H.264 gate. Imported aperture/perspective/tone/denoise/animation and unknown fields
remain preserved-only. Processed recipes may be baked, so identity is default;
explicit user EXIF transforms are separate from preserved sidecar/catalog edits.
Catalog 0 is invalid EXIF. No Desktop parity, calibrated synthesis, Illum support,
manual VoiceOver certification, or measured 95% native+WASM line coverage is
claimed. The final acceptance bead owns coverage instrumentation and exact CI
acceptance; this proof supplies meaningful tested paths for that gate.

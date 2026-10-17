# Supported variants and capabilities

This is the release capability matrix for the initial version. A capability is
**supported** only where the evidence column names a passing test or recorded
validation. Everything else is stated as conditional, not verified or out of
scope; a failing optional gate narrows that capability and never removes the
supported baseline (original preservation, static JPEG browsing, download).

The source repository is public. Originals, pixels, catalog identifiers, camera
serial numbers and private validation reports stay local: CI and every committed
report use independently authored synthetic fixtures only. Release builds and
image publication remain disabled; images are built locally.

## Library and camera variants

| Variant | Status | Evidence / boundary |
|---|---|---|
| Lytro Desktop library, schema 23 | Supported | Synthetic import, album order, metadata and restore tests in CI; private 109-capture library validated locally (aggregate counts only, [validation](#private-library-validation)) |
| Desktop library, other schema versions | Preserved only | Byte-exact preservation; no semantic import (synthetic schema-99 fixture) |
| Desktop library larger than about 1,700 captures | Bounded streaming | No fixed capture/cell-count ceiling; source DB <=32 MiB and value/parser/scratch bounds still apply ([limitations](linux.md#limitations), [validation](../validation/lytronaut-5ld-desktop-streaming.md)) |
| Original Lytro camera (F01) containers | Supported: preserve, validate, download | Container/reference validation and lossless 12-bit extraction; sensor samples are not shown as photographs |
| LFE 11 JPEG focus stacks | Supported: discrete focus | Nearest stored plane by validated lambda; no continuous optical refocus |
| LFE 10/11 archived H.264 focus stacks | Conditional | Browser WebCodecs only; decode failure falls back to JPEG/static ([H.264](#browser-h264-focus)) |
| Lytro Illum, other Desktop schemas | Not supported | No authorized samples or calibration; files are preserved, never guessed |
| Calibrated raw light-field synthesis, perspective, aperture, depth picking | Not supported | No verified calibration/mapping ([design 001](../design/001-lytronaut.md#per-feature-go--no-go)) |
| Imported Desktop edits | Preserved only | Kept as unknown recipe data; only verified orientation transforms are applied; no appearance-parity claim |

## Deployment and platforms

| Target | Status | Evidence / boundary |
|---|---|---|
| Linux amd64 and arm64 containers (CPU only) | Supported | Native GitHub runners build the image and pass startup, import, browse, export, backup and restore with SQLite, bundled PostgreSQL and external PostgreSQL ([operation](linux.md), [deployment validation](linux.md#deployment-validation)) |
| Native service on macOS for development | Development only | `just serve`, `just demo`, tests and coverage run there |
| Catalog backends | SQLite (default), PostgreSQL 18 | Strict PostgreSQL parity suite in CI against the pinned release ([design 006](../design/006-database-boundary.md)) |
| Access control | Deployment level | No application accounts or login; protect access at your HTTPS proxy ([ADR 002](../design/002-deployment-access.md)) |
| Server GPU / hardware codec | Not required | The service never uses one |

## Browser capabilities

| Capability | Status | Evidence / boundary |
|---|---|---|
| Library browsing, albums, search, original download (no GPU, no codec) | Supported | Actual Rust/WASM client in headless Chrome against the real service in CI (`just browser-test`) |
| Discrete JPEG focus | Supported | CI synthetic stacks; private 58 stacks / 420 planes validated locally at the release source |
| CPU focus latency p95 at most 100 ms | **Not met** | Release measurements exceed 100 ms; the later merged-tree runs at `b81e689` measured 137.9–150.4 ms. Historical stages and limitations remain visible ([design 004](../design/004-live-photo-export.md#owner-results-and-accepted-limitations-2026-10-13)) |
| Browser H.264 focus <a id="browser-h264-focus"></a> | Conditional | All 109 private streams / 654 frames decoded on Chrome 154, macOS, Apple M1 Pro, by the actual Rust/WASM decoder path with `prefer-software` retry ([report](../validation/lytronaut-hyy-video.md)); functional only, no latency or hardware-acceleration claim; other browsers unmeasured |
| Rust/wgpu GPU presentation | Conditional, automatic | Probed automatically, falls back to CPU on any failure. Correctness verified on a physical-GPU reference host (Apple M1 Pro, Chrome 154: exact pixels for all eight EXIF transforms, [report](../validation/lytronaut-us7-gpu.md)). Software adapters (SwiftShader) are never reported as physical-GPU results |
| GPU compositor presentation target (16.7 ms) | Not verified | Accepted limitation; headless compute timings are not submit-to-present evidence |
| Safari, Firefox, Windows, Linux and mobile browsers | Not verified | Only Chrome 154 is measured |
| Screen readers (VoiceOver) | **Not verified** | Owner signed off as out of scope for the initial version (2026-10-13); the only evidence is automated accessibility-tree inspection |

## Export, backup and restore

| Capability | Status | Evidence / boundary |
|---|---|---|
| Flattened PNG/JPEG export with provenance | Supported | Exact-pixel tests for all eight EXIF transforms; originals unchanged |
| Apple Live Photo pair export | Supported on macOS Photos | Owner-reported PASS importing into an isolated Photos library ([design 004](../design/004-live-photo-export.md#owner-results-and-accepted-limitations-2026-10-13)); counted 64/32 MiB memory budget only partly proven (accepted limitation) |
| Live Photo on iOS | **Out of scope** | Owner sign-off for this release: not verified and not supported |
| Portable backup and restore | Supported, any size | 64-bit bundle, streamed with constant memory; CI round-trips a bundle above 4 GiB (`large-bundle` job). The logical catalog section holds about 33,800 captures; a larger catalog fails at backup time without a partial bundle ([sizing](linux.md#backup-bundle-size-and-free-space)) |
| Restore into a running deployment | Isolated copy | Restores into a separate store you verify before a stopped-service volume swap |

## Test coverage rule

CI enforces at least 95% first-party line coverage for native code, the actual
Rust/WASM client running in a real browser, and their union
([coverage](coverage.md)). GPU-specific WebGPU rendering code is an explicit,
itemised, owner-approved exception (2026-10-13): it is measured and reported on
its own row, kept out of the WASM and union percentages, may not grow beyond its
recorded baseline, and is verified instead by physical-GPU reference-host runs.

## Private library validation

The operator's private library is validated locally only. The retained result is
aggregate and non-identifying (counts, pass/fail, program hashes); no path,
name, identifier, hash of a private file or pixel leaves the machine. Current
aggregate: see [the final validation report](../validation/lytronaut-cjq-final.md).

## Open items (next evidence prerequisites)

- Required-check enforcement on `main` through homelab (not yet enforced;
  [adoption procedure](required-checks.md)).
- Exact delivery-revision CI, coverage and container results after final changes.
- CPU focus p95 at most 100 ms on the release reference host (not met).
- Performance under grid load at the current source.
- H.264 Level 5 frame-size check (1138 x 1842).
- Live Photo export end to end on PostgreSQL.
- Physical-GPU presentation timing (compositor target) and non-Chrome browsers.
- Illum and calibrated synthesis need authorized samples and calibration data.

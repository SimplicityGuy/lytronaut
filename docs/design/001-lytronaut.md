# Lytronaut: architecture, compatibility gates, and roadmap

Status: proposed; initial research molecule awaits operator review and kickoff.
Name: **Lytronaut**. Repository: `SimplicityGuy/lytronaut` (private).
Tagline: **an explorer for your light-field library**.

## Intent and scope

Build an MIT-licensed, self-hostable Rust application that imports old Lytro
Desktop libraries and individual captures, preserves the originals and library
organization, and lets people browse, refocus, change perspective and aperture,
make non-destructive adjustments, and export useful photographs.

Support both original Lytro and Illum through independently verified variants.
Every capture exposes capabilities: archival-only, embedded preview, processed
focus stack, calibrated light field, imported depth. Missing calibration and
unknown sections must never become silent data loss or claims of full support.

The initial project is a toolchain/CI scaffold, not an implemented product.
Setup is authorized; application implementation and dispatch require the user's
later approval. No publication, cloud account, external telemetry, or server
GPU is required. New depth estimation, camera USB control, native mobile apps,
cloud sync, and reconstruction of lost source data are outside the first MVP.

## Evidence gathered on 2026-10-11

Read-only inspection of an operator-supplied private Desktop library found:

- SQLite `lytrolibrary.db`, library metadata version **23**.
- **109 pictures, three albums, two image bins**, album membership and order,
  event records, captions, ratings, flags, and picture metadata tables.
- **109 raw LFPs, 109 depth-map LFPs, 109 view-stack LFPs, 109 stack LFPs,
  58 low-quality stacks, 109 JPEGs**, plus JSON edit sidecars and backup material.
- Camera model identifier `1` for the cataloged pictures; this identifier needs
  cross-checking against capture metadata before classifying the model.
- Light-field engine major versions 10 and 11. Edit keys include crop, focus,
  tilt, aperture, perspective, exposure, white balance, tone, noise reduction,
  sharpening, saturation, and orientation.

Only schema, counts, file headers, and sidecar key names were inspected. The
source stays read-only. No personal images, UUIDs, serial numbers, or database
rows are committed or uploaded. Private fixtures remain operator-local.
This establishes a useful original-library corpus, not Illum compatibility.

### Reference evidence and reuse policy

| Reference | Useful research | Implementation boundary |
| --- | --- | --- |
| [lytro-py](https://github.com/yamada-sexta/lytro-py) | Extracted RAW/TXT, packed 12-bit pixels, calibration, subapertures | Does not parse LFP; no root license observed at reviewed revision |
| [lytro-lumi](https://github.com/hypothete/lytro-lumi) | Browser GPU refocus, viewpoint, aperture | Experimental preprocessing and camera-specific constants; no root license observed |
| [Jaramilloh](https://github.com/Jaramilloh/Process-Light-Fields-Captured-With-Lytro-Camera) | Original camera rectification, color and sample captures | CC-BY-4.0 project; bundled MATLAB toolbox has separate terms |
| [plenopticam](https://github.com/hahnec/plenopticam) | Illum decoding, calibration, refocus, depth | GPL-3.0; reference only, no copied or translated code |
| [Lytro.Net](https://github.com/mscappini/Lytro.Net) | LFP sections, metadata, focused JPEGs | Older format evidence; no root license observed |

Reviewed source HEAD prefixes: `dcd1c106`, `d5f1eafc`, `aae3d3c2`, `24bb5237`,
and `55dc84ba`, respectively. Verify licenses again before any reuse.
Independent format and algorithm implementations are MIT; attribution alone
does not authorize copying code or redistributing images. Fixture provenance
and redistribution rights must be recorded individually.

References indicate original-camera 3280 × 3280 packed 12-bit Bayer and Illum
7728 × 5368 packed 10-bit Bayer with different metadata/CFA. Treat these as
sample hypotheses, not hardcoded rules. LFP is a section container that can hold
raw data or derived content. Processed stacks may contain JPEG or H.264;
calibration bundles differ by generation and camera serial/zoom/focus.

## Decisions proposed for review

1. **Rust throughout:** native parsing and image processing; a Rust HTTP service;
   Rust/WASM application logic in the browser. CSS, WGSL shaders, and generated
   WASM bindings are necessary web assets. No Python/C# processing subprocesses.
2. **Independent decoder with CPU authority:** checked section lengths,
   references and metadata; exact pixel unpacking; calibrated CPU reconstruction
   as the correctness baseline. GPU work must agree within measured tolerances.
3. **Self-hosted default:** one service/image with SQLite catalog and durable
   jobs, immutable originals on a mounted local volume, and generated assets in
   a separate cache. Candidate stack: Axum/Tokio, SQLx/SQLite, Leptos/WASM,
   wgpu/WebGPU. Final versions and GPU strategy follow the spike results.
4. **Browser acceleration first:** decode and calibrate in bounded native jobs;
   transfer normalized, resolution-aware assets for interactive GPU refocus,
   perspective and aperture. A server GPU is optional future work. Fall back
   to CPU-generated/precomputed assets when GPU support or memory is insufficient.
5. **Preserve first:** copy originals into content-addressed storage, hash and
   validate before catalog commit, deduplicate safely, retain provenance and
   unknown files, and recover imports across interruption. Never open source
   library databases writable or execute uploaded database content.
6. **Honest edit support:** preserve every original edit field, but apply only
   verified semantics. Distinguish preserved-only edits from rendered edits.
   Do not claim parity with proprietary Desktop algorithms.
7. **Research before implementation filing:** complete a small spike molecule
   for corpus/layout, calibration, processed stacks, and browser GPU feasibility.
   A decision bead depends on all four. Its GO result re-enters Beadhive planning
   to file the supported implementation slices; a NO-GO narrows scope explicitly.

## First Beadhive molecule: compatibility and architecture evidence

Four independent research tasks converge on one decision. Each produces
`docs/spikes/<bead-id>-<slug>.md` with Question, Method, Evidence, Verdict
(GO/NO-GO), and Recommendation. Research probes may be disposable, but no product
application code or private fixture data lands in this milestone.

| Task | Evidence needed before implementation |
| --- | --- |
| Corpus and Desktop library fidelity | Version-23 mapping of all 109 pictures, three albums and order; raw/derived/sidecar associations; permissions and hashes; availability of Illum samples; unsupported versions policy |
| Decode and calibration | Original/Illum section and pixel variants; calibration location/association; sample-based expected images; missing-calibration behavior; independent implementation feasibility |
| Processed stacks and original edits | JPEG/H.264/depth variants; WebCodecs or other Rust-accessible decode options; browser support and server fallback; exact versus preserved-only edit semantics; licensing constraints |
| Rust WebUI and GPU budget | Rust/WASM build strategy; GPU versus CPU correctness; measured asset/VRAM/latency budget; no-GPU behavior; context loss; responsive mid-2010s visual direction |
| Scope verdict and implementation planning | Per-feature GO/NO-GO matrix, chosen stack, limitations, revised ADR, supported-variant acceptance criteria and next implementation molecule preview |

The available Desktop library satisfies initial private sample intake. Illum
and per-camera calibration coverage still need evidence. Missing Illum samples
must not block a verified original-camera MVP or be mistaken for proven support.

## Full product roadmap after the GO verdict

This is the proposed destination, not speculative implementation beads.
Each phase will be filed through `bh plan file` after its prerequisites are known.

### Phase 1 — preservation and usable library browsing

- Split crate boundaries: container/metadata decoder, calibration and CPU render,
  persistent catalog/jobs, service/API, shared DTOs, Rust/WASM WebUI, CLI diagnostics.
- Establish first-run local credentials, authenticated browse/upload/download,
  session/CSRF controls and safe reverse-proxy behavior before exposing imports.
- Define bounded library-directory, ZIP/TAR and individual-file import with
  resumable upload, checksum/dedup, progress, per-file errors, crash recovery,
  staging cleanup, and transactional catalog visibility.
- Block traversal, absolute paths, symlink escapes, decompression bombs,
  inconsistent lengths, excessive dimensions, and untrusted SQLite attachments.
- Import verified Desktop schema 23: pictures, albums/order, events, captions,
  ratings/flags, timestamps, original metadata, edit sidecars, and associations.
  Preserve unknown files and fields in an import manifest.
- Extract embedded previews and metadata; support thumbnails, albums, search,
  sorting, capture detail, capability badges and original download/export.
- Add native inspect/import CLI and a machine-readable support report.

Acceptance: the private reference library imports all 109 records and three
albums without modifying source hashes; interrupted and repeated imports recover
without duplicate records; malformed input tests fail safely; browsing/export
work without calibration, internet access, or GPU.

### Phase 2 — calibrated independent decoding

- Implement section/reference validation, exact packed pixel variants, metadata
  normalization, CFA/black-level/white-balance/color transform handling.
- Resolve factory calibration and per-camera/zoom/focus matching, microlens
  geometry, rectification and lens correction; retain provenance and uncertainty.
- Build a bounded deterministic CPU pipeline for central/subaperture views and
  refocus, with cached derivatives and visual/golden tests on authorized fixtures.
- Add each original-camera and Illum variant only after verified samples pass.

Acceptance: exact unpacking tests, malformed-section and oversized-input tests,
calibration mismatch diagnostics, reproducible CPU outputs and measured quality
at multiple capture settings. No silent guessed calibration.

### Phase 3 — interactive light-field workspace

- Produce a versioned tiled/multiresolution normalized asset format, bounded
  downloads and GPU texture residency; invalidation follows source/calibration edits.
- Implement Rust-driven GPU refocus/viewpoint/aperture, progressive loading,
  request cancellation and quality controls against the CPU reference.
- Handle absent adapters, context loss, oversized assets and mobile limits with
  useful preview/precomputed fallback; measure focus-drag responsiveness.
- Build the graphite/cyan workspace: album sidebar, photo grid, filmstrip,
  large viewer, compact inspector, focal/aperture dials and restrained animation.
  Include keyboard operation, focus states, readable contrast, reduced motion
  and responsive layouts. Validate a concrete visual prototype before polish.

Acceptance: numerical GPU/CPU tolerance and fixed benchmark scene set established
by the spike; accessible controls; useful operation on a browser without GPU;
no need for server hardware acceleration.

### Phase 4 — edits, interoperability, and export

- Persist non-destructive Lytronaut edit recipes and undo/revert; preserve
  Desktop originals and clearly identify unsupported legacy adjustments.
- Apply verified crop/orientation/exposure/white-balance/refocus controls;
  handle processed focus stacks and imported depth where the spike permits.
- Export JPEG/PNG plus metadata/edit recipe, unmodified original bundles and
  lossless catalog backups. Add cancellable batch exports and integrity reports.
- Extend library adapters only through versioned compatibility fixtures.

Acceptance: edit round trips preserve unknown fields; rendered exports match
the reference pipeline; imported metadata and album order survive backup/restore;
exports never overwrite source captures.

### Phase 5 — self-hosting, hardening, and first release

- Build non-root Linux amd64/arm64 images and Compose example with explicit
  original/catalog/cache volumes, resource budgets, health/readiness and shutdown.
- Add local first-run credentials, authenticated uploads/downloads, CSRF/session
  controls, safe reverse-proxy configuration and no unauthenticated public setup.
- Add storage quotas, retention of derivatives, backup/restore, migration rollback,
  worker concurrency limits, redacted logs and optional local metrics.
- Add fuzzing/property tests for untrusted parsers, end-to-end import/render/export
  fixtures, browser fallback tests, release checks, SBOM/notices, Conventional
  Commit version tooling and independent image/release publication approval.
- Document setup, calibration acquisition, supported variants, private fixture
  handling, troubleshooting, upgrade/recovery and honest quality limitations.
- Extend Dependabot to Docker/Compose when those manifests exist.

Acceptance: fresh Compose install and offline processing, authenticated
unauthorized-access tests, low-memory/CPU-only operation, backup restoration,
passing Linux platform builds and release gates, verified licenses/notices.

## Dependency and review gates

Research → scope verdict → import/catalog + decoder foundations → CPU calibrated
assets → GPU viewer → edit/export → packaging/release. Preservation browsing may
ship before calibrated synthesis; Illum may follow original-camera coverage.

The first molecule stays `kickoff=pending`. The user reviews this ADR and the
filed research plan, then explicitly approves kickoff before any dispatch.
The implementation molecule is separately reviewed after the evidence verdict.
New disparity/depth estimation is a later, explicitly experimental milestone.

# Live Photo acceptance reconciliation

Status: **open**. This source review does not clear the release gates.

The acceptance worktree started at application revision
`5a9ece369185c8e81908db2421d2a6d2e5223373`. Approved validation bead
`lytronaut-cjq` landed in the epic at
`37809a2294223384717cda33a4a285c8b4818788`; its complete source tree is
`0da6c71f574992f7e1d60a2a90f0468cb1a8a77e`, identical to validated `469aa4f`.
Acceptance merge `440b9b5117b6772f7fec1619be31cfef669c3a05` incorporated that
baseline cleanly while preserving the three acceptance commits. Differences
from that baseline were the encoder comment, geometry regression, acceptance
report and release-grid assertion. The merged acceptance source has not passed
the canonical check or runtime acceptance yet; neither has `main` delivery.

## Gate disposition

| Gate | Evidence and remaining work |
| --- | --- |
| Apple Photos recognition | Preserve the owner's reported pass using rebuilt pair `9fw-gate5-fbe615a`; never substitute the deleted, stale `35c7770` pair |
| VoiceOver | Not verified; owner signed off as out of scope for this version. Automated AXTree checks do not replace VoiceOver |
| Counted 64/32 MiB memory | Partial evidence, accepted limitation; unmeasured owners remain unproven |
| GPU submit-to-present ≤16.7 ms | Not verified, accepted limitation; animation-frame callbacks do not establish presentation |
| Performance under grid load | Open: retain actual pending grid requests during measured focus interactions, with a separate warm batch |
| Private-corpus export rerun | Pending reconciliation with the final integrated application source; see below |
| iOS | Owner signed off as out of scope; not verified or supported |
| H.264 Level 5 first-frame check | Geometry bounds reviewed; encoded first-access-unit bound remains open, as detailed below |
| Live Photo on PostgreSQL | **Pass for validation `469aa4f`, native HTTP flow**: five Live Photo cases passed on a byte-identical tested tree; final integrated-revision verification remains pending |
| CPU focus p95 ≤100 ms | Not met in historical merged-tree runs; quiet-host A/B and final-source measurement or explicit owner acceptance remain required |

The release benchmark now rejects a first batch with zero pending grid images
instead of accepting it as load-contended evidence. This prevents a false pass;
it does not manufacture concurrent requests or prove load-contended latency.
The future runtime recipe must establish real grid traffic before measuring.

The written owner dispositions above are recorded in the integrated
`docs/design/004-live-photo-export.md`, “Owner results and accepted limitations
(2026-10-13)”. They are
neither new approvals nor newly measured passes.

## H.264 bounds

The encoder admits even dimensions ≤2048 with ≤2,097,152 visible pixels.
The regression `admitted_geometry_fits_level_five_frame_size_and_processing_rate`
enumerates all even shapes and rejected pixel-limit shapes. It checks padded
macroblocks and the conservative 64-frame / three-second processing rate.
It allocates no images and does not run a decoder. It passed at clean source
`d2b63f27793bc9849d9c2369ec319203c2187e25` using mise-pinned Cargo: one test
passed, seven filtered out; compilation took 14.26 s and the test took 0.37 s.
The command was `cargo test --locked -p lytronaut-format --test live_photo
admitted_geometry_fits_level_five_frame_size_and_processing_rate -- --exact`,
with one build job, incremental compilation off and debug information off.
Its source identity, command, environment and output are retained in the
external evidence folder's `focused-cargo-regression.log`. This focused package
test does not replace the canonical check.

At 1138×1842, coding padding gives 72×116 = 8,352 macroblocks. Level 5
permits 22,080 and each axis ≤sqrt(8×22,080); the conservative processing rate
is 178,176 macroblocks/s, below 589,824. These checks do not prove the
first-access-unit byte constraint. Annex A.3.1(c) additionally bounds that unit
by `384 * (max(PicSizeInMbs, MaxMBPS/172) + MaxMBPS * initial_delay) / MinCR`;
Level 5 uses `MinCR=2`. The I_PCM payload alone is 3,207,168 bytes. The
zero-additional-delay bound is only 1,603,584 bytes. An applicable HRD timing
derivation and the actual encoded byte count are required before claiming pass.
The movie byte cap and average bitrate cannot substitute for that proof.

A bounded standalone probe included the actual `avc.rs` with only crate-level
pixel/error type shims, using mise-pinned rustc 1.99.0 and synthetic solid RGB.
It produced a 3,223,876-byte first IDR NAL (3,223,897 bytes including SPS/PPS).
An independent SPS reader confirmed coded 1152×1856, displayed 1138×1842,
Baseline profile 66, level 50, no reference frames and progressive coding.
The formula above therefore needs at least 14.308 ms additional initial CPB
delay; its applicable timing derivation remains unresolved. Evidence, exact
commands and synthetic source/program hashes are outside this worktree at
`/private/tmp/lytronaut-re8-3-evidence`. This does not run the Plan regression,
MOV assembly, a decoder or the canonical check.

Source: [ITU-T H.264 (08/2024), Annex A.3.1 and Table A-1](https://www.itu.int/rec/T-REC-H.264-202408-S/en).
This is a geometry and first-frame investigation, not full stream conformance.

## Existing private and PostgreSQL evidence

The validation branch's `lytronaut-cjq-final.md` records the private run at
`30800de`: 109 captures, three albums, 109 memberships, all 109 thumbnails and
420 JPEG planes; 728 unchanged source files/bytes; three successful private
Live Photo exports; and PostgreSQL restore with original-download equality.
These aggregate results do not identify captures. That earlier report left
Live Photo end-to-end on PostgreSQL open. The newer CI evidence below supplies
that native HTTP proof for the validation branch. Historical `b81e689` parity
exercised four Live Photo API cases, but does not attest the final delivery
revision.

### PostgreSQL native HTTP result (2026-10-16)

[CI run 38900423013, persistence-parity job 116971015380](https://github.com/SimplicityGuy/lytronaut/actions/runs/38900423013/job/116971015380)
succeeded with 53 passed, zero failed and zero ignored. Its exact-count
`live-photo-api` group passed all five cases at 12:27:48 UTC, including
`live_photo_create_download_complete_private_metadata_free_pair`, missing
stored-plane failure, invalid-plane/restart, ready-pair deletion/restart and
in-flight cancellation. The runner explicitly selects PostgreSQL and refuses
missing test configuration; it cannot silently fall back to SQLite.

The create/download case imports public synthetic input through HTTP, creates
a Live Photo export, polls the worker to success and downloads the complete ZIP
for all eight EXIF orientations. It verifies the decoded JPEG dimensions/color,
MOV still-image-time metadata marker, provenance, reserved-quota release,
private-marker exclusion and unchanged original bytes. This satisfies gate 9's
native HTTP end-to-end scope. It does not establish browser UI behavior, Apple
Photos recognition or independent MOV decoding/conformance; those have separate
evidence and gates.

The run's head is `469aa4fa729065eca3abdf37e81637e7254edad4`. Checkout actually
tested GitHub's PR merge `d15f97d84027030a1516fc28a7c1705a377c652e`; GitHub's
commit API and the local validation head both give the identical full source
tree `0da6c71f574992f7e1d60a2a90f0468cb1a8a77e`. This supports a source-bound
pass for validation `469aa4f`, without claiming that future integration was
tested. The retained log and metadata are
`cjq-pg-parity-469aa4f.log` / `cjq-pg-parity-469aa4f.json` in the external
evidence folder. Bead state and human gates have not been changed.

Source comparison on 2026-10-16 found no changes from private-run revision
`30800deb642d94c3bdcc7dc6956043829eb7202d` to validation revision
`469aa4fa729065eca3abdf37e81637e7254edad4` in the client, service, storage,
format and contracts crate trees, root Cargo manifests/lock, mise/toolchain
configuration, `.cargo`, or assets/web/static paths. The only changed crate file
in that interval is the coverage tooling in `crates/xtask/src/coverage.rs`.
This establishes application-source equality for that interval, not a rebuild
or new runtime measurement. The epic's `37809a2` merge preserves the entire
validated tree. The acceptance branch now contains those application fixes;
later application changes still require scoped evidence review.

Before repeating a multi-gigabyte private copy, compare the application source
and program provenance used by that run with the integrated delivery source.
Documentation/test-only differences can be explained explicitly; application
differences require a scoped rerun or an honest unverified status. Do not copy
private source identifiers, paths, hashes, pixels or calibration into this repo.

## Runtime work still required

The current host has insufficient free disk for a fresh build and excessive
load for reliable latency acceptance. No local browser, private-corpus,
PostgreSQL or performance rerun has been performed for this bead. The existing
remote PostgreSQL result above is retained instead. The bounded AVC probe above
and focused Cargo regression above are the executable correctness experiments.
Do not reinterpret the later
`b81e689` CPU p95 of 137.9–150.4 ms as the older post-export 104.7–150.7 ms stage,
or attribute either stage to the current delivery revision.

After integration and resource availability, run the canonical mise-pinned
check and focused format regression, reconcile PostgreSQL evidence against the
integrated source, and run quiet reference-host performance A/B against
`623f2b9`. Preserve exact
source/program identities and aggregate evidence outside the repository.

## Opt-in grid contention experiment

`LYTRONAUT_GRID_LOAD_TEST=1` selects `scripts/focus-grid-load.cjs` from the
existing browser suite. It requires public synthetic CPU release mode
(`LYTRONAUT_RELEASE_BENCHMARK=1`) and refuses private, owner and GPU modes.
Existing test-root, binary, client and revision configuration still applies.
No product controls or production code are added.

Each of 24 interactions uses a fresh isolated page on the same synthetic
24-capture catalog. The helper holds an actual nonselected grid preview's
single chunk, starts focus timing, then continues that real HTTP request.
It requires positive body bytes and network response completion within the
exact input-to-decoded-frame-ready interval, followed by decode of that same
capture's displayed grid image before the interval ends. No mock bytes are
returned and focus timing never waits for the grid image to finish. A late
response, late decode, cached/warm-only page or held-only request fails the
experiment instead of claiming contention. The original page supplies a
separate warm batch afterward.

The helper is authored but has not run in a browser. Its evidence guard's nine
unit cases pass (including adversarial missing/late body and decode cases), and
syntax checks pass. This verifies evidence rejection only; it establishes no
runtime overlap or latency result. A quiet reference host with built release
artifacts remains necessary for the actual acceptance measurement.

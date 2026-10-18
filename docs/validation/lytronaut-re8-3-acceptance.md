# Live Photo acceptance reconciliation

Release acceptance: **open**; staged main landing requested (2026-10-18).
The owner reports successful verification of the updated pair in Photos.
Current-byte public decoding, largest padded writer execution and the
independently reviewed narrow external HRD model pass. Integrated application
PostgreSQL CI passes. [Final-source private exports pass](lytronaut-f5a-private.md)
for all 109 streams and 320 exports, with unchanged originals. CPU acceptance
and actual grid overlap remain open in the
[deferred release acceptance follow-ups](#deferred-release-acceptance).
Historical owner limitations retain their exact scope. The owner report and
staged landing below supersede earlier checkpoint statements that Photos
verification was pending; those historical observations remain recorded.

The acceptance worktree started at application revision
`5a9ece369185c8e81908db2421d2a6d2e5223373`. Approved validation bead
`lytronaut-cjq` landed in the epic at
`37809a2294223384717cda33a4a285c8b4818788`; its complete source tree is
`0da6c71f574992f7e1d60a2a90f0468cb1a8a77e`, identical to validated `469aa4f`.
Acceptance merge `440b9b5117b6772f7fec1619be31cfef669c3a05` incorporated that
baseline cleanly while preserving the three acceptance commits. Differences
from that baseline were the encoder comment, geometry regression, acceptance
report and release-grid assertion. That historical merge had not passed the
canonical check or runtime acceptance. The resumed source-bound checks below
supersede that status for `c1b480f`; release acceptance remains open.

## Gate disposition

| Gate | Evidence and remaining work |
| --- | --- |
| Apple Photos recognition | **Owner-reported pass for the updated pair**, 2026-10-18; exact report and current-pair byte binding below. Automated resource recognition remains separate evidence. Historical `fbe615a` and stale `35c7770` do not substitute for this report |
| VoiceOver | Not verified; owner signed off as out of scope for this version. Automated AXTree checks do not replace VoiceOver |
| Counted 64/32 MiB memory | Partial evidence, accepted limitation; unmeasured owners remain unproven |
| GPU submit-to-present ≤16.7 ms | Not verified, accepted limitation; animation-frame callbacks do not establish presentation |
| Performance under grid load | Open in [lytronaut-b0e](#deferred-release-acceptance): retain actual pending grid requests during measured focus interactions, with a separate warm batch |
| Private-corpus export rerun | **Pass at final production candidate `aa0073f`**, using its freshly built release client: 109 streams, 320 verified exports, zero failures; all 728 originals and 638 sealed blobs unchanged. [Source binding and aggregate evidence](lytronaut-f5a-private.md) |
| iOS | Owner signed off as out of scope; not verified or supported |
| H.264 Level 5 first-frame check | Actual largest padded NAL/RBSP and reviewed external Type I/II HRD model pass at `b3f0850`; this narrow result does not claim general conformance or Apple consumption of that schedule |
| Live Photo on PostgreSQL | **Pass for integrated application `877116c`**: five native HTTP Live Photo cases; full PostgreSQL parity 64 passed, zero failed/ignored. Final changed branch remote checks remain required |
| CPU focus p95 ≤100 ms | Open in [lytronaut-b0e](#deferred-release-acceptance). Historical full-suite runs missed the target; warm-only diagnostic p95 is 79.4–87.1 ms. No quiet-host or grid-load acceptance is established |

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
by `384 * (max(PicSizeInMbs, MaxMBPS/172) + MaxMBPS * removal_delay_excess) / MinCR`;
Level 5 uses `MinCR=2`. The I_PCM payload alone is 3,207,168 bytes. The
zero-additional-delay bound is only 1,603,584 bytes. An applicable HRD timing
derivation and the actual encoded byte count were required before claiming pass;
the later reviewed model and actual movie runs below supply that narrow proof.
The movie byte cap and average bitrate cannot substitute for that proof.

A bounded standalone probe included the actual `avc.rs` with only crate-level
pixel/error type shims, using mise-pinned rustc 1.99.0 and synthetic solid RGB.
It produced a 3,223,876-byte first IDR NAL (3,223,897 bytes including SPS/PPS).
An independent SPS reader confirmed coded 1152×1856, displayed 1138×1842,
Baseline profile 66, level 50, no reference frames and progressive coding.
The formula above therefore needs at least 14.308 ms excess actual-over-nominal
CPB removal delay. Timing was unresolved at this historical standalone probe;
the later [reviewed external HRD model](lytronaut-re8-3-hrd.md) supplies an
applicable schedule and exact actual movie checks, without claiming Apple
consumes that schedule. Evidence, exact
commands and synthetic source/program hashes are outside this worktree at
`/private/tmp/lytronaut-re8-3-evidence`. This does not run the Plan regression,
MOV assembly, a decoder or the canonical check.

Source: [ITU-T H.264 (08/2024), Annex A.3.1 and Table A-1](https://www.itu.int/rec/T-REC-H.264-202408-S/en).
This is a geometry and first-frame investigation, not full stream conformance.

### Corrected per-picture VUI limit

Independent full-VUI parsing and primary E.2.1 review found a definite defect:
the old SPS omitted bitstream restrictions, inferring picture denominator two.
Its 3,223,876-byte VCL picture exceeded the inferred 1,603,584-byte ceiling.
Denominator one is also too strict because PCM headers exceed raw picture bytes.
The corrected SPS explicitly signals permitted denominator zero, retains the
3,200-bit macroblock bound (actual syntax at most 3,088), and declares no
inter-boundary prediction, motion vectors or reordering, with one decoded frame.
All generated pictures are IDR/I; no predicted P/B pictures are introduced.

Two independent-parser regressions pass using pinned standalone rustc against
the actual writer, including the padded encode. Both fail under a deliberately
wrong denominator-two mutation. The new SPS is 18 bytes rather than 17; VCL
bytes remain 3,223,876 and SPS/PPS plus VCL totals 3,223,898. This fixes the VUI
picture-size claim only. At that checkpoint Annex A.3.1(c,d,i,j) still needed
an applicable HRD schedule; the later reviewed external model supplies it. Absent timing infers fixed-rate zero and low-delay one, so assuming
actual removal always equals nominal removal would also be unjustified.

Decoder/resource-loader compatibility passed for regenerated output at
`c1b480f`, as recorded below. Isolated Photos recognition still needs new
observation. The owner's earlier Photos result remains
historical evidence for the rebuilt `fbe615a` pair. No current-byte Photos GUI pass,
full Level 5 conformance or quiet-host latency is claimed. Canonical validation
subsequently passed at `c1b480f` and `85397a2`, within their stated source scope.
The format source has now changed relative to the private-run baseline, so
that run does not attest these newly emitted SPS bytes.

The [candidate external HRD experiment](lytronaut-re8-3-hrd.md) now records an
independent actual-NAL parser and rational schedule model, its normative
coverage and actual public fixture results. Fresh review of `4e93d9b` found no
remaining blocker within that narrow scope. Both largest padded orientations
were then generated and passed every Type I/II initialization at `b3f0850`.
This narrow proof does not close overall release acceptance or establish that
Apple consumes the externally supplied parameters.

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
immediate DELETE with completion races. The latter permits an already-finished
worker; it proves lifecycle cleanup and no half-pair publication, not
deterministically active cancellation. Browser active-cancellation evidence is
reported separately. The runner explicitly selects PostgreSQL and refuses
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

The earlier investigation stopped with insufficient free disk and excessive
load for reliable latency acceptance. Disk cleanup has since completed; that
historical resource condition is not a current blocker. At that checkpoint no
local browser, private-corpus, PostgreSQL or performance rerun had been performed
for this bead; the later public checks below supersede only their stated scope. The existing
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

## Resumed delivery work (2026-10-17)

Preserved merge `41c1a90` incorporates delivered main `c9090ff`, including the
restore reconciliation and PostgreSQL operations follow-ups. The only textual
merge conflict was the Node test recipe: both the sidebar request tests and the
grid evidence guard remain in the canonical test command. This integration is
not a validation result. Builds remain serialized with the other dispatched
beads; no competing build or latency run is authorized by this report.

The next executable work is staged as follows:

1. On receiving the machine lease, record source/tree, tool versions, free disk,
   process ownership and load. Use one Cargo job, no incremental compilation,
   and no dev/test debug information. Clean the cached `xtask` package before
   a pristine checkout check so its compiled manifest path names that checkout.
   Run `bh work check lytronaut-re8.3` through the mise-pinned environment.
2. Generate public native resources with `mise exec -- just live-photo-apple`
   and an external task-specific output directory. Preserve the new JPEG/MOV
   hashes and Apple verifier result. Run the public Rust/WASM Live Photo
   browser harness against same-source artifacts. These test platform decoding
   and resource-loader recognition; neither replaces isolated Photos import.
3. Build release service and WASM artifacts once, record their hashes and source
   identity, and prepare the public synthetic fixture. Run
   `scripts/browser-test.cjs` with `LYTRONAUT_RELEASE_BENCHMARK=1`,
   `LYTRONAUT_GRID_LOAD_TEST=1`, explicit release `LYTRONAUT_BINARY` and
   `LYTRONAUT_CLIENT`, and external `LYTRONAUT_TEST_ROOT`/report paths. Require
   real response/decode overlap and both unchanged ≤100 ms gates. The shell
   wrapper defaults to debug artifacts, so it is not a release recipe merely
   because the benchmark flag is set. Keep a failed result as failed.
4. Use the same quiet-host protocol for the `623f2b9` A/B baseline. A failed or
   noisy comparison does not establish a regression cause. Schedule the private
   export rerun separately against an authorized sealed copy; retain only
   aggregate evidence outside source control.

The H.264 first-access-unit question also remains independent work. Annex E.2.1
requires application-derived HRD parameters when HRD signaling is absent; the
absence does not waive Annex A.3.1 or Annex C. Any proof must state arrival,
nominal/actual removal and output schedules, CPB size, rate and clock tick, use
actual access-unit byte counts, and check each applicable inequality. A decoder
pass, average bitrate or total movie bound is insufficient by itself.

Only the isolated Photos GUI import requires new owner observation after the
SPS change. Do not open the System/iCloud library to attempt it. Existing written
VoiceOver/iOS exclusions and partial memory/GPU limitations retain exactly their
recorded scope; no CPU latency or new-byte Photos exception has been granted.

### Executed public checks at `c1b480f`

The source was clean commit `c1b480f192f136235dd87bf8ff33b30da0418e25`,
tree `f9c73276dcf0b74c802adcbf75282ab52e3231e7`. Only this revision is attested
by these runs. A sole local build lease enforced one Cargo job, no incremental
compilation, no dev/test debug information, one native test thread, task-owned
temporary storage and a 2 GiB free-space watchdog. Cached `xtask` artifacts were
cleaned once before canonical validation to remove stale manifest paths.

| Check | Result |
| --- | --- |
| Canonical `bh work check lytronaut-re8.3` / mise-pinned `just check` | **Pass**, 516.0 seconds; 246 Rust tests passed, 48 opt-in tests ignored, 21 Node guards passed; lint, native/WASM builds, boundaries and 339 package licenses passed |
| Actual padded PCM RBSP parser | **Pass** in canonical tests: all 8,352 I_PCM macroblocks parsed, actual syntax bits within declared 3,200-bit ceiling; not an HRD schedule proof |
| `just live-photo-apple` with regenerated public pair | **Pass**, 16.4 seconds; three decoded frames, correct selected still/key time, no audio, resource-loader recognition and mismatched-pair rejection; `photosLibraryOpened=false` |
| Public `scripts/live-photo-test.cjs` | **Pass**, 6.1 seconds, Chrome 155.0.8059.82; actual Rust/WASM small, 2048×1024 and padded 1138×1842 complete pairs, software retry, cancellation and decoder cleanup |

The public browser test observed all 13 created decoders closed, maximum decode
queue two, and maximum platform pixel copy 131,072 bytes. These observations
do not upgrade the accepted partial memory budget. New SPS output now has
current-source Apple resource-loader and Chrome decoding evidence, while GUI
Photos import and full Level 5 conformance remain separate open gates.

Programs, resources, logs, source metadata, `manifest.json` and `SHA256SUMS`
are retained outside the repository under
`delivery-evidence/re8-3-c1b480f` in the managed worktree root. All 23 listed
public evidence/program files were rehashed successfully. The new owner-import
candidate is its `apple-public/focus.JPG` plus `apple-public/focus.MOV`; never
substitute the historical pair for this new-byte observation.

### Opt-in focus-only benchmark evidence

`LYTRONAUT_FOCUS_BENCHMARK_ONLY=1` stops the public browser driver after native
focus decoding, the actual grid-overlap batch and the separate warm batch.
It skips unrelated sidebar/lifetime/fault suites; its report explicitly says
that the full browser suite did not run. Ordinary invocations keep their full
behavior and thresholds. This mode requires release and grid-load flags,
explicit native/client/report paths, exact application revision and matching
`<native-binary>.source-revision` / `<client-directory>/source-revision` sidecars.
The client directory must also preserve raw `application.wasm`. Private, GPU,
owner, coverage, external-server and sidebar-repetition modes are rejected.

Set `LYTRONAUT_ARTIFACT_PROFILES='native release; wasm release'` only after
recording the actual release build commands and source provenance. This label
and the sidecars do not independently establish compiler profile. The report
hashes the actual native binary, raw/generated WASM, glue and all three benchmark
harness modules. It preserves all 24 warm samples in acquisition order, checks
percentile summaries against those samples and the 24 grid intervals, and
writes measured native/grid/warm values and `passed=false`
before failing unchanged 100 ms thresholds, and refuses to overwrite an earlier
experiment. Unit guards cover failed-value retention, source mismatches,
incompatible modes, missing overlap and nonfinite measurements. The bounded
setup attempts and subsequent warm-only release A/B/B/A diagnostic are recorded
below. They do not establish real grid overlap or quiet-host acceptance.


### Release artifacts and largest public writer checks

Release service/WASM programs were compiled and frozen separately for baseline
`623f2b9892f933d1735c818bd0de3a93517699a0` and integrated application
`877116cbba57c0232a622ff59d4bc1ab326f8dbe`. Build receipts bind exact source
paths, commands, fresh first-party compilation and program hashes. An initial
current WASM attempt exposed stale baseline dependency metadata and failed;
its receipt is retained. Explicit target-specific first-party cleaning produced
a successful fresh current WASM build. This is build evidence, not a latency
measurement. No quiet-host performance pass has been established.

The public default 320×240 writer at `877116c` passed the reviewed HRD model
and Apple resource checks. Ordinary native generation at 1138×1842 returned
`Budget`, preserving the existing conservative JPEG working-set limit. The
explicit prepared-still example at `b3f0850` exercises the browser's existing
validated-still route without changing product source or limits. Both largest
padded orientations (1138×1842 and 1842×1138) pass actual NAL/RBSP parsing,
Type I/II HRD simulation at every initialization, and Apple three-frame decode,
selected-still/key-time/no-audio, pairing and mismatched-pair rejection. See the
[HRD scope and results](lytronaut-re8-3-hrd.md). This does not establish that
Apple consumes the external HRD schedule or accepts a GUI Photos import.

Evidence remains outside the repository in
`delivery-evidence/re8-3-release-baseline623`,
`delivery-evidence/re8-3-release-current-877116c` and
`delivery-evidence/re8-3-prepared-still-b3f0850`. These contain public synthetic
resources only. Final canonical validation, final exact-source remote checks,
private export verification, CPU latency and owner Photos observation remain
open at this checkpoint.


### Integrated validation and bounded diagnostic checkpoint (2026-10-18)

Canonical validation at clean `85397a210b0ff50420d25dc2ddfbbd16ec5fe163`
passed in 847.0 seconds: 255 Rust tests, zero failed, 50 opt-in ignored;
35 Node guards and seven Python HRD tests passed, along with lint, host/WASM
builds, boundaries and 339 package licenses. Host and explicit WASM first-party
debug outputs, including manifest-bound `xtask`, were cleaned first. The outer
and nested owned process groups were supervised at a 2 GiB disk floor; the
run ended with 16.39 GiB free. This verdict does not attest later tool/doc edits.

All three exact-`877116c` remote workflows passed:
[CI](https://github.com/SimplicityGuy/lytronaut/actions/runs/39018363264),
[coverage](https://github.com/SimplicityGuy/lytronaut/actions/runs/39018365953)
and [Linux containers](https://github.com/SimplicityGuy/lytronaut/actions/runs/39018368658).
The CI [PostgreSQL job](https://github.com/SimplicityGuy/lytronaut/actions/runs/39018363264/job/117355013470)
passed all five Live Photo API tests and all 64 parity tests, zero failed or
ignored. This is integrated application evidence; subsequent changes are
example/test/tool/docs only, with application-source equality recorded outside
the repo. Final exact-tip canonical and remote checks still apply.

Frozen production `877116c` WASM also passed public pair/ICC, large/padded,
software retry, cancellation and cleanup checks in 8.97 seconds: all 13 created
decoders closed, queue at most two, pixel copies at most 131072 bytes and seven
ICC rejection cases. Its regenerated 320×240 JPG/MOV are byte-identical to the
frozen `c1b480f` owner-import candidate. Neither observation is a Photos GUI pass.

The early grid setup attempts failed before valid latency evidence: aggregate
button text included pending-thumbnail icons, and then holding grid chunks
prevented selected metadata from acquiring its full shared 64 MiB reservation.
Those attempts remain in external `re8-3-focus-abba-contended`,
`re8-3-focus-abba-selector-fixed-d375d43` and
`re8-3-focus-abba-one-held-48f408c`. No failed attempt was relabeled as passing.

The independently reviewed `e0ee642` harness saves native plus 24 warm samples
**before** attempting grid traffic, in a distinct immutable `.cpu.json` receipt.
Every partial receipt has `complete=false`, `passed=false` and
`gridAcceptanceVerified=false`, with raw samples and exact program/harness
hashes. A separate unavailable-grid receipt binds its CPU receipt hash. The
complete report still requires 24 actual response-body and same-image decode
overlaps, and all unchanged 100 ms targets. No warm sample substitutes for grid
work. Default full-suite behavior remains unchanged.

A single serial A/B/B/A diagnostic used the same 74-file immutable public
fixture and reviewed harness for baseline `623f2b9` (A) and frozen current
application `877116c` (B):

| Run | Native HTTP/decode p95 (ms) | Warm focus p95 (ms) | Warm maximum (ms) |
| --- | ---: | ---: | ---: |
| A1 | 3.3 | 86.4 | 86.9 |
| B1 | 3.3 | 86.9 | 87.5 |
| B2 | 2.9 | 79.4 | 81.1 |
| A2 | 2.9 | 84.6 | 92.5 |

All four CPU-only target checks passed. CPU idle samples ranged from 54.9% to
80.68%; no quiet-host condition or cause of the historical miss is established.
A1/B1/B2 had no naturally pending grid images after selected focus readiness.
A2 failed the strict 24-preview discovery before the grid loop. These are
CPU-only diagnostics, not a complete acceptance pass or a grid latency result.
Evidence and all raw samples are in `re8-3-focus-abba-warm-first-e0ee642`.

One separately reviewed current-only probe at `2da8e64` removed the additional
selection click, relying on the application's natural first-capture selection.
It retained native/warm p95 4.5/82.8 ms (warm maximum 85.2 ms), but rejected the
fixture's actual first capture, Synthetic 016, rather than assuming the focus
capture Synthetic 000 was the default. No grid loop ran. Its failure and partial
CPU receipt remain in `re8-3-focus-current-natural-2da8e64`; no further grid
experiment was repeated on this setup. New workloads must establish their own
selection/order and real overlap without modifying product admission merely to
make the test pass. No product lease or scheduling behavior changed here.

At this pre-owner-report checkpoint, the remaining release gates were isolated
Photos import for the current bytes,
private exports from an authorized sealed catalog at final application source,
and quiet-host CPU/grid-load acceptance. Earlier owner memory/GPU limitations
and VoiceOver/iOS exclusions are unchanged. Human CI-conditional integration
approval is not a waiver of these acceptance requirements.


### Terminal supported name-sort probe

The assumption that the earlier Synthetic 000 click was redundant was refuted:
this fixture's natural library-order first item is Synthetic 016. Removing the
click did not test an equivalent focus-selection workload. A separate supported
name-sort workload was therefore reviewed at `4500f23`: actual `#sort` change
on the original page and every fresh probe, discovery with explicit `sort=name`,
an exact post-change successful response and all 24 rendered labels matching
that ordered API page. This establishes Synthetic 000 as the natural first
selection in this explicitly different workload. No capture was clicked again,
no image was artificially reloaded, and product admission was unchanged.

Its single current-release probe retained native/warm p95 3.0/87.1 ms (warm
maximum 89.2 ms). Those 24 warm samples were collected on the prior library-order
page, before the name-sort grid phase; they are not name-sorted grid timings.
The real name-sorted grid phase again reported **unavailable**: every grid image
had finished when the selected discrete focus became ready. There were zero
valid grid intervals and no complete acceptance report. Partial receipts remain
`complete=false` / `passed=false` in external
`re8-3-focus-current-name-4500f23`. All failed and unavailable setups are
preserved. No further grid experiment or baseline rerun followed this result.

The helper now supports this explicit ordered workload and fails closed when
natural post-readiness grid traffic is unavailable. These diagnostics do not
establish responsiveness while actual grid bodies/decode overlap a focus input,
quiet-host latency, or a cause of the older full-suite CPU miss. No production
admission or scheduling behavior was changed to enable the experiment.


### Owner-reported Photos verification and staged landing (2026-10-18)

The owner reported: “I already verified the updated pair in Photos. Looked
great.” This is recorded as an **owner-reported pass for the current pair**.
No additional itemized observations were recorded. The automated decoder and
resource-loader results above remain separately scoped evidence.

The supplied current-pair candidate was frozen at `c1b480f` in external
`re8-3-c1b480f/apple-public`. Regeneration from integrated application `877116c`
produced byte-identical resources, recorded in external
`re8-3-release-current-877116c/public-320/source-equivalence.json`:

| Resource | SHA-256 at both revisions |
| --- | --- |
| `focus.JPG` | `9d8f443b2cd5bd8d7a3dff98cb0b6954a8b09f86be606cfe0f4e710991f65e32` |
| `focus.MOV` | `bcf9d9bd0a3f9184c2441cafb198d684f12e04e34b1cf5a3cc2491b9c5f85155` |

The owner then requested: “please wrap up the work and land it on main”. Bead
`lytronaut-re8.3`, now **Land validated Live Photo fixes and acceptance
evidence**, covers that staged source landing with canonical validation,
exact-source CI/coverage/container checks and a reviewed preserved merge.
Application source is unchanged from `877116c` and validated `7724aea`; the
frozen application artifacts retain their existing provenance. This landing
does not declare release acceptance or change the historical owner limitations.

### Deferred release acceptance

Open epic **lytronaut-m7v — Complete deferred Live Photo release acceptance**
preserves the release requirements in these children. The owner subsequently
approved its kickoff and requested completion of both follow-ups:

- **lytronaut-f5a — Verify final-source Live Photo exports on the sealed private
  corpus.** The [actual final-source rerun passed](lytronaut-f5a-private.md):
  final production candidate `aa0073f`, 109 streams, 320 independently verified
  exports, zero failures, with full
  original and sealed-copy integrity. This supplies current-byte evidence;
  the earlier private run remains separately scoped to its historical source.
  Documentation integration still requires canonical validation and remote CI.
- **lytronaut-b0e — Complete quiet-host CPU and actual grid-overlap focus
  acceptance.** Establish reference-host conditions and actual grid response-body
  plus same-image decode overlap during measured focus interactions. Preserve
  the unchanged 100 ms p95 target, warm comparison, failed attempts and raw
  measurements. Existing warm-only diagnostics do not complete this criterion.

The remaining CPU/grid criterion is required and has not been waived or marked
passed by the private-corpus result. The
written memory/GPU limitations and VoiceOver/iOS exclusions keep exactly their
original scope.

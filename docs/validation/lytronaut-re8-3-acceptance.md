# Live Photo acceptance reconciliation

Status: **open**. This source review does not clear the release gates.

The acceptance worktree starts at application revision
`5a9ece369185c8e81908db2421d2a6d2e5223373`. Validation bead `lytronaut-cjq`
has additional, unmerged validation changes; its results do not attest this
worktree, and this report does not attest their eventual integration.

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
| Live Photo on PostgreSQL | Open at the delivery revision; restore parity alone does not exercise job creation, rendering, download and pair verification |
| CPU focus p95 ≤100 ms | Not met in historical merged-tree runs; quiet-host A/B and final-source measurement or explicit owner acceptance remain required |

The release benchmark now rejects a first batch with zero pending grid images
instead of accepting it as load-contended evidence. This prevents a false pass;
it does not manufacture concurrent requests or prove load-contended latency.
The future runtime recipe must establish real grid traffic before measuring.

The written owner dispositions above are recorded in the validation branch's
`docs/design/004-live-photo-export.md`, “Owner results and accepted limitations
(2026-10-13)”. Preserve those records when integrating that branch. They are
neither new approvals nor newly measured passes.

## H.264 bounds

The encoder admits even dimensions ≤2048 with ≤2,097,152 visible pixels.
The regression `admitted_geometry_fits_level_five_frame_size_and_processing_rate`
enumerates all even shapes and rejected pixel-limit shapes. It checks padded
macroblocks and the conservative 64-frame / three-second processing rate.
It allocates no images and does not run a decoder. It has not yet been run.

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
These aggregate results do not identify captures. The report explicitly keeps
Live Photo end-to-end on PostgreSQL open. Historical `b81e689` parity exercised
four Live Photo API cases, but does not attest the final delivery revision.

Source comparison on 2026-10-16 found no changes from private-run revision
`30800deb642d94c3bdcc7dc6956043829eb7202d` to validation revision
`469aa4fa729065eca3abdf37e81637e7254edad4` in the client, service, storage,
format and contracts crate trees, root Cargo manifests/lock, mise/toolchain
configuration, `.cargo`, or assets/web/static paths. The only changed crate file
in that interval is the coverage tooling in `crates/xtask/src/coverage.rs`.
This establishes application-source equality for that interval, not a rebuild
or new runtime measurement. Integration must preserve those trees before
carrying that private evidence forward; this older acceptance branch does not
yet contain the validation branch's application fixes.

Before repeating a multi-gigabyte private copy, compare the application source
and program provenance used by that run with the integrated delivery source.
Documentation/test-only differences can be explained explicitly; application
differences require a scoped rerun or an honest unverified status. Do not copy
private source identifiers, paths, hashes, pixels or calibration into this repo.

## Runtime work still required

The current host has insufficient free disk for a fresh build and excessive
load for reliable latency acceptance. No browser, private-corpus, PostgreSQL or
performance run has been performed for this bead. The bounded AVC probe above
is the only executable correctness experiment. Do not reinterpret the later
`b81e689` CPU p95 of 137.9–150.4 ms as the older post-export 104.7–150.7 ms stage,
or attribute either stage to the current delivery revision.

After integration and resource availability, run the canonical mise-pinned
check, the focused format regression, disposable PostgreSQL Live Photo flow,
and quiet reference-host performance A/B against `623f2b9`. Preserve exact
source/program identities and aggregate evidence outside the repository.

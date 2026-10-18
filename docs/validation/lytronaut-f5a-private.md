# Final-source private Live Photo verification

Bead **lytronaut-f5a — Verify final-source Live Photo exports on the sealed
private corpus**: runtime acceptance **passed**, 2026-10-18. This closes the
private-corpus rerun criterion, separately from the CPU/grid criterion in
**lytronaut-b0e**. It does not declare complete release acceptance.

## Source and artifact binding

The checked application source is main
`17524c31b45252385082e33501f789fc0997c394`. The frozen production release programs
were built at `877116cbba57c0232a622ff59d4bc1ab326f8dbe`. Comparing all crates,
assets, Cargo manifests and Cargo.lock found only the subsequently added Live
Photo example and test changes; production application source is byte-identical.
The existing build receipts and all eight frozen program-file hashes were
verified before opening the private corpus. These artifacts are not represented
as newly built at the later revision.

| Production browser resource | SHA-256 |
| --- | --- |
| Rust/WASM | `3b707f966956cf6495c406a584fa61ff558fb7ee0d7ce4930b12f4a98d10271d` |
| Generated JavaScript glue | `186bbd058a89521c1fde34e3983abe12b13069a0655aae959d272bace4657afd` |

The existing `scripts/live-photo-private-test.cjs` ran unchanged through the
mise-pinned Node runtime and lockfile-pinned browser dependencies, using the
production artifact rather than the instrumented browser harness. Browser:
Chrome 155.0.8059.82. Headless local execution allowed only loopback GET requests;
there were zero external requests and zero page errors.

## Sealed corpus and original integrity

The earlier report states that its derived copies were deleted. Bounded local
metadata research located the authorized original library, matching its
historical 728 files, 2,001,830,074 bytes, 109 captures, three albums and 109 album
memberships. The frozen native production importer read that library into a new
private directory outside Git: catalog ready, zero item errors. Its genuine
`catalog.db` was independently copied to the verifier's expected `app.db` name.
The first preparation attempt stopped at this filename difference before any
browser verification; that failure is retained rather than counted as a test.

All 728 catalog rows were matched to the original SHA-256/length inventory.
The 638 unique copied blobs were checked for independent inodes, correct hashes
and lengths, then only those destination files were sealed mode 0400. Original
files were never chmodded, linked, written or removed. Before and after the
browser run, all 638 sealed blobs passed hash/size/mtime checks. A separate
post-run rehash confirmed all 728 originals unchanged in content, size, mtime,
mode, inode and device; the complete source tree inventory was also unchanged.

Private locators, file inventories, fixture hashes, sealed resources and detailed
integrity receipts remain in a mode-0700 directory outside Git, with private
receipt files mode 0600. No private paths, identifiers, images, calibration data
or fixture hashes are included here. The browser saved no derived pair resources.
The sealed test copy is retained pending evidence review.

## Actual export results

| Check | Observed result |
| --- | --- |
| Stream denominator / processed / failed | 109 / 109 / 0 |
| Source planes | 654 |
| Export attempts / independently verified / failed | 320 / 320 / 0 |
| Pre-aborted exports / active-decode cancellations / recovered streams | 109 / 109 / 109 |
| Decoder instances created / closed | 1,927 / 1,927 |
| Decoded frames / closed frames | 6,180 / 6,180 |
| Maximum decoder queue / pixel readback | 2 / 69,120 bytes |
| Maximum source / pair ZIP | 2,674,160 / 11,140,255 bytes |
| Visible source geometry | 109 streams at 1080 × 1080 |
| Independent output-frame histogram | 2: 14; 3: 27; 4: 30; 5: 93; 6: 156 |
| Exported JPEG vs selected-frame JPEG-reference comparison | Maximum channel difference 0 over 1,119,744,000 samples |
| Selected-key MOV frame vs JPEG-reference comparison | Maximum channel difference 23; absolute sum 1,145,628,587 and squared sum 2,479,908,451 over 1,119,744,000 samples |
| Completion / exit status | Complete corpus / 0 |

The verifier selects distinct first, middle and last source planes, yielding
320 attempts rather than 327 because seven streams have only two distinct
selections. Every pair passed independent metadata/container, JPEG/MOV decode,
selected-key geometry, ascending-lambda source order and sweep-endpoint checks.
Every stream recovered after the real active-decode cancellation. No streams,
exports or failures were skipped.

The reference is a browser JPEG made from the platform-decoded selected AVC
frame, then independently decoded for comparison. Zero still difference is
agreement between those JPEG decodes, not raw source-RGB, archival or color
parity. Motion pixel comparison covers only the selected-key MOV frame; the
other MOV frames undergo decode, count, geometry and order checks rather than
all-frame pixel-parity checks.

The verifier completed in 192.45 seconds; the supervisor including the separate
original rehash completed in 194.89 seconds. These are corpus execution timings,
not focus-latency or quiet-host acceptance. Minimum free disk was 9.84 GiB, above
the 2 GiB watchdog floor; the watchdog did not fire. Browser, loopback server and
task-owned workers drained before releasing the local execution lease.

The runner's generic `photosGuiAcceptancePending=true` and memory-pending flags
remain in its raw report: this run did not open Photos or measure opaque browser
memory. The separately recorded owner report for the byte-bound public pair is
unchanged. No new Photos observations, memory-budget proof, GPU presentation
proof or CPU/grid performance pass is inferred from this result. Canonical
validation and exact-source remote checks still gate integration of this
documentation checkpoint.

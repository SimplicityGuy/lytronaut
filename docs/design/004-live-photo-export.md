# Portable Live Photo export

Status: implementation in progress; Photos-library import acceptance is pending.

A Live Photo is a still image plus a paired movie. Neither an ordinary JPEG nor
an ordinary MOV alone is advertised as a Live Photo. Lytronaut packages the complete
`focus.JPG`, `focus.MOV`, and `provenance.json` in one ZIP download. Extract all
three files; import the JPEG and MOV together into Photos. Import behavior is a
platform compatibility gate; a ZIP itself is not a Photos-native photo asset.

Motion is a generated focus animation from verified stored planes. It is not a
camera-recorded temporal movie, a calibrated reconstruction, or a replay of
imported aperture, perspective, tone, denoise, or animation settings. Originals
and imported recipes are never changed.

The portable native/Rust-WASM core independently writes H.264 Baseline Level 5
I_PCM pictures and QuickTime sample tables. Every picture is an independent IDR;
there is no temporal prediction or external multimedia executable. The larger
uncompressed movie is intentionally bounded rather than silently installing a
C++/FFmpeg/GPL dependency or requiring an Apple production server. Native exports
decode verified stored JPEG planes. Browser exports use actual WebCodecs
configure/decode/flush results for archived AVC, then pass verified decoded RGB
into the same Rust encoder. No browser `isConfigSupported` result alone establishes
availability.

The source stack has 2–64 finite unique lambdas. The deterministic ascending-lambda
sweep preserves the selected key and both endpoints, subsampling intermediate
planes when the worst-case encoded size would exceed the movie budget. Duration
is exactly three seconds, with uniform sample durations and no audio track.
The cap is 16 MiB MOV and 18 MiB complete ZIP. Geometry is preserved, with even
width/height at most 2048 and at most two million visible pixels; unsupported
geometry receives an actionable still-export fallback. Existing JPEG stacks
retain their stricter 512-pixel limit. Explicit EXIF orientation is baked into
both resources; the new JPEG declares orientation 1. Archived AVC initially uses
identity orientation, matching its existing verified viewer capability.

Source color profiles remain unknown. Numeric decoded RGB is encoded with sRGB
transfer, BT.709 primaries, BT.601 limited-range YCbCr and 4:2:0 chroma averaging.
This documents an export interpretation, not recovered camera calibration.
The key JPEG adds compression loss; movie chroma averaging and quantization also
add loss. Provenance carries these facts separately. Neither resource copies
serial numbers, source paths, source UUIDs, legacy metadata, or private hashes.
A fresh export-only UUID pairs the JPEG Apple MakerNote key 17 and MOV content
identifier. The `mebx` timed metadata track uses `still-image-time` SInt8 -1 at
the selected image's actual presentation time.

Native exports reuse durable job admission, cancellation, cleanup, download
streaming, and completion publication. A failed, cancelled, or interrupted job
cannot offer a half-pair. Deployment-level exact Host/Origin and mutation intent
protection applies; this feature introduces no accounts or login setup.

Archived AVC export runs locally in the browser, with cancellation on navigation
or unmount, bounded progress, two queued decoder chunks and one decoded plane at
a time. It offers only the complete ZIP and revokes its temporary download URL.
It does not claim a durable server job or resume after the tab closes.

The independently authored synthetic native example is
`mise exec -- just live-photo-native /tmp/task-specific-output`.
The macOS test-only `scripts/verify-live-photo.swift` checks decoded frame colors,
selected still, duration, geometry, no audio, pairing and timed metadata, plus
rejection of an intentionally mismatched pairing identifier. It uses resource
loaders without opening a Photos library. `scripts/live-photo-test.cjs` exercises
the actual Rust-WASM path with the existing public synthetic AVC fixture and a
test-only independently generated 2048×1024 all-key color fixture, including
cancellation, unsupported codecs, actual software retry and complete pair
download. It records WASM memory high water, declared references/coded geometry,
maximum strip-copy bytes and decoder/frame closure. These programs still require execution against the final build;
their presence alone is not compatibility evidence.

## Repeatable validation recipes

Run these through the mise-pinned environment after normal setup:

```sh
mise exec -- just live-photo-test
mise exec -- just live-photo-native /tmp/task-specific-output
mise exec -- just live-photo-browser-test
mise exec -- just live-photo-owner-test
mise exec -- just live-photo-apple /tmp/task-specific-apple-output
```

`live-photo-test` covers portable format, wire contracts and native service API.
`live-photo-browser-test` builds the WASM application and installs locked test
tooling before exercising the public synthetic codec and resource tests. The
existing `browser-test` recipe, reused by Linux browser CI, also runs that public
Live Photo harness after the service/UI proof. `live-photo-apple` first generates
the native public fixture, then compiles/runs the Apple-framework verifier with
a task-specific module cache. It requires macOS and does not run in Linux CI;
it does not open a Photos library or replace the isolated GUI import gate.

`live-photo-owner-test` builds the explicit `test-harness` feature into separate
`dist-owner` glue. Normal builds keep their existing root mount and expose no
dispose/remount hooks. This harness drops Leptos's actual mounted view/owner during
pending source fetch, decoder flush, queued codec callbacks and the final download
timer, then remounts the same app and checks recovery. The feature is built once;
three isolated synthetic libraries exercise 32×32, 2048×1024 and 1138×1842 UI
geometry with the same raw/generated WASM identities. Large/padded AVC fixtures
reuse the independent public authoring helper outside observation collection.
The 30-capture catalog, 24-item pages and JPEG baseline are preserved in each run;
the actual name-sort UI selects the target deterministically. CI preserves its synthetic
aggregate, source archive, raw WASM and generated glue in a bounded 128 MiB evidence
archive. Feature identity and hashes are distinct from production artifacts. This
source addition still requires actual browser validation; pagehide is not owner
disposal. These tests do not prove the counted application allocation budget or
Photos GUI recognition. Optional whole-browser observations are separate; opaque
codec/driver memory and RSS are not a whole-process acceptance limit.

Private fixture verification uses separate explicit commands:

```sh
mise exec -- just --quiet video-private-test "$AUTHORIZED_SEALED_COPY"
LYTRONAUT_CLIENT="$FROZEN_PRODUCTION_DIST" LYTRONAUT_TEST_REVISION="$FROZEN_SOURCE_HEAD" \
  LYTRONAUT_TEST_WASM_SHA256="$FROZEN_WASM_SHA256" LYTRONAUT_TEST_GLUE_SHA256="$FROZEN_GLUE_SHA256" \
  mise exec -- just --quiet live-photo-private-test "$AUTHORIZED_SEALED_COPY"
```

The video recipe invokes the existing aggregate-only AVC decoder/SPS corpus proof.
The Live Photo recipe uses already frozen production WASM/glue and previously
installed locked browser tooling; it builds and installs nothing. Before running,
verify the frozen manifest binds the exact source revision, production feature
set and supplied program hashes. The harness checks those hashes before opening
the sealed catalog and rejects owner-harness artifacts. A supplied revision alone
does not prove program/source correspondence.

It attempts all 109 archived streams, deduplicates first/middle/last selections,
checks strict cancellation and successful retries, and reports failures by bounded
phase categories without identifiers or error strings. Source responses retain
the existing 8 MiB admission, checked before serving and before bounded streaming
into one exact-size buffer. An independent test-only reader checks stored ZIP
checksums, JPEG pairing metadata, QuickTime sample tables, timed key metadata,
silent duration, orientation and provenance. WebCodecs independently decodes all
generated movie frames and checks the selected key against independently decoded
source pixels. Aggregate RGB loss statistics describe JPEG/chroma conversion;
they impose no invented lossless threshold. Elapsed corpus/export totals and
min/max attempt times are aggregate measurements. SPS geometry/reference/B-frame
aggregates support the geometry model; they do not establish the required counted
application allocation budget. Optional whole-browser peak observations remain
separate from that budget.

The independent reader accepts a single bounded ICC_PROFILE APP2 only after RGB
display-profile header/tag-extent checks and complete byte equality with an ICC
profile independently encoded from a fresh 16 × 16 sRGB canvas on that platform.
It keeps source pixels out of the reference canvas and clears that canvas on every
path. ICC-free JPEGs need no DOM reference. Arbitrary APP2, split/reordered profiles,
bad tag extents, truncated profiles and foreign payloads are rejected through a
JPEG-only test seam so outer ZIP checksums cannot mask those negative tests. This
verifies platform-generated color metadata; the source capture profile remains
unknown and no calibrated synthesis is claimed.

The harness checks every sealed source blob's digest, size, mode and mtime before
testing and again in `finally`, including on failure. It serves GET requests only
on loopback in an isolated 0700 browser profile, blocks external network requests
and removes its temporary profile. Private bytes remain in browser/process memory;
no private JPEG, MOV, ZIP or catalog rows are saved or logged. This source addition
requires a coordinated runtime run before claiming corpus acceptance. Neither
private recipe runs in default checks or CI. Public
video/browser recipes remove the inherited private-root environment variable so
public validation cannot accidentally select private corpus mode. Private paths,
captures and identifiers must never enter public reports or CI artifacts.

## Resource verification still pending

Platform-generated JPEGs use bounded metadata-only inspection for their encoded
size, segment bounds and geometry, followed by independent browser image decoding
to verify the selected pixels' dimensions. Metadata inspection allocates no Rust
codec scratch. Actual Rust JPEG decoding retains its conservative working-memory
admission; accepting a platform-generated still does not raise that decode budget.

Export admits one queued compressed chunk; ordinary viewing retains its existing
two-chunk upper limit. Pixels are copied in at most 16-row strips, bounding each
platform ImageData and Rust temporary to 128 KiB. The canvas backing and current
VideoFrame are released before the Rust PCM encoder allocates its scratch and
picture buffers. Encoded pictures are moved into bounded owned segments without
cloning or growing a contiguous movie during decoding. The decoder is dropped
before the independent selected-JPEG decode. After that proof, final movie
assembly reserves ZIP slack once and packaging reuses the allocation; source
bytes are dropped before the final Uint8Array/Blob download copies. Browser
garbage collection and codec internals are not controlled by Rust.
ZIP slack includes the 16 KiB provenance cap plus exactly 316 bytes of local and
central headers, duplicated fixed entry names and the end record. Both assembly
and packaging use that shared bound; a final 18 MiB ZIP is allowed and one byte
over is rejected before mutation.

The 64 MiB admission lease covers counted transient application work. Retained
stage/grid resources have a separate 32 MiB budget. These are not a whole-Chrome
RSS limit: engine baseline, opaque codec/driver pools and garbage collection are
separate. A lease alone does not prove that every allocation fits it. The previous
36.087/44.798 MiB totals omitted owned capacities and JS string/metadata overlaps
and must not be used as completion evidence.

| Phase | Counted owners and overlap | Unmeasured or partial attribution |
| --- | --- | --- |
| UI fetch | Growing Rust source Vec, temporary Rust chunk and original JS response backing (plus a copied backing for constructor fallback); old/new Vec storage may overlap during growth | Stream/network queues; manifest allocations retained during export; allocator growth overlap |
| Parse | Source **capacity**, owned processed JSON and plane/section tables | serde map nodes/allocator overhead; UI manifest allocation |
| Feed | Source, owned lambdas/range tables, JS Uint8Array input and EncodedVideoChunk backing; processed JSON and plane/section tables already dropped | UI manifest allocation; released JS objects awaiting GC; opaque decoder pool |
| Selected JPEG | Rust data URL String, temporary Rust atob String, raw JPEG Vec **capacity**, then paired JPEG allocation | JS data URL/atob strings (conservative UTF-16 estimates differ from physical storage); canvas/frame backing |
| Strip copy | Source/UI manifest, retained encoded segments and paired JPEG, full Rust RGBA capacity, one Rust strip and one ImageData strip | Canvas/frame backing and GC retention after close/reset |
| PCM encode | Source/UI manifest, RGBA, existing segments, paired JPEG, raw PCM capacity, new escaped segment capacity | UI manifest and small writer tables not fully observed |
| Assembly/package | Source/UI manifest still retained, encoded segments, final preallocated movie/ZIP, selected JPEG and table/provenance capacities | Temporary atom/table construction, allocator overhead |
| Download | Source dropped; Rust ZIP + JS Uint8Array copy, then Rust ZIP dropped before Uint8Array + Blob | JS backing sharing/copy policy, GC retention; retained UI baseline remains separate |

The actual UI borrows its fetched Rust Vec into export; processed plane payloads
and Annex-B ranges borrow source bytes. It does not clone the full source again.
The public wasm-bindgen fixture entry copies a JS input into a Rust Vec, so its
external JS source must be attributed separately from the UI path. Independent
pair verification also allocates test-only decoded images and must run after
pipeline observations; its WASM page highwater is not the live owned-buffer sum.

The opt-in `test-harness` feature propagates from client to format. Explicit
`harness_capacity_start()` enables a fixed 1024-event buffer;
`harness_capacity_take()` disables recording before serializing numeric records.
No observer or these exports exists in default production builds. Field meanings:
`fetch_before_extend` = destination capacity / Rust chunk capacity / original JS
backing bytes / copied JS backing bytes; `fetch_after_extend` = resulting capacity
/ zero / original backing bytes / copied backing bytes. Same-realm native byte
views are reused; cross-realm/custom values retain the existing constructor
conversion. Copied backing is zero when the exact input object is reused; original
backing zero means unavailable attribution (custom arrays have no buffer), not
zero ownership. View length is not assumed to equal backing length. Each snapshot is immediately preceded, without an intervening await, by
`fetch_category` = category / zero / zero / zero: 0 other, 1 download, 2 manifest,
3 capture_detail, 4 asset_chunk. Chunk requests can include stage previews and
concurrent grid work; none is assumed export-only. No URL or identifier is
recorded. Feature-only URL classification temporarily allocates URL/path strings;
that observer overhead is separate and unmeasured. These paired snapshots still do not measure old/new allocation overlap inside Vec growth;
`wasm_boundary_source` = Rust capacity / length (external JS input separate);
`jpeg_platform_result` = Rust data URL capacity / byte length / canvas width /
height, recorded before the existing 2MiB output check (including rejection);
`ui_source_begin` = fetched capacity / length; `ui_source_end` = fetched capacity; `selected_jpeg` = data URL / atob Rust String /
raw Vec capacities; `pixel_strip` = RGBA / Rust strip capacities / ImageData byte
extent; `pcm_input` = RGBA / raw PCM capacities; `pcm_escape` = raw / escaped
capacities at initial reserve; `segments` = summed segment / sample table / SPS /
PPS capacities; `paired_jpeg` = borrowed input length / output / EXIF capacities;
`assembly` = summed segments / final buffer / moov / offset-size table capacities;
`package` = final buffer / JPEG capacities / borrowed provenance length;
`download_array` = Rust ZIP capacity / intended JS copy length, recorded before
allocation (not an observed JS backing); `download_array_live` = Rust ZIP capacity
/ actual Uint8Array view length / backing-buffer length / zero, after creating
the array and before dropping ZIP. The temporary Rust array handle drops at the
prior insertion point; the parts array retains the same JS object. Zero fields
carry no claim.
These snapshots do not capture every reallocation, JS physical backing or JSON
heap. `complete` means only that the event buffer did not overflow; `dropped > 0`
invalidates even that partial coverage. Fixed observer buffer bytes are reported; returned report Vec and serialization
String allocations remain separate and unmeasured; no event contains an identifier, path or pixel contents.
The synthetic owner harness invokes start/take around its existing final
successful mounted UI export, after the viewer has settled. It validates the
bounded fixed schema, requires every UI stage and no overflow, excludes the
external JS-input seam, and writes the partial numeric trace into the existing
owner result artifact with the same feature program/source identity. Around that
same final export only, native canvas and Blob wrappers collect at most 128
numeric records and are restored in finally. `canvas_result` = canvas width /
height / actual JS result character length / conservative UTF16 logical bytes
(twice length), before the result enters Rust. `zip_blob` = part count / typed
array view bytes / backing bytes / actual Blob size; exactly one ZIP part is
required. No result strings, canvases, typed arrays or blobs are retained by
observations. Concurrent preview canvas calls are included within the existing
2048-dimension/2Mi-pixel admission bounds, including 128×128 grid previews; each
selected Rust JPEG result must match a canvas result's geometry and ASCII byte
length, counting repeated outputs from codec fallback separately. Installation and operation failures restore wrappers.
Zero dropped records is required, and the serialized JS report is capped at
32KiB. Numeric array/object storage and serialization overhead remain separate
and physically unmeasured. JS character counts and Blob sizes do not establish
physical string storage, Blob copies/sharing or opaque canvas/codec heap sizes.
These observations do not prove the full counted transient/retained budget;
metadata ownership and retained stage/grid/DTO capacities remain separate gaps.
The baseline
`runtime.json` and separate `runtime-large.json` / `runtime-padded.json` records
are bounded and checked for matching feature/source/program identity. It performs
no independent pair verification during collection. Each trace labels its declared synthetic geometry, observed source length and
existing 8 MiB source admission cap. It is not a maximum-geometry or metadata-heap
proof. Historical `2cedaa4` CI run `38226127927` passed all three checks and collected
36 events across 13 stages from a 32×32 UI fixture with a 992-byte source and no
overflow. That partial trace predates the early metadata-drop change.
The metadata-drop source `15150c1` then passed CI `38227514154`, including its
small mounted-UI trace. The three-case `f995ced` source then passed all five CI/container checks
(CI `38229335947`, containers `38229335945`); the independently verified checkout tree was
`e23dacbc68ebbe4640fc9fa33d6724bb3c462d1b`. Preserved remote evidence reports
120/242/382 events for small/large/padded, sources 992/32,208/33,408 bytes,
13 stages and zero drops in each case, with eight decoders closed, 40 frames closed
and three downloads per case. Concurrent thumbnail/detail fetches are included;
these events are not exclusively export fetches. The new reader fast path still
awaits runtime verification. Private traces, parse/string/copy attribution and
the full counted application budget proof remain pending.

Current export metadata admission inherits format defaults (2 MiB JSON,
100,000 nodes, 1,024 sections), whereas viewing specifies 256 KiB JSON,
16,384 nodes and 256 sections. The parser validates all references and descriptors under those same limits,
then returns only the source-borrowed payload, copied geometry and owned lambdas.
Its `Processed.original` Value and plane/section tables are dropped at that lexical
boundary before Plan/codec/PCM/assembly allocations. Source bytes and the UI
manifest still survive until export completes. Parse-phase JSON heap remains
unmeasured in the counted ledger; these limits are not tightened and no additional
captures are rejected. A production
policy change requires separate review.

Enumerating admitted even dimensions up to 2048 and 2,097,152 visible pixels gives
at most 8352 macroblocks. The tested padded 1138×1842 → 1152×1856 shape bounds raw
PCM scratch to 3,223,936 bytes and initial escaped scratch to 4,835,905 bytes;
2048×1024 is not the largest padded shape. Actual capacities, JS owners and
metadata must be combined by simultaneous phase, not by summing nonoverlapping
maxima. No whole-pipeline 64 MiB empirical proof is claimed. Opaque decoder DPB,
B-picture reorder and reference pools remain separate from counted owned buffers.

The prior private aggregate decoder proof contains 109 streams and 654 frames,
but no SPS reference/geometry histogram. The updated decoder harness records
only aggregate coded geometry/reference/B-picture counts. Classification of
those verified streams requires the coordinated read-only sealed-copy run;
no additional stream rejection has been introduced on estimates alone.
Standalone Preview feedback confirms visual appearance only. Apple Photos
native pair import and visible motion remain a separate pending gate.

## Primary evidence

- [Apple capture and save resource model](https://developer.apple.com/documentation/avfoundation/capturing-and-saving-live-photos)
- [Apple Live Photo movie pairing](https://developer.apple.com/documentation/avfoundation/avcapturephotosettings/livephotomoviemetadata)
- [PhotoKit paired movie resource](https://developer.apple.com/documentation/photos/phassetresourcetype/pairedvideo)
- [Apple movie content identifier](https://developer.apple.com/documentation/avfoundation/avmetadataidentifier/quicktimemetadatacontentidentifier)
- [Apple timed metadata description](https://developer.apple.com/documentation/quicktime-file-format/timed_metadata_sample_descriptions)
- [Apple timed sample format](https://developer.apple.com/documentation/quicktime-file-format/timed_metadata_sample_data_format)
- [Apple metadata key definitions](https://developer.apple.com/documentation/quicktime-file-format/metadata_key_declaration_atom)
- [Apple metadata datatype definitions](https://developer.apple.com/documentation/quicktime-file-format/metadata_datatype_definition_atom)
- [ITU-T H.264 (08/2024)](https://www.itu.int/rec/T-REC-H.264-202408-S/en), clauses 7.3.2.1, 7.3.2.2, 7.3.3, 7.3.5 and Annex E.

The independent format spike used invented solid red/green/blue pixels. Chrome
154 software-preference decoding and Apple's AVAssetReader decoded all three
Rust I_PCM frames with matching color (within one byte of rounding). Apple's
metadata reader found one content identifier, one timed metadata track, and the
selected key time. `PHLivePhoto.request` recognized the fully Rust-metadata pair
without a library import or any owner Photos/default-state change. The actual
production mux/encoder core was separately exercised with a test-only synthetic
JPEG provider: ascending reordered blue/red/green playback and key time 1.0 s
were read correctly; the ZIP passed an independent `unzip -t` check. These are
format-spike results, not a substitute for final native/WASM/API/UI validation or
an isolated Photos-library import with visible working motion. No iOS device
verification is claimed.

The feature-only shared-reader tests run before owner observation collection on
public same-realm offset views, isolated same-origin cross-realm offset views and
custom arrays. They preserve absent/incorrect Content-Length behavior and assert
oversize header/chunk rejection, stream interruption and actual aborted-signal
mapping. The fast path and these new tests still await runtime evidence; reader
cleanup behavior and source/metadata admission are unchanged.

Pinned Rust 1.99.0 source audit is recorded in the external receipt
`lytronaut-9fw-rust199-capacity-source.json` with compiler identity and exact source
digests. This reader's Vec extension reserves, copies and deallocates the chunk;
it does not steal its buffer. Growth stores `max(2*old_capacity, required, 8)`,
yielding capacity below twice the admitted limit when the limit is at least eight
bytes. Count old/new storage conservatively during growth. This implementation
reasoning is not a public Vec API guarantee; actual WASM capacities and allocator
overhead still require evidence, and JS backing size is not view length.

The native byte-view reader change `ec646f7` passed all three CI checks in
`38231526272`, and both container checks in `38231526275`.
The independently matched tree was `d04a1ffc4f4c1276e23778c4f10a82be662d673a`.
All nine public reader checks passed; small/large/padded traces had 98/212/396
events, source lengths 992/32,160/33,360 bytes, zero drops and observed copied JS
backing maxima zero. Each case retained eight closed decoders, 40 closed frames
and three downloads. The new pre-rejection/platform/array/category observations
remain source-only until a fresh actual run; these prior traces do not prove the
full counted budget.

### Public real-encoder JPEG rejection experiment (source-only candidate)

The owner recipe builds its feature program once, runs the unchanged three-shape
owner suite, then a separate public experiment. The standalone
`just live-photo-jpeg-stress-test` recipe reuses that build and requires its
successful small owner report to match current source/program hashes. Two fixed 32/64Mbps AVC trials
author a deterministic 2048×1024 noisy keyframe with four repeated frames
requested as predictive, using the same five-plane/nonmonotonic-lambda framing.
Actual key/delta chunk counts are recorded; proof requires one key/four delta
chunks, five authored frames, cumulative chunk bytes including AUD at most 8MiB
and final container admission at most 8MiB. Existing color fixtures are unchanged.

Each trial has an isolated public catalog, service and browser. It explicitly
selects the completed import's library and Synthetic 001 (video plus static
preview, no JPEG focus stack), verifies the selected source manifest against the
authored bytes, and checks fresh retry video geometry is 32×32. Synthetic 000's
valid JPEG focus export remains untouched. The real viewer
JPEG rejection must preserve its verified static preview and disable Live Photo
export. A separate direct Rust export must also reject, with actual native JS
JPEG character lengths matching Rust pre-check byte lengths above 2MiB in BOTH
paths. Generic codec errors, injected strings, unadmitted fixtures and output
remaining below the limit cannot count as rejection proof. In-flight Rust
cancellation requires `cancelled` plus an aborted signal; a fresh admitted color
catalog then verifies viewer recovery and complete UI/direct export. Decoder and
frame closure counts must match. This fixture JS/Rust-input path is outside the
final mounted owner capacity collection and does not establish a total-memory
bound. Noisy pixels/containers are temporary, never evidence artifacts.

The bounded `jpeg-stress.json` contains numeric measurements, fixed outcome enums
and exact feature/source hashes only. Both fixed outcomes are retained; lack of
any proven trial fails the experiment. Setup, validation or cleanup failures also
fail regardless of another proven trial. Neither bitrate guarantees a JPEG
exceeding the cap: source-admission, predictive-output and under-threshold
outcomes remain explicitly unproven. Native wrappers restore in finally and
owned service/browser processes have bounded graceful shutdown/forced cleanup.
Every UI wait has a finite timeout, and direct Rust calls use a 60-second abort
deadline with the existing cancellable codec polling. The driver awaits helper
restoration before resource cleanup; no outer proof Promise race leaves the
helper running while its browser is closed. Restoration failures retain the
partial numeric result and separately fail cleanup.

The evidence wrapper packages current source/programs and available bounded
reports even after test failure, marks them partial with the original test exit
status, checks a temporary archive against 128MiB, then atomically renames it.
Packaging failure also fails successful tests; incomplete/stale archives are
removed. The existing private CI artifact uploads on failure only when a finished
archive exists. No Photos/private runtime or empirical rejection result is
claimed by this source-only candidate.

Before this candidate, `d03fc195` passed all five checks: CI `38232888965` and
containers `38232888971`. Its three geometry reports had 16 Rust stages and zero
drops, eight closed decoders/40 closed frames/three downloads each. The large
case included eleven concurrent 128×128 canvas observations alongside the
2048×1024 selected export; matching observations remain partial logical capacity
evidence, not full counted-memory proof.

### Feature-only retained UI inventory (source candidate)

The `test-harness` build borrows the actual mounted `Ui` synchronously at browser
Live Photo export entry and after complete-download status. The hook has no await,
map/DTO clone, registry, reactive subscription, or production-build API. The client
feature also enables the contracts' test-only numeric `OpaqueId` and
`ContentDigest` capacity accessors. No contents, identifiers or URLs are reported.

`retained_snapshot` is `[phase,0,0,0]`, with phase 0 at entry and 1 at completion.
The following rows always have the same order: five `retained_layout_*` rows,
`retained_layout_ui`, then `retained_stage`, `retained_thumbs`, `retained_lists`,
`retained_detail`, `retained_focus`, `retained_controls`, and `retained_counts`.
The layout rows report actual target `size_of` values (pointer/String/Vec/BTreeMap;
library/album/capture/file DTOs; detail/focus/derivative/export DTOs;
JSON/plane/processed/section types; optional detail/optional focus/reference/error).
`retained_layout_ui` is the size of one `Ui` handle bundle, not a count of all
`Ui` clones or signal allocations. `retained_counts` is
`[thumbnail_entries,capture_items,0,0]`.

The six owner-group rows use `[known_inline_and_vec_slot_bytes,
nested_owned_heap_capacity_bytes,string_utf16_units,string_owner_count]`.
Each signal value's inline header is counted once. A `Vec` includes all its
capacity slots, including unused slots; nested fields only add their own heap
capacities. Different selected/list/detail DTOs count as different owners even
when their contents match. The thumbnail map reports its header and each key/value
String heap capacity; its BTree nodes and inline key/value slots are **unobserved**.
`retained_detail` also includes preserved files and the export-job DTO;
`retained_focus` includes both focus DTOs and video lambdas. Controls include
status, GPU status, video label, search, sort and import-job strings. All additions
and capacity multiplications are checked; overflow invalidates the existing
observer's `complete` flag rather than wrapping or silently providing zero.

UTF16 units describe signal-owned strings, not actual DOM string allocations.
These observations exclude Tachys cached attribute/text strings, BTree nodes,
signal infrastructure, render/control closures, GPU resources, temporary async
future owners, DOM storage, allocation rounding and physical Blob sharing.
Tachys 0.2.19 `html/attribute/value.rs` lines 388–433 separately establish that a
String attribute retains a String (`SHA256
63ef671fa4e98cfc14ca97bd2aeca39e2eed6d9165a566e88d58943d59ff3a97`).
Those owners must not be treated as zero or folded into observed Rust capacities.
Existing JS Blob observations give logical sizes, not physical sharing. The fixed
1024-event observer and its bounded reports retain their earlier overhead caveats.

The existing `just live-photo-owner-test` recipe runs the scoped native command
`cargo test --locked -p lytronaut-client --features test-harness retained_observations::tests`
before the single feature WASM build. Five native feature tests (execution/count
pending) cover reserved capacity greater than length, distinct owners
with equal contents, empty options with reserved empty vectors, UTF16 accounting,
private-wrapper capacity access and arithmetic overflow. The existing three UI
cases require both complete snapshots and numeric target-layout receipts; they
add no geometry/codec workload. This source candidate has not run yet and provides
**partial retained evidence only**, never total 32MiB/64MiB acceptance. The sidebar
list accumulation and parsed-JSON phase bounds identified in the historical
`0f2fba4` audit remain unresolved; no admissions or navigation changed here.


### Bounded sidebar ownership (source candidate)

Library and album sidebars retain one 24-item page each. Previous/Next controls
preserve access to all pages; library labels use the absolute page offset. Paging
a sidebar preserves the selected library, album and capture even if its sidebar
row is no longer visible. Choosing a different library retains the existing
album/capture reset behavior. A pending request keeps the last committed page
visible, while Previous can supersede a pending Next; an error restores the
requested offset to the committed page so retry and back navigation remain useful.

Independent library/album request epochs reject stale responses after rapid
paging, library changes and owner cleanup. Capture selection/paging does not
invalidate an unrelated sidebar response. This replaces accumulated sidebar rows;
it does not lower catalog admissions or establish the remaining 32MiB retained
budget. DOM/framework, DTO capacity and parsed-JSON phase gaps remain as above.

The existing public `just browser-test` path includes an isolated synthetic
sidebar case with 30 legal library and album response DTOs, backed by the existing
public capture/source fixture. It exercises two-page navigation, absolute labels,
off-page selections, delayed responses after Previous/library changes, capture
selection during pending album paging and failed-page retry. All traffic stays
on the existing loopback service; no fixture mutations or private intake are
used. The ordinary stale-page reload test remains intact. These new source tests
have not run; CI account startup and local build headroom remain blocked.


### Feature-only completed metadata inventories (source candidate)

The observer's constant-time `enabled()` guard runs before any metadata/string
walk. Inactive feature builds perform no metadata traversal. The hook borrows
completed owners without cloning keys/JSON or creating reference strings. Its
recursive visitor stops at depth 64; overdeep helper input or checked arithmetic
overflow marks the bounded report incomplete. Parser limits, errors and validation
are unchanged. A disabled or incomplete observer is never empirical budget proof.

`processed_asset` observes the original JSON immediately after directory parsing;
`Container::references` observes its validated reference Vec immediately before
return, without extending its lifetime into plane construction. The final plane
and map Vec capacities are observed before the processed asset returns. Browser
Live Photo export observes source length, section/lambda capacities just before
its existing lexical parser-owner drops. No payload alias is counted as another
owned source buffer. The exact fixed rows are:

| Row | Four numeric slots |
| --- | --- |
| `metadata_json_storage` | root Value inline bytes plus array Vec capacity slots; actual String/key heap capacities; logical occupied map key/Value inline bytes; maximum visited depth |
| `metadata_json_counts` | Value nodes; map entries; array owners; String/key owners |
| `metadata_references` | reference Vec capacity slot bytes; pointer String capacities; identity String capacities; reference length |
| `metadata_planes` | plane Vec capacity slot bytes; map Vec capacity slot bytes; plane length; map length |
| `metadata_export` | borrowed source length; section Vec capacity slot bytes; lambda Vec capacity slot bytes; lambda length |

The map row measures **logical occupied slots**, not physical BTree nodes, headers
or slack. Earlier completed-JSON capacities remain live at the later reference
and plane phases, but references and planes are not simultaneous. Source Vec
capacity comes from `ui_source_begin`, not the slice length in `metadata_export`.
The layouts come from the existing actual-target `retained_layout_*` rows.
Snapshot quantities are not additive independent peaks or a whole-memory total.

Source receipt `/private/tmp/lytronaut-9fw-metadata-source-audit-1f8e166.json`
records pinned Rust 1.99 BTree slots (11 leaf slots, 12 internal edges) and
serde_json 1.0.150's BTree map implementation. It establishes neither private node
WASM ABI nor physical allocation sizes. Deserializer scratch is cleared/reused for
escaped text while the output String is copied, and is gone when `bounded_json`
returns. Its overlap with in-flight children, array old/new growth, map insertion
and active `collect_refs` path/escape temporaries is **not observed** here. Tiny
Value arrays can have minimum capacity four; byte-Vec minimum-eight or a blanket
capacity-less-than-twice-length formula is not a bound for these arrays. Actual
finished Vec capacity is reported, while growth overlap and allocator rounding
remain separate unfinished proof.

Six scoped native feature tests exercise reserved/nested owners, repeated borrowed
payload aliases, wide escaped public JSON, enabled/disabled and depth-bound behavior,
existing byte/node/depth admission boundaries, no-content reports and arithmetic
overflow. `just live-photo-owner-test` runs
`cargo test --locked -p lytronaut-format --features test-harness capacity_observations::metadata_tests`
before the existing single feature WASM build. The same three public mounted UI
cases require the five phase rows, five plane descriptors and source-length
correspondence. Tests and observations are source-only candidates; their execution
and the remaining 64MiB transient/32MiB retained acceptance are pending.

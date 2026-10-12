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

Private fixture verification is a separate explicit command:

```sh
mise exec -- just --quiet video-private-test "$AUTHORIZED_SEALED_COPY"
```

That recipe invokes the existing aggregate-only AVC decoder/SPS corpus proof
against an authorized prepared read-only sealed copy. It is not a private Live
Photo export acceptance claim. It is excluded from default checks and CI. Public
video/browser recipes remove the inherited private-root environment variable so
public validation cannot accidentally select private corpus mode. Private paths,
captures and identifiers must never enter public reports or CI artifacts.

## Resource verification still pending

Export admits one queued compressed chunk; ordinary viewing retains its existing
two-chunk upper limit. Pixels are copied in at most 16-row strips, bounding each
platform ImageData and Rust temporary to 128 KiB. The canvas backing and current
VideoFrame are released before the Rust PCM encoder allocates its scratch and
picture buffers. Movie assembly and ZIP packaging reuse one allocation; source
bytes are dropped before the final Uint8Array/Blob download copies. Browser
garbage collection and codec internals are not controlled by Rust.

For the largest 2048×1024 geometry with three retained frames, allocated PCM
scratch is at most 3.016 MiB, escaped picture scratch 4.524 MiB and movie assembly
13.602 MiB. A conservative known-live encoding estimate is 41.142 MiB with the
full 8 MiB source plus RGBA, selected JPEG and bounded data URL. A declared
one-reference decoder plus one queued picture may add 16 MiB if each coded picture
occupies RGBA storage. During strip extraction, the canvas/current frame overlap
and pre-final movie instead give approximately 61.312 MiB including that
conservative reference/queue allowance. These estimates do not prove an opaque
browser process stays within 64 MiB. B-picture reorder, additional references,
padding, conversion, garbage collection and final ZIP/Uint8Array/Blob copy phases
need actual source-bound measurements.

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

# Processed stacks, depth and legacy edits

> Historical source- and date-bound record. See the [delivered application baseline](../validation/final-main-status.md) for current status.

Bead: **lytronaut-w6z**. Observed: 2026-10-11. Research only; no product
implementation or private capture assets are committed.

## Question

Which existing Desktop derivatives can Lytronaut render through Rust or
Rust/WASM platform APIs, and which legacy edits must be preserved without
claiming that their appearance matches Lytro Desktop?

## Method

Read the authorized offline library, using the schema-23 catalog's LFE major
versions to classify companions. An independently written disposable Python
probe read container headers, bounded payload lengths, JSON tables and SHA-1
references. Each section has a 96-byte header and the next section is aligned to
16 bytes in this corpus; payload lengths are big-endian. No reference project
code was translated or incorporated. Source references were used to compare
format descriptions, then tested against actual bytes.

The probe inspected all 385 processed containers, resolved derivative references
locally, verified every embedded section digest, and read float-map payloads.
References to source metadata/originals in processed JSON are not assumed to
refer to sections in that same derivative file. It extracted JPEGs and two
representative H.264 streams only into a mode-0700 temporary private directory,
with mode-0600 files. The read-only source was never rewritten.

FFprobe/FFmpeg 9.0.2 reconstructed every H.264 block to a null sink as an
independent native CPU reference. This installed build enables GPL codecs and
is **research tooling only**, not a product dependency or a proposed Rust
runtime. A separate native Rust 1.99 probe using `image=0.25.9` with only the
`jpeg` feature (transitive `zune-jpeg=0.5.15`) decoded every extracted JPEG and
thumbnail. Its Cargo.lock has no C codec binding.

A disposable Puppeteer probe ran real Chrome 154 headlessly on this macOS
arm64 host, using an isolated temporary profile and a loopback-only HTTP server.
It fed the actual representative LFE 10 and LFE 11 Annex-B streams to WebCodecs
as access units separated by AUD NALs, preserving initial SPS/PPS and identifying
IDR keyframes. The browser exposed no private pixels to the tool output; frames
were counted, dimensions recorded and `VideoFrame.close()` called immediately.
The temporary server and browser were shut down afterward. This JavaScript is
a disposable platform experiment; application logic remains Rust/WASM.

Reproduction commands and probe SHA-256 values:

| Probe | Command / local artifact | SHA-256 |
|---|---|---|
| Container/reference experiment | `python3 /private/tmp/lytronaut-provisioning/interop_probe.py` | `3a9d5efb1b7fda5b88432af8744937b4f30a81a926867b030fcb5435d047ce69` |
| Native JPEG experiment | `mise exec -- cargo run --release --locked --manifest-path /private/tmp/lytronaut-interop-rust/Cargo.toml` | `src/main.rs`: `76c4042d6b656b539922bc9576041a96c8c2ef7f33c711ff1efac843b09ac2d2` |
| JPEG dependency lock | `/private/tmp/lytronaut-interop-rust/Cargo.lock` | `263ec38196a349899ec6702ab8a25b68a2cffe25ad217e81610dd4595778a1be` |
| Browser codec experiment | `node /private/tmp/lytronaut-provisioning/webcodecs_probe.cjs` | `43becc07cac20bb0370c0bcddf1cf42ed6c81f858f21f93d5a1bce3ded5423b2` |
| Rust/WASM codec binding | `/private/tmp/lytronaut-interop-web-binding/src/lib.rs` | `db4447b444640b5ebecdce8ca989c41874c59292ee5ac630624fa2692534d6a6` |

The exact Rust/WASM binding probe compiled successfully with Rust 1.99,
`web-sys=0.3.106` and features `VideoDecoder`, `VideoDecoderConfig`,
`VideoDecoderSupport`, `EncodedVideoChunk`. Its dependency lock SHA-256 is
`09c5e1dde9bc8309c7309b8df6e65cd8b51f0f041ba6cb25a66fa9120b905a50`.
The command was `RUSTFLAGS='--cfg=web_sys_unstable_apis' mise exec -- cargo check
--locked --target wasm32-unknown-unknown --manifest-path
/private/tmp/lytronaut-interop-web-binding/Cargo.toml` (one shell command).
It checks support-query/configure/decode/flush/close API bindings; the actual
browser decode above was measured separately, not through this compile-only
Rust probe. Initial compilation identified the required `VideoDecoderSupport`
feature; enabling it completed validation.

Aggregate results are operator-local `interop-summary.json` and
`webcodecs-summary.json` beside those probes. The initial direct JSON-value
comparison found differing types: sidecar numbers are strings, recipe numbers
are numeric. Numeric normalization was then tested separately, and the retained
probe updated to reproduce that comparison. No general enum conversion was
assumed. Scratch files may be cleaned by the OS. The durable, Git-ignored
source manifest is `fixtures/private/library-corpus-manifest.json` in the
operator's main clone, mode 0600. Rechecking all 728 source file SHA-256 values,
sizes and modification times against that manifest found no changes after the
processed probe. Individual hashes, identifiers and filenames remain private.

## Evidence

### Actual payloads and LFE variants

| Artifact / variant | Observed content and verification |
|---|---|
| All processed containers | 385 files; **1,307/1,307 section SHA-1 digests valid** |
| `stack.lfp`, LFE 10 | 51 refocus blocks; 2–9 frames per block |
| `stack.lfp`, LFE 11 | 58 refocus blocks; 4–12 frames per block |
| Standard refocus total | 109 H.264 High-profile Annex-B blocks; **654 frames decoded**; 1080×1080 displayed image, 8-bit `yuv420p`; metadata frame counts/dimensions agree with FFprobe |
| Additional LFE-10 perspective derivative | 8 `edofParallax` H.264 blocks of 37 frames each; **296 frames decoded**; metadata has `coord.x/y`, not a focus `lambda` |
| `stacklq.lfp`, LFE 11 only | 58 JPEG refocus stacks; **420 planes**, 4–12 per stack, each 360×360; frame lambdas strictly increasing in all 58 |
| `stacklq.lfp`, LFE 10 | Absent for all 51; static thumbnail is the codec-independent fallback |
| `thumbnail.jpg` | All 109 decode in Rust; 85 are 300×300 and 24 are 1080×1080; high-resolution thumbnails are a subset, not universal |
| Native Rust JPEG result | **529/529 images decoded**, 90,075,600 pixels: 420 planes plus 109 thumbnails; no source modification |
| `stack.view.lfp` | All 109 contain exactly one JSON section and **no encoded image planes**; recipes/acceleration descriptions do not substitute for a JPEG stack |
| `dm.lfp` | 109 depth plus 109 confidence maps; each 328×328, 430,336 bytes; all values finite when read as little-endian f32 |
| Stack depth LUTs | 167 maps, same 328×328/f32 layout, including 58 low-quality stacks; all finite |
| Scalar ranges | Depth/LUT aggregate −20 to 11; confidence approximately 0.501058 to **1.000000238**; preserve original values, allow float tolerance rather than rejecting slightly above 1 |

The independently observed 328×328 dimensions differ from the older
[lfptools Version-2 description](https://github.com/nrpatel/lfptools/blob/master/README_V2),
which describes 330×330 maps and H.264 stacks. Dimensions and lengths must come
from validated per-file metadata, not a hard-coded historical constant. Depth
values are relative focus/lambda data, **not measured distance in metres**.
Click-to-focus using a depth lookup and nearest stored plane is a discrete
selection operation; it is not calibrated light-field synthesis. The older
[format description](https://github.com/nrpatel/lfptools/blob/master/README)
describes this relationship, and actual stack metadata supplies per-plane lambda.
The confidence-to-quality mapping and image-coordinate conventions still need
validation before interactive depth picking is claimed correct.

### Browser H.264 experiment

Both representatives contain five access units, with codec string
`avc1.640029` derived from SPS profile/constraint/level bytes. There is no AVCC
configuration record. [WebCodecs AVC registration](https://www.w3.org/TR/webcodecs-avc-codec-registration/)
defines the Annex-B form when decoder configuration omits `description`.

| Chrome 154 configuration | LFE 10 | LFE 11 |
|---|---|---|
| `isConfigSupported`, `no-preference` | true | true |
| Actual decode, `no-preference` | 0/5, `EncodingError` | 0/5, `EncodingError` |
| `isConfigSupported`, `prefer-software` | true | true |
| Actual decode, `prefer-software` | **5/5**, no error | **5/5**, no error |
| Output geometry | coded 1088×1090, displayed 1080×1080 | coded 1088×1090, displayed 1080×1080 |

Config support alone is insufficient. Use a real decode probe and retry with
software preference after failure. Honor the output visible/display geometry
rather than treating coded buffer padding as pixels in the picture. Software
preference is a platform request, not a guarantee about the browser's internal
implementation. Hardware codec acceleration is **not demonstrated** by this
experiment. Only these two refocus blocks were decoded in-browser; all 117
blocks were decoded by the native reference. Perspective blocks, other Chrome
versions, Firefox, Safari, mobile browsers and Linux/Windows are unverified.

### Legacy edit semantics

All 109 version-4 sidecars numerically match their corresponding embedded view
recipes for focus, orientation, aperture, exposure, pan X/Y, zoom, contrast and
saturation. Each field agrees in 109/109 comparisons with maximum numeric
difference zero. The sidecar white-balance enum string `"1"` pairs with recipe
`"auto"` in all 109; other enum values are unverified. Matching stored values
establishes correspondence, not a verified rendering transfer function.

The archived vendor [Power Tools parameter documentation](https://github.com/kmader/lytro-power-tools/blob/master/lpt/recipe/params.py)
independently documents the following meanings. Its custom vendor license is
not permission to copy a processing implementation into this MIT project.

| Field family | Documented meaning / corpus evidence | Rendering decision |
|---|---|---|
| Focus | Focal-plane parameter; stored stack planes carry lambda | GO for explicit nearest-plane selection, labelled discrete refocus; do not promise continuous Desktop focus |
| Orientation | Standard EXIF rotation/reflection codes; sidecars use 1 and 8, while catalog orientation is 0 | GO for standard EXIF transforms of a verified derivative; do not use catalog 0 as an interchangeable enum |
| Crop | Bounds and angle in recipe; corpus has only full-frame, zero-angle crop | Preserve originals; verify synthetic nontrivial crop/rotation and coordinate mapping before applying imported crop |
| Aperture/focus spread | Normalized aperture diameter and near/far lambda spread | Preserve only; finite focal planes do not prove faithful aperture synthesis |
| Pan/zoom | Window centre in reference coordinates and window scale | New viewer navigation may be implemented independently; imported mapping/precedence remains unverified |
| Perspective/stereo/tilt | View position and focus-plane changes; eight parallax blocks have coordinates | Preserve only until coordinate geometry, interpolation and occlusion handling are verified |
| Exposure, white balance, tone/color, denoise/sharpen | Vendor parameter intent is documented; no independent pixel-equivalence reference was measured | Preserve only as imported state; new Lytronaut adjustments must identify their own semantics |
| Animation | Two animation files and recipe animation arrays exist | Preserve only; timing and interpolation not validated |

Never reapply a complete imported recipe blindly to an already processed stack:
some processing may be baked into pixels. Preserve both sidecar and embedded
recipe, including unknown fields and original scalar types, record their
provenance and require an explicit precedence policy if they disagree.

### Rust decoder choices and licensing

| Candidate | License / constraints | Decision |
|---|---|---|
| `image` 0.25.9 / `zune-jpeg` 0.5.15 | Pure Rust JPEG path actually tested; [image MIT or Apache-2.0](https://github.com/image-rs/image/blob/main/LICENSE-MIT), [zune-jpeg license/documentation](https://docs.rs/zune-jpeg/0.5.15/zune_jpeg/) | GO native CPU JPEG fallback; bounded dimensions/allocation required; WASM JPEG build needs implementation validation |
| WebCodecs through `web-sys` 0.3.106 | Rust binding to browser platform codec, not a Rust H.264 implementation; [`VideoDecoder` binding](https://docs.rs/web-sys/0.3.106/web_sys/struct.VideoDecoder.html) needs feature flags and `--cfg=web_sys_unstable_apis`; secure context and actual platform support required | GO for tested Chrome software path, gated by runtime decode; exact Rust/WASM API binding compile passed, integrated Rust browser flow still requires implementation validation |
| `openh264` 0.9.8 | [BSD-2 wrapper and core](https://docs.rs/openh264/0.9.8/openh264/); wraps C++ core; published platform table does not support wasm32 | Not a default for the requested Rust implementation; an FFI dependency exception would need an explicit later scope decision |
| `oxideav-h264` 0.1.8 | [MIT published crate](https://docs.rs/oxideav-h264/0.1.8/oxideav_h264/) states no registered working decoder; development repository has newer claims | NO-GO as a proven published native fallback; a pinned later revision requires actual corpus/conformance validation |
| FFmpeg | Native reference probe only; current local build enables GPL | Do not ship or invoke it as product runtime |

Retain required dependency notices. Format knowledge, attribution and software
license compatibility do not establish blanket permission to reuse private
samples or vendor code. No Python/C# processing service is proposed.

## Verdict GO | NO-GO

**GO** for JPEG thumbnail browsing of all 109 captures, and discrete JPEG-stack
focus selection for the 58 LFE-11 captures using native Rust. **GO, narrowly
gated**, for Chrome 154 browser H.264 refocus through the measured software
platform-codec path with decode-probe, software retry and visible-crop handling.
This does not require a server GPU.

**GO** for preserving and inspecting depth/confidence/recipe artifacts as
explicit legacy data, with bounded parsing. **NO-GO** for advertised depth-pick
accuracy, perspective/stereo/tilt playback, aperture synthesis, Desktop pixel
parity, generalized browser codec support, or a proven published pure-Rust
native H.264 fallback. Illum processed interoperability remains unverified.

## Recommendation

Implement a derivative capability ladder: static thumbnail first; JPEG focus
stack when available; tested browser H.264 only after successful actual decode.
On absent WebCodecs, decode error, context loss or no GPU, use CPU/native JPEG
derivatives and ordinary image display. A codec-independent browser must still
browse every capture. For LFE 10 without a working H.264 decoder, explicitly
show "preview only" rather than pretending thumbnail controls refocus.

Cache decoded JPEG planes by original container hash, section identity and
decoder/version parameters. Use bounded workers and a bounded browser frame
queue; close frames on all success/error paths. Derive Annex-B access units and
keyframe boundaries correctly, preserve SPS/PPS across resets, and test late
decoder errors, not only startup. Capability probing must not publish private
frames, metadata or samples.

Export verified displayed pixels as a flattened JPEG/PNG with explicit source
variant, frame/lambda, orientation, visible crop and Lytronaut adjustment
provenance. State that a flattened export discards refocus capability. Test
dimensions, color-space/range conversion, transform order and deterministic
CPU/GPU agreement on lawful synthetic fixtures before release. Reject export
of unsupported imported edits with an actionable explanation; never silently
claim they were applied. Archive/export-original must preserve every source
byte, unknown sidecar field and calibration association without rewriting LFPs.

Acceptance for the implementation molecule: all 109 thumbnails and all 420
JPEG planes decode; a browser probe reproduces both measured five-frame stacks
or cleanly enters preview fallback; coded padding never appears in output;
missing optional stacks do not block import; synthetic malformed-length,
reference mismatch, nonfinite-map, oversized-image and interruption cases are
bounded. Add full browser coverage of the 654 refocus frames before claiming
corpus-wide browser compatibility. Validate imported nontrivial crop/orientation
and coordinate mappings separately. Preserve source-manifest hashes unchanged
through import/export, and keep private validation distinct from public CI.

# Native decoding and calibration feasibility

Bead: **lytronaut-4li — Spike: validate native format decoding and camera calibration feasibility**. Observed 2026-10-11. Research only; no application decoder has been implemented.

## Question

Can independent Rust code recover sensor samples from the supplied library, and is there sufficient matching calibration to promise new refocus/viewpoint photographs for original Lytro and Illum?

## Method

Read the authorized local Desktop library without changing it. Independently describe container boundaries from byte observations and the [lfptools format specification](https://github.com/nrpatel/lfptools/blob/master/README). Write a disposable Rust standard-library probe that traverses `raw.lfp`, bounds-checks sections, extracts the uniquely sized sensor section, unpacks 12-bit samples and repacks them. The probe uses integer word arithmetic, not translated reference implementation code. A separate Python audit reads JSON, validates section SHA1 and resolves content references; Python is a research tool, not an application dependency.

The local experiment source is `/private/tmp/lytronaut-decoder-spike/probe.rs`, SHA256 `d1eb0298df726789f7d5a45eb05cadea5aef51b5b2f1539570fc2165ebc869ec`. It is disposable, not a product component. Reproduce locally while that artifact remains available:

```sh
rustc --edition=2024 -O /private/tmp/lytronaut-decoder-spike/probe.rs \
  -o /private/tmp/lytronaut-decoder-spike/probe
/private/tmp/lytronaut-decoder-spike/probe "$HOME/Pictures/Lytro Library.lytrolibrary"
```

The traversal sorts paths, reads each capture once, times parsing and unpacking separately with `Instant`, and compares every repacked byte. The sensor output is a 3280×3280 big-endian 16-bit PGM with maximum 4095. One such private output was written, its length checked, then deleted; no private pixels, names, hashes, UUIDs or serial numbers are committed. Source files are opened for reads only. Measurements below are one run over 109 captures on macOS arm64 on Apple M1 Pro with 32 GiB RAM, Rust 1.99.0 optimized (hardware confirmed with read-only `sysctl`). I/O, reconstruction, image encoding and concurrent workload are outside the reported unpack timings.

Calibration discovery additionally examined the existing Lytro application support directory, read-only. This is outside the uploaded library: discovering it locally does not mean a server receives it, or authorize redistribution. The audit reads table-of-contents file entries, resolves references, validates digests and extracts only aggregate camera/lens-position evidence.

## Evidence

### Containers and sample representation

All 109 capture metadata documents identify `camera.model = F01`, beyond the catalog's ambiguous model identifier `1`. All declare 3280×3280 pixels, 12 bits per pixel, big endian packing and Bayer top-left B. All contain a 16,137,600-byte sensor payload. This verifies original F01 corpus coverage, not every original camera/firmware or Illum.

Observed package header: 12-byte `89 4c 46 50 0d 0a 1a 0a 00 00 00 01`, followed by a zero u32 length. Nonempty LFM/LFC sections have 12-byte magic/version, big-endian u32 payload length, 80-byte name/padding area, then payload at section start +96. The section name occupies the first 45 bytes and has `sha1-` plus 40 hex characters. Advance to the next multiple of 16 **from the computed payload end**; do not search arbitrary payload bytes for magic or skip arbitrary zero bytes. Observed raw layout: one LFM directory and three LFC sections (sensor, public metadata, private metadata). Counts are observations, not a universal parser requirement. All 436 payload SHA1 values matched; all 327 directory `*Ref` links resolved.

For 12-bit packing, three bytes form one big-endian 24-bit integer. Its high and low 12 bits are successive row-major pixels. Synthetic `ab cd ef` gives `abc, def`. This arithmetic is also supported by [imageio's BSD-licensed Lytro reader](https://github.com/imageio/imageio/blob/master/imageio/plugins/lytro.py); no code was reused. Illum's documented 10-bit representation differs: four high-eight-bit bytes followed by one byte containing four two-bit low groups, low group first. Do **not** apply a generic contiguous big-endian bitstream decoder to Illum. [LFToolbox's raw-reader documentation](https://github.com/doda42/LFToolbox/blob/master/LFReadRaw.m) describes F01 3280×3280 and Illum 7728×5368 sensor sizes. Illum would therefore require 51,854,880 packed bytes; this is a documented expectation, not a locally verified payload.

| Local CPU check | Result |
|---|---:|
| Captures unpacked | 109 |
| Samples unpacked | 1,172,665,600 |
| Repacked bytes differing from original sensor payload | 0 |
| Observed sample range | 0–4095 |
| Median parser time, excluding I/O | 0.0027 ms |
| Median allocated unpack time | 5.7141 ms |
| p95 allocated unpack time | 7.0508 ms |
| Truncated/invalid container cases rejected | 7 |
| Synthetic known 12-bit vector | Pass |

The seven negatives are lengths 0,15,17,111 and file-length minus one, an oversized first-section payload length and corrupted section magic. This small research probe does not establish production parser robustness: duplicate references, hostile JSON, zero-length sections, integer limits, unknown versions, padded layouts and fuzzing remain implementation acceptance work.

PGM output was 21,516,818 bytes including its header. Exact unpack/repack establishes lossless **sensor extraction**. It establishes neither correct RGB color nor a reconstructed scene photograph; no photograph PSNR/SSIM, refocus quality or Desktop parity was measured.

### Metadata and color requirements

Prefer declared dimensions/packing/CFA over catalog image dimensions, which are zero in this corpus. Retain raw metadata losslessly and normalize validated typed fields separately. Require pixel packing/right shift, Bayer orientation, per-channel black/white levels, white-balance gains, color matrices/gamma and orientation before attempting scene color. The inspected representative capture declares black 168 and white 4095 for all four Bayer channels, top-left B, and unequal white-balance gains. Those values are capture-specific, not constants for the library. Preserve sensor/MLA pitch, sensor offsets, lens zoom/focus steps, optical parameters and private camera/sensor identity for calibration matching; never expose private metadata in default public API responses or diagnostics.

A sensor mosaic, even after ordinary Bayer demosaic, still contains microlens images. Its 3280×3280 dimensions are not the output photograph resolution. Microlens grid estimation, flat-field correction, angular resampling and color handling are required for actual light-field reconstruction; [Cho et al., ICCV 2013](https://openaccess.thecvf.com/content_iccv_2013/papers/Cho_Modeling_the_Calibration_2013_ICCV_paper.pdf) specifically studies raw-to-light-field calibration and sensor/MLA misalignment.

### Calibration found, with unresolved matching

Contrary to the initial library-only inspection, four `data.C.0`–`data.C.3` files exist under `~/Library/Application Support/Lytro/`. Their total is **1,017,796,624 bytes**. The containing camera directory matches the capture's private camera serial (comparison only; serial omitted). Across the bundles: 195/195 section digests verified,191 table-of-contents file references resolved,63 sensor-sized payloads found. This is promising acquisition evidence, not proof that every payload is a suitable flat field.

There are 62 `MOD_*.TXT` metadata records, all F01, covering 31 distinct `(zoomStep, focusStep)` pairs. The library covers 60 distinct pairs; **zero pairs match exactly**. The MOD files contain a JSON prefix followed by a 46-character non-whitespace trailer. A research prefix parser recovered their metadata, but the trailer semantics and authenticity are unverified. A production implementation must preserve and identify that wrapper instead of accepting arbitrary trailing text silently. The difference between 63 sensor payloads and 62 MOD metadata records must also be reconciled before calibration ingestion is accepted.

[Jaramilloh's calibration guide](https://github.com/Jaramilloh/Process-Light-Fields-Captured-With-Lytro-Camera#calibration-data) identifies original-camera data.C bundles as white-image sources and distinguishes these from checkerboard calibration for metric rectification. [LFToolbox's decoding procedure](https://github.com/doda42/LFToolbox/blob/master/LFLytroDecodeImage.m) selects calibration using camera identity and lens settings, reinforcing that arbitrary borrowed calibration is inadequate. These are research references, not code to port.

For Illum, [PlenoptiCam's user guide](https://github.com/hahnec/plenopticam/blob/master/docs/source/guide.rst#using-your-own-lytro-illum) documents camera Settings → General → Transfer Pairing Data to SD Card, producing a tar archive; preserve its original filename and pair it with the camera's LFR. No Illum capture, exported pairing archive or matching rights/provenance has been supplied. No camera commands were performed during this spike.

## Verdict

**GO** for independent F01 container validation, lossless sensor extraction and preserving archived previews. **NO-GO** for advertising new calibrated synthesis on either camera model today.

| Model / variant / capability | Verdict | Basis and remaining gate |
|---|---|---|
| F01 observed raw.lfp package, 12-bit sensor extraction | GO | 109 actual captures, metadata and byte-exact roundtrip |
| F01 standalone extracted RAW + metadata | NO-GO for advertised input support | Same packing is feasible; no authorized paired standalone sample verified |
| F01 archival JPEG / existing processed stack display | GO to implement preservation/display path | Existing assets; rendering/codec decisions belong to interoperability spike |
| F01 data.C calibration container acquisition | GO for separate private intake/inspection | Matching camera directory, section digests and references verified |
| F01 calibrated refocus / viewpoint / aperture synthesis | NO-GO | Lens-position matching/interpolation, MOD wrapper and grid/color quality unverified |
| Illum LFR package / standalone 10-bit RAW | NO-GO for advertised decoding | Documented candidate format, zero authorized local captures or bit-packing oracle |
| Illum pairing tar / XRAW embedded calibration | NO-GO for synthesis | No acquired matching calibration; XRAW variant unverified |
| Unknown camera, firmware, container version or malformed metadata | NO-GO | Preserve originals; report unsupported capability, never infer support from extension |

NO-GO means retain the capability as gated future work, not discard raw captures or block supported archived browsing.

## Recommendation

Ship archive-first import and previews while developing a bounded native decoder. Separate statuses `original preserved`, `archival preview available`, `sensor samples decoded` and `calibrated synthesis available`. Do not label demosaiced sensor mosaics as refocused photographs.

Implement camera calibration as a separate explicitly supplied asset with private provenance, hash and model/camera/sensor identity. Validate every reference and declared image size. Match camera and sensor identity first, then format/CFA/dimensions and lens-position coverage. An absent/mismatched identity must disable calibrated synthesis; never substitute a different camera's bundle. Where lens positions differ, permit selection/interpolation only after a separately documented, measured method establishes coverage and error bounds. Keep originals/metadata and compatible previews usable, and explain the missing capability in the UI. Missing calibration must not produce a plausible-looking silently incorrect image.

Before lifting synthesis gates, a Rust compatibility bead should:

1. Reconcile all calibration sensor/metadata pairs and identify MOD trailers and bundle chaining. Authenticate private provenance without publishing identifiers; acquire Illum LFR + pairing archive and documented redistribution rights for CI fixtures separately.
2. Establish independent microlens center/grid fitting with held-out calibration residuals. Proposed provisional acceptance: median center residual ≤0.25 sensor pixel and p95 ≤0.5 pixel; validate that those limits predict visible quality on both models before treating them as release guarantees.
3. Validate black/white scaling, flat-field stability (finite outputs, masks for near-zero denominators) and per-channel color using controlled neutral/color captures. Proposed white-field central-region channel coefficient of variation ≤5%; scenes alone cannot certify this. Obtain suitable test targets rather than inventing metrics from ordinary photographs.
4. Use authorized near/far focus scenes: measured sharpness must peak at the corresponding focus settings, viewpoint displacement must be directionally consistent, outputs deterministic and finite, and no systematic honeycomb/incorrect Bayer artifacts. Compare approved native output with an independent oracle or permitted Desktop export using a fixed crop/color policy; set PSNR/SSIM tolerances only after real paired references exist. Metric stereo rectification additionally needs checkerboard calibration.
5. Require byte-exact packing tests and negative cases for both 12-bit and Illum's distinct 10-bit layout, fuzz bounded container/reference/JSON handling, enforce memory/time budgets and version cache keys by source hash plus decoder/calibration/settings versions.

No GPL PlenoptiCam code, unlicensed project code or MATLAB code was translated or added. The MIT project may cite independent scientific/format facts; importing reference implementation code or fixtures requires its own license/provenance review. All photographic/calibration artifacts remain private and outside Git.

# Optional Rust GPU derivative presentation

The Leptos viewer automatically tries GPU presentation for verified stored JPEG
focus planes. CPU JPEG presentation and original downloads remain the baseline. GPU
absence, failed acquisition, validation errors and device loss retain the last
verified JPEG and controls. A failed GPU is not repeatedly reacquired in the page session; a fresh page may
probe capability again. No renderer-selection or retry controls are exposed. Native
server builds have no wgpu dependency. The WASM dependency pins wgpu 30.0.1 with
only `webgpu` and `wgsl`; product orchestration is Rust, the shader is WGSL.

The native explicitly oriented derivative remains the fallback. For EXIF 2–8,
Rust separately retrieves the authenticated identity derivative, validates its
FocusDerivative identity/plane/orientation/extents and generated SHA256, and
actually decodes it before uploading. WGSL applies the explicit viewer EXIF
coordinate transform exactly once to this identity input. Imported orientation
and other recipes remain preserved only. Identity does not undo an imported
baked recipe. GPU pixels are compared against the identity JPEG decoded in the
browser, rather than a second, differently recompressed native JPEG.

Rust obtains an actual wgpu canvas surface, inspects adapter/device limits,
creates the pipeline under a validation error scope, stages bounded RGBA bytes
and calls `queue.write_texture`, a render pass, `queue.submit`, and
`queue.present`. The WGSL fragment uses integer texture coordinates for all eight
EXIF mappings and nearest-neighbor scaling; it invents no focus, aperture,
viewpoint or calibration. Unknown color remains labelled unverified. The tested
adapter's preferred canvas format reproduces decoded midtones exactly; no
cross-platform color-management parity is claimed.

## Admission and release

An active optional operation reserves **24 MiB through the existing shared
64 MiB transient admission**, whose two-transfer policy also covers grid work.
At the FocusStack contract's 512² source limit, conservative accounting covers
eight 2 MiB encoded copies for fallback plus identity inputs (16 MiB), two RGBA
source decodes (2 MiB), scratch canvas/ImageData/WASM upload copies (3 MiB), and
3 MiB for metadata/runtime allowance. This includes both response bodies and
staging/intermediates inside the reservation. Shared transient admission plus
32 MiB persistent DOM remains at most 96 MiB; this is counted application
allocation, not measured browser RSS, decoder internals or driver residency.

The independently tested allocation policy rejects zero/overflow/over-2048²
geometry before allocation. A hypothetical larger caller must reserve its
computed transient bytes before decode. The current product accepts only typed
512² focus sources. CPU staging downsamples any otherwise admitted source above
1024² before upload. GPU input and interactive output are capped at 1024², then
bounded by actual device limits. The default output cap is reduced to **512**;
a **256** option is available. GPU accounting includes input texture, queue
upload staging, three surface images and the 16-byte uniform, below 64 MiB.

No focus plane cache is retained. Upload arrays and scratch canvases are
released, input textures and uniforms are destroyed after submission validation,
and replaced/cleared renderers explicitly destroy their device. Acquisition
expires after 700 ms; its future continues solely to destroy a late device,
rather than abandoning `requestDevice`. Observed device loss/error is polled at
100 ms; a failed renderer releases resources and exposes CPU fallback. Generation
checks discard stale results; a concurrent operation retains the CPU JPEG.
Native controls and authenticated preserved originals remain usable.

## Evidence and limitations

[GPU validation](../validation/gpu-validation.md) records actual Rust/WASM,
uploaded textures, compositor-visible pixel comparisons, failures and resource
release on the declared Mac/Chrome. Rust submit markers and a headed compositor
trace attempt distinguish queue submission from frame readiness and presentation.
A double-rAF callback is **post-frame callback evidence**, not presentation
feedback. Unrelated `FramePresented` events cannot establish the canvas's
submit-to-present latency. The 16.7 ms compositor-presentation target remains
unverified where a frame cannot be attributed; physical display photon timing
is also unmeasured. Optional quality is reduced and no frame-rate guarantee or
Safari/Firefox/Windows/Linux/mobile renderer support is claimed.

Final exact-main coverage passed: native 96.14%, client 96.00%, and union 96.19%
under the unchanged itemized GPU exception. This renderer's behavioral and
physical-host tests remain the correctness evidence for the exempt GPU code;
they do not establish the 16.7 ms compositor-presentation target.

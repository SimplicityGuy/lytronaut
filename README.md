# Lytronaut

*an explorer for your light-field library*

A self-hostable home for decoding, viewing, preserving, and working with old
Lytro camera libraries. Planned support covers the original Lytro and Lytro
Illum, promoted one verified file variant at a time.

## Status

The implementation epic `lytronaut-re8` is filed and kickoff is approved.
Foundation development is active; the usable library explorer is still under
construction. Beadhive research molecule
`lytronaut-ck2` is complete: its four spikes and scope decision are reviewed and
landed. Evidence supports a preservation-first original-camera
MVP: schema-23 library import, all 109 archived thumbnails, and discrete JPEG
focus stacks for the 58 verified LFE-11 captures. Browser H.264 and Rust/wgpu
integration have explicit runtime gates. Calibrated raw synthesis and Illum
compatibility remain unverified.

See [the evidence and scope decision](docs/design/001-lytronaut.md) and the
[approved implementation plan](plans/implementation.yaml). This repo
remains private; no release or image publication is enabled.

## Intended experience

- Upload a Lytro Desktop library, captures, and camera calibration data.
- Preserve untouched originals, albums, metadata, and non-destructive edits.
- Explore verified captures through refocus, viewpoint, and aperture controls.
- Use browser hardware acceleration where it helps, with a usable fallback.
- Run locally or in a self-hosted container without a cloud service or server GPU.

The interface will draw on mid-2010s creative software: a graphite photo
workspace, luminous cyan accents, crisp typography, a filmstrip, restrained
gradients, and tactile controls. Accessibility and reduced motion remain part
of the design.

## Development

Install [mise](https://mise.jdx.dev/), then:

```sh
mise trust
mise install
mise exec -- just setup
mise exec -- just check
```

`just test` runs contract, live SQLite and HTTP tests. `just check` also builds
the Leptos WASM client and matching wasm-bindgen glue and checks dependency
boundaries and redistribution notices. `just serve` builds the Rust/WASM browser
and starts the native service on http://127.0.0.1:3000. Open the library workspace directly, then select a library directory or ZIP to
preserve and browse its supported JPEG previews, albums and originals. Export
verified static/stored-focus pixels as flattened PNG or JPEG with separate
provenance, or stream a lossless originals/catalog bundle and restore it to an
isolated application copy. See [export and restore](docs/architecture/api.md#verified-export-and-lossless-bundles).
`LYTRONAUT_DATA` selects the private application state directory; never point it
at a source library. `just browser-test` runs synthetic actual-service/browser
integration after `just wasm-build build`. See [private Linux Docker operation](docs/operations/linux.md) for image builds,
Compose configuration and persistent mounts.
See [browser operation and evidence](docs/architecture/browser.md),
[deployment access](docs/architecture/authentication.md), and
[workspace contracts](docs/architecture/contracts.md).

## Architecture direction

An independent Rust implementation using Axum/Tokio, SQLite with durable jobs,
and Leptos client-side Rust/WASM. Pure Rust JPEG decoding provides the native
preview baseline. Optional browser wgpu/WebGPU presentation and WebCodecs
through Rust bindings require successful runtime probes and retain static/CPU
fallbacks. The server requires no GPU. Existing derivatives support discrete
focus selection; new calibrated light-field reconstruction is a later evidence
gate. Imported edits are preserved, with only independently verified transforms
applied and no claim of Desktop appearance parity.

Private photographs and camera serial numbers stay outside source control.
Import will preserve originals and will never edit an uploaded source library.

## Contributing

Use Conventional Commits. The project is versioned through Cargo metadata;
local Linux containers are supported; release tooling remains separate.
No automated publishing is enabled. Preserve merge commits for Beadhive work.

## License

Lytronaut's original code is [MIT licensed](LICENSE). Reference projects and
sample assets retain their own licenses; acknowledgement does not grant reuse
rights. The [resolved inventory](DEPENDENCY_LICENSES.md) and
[third-party notices](THIRD_PARTY_NOTICES.md) cover the locked workspace graph.

## Acknowledgement

Lytronaut acknowledges the work that has made Lytro preservation and light-field
research possible:

- [yamada-sexta/lytro-py](https://github.com/yamada-sexta/lytro-py): RAW decoding,
  metadata, calibration, and subaperture exploration.
- [hypothete/lytro-lumi](https://github.com/hypothete/lytro-lumi): interactive
  browser light-field viewing and GPU rendering experiments.
- [Jaramilloh/Process-Light-Fields-Captured-With-Lytro-Camera](https://github.com/Jaramilloh/Process-Light-Fields-Captured-With-Lytro-Camera):
  original-camera decoding, calibration, rectification, and color correction.
- [hahnec/plenopticam](https://github.com/hahnec/plenopticam): plenoptic camera
  processing, calibration, refocusing, and light-field research.
- [mscappini/Lytro.Net](https://github.com/mscappini/Lytro.Net): native Lytro
  container and metadata exploration, including focused-image extraction.

These are research references, not bundled runtime dependencies. Lytronaut will
not translate GPL or unlicensed source into its MIT implementation.

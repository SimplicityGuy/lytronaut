# Lytronaut

*an explorer for your light-field library*

A self-hostable home for decoding, viewing, preserving, and working with old
Lytro camera libraries. Planned support covers the original Lytro and Lytro
Illum, promoted one verified file variant at a time.

## Status

The initial release candidate completed its planned delivery and landed on
`main` at `a3d0482` on 2026-10-18. It
imports Lytro Desktop schema-23 libraries from the original camera, preserves
every original byte-exact, browses static JPEG previews and albums, offers
discrete focus for archived JPEG focus stacks, optional browser H.264 focus and
GPU presentation with automatic CPU fallback, flattened and Apple Live Photo
export, and a portable backup/restore of any size. Lytro Illum, calibrated
raw light-field synthesis and imported-edit appearance parity are not supported.
See the [supported variants and capabilities](docs/operations/supported-variants.md)
for exactly what is verified, conditional, not verified or out of scope, and the
[evidence and scope decision](docs/design/001-lytronaut.md). The
[documentation index](docs/README.md) separates current operator guidance from
date- and source-bound design, spike, and validation records.

The source repository is public. Private photographs, originals, catalog
identifiers and camera serial numbers never enter it: CI and committed reports
use independently authored synthetic fixtures only. No release, registry image
or image publication is enabled; build images locally.

## Intended experience

- Upload a Lytro Desktop library, captures, and camera calibration data.
- Preserve untouched originals, albums, metadata, and non-destructive edits.
- Explore verified captures through discrete refocus today; viewpoint and
  aperture controls wait on verified calibration evidence.
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

`just demo` builds the service and Rust/WASM client, writes a synthetic library
(30 captures, 3 albums, one JPEG focus stack) and a fresh application state into
a disposable directory, and serves it on http://127.0.0.1:3000; it prints the
folder to import. No private data is involved.

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
integration after `just wasm-build build`. See [self-hosted Linux operation](docs/operations/linux.md) for image builds,
SQLite or PostgreSQL Compose configuration, deployment-level access control,
storage locations, upload budgets, health checks, backup, recovery, upgrade and
rollback, and known limitations.
`just postgres-parity` runs the strict PostgreSQL suite against a disposable
container of the pinned PostgreSQL release.
`just coverage` enforces at least 95% first-party line coverage for native code,
the Rust/WASM client in a real browser, and their union (see
[coverage](docs/operations/coverage.md)). GPU-specific WebGPU rendering code is
an owner-approved exception to the 95% gate: it is still measured and reported,
and it is verified by physical-GPU reference-host runs. Optional software-adapter
runs are never presented as physical-GPU results. The CI jobs and how they become
[required checks](docs/operations/required-checks.md) are listed separately.
See [browser operation and evidence](docs/architecture/browser.md),
[deployment access](docs/architecture/authentication.md), and
[workspace contracts](docs/architecture/contracts.md).

Exact-main validation for `a3d0482` passed CI run `39167915596`, coverage run
`39167915581`, and Linux containers run `39167915561`; PostgreSQL parity passed
64 cases with none failed or ignored. The final coverage report measured native
96.14%, client 96.00%, and union 96.19% under the documented GPU exception.

## Architecture direction

An independent Rust implementation using Axum/Tokio, SQLite or PostgreSQL with durable jobs,
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

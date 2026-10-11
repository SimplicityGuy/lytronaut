# Lytronaut

*an explorer for your light-field library*

A self-hostable home for decoding, viewing, preserving, and working with old
Lytro camera libraries. Planned support covers the original Lytro and Lytro
Illum, promoted one verified file variant at a time.

## Status

This repository is a validated Rust development scaffold. The application has
not been implemented. Beadhive planning is pending review and kickoff approval.
See [the design and roadmap](docs/design/001-lytronaut.md).

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

`just test` runs Rust tests; `just build` builds the scaffold. There is no
application development server or deployment image yet. CI invokes the same
formatting, Clippy, test, and build gates with the committed Cargo lockfile.

## Architecture direction

An independent Rust implementation: a native decoder and CPU reference
renderer, a Rust HTTP service with persistent import jobs and a local catalog,
and a Rust/WASM WebUI. GPU rendering and browser compatibility will be measured
before selecting the final renderer. Full Desktop library fidelity, calibration,
and processed-stack support require verified samples.

Private photographs and camera serial numbers stay outside source control.
Import will preserve originals and will never edit an uploaded source library.

## Contributing

Use Conventional Commits. The project is versioned through Cargo metadata;
release tooling and containers are planned after the compatibility milestone.
No automated publishing is enabled. Preserve merge commits for Beadhive work.

## License

Lytronaut's original code is [MIT licensed](LICENSE). Reference projects and
sample assets retain their own licenses; acknowledgement does not grant reuse
rights. Third-party notices will accompany any permitted redistribution.

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

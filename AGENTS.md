# Lytronaut

Self-hostable Lytro library explorer. Original project code is MIT licensed.
Backend, decoding, and WebUI application logic must be Rust; the browser client
targets WASM. Generated browser glue, CSS, and GPU shaders are permitted assets.

Use mise-pinned tools and `mise exec -- just check` before submitting changes.
Keep Cargo.lock committed. Use Conventional Commits and preserved merge commits.

Plan work through Beadhive; no application implementation or dispatch until the
operator approves the plan and kickoff. Do not approve gates on their behalf.
See docs/design/001-lytronaut.md for scope, gates, evidence, and the roadmap.

Never modify source libraries or commit personal captures, calibration bundles,
serial numbers, credentials, or unlicensed sample data. Use independent format
implementations; do not translate GPL or unlicensed reference code. Preserve the
README's Acknowledgement section and third-party notices where required.

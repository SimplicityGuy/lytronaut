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

<!-- bh:agf:start (managed by `bh hive init` — edit outside these markers; `-f` refreshes) -->
## AGF — Agentic Git Flow

This repo is onboarded as a **`bh` hive** and develops via **AGF**: work is tracked in beads
and driven through `bh`, **not** raw `git` / `bd` / `gh`.

- **Is this repo set up for AGF?** → run `bh hive ready` (add `-v` for the line-item breakdown).
- **Lifecycle, roles, conventions:** see `docs/AGF.md` and the bh plugin's role skills.
- Drive beads with `bh work`; load the role skill for your seat (coordinator / developer / merger).
- Batch/collapsed work lives in ONE shared `wt/batch/<group>` worktree and completes as a UNIT:
  `bh work submit --group` then `bh work merge --group` — per-bead `submit`/`check` don't apply.
<!-- bh:agf:end -->

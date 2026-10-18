# Decision: Apple Live Photo export in the initial release

> Historical source- and date-bound record. See the [delivered application baseline](../validation/final-main-status.md) for current status.

The operator explicitly requested dispatch of `lytronaut-9fw` for the initial
release. This supersedes its previous separate-follow-up disposition. The release
molecule `lytronaut-re8` now contains fourteen implementation tasks; final
acceptance `lytronaut-cjq` depends on Live Photo export as well as the existing
implementation prerequisites.

The existing filed export scope remains: a selected still/key frame paired with a
bounded generated refocus animation from verified stored planes, packaged as a
complete Apple-compatible resource pair. The motion is a generated focus
animation. Original captures and legacy recipes remain unchanged.

The developer must verify Apple's metadata/container requirements and a feasible
Rust/native or Rust/WASM codec path before implementation claims. Production
remains self-hostable on CPU-only Linux, without a C++/FFmpeg runtime or an
Apple-only server. Unsupported capabilities must report their limitations and
preserve ordinary still export. An actual feasibility blocker requires evidence
and an operator decision; it does not authorize a substitute export format.

Compatibility validation uses lawful synthetic assets and an isolated Photos
test library, never the operator's existing Photos library or iCloud account.
iOS verification requires an available authorized device. Private capture
verification stays local and preserves source hashes.

The deployment-level access decision and automatic-rendering decision still
apply. Live Photo application code belongs in the final native plus actual WASM
application line-coverage denominator, with the same 95% GitHub Actions gate.

Dispatch starts format/codec research alongside Docker deployment work. Shared
builds and validation remain serialized, and the Live Photo developer integrates
the login-free Docker changes before final submission.

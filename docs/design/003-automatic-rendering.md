# Automatic rendering selection

> Historical source- and date-bound record. See the [delivered application baseline](../validation/final-main-status.md) for current status.

Status: accepted by operator, 2026-10-12 (UTC).

The operator requested: “the CPU/GPU selection should be automatic. exposing a button for it makes it overly complicated.”

Lytronaut automatically attempts GPU presentation when supported and uses CPU rendering when GPU capabilities are absent or initialization or presentation fails. Browsing and focus selection do not require a renderer choice. Remove CPU/GPU selection, toggle and renderer retry buttons. Renderer diagnostics, if shown, are informational only.

Keep existing allocation limits, cancellation, resource cleanup and failure handling. Test missing GPU APIs, missing adapters, device rejection, pipeline failure, device loss and repeated capture switches through the actual Rust/WASM browser. Failure must leave useful CPU rendering and controls available without a user action. Do not repeatedly attempt a known failed GPU initialization during the same page session.

Implement this amendment in GPU validation and verify the final integrated behavior in MVP validation. The existing work group and dependency order remain intact.

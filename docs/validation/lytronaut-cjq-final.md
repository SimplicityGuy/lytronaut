# Final MVP validation (lytronaut-cjq)

This report collects the acceptance evidence for the initial release candidate.
Public results come from GitHub Actions on synthetic fixtures only. Private
library results are local, aggregate and non-identifying, and are reported
separately. What each result means for users is summarised in
[supported variants and capabilities](../operations/supported-variants.md).

## Line coverage

Measured by the `coverage` workflow (`just coverage`; method in
[coverage](../operations/coverage.md)) on GitHub-hosted `ubuntu-24.04` runners,
Chrome 154, Rust 1.99.0 (WASM instrumented with the pinned nightly of the same
LLVM 23.1).

Run 38538617238 at `b9e267794c6956f4da9b3bd17e32b3a80f8ffca8` (pull request #4),
gate passed:

| Gate | Covered / lines | Percent | Floor |
|---|---:|---:|---:|
| native (contracts, format, storage, service) | 8699 / 9049 | 96.13% | 95% |
| WASM(client), actual client in Chrome | 2658 / 2763 | 96.20% | 95% |
| union | 11392 / 11848 | 96.15% | 95% |
| GPU-specific (exempt), WASM | 66 / 500 | 13.20% | not gated |
| WASM(client) without the exemption | 2724 / 3263 | 83.48% | not gated |
| union without the exemption | 11488 / 12349 | 93.03% | not gated |

GPU-specific WebGPU code is an owner-approved, itemised exception (2026-10-13):
it is measured on its own row above and excluded from the WASM and union
percentages. Without the exception the WASM(client) and union figures are the
lower ones shown. GPU rendering correctness is verified on the physical-GPU
reference host ([GPU report](lytronaut-us7-gpu.md)), never by a software adapter.

### Repeatability

Early CI results swung by up to 4.7 WASM points. Every swing traced to one
browser flow failing and skipping the rest of `browser-test` (that script's own
count fell from 4229 to 3276 lines while every other script was unchanged); the
step failed visibly, but the coverage report did not say why. The fixes were in
the flows (find captures through search, settle before dumps, hold requests
where a newer one must supersede them, inject failures deterministically) and
in the client (an upload that could not start left its import holding an
admission slot, which made a later flow fail with a resource limit).

Three sequential runs of one commit gave identical WASM(client) coverage,
2658 / 2763: 38530962243, 38533389706 and 38535638197. Native varied by two
lines (8699 or 8697): the PostgreSQL desktop test runs two imports of one
library concurrently and only sometimes reaches the "already installed" branch
(`lytronaut-storage/src/desktop.rs`). The gate also checks each browser
script's own client lines against a reviewed baseline
(`crates/xtask/coverage-script-baseline.txt`).

## Public synthetic CI

At `b9e267794c6956f4da9b3bd17e32b3a80f8ffca8`: CI run 38538617176 (`check`,
`wasm`, `browser`, `persistence-parity`, `large-bundle`) and Linux containers
run 38538617454 (`container-amd64`, `container-arm64`, each building the image
natively and passing the SQLite, bundled-PostgreSQL and external-PostgreSQL
Compose deployments with all four cross-backend restores) all succeeded. The
jobs and how they become required checks are in
[required checks](../operations/required-checks.md); they are not enforced as
required checks yet.

## Private library

Not rerun at this source. The earlier protected runs remain the latest
private evidence: 109 captures, 3 albums and 109 ordered memberships, 58 focus
stacks with 420 JPEG planes, and 728 logical originals unchanged through import,
export and isolated restore ([export report](lytronaut-u8i-export.md),
[focus report](lytronaut-m5h-focus.md)). A rerun at the final source needs the
operator's authorised sealed copy and is listed as open below.

## Not met or not verified

These stay visible; none is claimed:

- CPU focus p95 at most 100 ms on the release reference host: **not met** (last
  integrated release measurements 104.7 to 150.7 ms).
- Performance under grid load, and the private-library import, restart,
  export and restore rerun at the current source: open.
- H.264 Level 5 frame-size check (1138 x 1842) and Live Photo export end to end
  on PostgreSQL: open.
- Counted 64/32 MiB Live Photo memory budget: partly proven (accepted
  limitation). GPU compositor submit-to-present at most 16.7 ms: not verified
  (accepted limitation).
- VoiceOver: not verified (owner sign-off, out of scope for this version).
  iOS: out of scope (owner sign-off).

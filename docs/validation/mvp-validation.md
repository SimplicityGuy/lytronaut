# Final MVP validation

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

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
reference host ([GPU report](gpu-validation.md)), never by a software adapter.

### Repeatability

Early CI results swung by up to 4.7 WASM points. Every swing traced to one
browser flow failing and skipping the rest of `browser-test` (that script's own
count fell from 4229 to 3276 lines while every other script was unchanged); the
step failed visibly, but the coverage report did not say why. The fixes were in
the flows (find captures through search, settle before dumps, hold requests
where a newer one must supersede them, inject failures deterministically) and
in the client (an upload that could not start left its import holding an
admission slot, which made a later flow fail with a resource limit).

Three historical runs reported identical aggregate WASM(client) counts,
2658 / 2763, across two commits. These are not three passing repetitions of one
commit:

| Run | Source commit | Workflow conclusion |
|---|---|---|
| [38530962243](https://github.com/SimplicityGuy/lytronaut/actions/runs/38530962243) | `64fc6a5d9f73918fb38b4843e2df645143352253` | failed (old per-script gate) |
| [38533389706](https://github.com/SimplicityGuy/lytronaut/actions/runs/38533389706) | `8eb318943480d45a28d427ac1b680ae52bc0dcc8` | failed (old per-script gate) |
| [38535638197](https://github.com/SimplicityGuy/lytronaut/actions/runs/38535638197) | `8eb318943480d45a28d427ac1b680ae52bc0dcc8` | passed |

Native varied by two lines (8699 or 8697): the PostgreSQL desktop test runs two imports of one
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

Run locally on 2026-10-14 at `30800deb642d94c3bdcc7dc6956043829eb7202d` with
release native service and release Rust/WASM client, against the operator's
library opened read-only. Only aggregate numbers leave the machine; every
derived catalog, export, bundle and restored copy was deleted afterwards. The
host was a shared Apple M1 Pro (MacBookPro18,1) with a load average of about
13 to 17 from unrelated work, so the timings are contended upper bounds.

| Check | Result |
|---|---|
| Source files / bytes / unique contents | 728 / 2,001,830,074 / 638 |
| Directory import through the HTTP upload protocol | catalog ready in 51.6 s |
| Pictures, albums, album memberships | 109, 3, 109 |
| Previews, JPEG focus stacks, JPEG planes | 109, 58, 420 |
| Preserved files / bytes in the catalog | 728 / 2,001,830,074 |
| Restart (release; rehashes every sealed original) to healthy | 7.4 s; all counts identical |
| Flattened PNG export | succeeded |
| Live Photo pair export (3 stacks) | 3 succeeded, 0.5 to 0.6 s each |
| Portable bundle (any-size backup) | succeeded in 17.8 s, 2,001,887,237 bytes |
| Restore into SQLite, served from the restored copy | succeeded in 29.4 s; all counts identical; 728 of 728 originals downloaded byte-identical to the source |
| Restore into PostgreSQL 18.6 (disposable container), served from the restored schema | succeeded in 26.0 s; all counts identical; 728 of 728 originals byte-identical |
| Source library afterwards | 728 of 728 files unchanged (hash, size and modification time) |

## Not met or not verified

These stay visible; none is claimed:

- CPU focus p95 at most 100 ms on the release reference host: **not met**. Earlier
  post-export runs measured 104.7–150.7 ms ([export report](export-validation.md));
  the later merged-tree `b81e689` runs measured 137.9–150.4 ms
  ([owner results](../design/004-live-photo-export.md#owner-results-and-accepted-limitations-2026-10-13)).
  These historical stages do not measure the current delivery revision.
- Required-check enforcement on `main`: open; workflows run but do not yet
  block merging ([adoption procedure](../operations/required-checks.md)).
- Exact delivery-revision CI, coverage and container validation must be checked
  again after changes; historical results above do not attest future revisions.
- Performance under grid load: open.
- H.264 Level 5 frame-size check (1138 x 1842) and Live Photo export end to end
  on PostgreSQL: open.
- Counted 64/32 MiB Live Photo memory budget: partly proven (accepted
  limitation). GPU compositor submit-to-present at most 16.7 ms: not verified
  (accepted limitation).
- VoiceOver: not verified (owner sign-off, out of scope for this version).
  iOS: out of scope (owner sign-off).

## Review corrections (2026-10-16)

The coverage exception still names the same seven scopes (646 source lines).
The guard now pins each reviewed source range by SHA-256 instead of accepting a
body containing the word “gpu”; the unused `video.rs` allowance was removed.
Adversarial unit tests reject changed source, unapproved files, ambiguous or
missing functions/blocks and growth beyond the existing ceiling. New exceptions
still require explicit owner approval; changing a digest does not grant it.

Coverage-driver contract tests use mocked browser events and raw profiles, not
product execution: they verify startup/popup accounting, dumps before navigation
and close, malformed/incompatible profiles, missing instrumentation and page
enumeration failures. LLVM diagnostics are checked after per-script exports too.
The sidebar test keeps its real rendered-selection checks and removes the
fixture-self comparison; the two 300 ms sleeps now wait for observed UI/fault
state. The canvas fault fence requires all six failed thumbnail canvases to be
zeroed and all their source URLs released, with only the displayed stage URL
remaining. Actual browser and coverage workflows at the
resulting revision remain required.

The optional software-adapter run remains optional under the owner clarification;
it does not establish physical-GPU correctness or presentation latency. Access
documentation now follows ADR 002, CPU timings are attributed to their historical
stages, and required-check enforcement remains an explicit release prerequisite.
The earlier archive-admission documentation in PostgreSQL operations validation was superseded by
the any-size streaming bundle change: sizing now follows unique originals, not
the configured upload-file limit. The operations guide also distinguishes active
reservations from completed exports that still occupy disk. Its remaining
deployment-test follow-ups are still open under that task.

The first review-repair revision `3d86d2c` failed the browser workflow at
`canvas-cleanup`: its global network-idle wait timed out and did not establish
thumbnail settlement. That failure is retained as failed evidence. The next revision uses
the six-thumbnail cleanup fence above; its exact-revision workflow must pass
before claiming the correction validated. Local full-check submission attempts
were interrupted at the host disk safety floor, including a serial retry; those
attempts do not count as successful checks.

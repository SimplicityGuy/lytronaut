# CPU focus and grid-overlap acceptance — lytronaut-b0e

## Current state

This follow-up starts from main `17524c31b45252385082e33501f789fc0997c394`.
The warm CPU and actual grid-overlap release gates remain open. Earlier release
warm A/B/B/A results met 100 ms but did not establish a quiet host. Every earlier
grid setup produced zero valid overlap intervals. Neither those unavailable
setups nor successful CI constitute a pass for this bead.

No application changes or builds were made for this follow-up. The reviewed
protocol's one authorized current-release diagnostic stopped at its first
unavailable grid window: zero valid overlap intervals. Native and warm CPU
measurements met 100 ms under the operator's chosen current load, but the grid
gate remains open. No retry or A/B/B/A comparison was run.

## Source causality

- `browser.rs` schedules two thumbnail workers before selecting the page's first
  capture. Every browse JSON read and preview manifest reserves the full 64 MiB
  working budget. Any positive thumbnail lease prevents that metadata admission.
  Contenders poll every 30 ms rather than receiving a queued reservation.
- The selected focus stack is installed and the real slider becomes enabled
  immediately before the initial focus derivative completes. CPU focus decoding
  uses a 20 MiB working lease. A slider action at that point is supported by the
  actual product: it supersedes the earlier selection operation without clearing
  the validated focus stack.
- Therefore waiting for the initial focused image and `Discrete focus:` status
  can miss a legitimate earlier control-enabled interval. This does **not** prove
  a pending grid request exists in that interval. If thumbnail work has already
  drained before metadata admission, observing sooner cannot create overlap.

The earlier default-selection hypothesis was refuted by the observed first
capture `Synthetic 016`. Any name-sorted experiment must use the real name-sort
control, the matching API order and all 24 rendered capture labels. It cannot
silently substitute name order for the application's default order.

## Independently reviewed bounded observation

1. Keep the public, CPU-only release artifacts and fixed hashed fixture. Preserve
   the native and 24 raw warm samples first, including misses, as an explicitly
   incomplete CPU receipt.
2. On a fresh name-sorted application page, install passive DOM/image-load observation before
   loading. Record the first exact selected `Synthetic 000` state with an enabled
   focus control and a complete existing stage image. Do not require the initial
   focus derivative's final status; do not mutate application state to manufacture
   readiness.
3. Announce that state once to the test driver. Until then, all intercepted real
   traffic continues immediately. Afterward at most one naturally issued,
   distinct nonselected thumbnail chunk may be held. No synthetic requests,
   image reloads, per-request DOM round trips or admission changes are used.
4. Dispatch a real slider action. End its interval at the actual changed stage
   image's decode plus animation frame, independently of grid work. Require that
   the held real response body and that same thumbnail's image decode both
   complete inside this interval. Keep the existing 100 ms targets.
5. Record bounded readiness and eligibility observations. If no real candidate
   exists, emit an unavailable receipt preserving CPU measurements and stop the
   experiment. Do not label a warm interval or a held-only request as grid load.

One bounded current invocation of up to 24 samples comes before any A/B/B/A
comparison. It stops immediately at the first unavailable interval or guard
failure; there are no retries. A quiet-host
comparison would require recorded load below 2. The operator subsequently chose
“run it on here with the current load”; this permits a comparison under the
Mac’s actual recorded load, **not** a quiet-host claim. An identical reviewed
harness still applies on baseline `623f2b9` and the final application. Source sidecars alone do
not establish the compiler profile; release commands, exact executable/WASM
hashes and fixture identities must remain in the external evidence receipt.

A separate grid page could create genuine concurrent application load, but it
would be a different workload from single-page grid contention. It is not an
authorized substitute for this protocol without a scope decision.


## Source checkpoint

The helper uses a one-shot pre-load DOM observer and a one-way notification;
request interception performs no per-request DOM round trip. Readiness requires
the entire 24-name vector, selected focus identity, a usable minimum-endpoint
control and the same existing stage image/source both before and after its real
decode. A passive capturing load listener also catches image readiness that changes
without a DOM mutation. Disposal suppresses late decode callbacks and removes
observers/listeners.

Each measured action requests the maximum endpoint (plane 3 of the unchanged
public three-plane fixture). Completion requires the actual published output and
status metadata to match that plane/lambda and selected identity, with a changed
stage URL, image decode and animation frame. The default plane arriving late
cannot end this interval. Initial cancellation/bridge drain stays inside timing.

Readiness and network-release observations distinguish page `performance.now`
from driver epoch time; they are never subtracted across clock domains. Each page
retains a bounded 32-entry eligibility timeline, full event count, readiness
snapshot and explicit release-command/ack bounds. Failed partial intervals keep
a null accepted end time and a separate failure-observation timestamp.

The scope is an initial enabled-focus interaction that may supersede the first
focus derivative, separate from the preceding warm batch. No product application
code or memory budget was changed. Lightweight mocked-observer/guard tests have
passed. Independent source review cleared harness commit
`9d443714c0ba8ecfc279e0a7f25ed2972e19dc61`; its actual diagnostic is recorded below.
The fixed harness hashes the new readiness module alongside the driver, grid
helper and evidence writer.

## Actual current-host diagnostic

On 2026-10-18 at 20:21 UTC, one current invocation used the unchanged release
application `877116cbba57c0232a622ff59d4bc1ab326f8dbe` and reviewed harness
`9d443714c0ba8ecfc279e0a7f25ed2972e19dc61` (tree
`f552190f69eb52f23f7bac033848b45b97b8288e`). The frozen release executable,
raw/generated WASM and glue matched their recorded hashes. Both baseline and
current eight-file program manifests were reverified; the unchanged 74-file
fixture manifest SHA-256 was
`61640a2630e9f3098300de188308f6ab53df0bea2111f3deaf011da116baf75f`.
The direct Node driver used Chrome 155.0.8059.82 and the explicitly selected
preserved puppeteer-core 25.13.0 dependency; no debug wrapper or build ran.

| Measurement | Actual result | Interpretation |
| --- | --- | --- |
| Native HTTP/decode p95 | 3.3 ms | Below 100 ms |
| Warm CPU samples | 24, retained in acquisition order | Separate warm workload |
| Warm CPU p50 / p95 / max | 66.4 / 75.3 / 77.6 ms | Below 100 ms |
| Actual grid-overlap intervals | 0 | Unavailable; acceptance unproven |
| Held grid chunks | 0 | No manufactured contention |

Passive readiness announced at page time 422.3 ms after 43 checks, with the exact
ordered labels, selected identity, enabled control and decoded existing stage.
However, `pendingGrid` was already zero and `stageKind` was `focused`. All 24
naturally issued grid-chunk requests occurred before that announcement, while
interception continued them immediately. The 25-event eligibility timeline
therefore refutes this particular earlier-observation setup as a source of real
overlap on this fixture. It does not prove that every possible application
workload lacks overlap. No slider interval was dispatched after this failure,
and no repeated attempt or baseline run followed it.

The load averages before the run were 3.99 / 4.52 / 4.90 and afterward
4.50 / 4.62 / 4.93. These are the operator-chosen current-host conditions;
quiet-host acceptance is not established. The CPU receipt explicitly retains
`complete=false`, `passed=false` and `gridAcceptanceVerified=false`, despite
`cpuThresholdsMet=true`. The unavailable receipt binds that CPU receipt's hash.

External evidence is retained under
`delivery-evidence/b0e-current-passive-9d44371-host/`: the immutable runner,
exact command/source/tree/artifact/fixture bindings, raw CPU samples, unavailable
receipt and timeline, driver failure log, before/after resource snapshots and
owned-process drain receipt. Its `SHA256SUMS.json` covers the evidence files.
The owned process group was verified empty before releasing the exclusive
runtime lease. A separate preserved prelaunch directory records the initial
sandbox denial of the host `top` snapshot; no browser or service started in that
prelaunch attempt. Host execution then completed the single authorized actual
invocation. Canonical validation and any further experiment remain pending;
these diagnostic artifacts do not satisfy the grid acceptance criterion.

## Admission investigation (proposal only)

The relevant scheduler is identical in baseline `623f2b9` and current `877116c`.
Competitive 30 ms admission polling dates to `249dfde`; full-budget metadata
admission and the six retained browse lanes were added by `a31f72b`, already an
ancestor of the baseline. This is a pre-existing responsiveness defect, not a
new regression demonstrated between those two benchmark revisions.

`Net::lease` tests current capacity without remembering older waiters. A
thumbnail worker can release its lease and immediately acquire the next
capture's full-budget metadata lease while the selected-capture worker remains
asleep. `page` submits both thumbnail workers before Selected. The actual
25-event timeline is consistent with this mechanism, but does not contain an
independent selected-lease enqueue timestamp. Its causal explanation is source
analysis, separate from the observed zero-overlap result.

The proposed product fix is FIFO admission with release-driven wakeups. Preserve
the 64 MiB ceiling, two active operations, every existing byte charge, all six
browse/four action lifetime owners, and bridge-drain ownership. Do not change
budgets, add delays, prioritize test paths or cancel unrelated thumbnails.
Every request joins the same bounded ticket queue; a newer thumbnail cannot
bypass a waiting selection. A full-budget request at the head waits for existing
leases to drain, then runs before newer work. Two compatible requests may still
run concurrently. This can improve selected-view progress without promising any
particular latency or guaranteeing that the current overlap experiment passes.

The waiter must own its ticket and cancellation listener through RAII. Grant,
cancellation, error and drop remove that ownership exactly once. Cancellation
removes only an ungranted ticket; active leases continue to cover actual work
until its existing drain finishes. Release/cancel returns pending wakers to be
invoked after dropping shared-state borrows, preserving reentrant cancellation
safety. A cancelled head must wake its successor even when no active lease is
left to trigger another release. Oversized requests fail immediately rather
than permanently blocking the head. No application implementation has been
authorized at this checkpoint; this proposal awaits root and independent review.

Meaningful validation should model the fast-thumbnail/selected-waiter schedule
without sleeps, show selection progresses despite continuous newer arrivals,
and reject out-of-order acquisition. It must cover full-budget head blocking,
two-slot accounting, oversized refusal, head/middle waiter cancellation,
drop-before-grant, cancellation racing grant, release-after-drain and reentrant
wakeup without a held borrow. Existing remount/reentrant-abort and browser
resource checks must remain intact. Actual release responsiveness and any grid
overlap still require new reviewed, source-bound runtime evidence after builds;
unit policy tests alone cannot establish either performance claim.

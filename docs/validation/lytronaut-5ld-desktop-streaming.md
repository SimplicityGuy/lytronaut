# Desktop catalog streaming (lytronaut-5ld)

Status: implementation in progress; executable backend/canonical validation has
not run. This post-release follow-up does not change the owner's initial-release
disposition or turn catalog size into a new release gate.

## Baseline and boundaries

The clean starting tree is main `069dec523271907e50467126c10f52a3a7ebfc1e`.
`bh work claim` refreshed the previously empty assigned leaf to that baseline.
The old adapter gathers every known table into a `Records` map, then creates
additional identity sets, membership sets and a missing-companion error list.
It refuses more than 100,000 cells/rows or 8 MiB of aggregate decoded values.
Replacing only `fetch_all` therefore would not bound the total live data.

No Cargo test or build was allowed during this initial source work: the host had
about 2.7 GiB free and the previous validator reached its 2 GiB safety floor.
This is an explicit environment limitation, not passing validation. Lightweight
Rust formatting and synthetic-fixture SQL inspection are permitted.

The intended boundary is bounded source-row decoding, disk-backed semantic
identity/reference validation, and one atomic destination publication transaction.
Opaque typed values and input row ordinal stay byte-exact; duplicate detection,
reference integrity, membership order, quarantine/reimport semantics, private
source protection and cancellation rollback stay intact. Source files remain
read-only; private temporary copies are independently verified before publication.
The 32 MiB source-file limit, 1 MiB cell limit and schema/parser safety bounds
remain resource limits. There is no promise of unlimited source size. The separate
288 MiB portable catalog section bound is unchanged.

Strict validation is appropriate for the publication boundary: focused positive
and adversarial tests, actual SQLite and PostgreSQL parity, fresh peer review,
then the mandatory clean-checkout `mise exec -- just check` gate. Builds remain
serialized under the dispatcher's exclusive build budget.

## Synthetic fixture evidence

The independently authored fixture contains 2,048 pictures with 64 picture
columns: 131,072 picture cells before metadata or other tables are counted. It
therefore exceeds the old 100,000-cell admission ceiling. Host Python SQLite
inspection generated a 303,104-byte database, below the existing test fixture's
2 MiB file budget, and verified the final opaque field as bytes `00ff`.
This is fixture-shape evidence, not an application or backend test pass.

The shared test assertion is called by the normal SQLite case and the existing
explicit actual-PostgreSQL integration case. It checks normalized counts,
reverse membership order, a late original record, opaque typed fields, original
source byte preservation, and idempotent reimport. Adversarial late rows cover
duplicate typed IDs, orphan references, duplicate membership index and an
oversized value; they must retain originals without partial semantic records.

## Outstanding evidence

- Implementation and fresh source review.
- Fail-first application regression result against the baseline.
- Actual SQLite and PostgreSQL positive/adversarial cases.
- Cancellation/drain and source-mutation refusal evidence.
- Bounded memory/disk accounting evidence.
- Canonical submission validation and remote checks at the final source tree.

## Reservation/ownership investigation

Production PostgreSQL connections set `idle_in_transaction_session_timeout=10s`,
`lock_timeout=5s` and `statement_timeout=10s` (`database.rs`). Holding the global
writer transaction throughout source copying and semantic validation is therefore
unsafe: source inspection itself permits ten seconds, and other adapters/jobs
would block on that transaction's advisory key. Production lifecycle edits remain
paused while the reservation surface is resolved; no long-held transaction has
been introduced.

Current intake admission (`Store::begin`) counts running `intake_jobs.reserved`
and all `export_jobs.reserved` under `Catalog::begin_write`; export admission and
restore re-admission count those same running intake reservations. Completion
sets the intake reservation to zero. Running export cleanup releases its durable
reservation only after filesystem reconciliation. Reusing either reservation
without extending its ownership/drain semantics would be incorrect.

`Store::open` holds exclusive locks on both original and staging roots for the
Store lifetime, and recovery removes interrupted flat staging files before
normal admission. That is useful evidence for Store-owned scratch ownership,
but it does not by itself make a new scratch allocation visible to every quota
reader. Any reservation must cover source-copy bytes plus the scratch database's
enforced page cap, remain charged until every SQLite worker/file is closed,
and coexist with running-job reservations without counting the same reservation
twice. A new durable schema is not yet justified by this investigation.

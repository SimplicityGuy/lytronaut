# Desktop catalog streaming (lytronaut-5ld)

Status: implementation and independent source review complete; executable
backend/canonical validation has not run yet. No runtime pass is claimed below.
This authorized post-release follow-up does not change the owner's original
initial-release disposition or introduce a new release gate.

## Baseline and change

The clean starting tree was main
`069dec523271907e50467126c10f52a3a7ebfc1e`; the assigned leaf was refreshed through
Beadhive. Regression checkpoint `56389c3` precedes the production change. The leaf
then preserved-merge synced landed restore/operations work through main
`c9090ffce609`. Initially no Cargo work was allowed at about 2.7 GiB free. Cleanup
restored headroom, but the dispatcher still serializes builds; this bead is queued
behind acceptance validation. Lightweight formatting/diff and fixture inspection
are the only checks executed so far.

The old adapter retained all `Records`, identity/membership sets and missing-file
errors. It rejected more than 100,000 rows/cells or 8 MiB of aggregate values.
The new adapter buffers one source row, builds disk-backed exact typed-ID/UUID
and reference indexes, validates them, then streams a second pass directly into
one atomic destination publication transaction. Errors stream without an
all-catalog list. Source row ordinal, opaque typed values, UTF-8 validation for
semantic fields, duplicate detection, membership ordering, version identity and
quarantine/reimport behavior are preserved.

The same independently copied, SHA-256-verified private 0400 source snapshot is
held for both passes under Store's exclusive root ownership. The original sealed
blob is never opened writable. Existing limits remain: 32 MiB source file,
1 MiB value, 128 columns and bounded schema/VM/parser work. The former 8 MiB
whole-catalog bound becomes a per-row bound, so no formerly admitted individual
row is narrowed. The separate 288 MiB portable logical catalog bound is unchanged.
There is no promise of unlimited source size or unlimited disk/allocator use.

## Ownership and reservation decision

Holding PostgreSQL writer admission across copy/hash/validation was rejected:
production idle-in-transaction timeout is ten seconds and competing admission
would stall. Reusing succeeded `intake_jobs.reserved` was also rejected: existing
columns cannot identify which root owns a crash-window reservation.

The additive `intake_scratch(owner,root,job,bytes)` table records independent
transient ownership. Source-copy plus enforced index capacity is reserved in a
short writer transaction before files exist; all intake/export/restore admission
sums include it. The index page cap is `4 MiB + 16 * source_size`, rounded to
SQLite pages. Both files' physical costs use actual staging allocation units plus
1 KiB metadata per file. Retained export artifacts remain separate; scratch is
counted once per admission sum and retained in full through cleanup, conservatively
including already materialized scratch bytes.

A task-owned worker receives caller-drop/Store-shutdown cancellation. Its registered
lifetime includes manifest hashing, source work, publication rollback and cleanup.
SQLx workers close before files are unlinked/fsynced and reservation released.
Ordinary unwind is caught so the same cleanup runs. Cleanup errors retain quota.
Root-exclusive startup drains staging before releasing only its own reservations;
a held directory FD anchors device/inode identity and replaced paths fail closed.

Portable v2's fixed table list omits runtime scratch ownership. The wire version,
table ordering and section bound are unchanged. Exact older physical schemas
without the new table remain accepted; newer physical schemas require it empty.

## Source review and lightweight evidence

An independent developer reviewed production ownership, quota, recovery, source
parity, PostgreSQL transaction lifetimes and backup compatibility. Two findings
were fixed before review clearance:

- Use actual filesystem allocation units and per-file metadata, rather than
  assuming a 4096-byte allocation unit for quota.
- Capture the locked directory identity at Store open, hold its FD and verify the
  pathname before admission/release/recovery; re-statting a replaceable path
  alone could adopt another directory's identity.

The reviewer rechecked both fixes and reported no remaining source blockers.
Runtime verification remains required. `cargo fmt --check` and `git diff --check`
pass. Host Python SQLite fixture inspection generated 2,048 pictures ×64 columns
(131,072 picture cells), a 303,104-byte database, and verified the final opaque
field `00ff`. This proves fixture shape, not application behavior.

## Authored regressions awaiting execution

- SQLite and actual PostgreSQL wide catalogs above the old capture/cell bound:
  exact typed opaque fields, normalized counts, reverse membership order,
  original byte preservation and idempotent reimport.
- More than 8 MiB aggregate typed values with individually bounded rows.
- Late duplicate ID, orphan reference, membership index collision and oversized
  value: preservation-only without partial semantic records.
- A same-length changed source copy is rejected by its digest.
- Caller cancellation retains quota until SQLite drain/unlink; Store shutdown
  cannot pass a paused task-owned worker; cancelled work publishes no version.
  Injected unlink failure retains quota and blocks restart until the owned path
  is reconciled; recovery then drains it and releases only its reservation.
- SQLite/PostgreSQL shared quotas, parent FK retention, foreign-root isolation,
  own-root restart and directory replacement refusal.
- Export/restore admission and extraction re-admission include scratch quota.
- Portable v2 round trips omit live scratch ownership, and legacy order/auth
  schema forms restore with and without the new empty table on both engines.
- 8 MiB filesystem allocation units and checked quota-arithmetic boundaries.

## Remaining delivery gates

Focused SQLite/PostgreSQL execution, fail-first application evidence where feasible,
canonical clean-checkout `mise exec -- just check`, remote CI/coverage/containers,
and the normal human review gate remain outstanding. No gate was approved and no
branch was submitted or merged by this developer.

# Desktop catalog streaming validation

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

Status: implementation and independent source review complete. Focused SQLite
contracts and actual PostgreSQL parity passed; canonical submission and final
remote checks remain pending. One Linux-only fault-fixture defect was corrected
after the first remote run; its corrected-source checks are still required.
This authorized post-release follow-up does not change the owner's original
initial-release disposition or introduce a new release gate.

## Baseline and change

The clean starting tree was main
`069dec523271907e50467126c10f52a3a7ebfc1e`; the assigned leaf was refreshed through
Beadhive. Regression checkpoint `56389c3` precedes the production change. The leaf
then preserved-merge synced landed restore/operations work through main
`c9090ffce609`. Initially no Cargo work was allowed at about 2.7 GiB free. Cleanup
restored headroom. After acceptance released its exclusive build lease, this
task ran serially with an owned 2 GiB free-space watchdog. The runtime results
below supersede that earlier source-only checkpoint.

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
Canonical and final remote verification remain required. `cargo fmt --check`
and `git diff --check` pass. Host Python SQLite fixture inspection generated 2,048 pictures ×64 columns
(131,072 picture cells), a 303,104-byte database, and verified the final opaque
field `00ff`. This proves fixture shape, not application behavior.

## Regression coverage

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

## Executed evidence and corrected failures

- Regression checkpoint `56389c3` failed the new wide-catalog assertion exactly
  as intended: old adapter returned `PreservedOnly`, expected `Ready`.
- The first shared-target final-source invocation reused the old nine-test binary
  without recompiling. That invocation is invalid final-source evidence. Cleaning
  affected workspace packages forced real recompilation; it found a cancellation
  predicate shadowing error, corrected in `7d05915`.
- A historical 30,000-picture-list assertion expected the removed cell ceiling to
  reject input. `0532da4` changes that one expected behavior to `Ready` and exact
  30,000 count; all schema, CHECK-expression and per-value defenses remain tested.
- At that corrected Rust source, Desktop integration passed nine tests with one
  explicit PostgreSQL ignore; six private Desktop tests passed, including caller
  cancellation/shutdown and cleanup-error quota retention/recovery. SQLite scratch
  ownership/quota, export/restore quota, eight legacy physical schema forms and
  portable live-ownership exclusion each passed their focused case.
- Full pinned task-owned PostgreSQL parity at `b64c23b` passed **64 tests, zero
  failed and zero ignored**. This includes the wide 2,048-capture test, scratch
  ownership/restart/root replacement, export/restore quota, seven portable matrix
  cases (423.81 seconds) and 24 restore fault cases (335.79 seconds). The exact
  task-owned container and volume were removed after the run.
- Initial remote CI `39016005899` failed the cleanup-error test on Linux at its
  attempt to restore a `.held` file inside staging. Startup recovery can legally
  delete that regular file before encountering the injected directory; APFS had
  visited the directory first. Quota-retention/startup-refusal assertions already
  passed. The corrected fixture unlinks the paused, cancelled worker's index and
  injects a directory at that exact path, then removes only that directory before
  retrying recovery. It has no second staging file or readdir-order assumption.
  It still checks SQLite close/drain, retained quota, denied restart, later clean
  recovery, zero partial versions and preserved original bytes.

Logs, original failures and source provenance are retained in the private
`/private/tmp/desktop-streaming validation-evidence` directory. The remote workflows for `b64c23b`
are CI `39016005899`, coverage `39016015112` and containers `39016018801`; their
results do not substitute for fresh checks of the corrected fixture source.

## Remaining delivery gates

Re-run the corrected cleanup fixture, then canonical clean-checkout
`mise exec -- just check` and fresh remote CI/coverage/containers. The shared
workspace package outputs must be cleaned before the pristine checkout is bound:
mtime-based cross-checkout freshness was demonstrably insufficient. No human
gate was approved and no branch was submitted or merged by this developer.

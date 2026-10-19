# Restore reconciliation validation

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

This follow-up extends the delivered main baseline, not the closed implementation
delivery. All fixtures are independent synthetic data; no source library is changed.

## Changes and evidence

- Startup retries `unverified-result` ownership/publication/schema verification.
  A successful retry restores readiness; unsuccessful verification retains files
  and accounting. This does not rehash every restored file.
- A recorded cancellation or journaled cleanup intent takes precedence over
  readiness. Actual DELETE from failed/unverified state is tested at cleanup
  intent, rename and removal cuts, with a backend trigger preventing the fallback
  status write, so restart sees the original unverified row and must resume cleanup.
- Cleanup journals verified directory device/inode before renaming to a private
  tombstone and syncing its parent. Partial deletion can lose the inner marker;
  recovery requires the recorded directory identity. Foreign replacements,
  symlinks and unjournaled tombstones fail closed, preserving files and quota.
  Identity changes after moving a partially cleaned store to another filesystem
  require operator resolution.
- Unverified status and missing accounting are updated atomically. A real trigger
  on each backend rejects any intermediate unverified row with zero accounting.
- One failed monitor state read is retried at the next poll; two consecutive
  failures stop active work. The test pauses a real HTTP-created export at a native
  render boundary, injects one failure, observes the successful next read without
  cancellation, releases the worker and requires successful completion.
- Recovery logs describe what is known without asserting a recreated row was once
  ready or that a never-published result failed content verification.

The non-restore startup verification policy is explicitly unchanged with rationale
in [design 007](../design/007-portable-backup.md): status records job completion,
download checks ownership/openability, and neither is a universal content-integrity
attestation. No full startup archive scan or new background verification API is
claimed.

## Executed stages

At clean source `47b4cb0` (reviewed implementation plus normal main synchronization):

| Command scope | Actual result |
|---|---|
| SQLite restore-fault matrix | 22 passed, 0 failed, 0 ignored |
| SQLite active monitor retry | 1 passed, 0 failed, 0 ignored |
| Strict PostgreSQL 18.6 parity | 62 passed, 0 failed, 0 ignored; restore group 24, control group 3 |

The strict server is the pinned task-owned disposable container with a restricted
application role and public sentinel. Tests preserve unrelated SQL/filesystem
sentinels and the live synthetic library.

Six later mutation executions (three guards × two engines) failed on their expected
assertions: atomic accounting, active monitor retry and cleanup-intent precedence.
Correct source was restored in `finally`; mutant patches, programs and logs were
frozen separately. Initial failing-first readiness tests on SQLite/PostgreSQL and
the SQLite cleanup-cut test also failed against the original implementation.

An independent exact-main CI run exposed an older free-space test race:
`sqlite_denied_cleanup` sampled free space then requested sampled value + 1, while
admission re-read live free space after an await. Parallel cleanup could legitimately
make that request fit. The forward test correction requests the fixed representable
`i64::MAX` amount, keeps `required_bytes == need` to prove retained bytes are not
counted twice, and checks the actual reported available amount is below need.
The corrected case passed separately on SQLite and PostgreSQL. No production
admission rule or assertion about retained accounting was relaxed.

Canonical submission and exact-final-source CI are separate lifecycle stages;
these historical results do not attest an unseen future SHA. Durable logs,
source/tree metadata, executed programs, patches and checksum manifests are retained
outside disposable worktrees in retained delivery-evidence directories for the
working run, source review, focused check, main-test failure, and mutation checks. The later canonical/CI receipt must name its actual source.

# Backup/restore disk accounting validation

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

Evidence snapshot authored before canonical submission: implementation and
focused SQLite results are recorded below. Later canonical, exact-source CI,
coverage/container and PostgreSQL results belong to Beadhive and the preserved
source-keyed delivery evidence; they are not inferred from this snapshot.
No approval or merge is implied.

Base: approved delivery `64ee928bd302546f5a5d19b4252e24f145472d9e`, after
“Strengthen Live Photo API tests and fix Flatten export durability gap”
(`export durability validation`) closed. No private libraries or captures are used.

## Findings and dispositions

| Finding | Change or disposition |
| --- | --- |
| M1: allocation-only file cost omitted inode/directory metadata | `file_cost` adds a separate 1 KiB/file allowance. Restore estimate and writer share `restore_base_need`; archive estimate and writer use the same rounded file/growth costs and auxiliary-file allowance. This is a model, not an exact physical usage promise. |
| L1: `shortfall` scope omitted import admission | Architecture contracts/API now include `POST /imports` alongside archives/restores. |
| L2: concurrent restores could spend the same inode count | Existing code already holds the one validation permit across re-admission and extraction because restore workers use `ArchiveOriginals`. Keep that protection, make restore eligibility explicit, and prove it through two actual HTTP jobs on each engine. No second lock or migration is added. Other filesystem writers remain outside this advisory inode check. |
| L3: capture spools charged after capture | Retained as an explicit bounded limitation. Catalog and inventory are format-bounded and checked after capture; transient reservation overshoot is possible, with owned spool cleanup before any artifact is created. Docs no longer claim per-write spool enforcement. |
| L4: fixed 3 MiB archive rounding assumed small allocation units | Archive sizing receives the actual work filesystem allocation unit and rounds every output/spool file. Growth charges block deltas without charging metadata again. An 8 MiB synthetic allocation-unit regression covers the former assumption. |
| `StorageError::Inodes` unused in production | Finite-inode refusal now constructs that error and follows the normal retryable job-failure mapping. Auxiliary entries and the 1,024-inode floor are included. |
| Catalog arithmetic fixture exceeded `MAX_BYTES` | Per-capture catalog assumption changes from 8,937 to 8,934 bytes; the fixture asserts that its catalog fits the format section. Payload shape stays fixed at 219,700 files / 829,164,700,000 bytes. Block rounding remains under 0.1%; adding required metadata gives 0.100002038% total, so the combined statement becomes under 0.11%. |
| Post-cleanup emptiness did not prove no extraction happened | Test-only probe marks entry into `restore_owned` before journal preparation. The existing two-engine tiny-file readmission test asserts it was never entered, independently of subsequent cleanup. |
| Upload frame claim lacked actual hyper proof | Two-engine actual TCP/HTTP/1.1 tests stream a valid bundle with a 2 MiB original, measure frames at the real restore handler, and require more than one frame and exact uploaded byte count. An un-sized chunked request is rejected before creating a job. A later canonical run observed a 679,936-byte frame, disproving the initial 417,792-byte cap assertion: pinned hyper 1.12.0 reserves its nominal `8192 + 4096*100` read target but exposes the underlying `BytesMut` spare capacity to the socket. The corrected test records the largest actual frame without imposing that false cap. This is a bounded documentation/test disposition: no HTTP/2, hard frame-size or total RSS guarantee is inferred, and production still writes each frame before polling the next. |

The new PostgreSQL group is explicitly counted by `scripts/postgres-test.py`:
`disk-accounting` requires two actual passes, zero ignored/failed cases. Existing
Live Photo, storage, import-idle, portable matrix and restore-fault groups remain
required. Source/test changes do not modify the approved GPU coverage allowances.

## Focused evidence

Before implementation, three regressions failed against the old accounting:
empty/one-block files cost 4,096 instead of 5,120 bytes; the writer admitted a
payload-only block; and archive sizing ignored an 8 MiB allocation unit.
After implementation the five accounting tests passed, as did both new SQLite
HTTP scenarios. Expanded testing caught the old combined-under-0.1% claim:
the required metadata makes it 0.100002038% without changing payload shape.
The test retains under 0.1% for block rounding alone and checks exact metadata
bytes plus under 0.11% combined. The subsequent five focused groups passed
6 + 2 + 1 + 1 + 1 tests, including existing per-file estimate/writer enforcement
and the 200,000-tiny-file refusal before extraction. Both the failing expanded
attempt and passing attempt remain local under `/private/tmp/disk-accounting validation-*`.

Mutation checks deliberately remove existing protections, then restore the exact
authored source. Bypassing the validation permit fails the retained-permit
assertion after both jobs are drained; bypassing the inode refusal must allow an incorrect second restore
and fail the expected refusal assertion. Final source is checked again after
restoration. These mutations are not submitted code.

The concurrency proof pauses the first job after re-admission, waits for the
second worker's explicit permit-attempt notification, and reads the isolated
API's held-permit state. It does not depend on a timed negative observation.
Both jobs drain and clean up before assertions are evaluated.

At this snapshot, PostgreSQL, coverage, container and final canonical execution
had not yet been recorded. Later lifecycle results must identify the tested
source and run explicitly.

# Portable backup and isolated restore

> Historical source- and date-bound record. See the [delivered application baseline](../validation/final-main-status.md) for current status.

Status: implemented; engine-verified on SQLite and PostgreSQL 18 (see Verification).

The archive carries logical application rows and immutable original blobs, never
database credentials, generated PostgreSQL display/search projections, authentication
state, or transient export jobs. Imported Desktop database files remain opaque original
blobs. Their unknown typed values, real-number bit patterns, integer text, and original
record bytes are not interpreted or rewritten by this archive format.

## Version 2

The bundle retains a 16-byte magic, a bounded catalog section length and SHA-256,
and a unique original-blob count. The catalog section has a versioned binary row
stream. Table identifiers and ordered column types are application constants, not
archive-supplied SQL or names. Cells are tagged null, signed little-endian 64-bit
integer, UTF-8 text bytes, or opaque bytes. Text is length-prefixed and may contain
NUL. Integers are never converted through JSON numbers or floating point. Table,
row, cell, and total byte limits are checked before allocation. Unknown tables,
wrong cell types, duplicate rows, broken references, invalid UTF-8, truncation,
trailing bytes, missing blobs, and digest/length conflicts reject the archive.

Logical source identities are decoded from PostgreSQL's binary-safe hex keys on
export and encoded only for the destination backend on import. Names and captions
come from original text bytes; PostgreSQL display, fold, and byte-count columns are
regenerated using the same application projection rules. Catalog IDs, imported
record ordinals, album indices and all source values preserve the full signed64
range. `application_job_order` is an auxiliary ordering representation: the archive
preserves logical job order, and bounded transactional compaction may regenerate
its ordinals at exhaustion. It does not claim byte-exact auxiliary ordinal values.

SQLite uses a consistent read transaction; PostgreSQL uses REPEATABLE READ READ
ONLY. All catalog tables and the original inventory are captured in that one bounded
transaction. Capture streams fixed table rows to an owned logical spool with a bounded wire budget; it does not retain all SQLx rows plus a second encoded buffer. The version-1 32 MiB limit is its SQLite FILE limit, not a new logical payload limit. Legacy extraction uses bounded read-only row cursors and must not acquire narrower cell/row acceptance limits. The untrusted-file validator keeps its VM-step and wall-clock budget; extraction runs only after that validation on a separate read-only cursor connection without the validator's deadline, because that deadline also counted time spent awaiting destination inserts and rejected a validated near-32 MiB many-row file. Extraction remains cancellable per row. A conservative expansion bound must account for rowid aliases, UTF-16 conversion and defaulted trailing columns and be proved before imposing a conversion spool cap. Near-limit legacy acceptance is a required independent fixture gate. The transaction
ends before hashing or streaming original files, avoiding long idle transactions.
Successful intake jobs and durable adaptation-pending state are included. Original
content-addressed files are immutable, so streaming that captured inventory cannot
mix later catalog changes into the snapshot.

The existing version-1 SQLite bundles remain supported in precisely four known
schema forms: current or pre-job-order, each with or without the three known legacy
authentication tables. Validation opens inbound SQLite read-only, query-only,
trusted-schema-off, with bounded VM work and memory. It checks exact trusted schema,
integrity, references, source inventory and successful intake state before extracting
logical rows. Restore creates tables using trusted application DDL and inserts bound
values; it never executes archive schema or preserves legacy credentials.

The new v2 codec currently specifies 288 MiB wire bytes and a 32 MiB plus 256-byte
row allowance. It streams a row without retaining the whole catalog or allocating
from row counts. These are new-format bounds, not proof that all legacy files expand
within them. Legacy conversion cannot use these limits to reject previously accepted
SQLite archives. The proposed expansion proof must account for INTEGER PRIMARY KEY
rowid aliases, UTF-16-to-UTF-8 expansion, and omitted defaulted trailing columns.
The adapter's working-set accounting must include the SQLx driver row, decoded row,
source-key encoding, regenerated projections and buffer capacities. Neither a wire
limit nor the draft codec alone proves the final native memory bound.

## Native working-set and storage accounting

Portable SQLite capture and legacy extraction use direct typed SQLx connections
with `row_buffer_size(1)`. The Any URL roundtrip discards this setting and is not
used for these readers. Each fresh capture connection sets query-only ON,
trusted-schema OFF, cache-size -2048, mmap-size 0 and temp-store MEMORY before
BEGIN. File snapshots are read-only/no-create; named shared-memory synthetic
fixtures retain their existing connection mode. The read transaction covers all
logical tables and inventory.

A stream does not mean a single driver row: SQLite can hold the application row,
one queued row and a worker-produced row simultaneously. The application also
owns logical cells during conversion; legacy import additionally owns copied
parameter values and any regenerated source-key/projection buffers while the
source driver row remains alive. Allocator capacity, SQLite sorting/temp storage,
SQLx statement buffers and database-driver allocations are additional costs.
Version-2's maximum row payload is 32 MiB + 256 bytes; legacy's per-row limit is
its admitted 32 MiB database file, without new codec cell/count restrictions.
A conservative subtotal must count all three SQLite rows plus conversion/bind
copies, not merely the wire buffer. SQLx PostgreSQL 0.9 executor yields each DataRow
before receiving the next (connection/executor.rs); its connection retains framed
receive-buffer capacity, and Any conversion plus owned logical cells may coexist.
This source audit is not a measured RSS or a proved whole-process bound.

Export admission permits at most two pending jobs. A shared validation permit
serializes portable archive/restore cursors within each API instance; the second
job can still upload or flatten. Deployment memory sizing must account for every
admitted worker and for multiple application instances, the active application
pool, intake/adaptation work and database/container overhead. No smaller implicit
legacy cell/row cap is introduced to simplify that accounting. Near-limit fixtures
and measured concurrent memory are still required validation gates.

Staging reservations are deliberately fixed upper bounds, not measured sizes; see
[Bundle size and admission](#bundle-size-and-admission) for what they are and why
no library-size ceiling applies. SQL database/index storage is deployment
storage too; these application reservations are not a physical bound on PostgreSQL
WAL/index growth. Cleanup retains reservations until every owned filesystem and
SQL namespace removal succeeds. Failed uploads use that same cleanup path rather
than releasing quota after a best-effort unlink.

## Bundle size and admission

Decision (`portable backup decision`, owner, 2026-10-13): a lossless backup and its restore
work for a library of any size. There is no documented library-size limit; the
only refusal is that the work volume genuinely lacks the space the job needs.
The single-upload budgets (`LYTRONAUT_UPLOAD_BYTES`, `LYTRONAUT_ENCODED_BYTES`)
protect one browser import and no longer bound a bundle; the aggregate import and
per-asset export budget (`LYTRONAUT_RESERVED_BYTES`) no longer bounds a whole-library
archive or restore either.

**One stream file, not a ZIP.** The bundle stays a single `.lytronaut` file in the
framed format above: a 60-byte header, the catalog section, then for each unique
original a 40-byte (SHA-256, little-endian u64 length) record and its bytes. Every
length is a u64 and nothing in the format stores a 32-bit size or offset, so there
is no 4 GiB boundary and no ZIP64 question; the format is not meant to be opened
with `unzip`, Archive Utility or any other tool. The only ZIP writer (the Live
Photo pair) is capped well below 4 GiB, and the intake ZIP reader (which accepts
ZIP64) reads browser imports, legitimately bounded by the single-upload budget.
Creation, download and restore each move bytes through one fixed 64 KiB buffer per
transfer (`copy_checked`, the download body), hashing as they go; no step holds an
original or bundle in an application copy buffer. A restore upload is written
one HTTP body frame at a time to its owned file and never accumulated; the frame
is written before the next is read. Transport frames are not the 64 KiB copy
buffer: hyper's nominal HTTP/1.1 read strategy target is 417,792 bytes (408 KiB),
but its underlying buffer's spare capacity can produce larger frames. The real
HTTP test verifies streaming and exact bytes, not a hard frame-size or total RSS
ceiling. Resuming an interrupted upload or download is not supported; the
transfer is restarted.

**Modeled disk needs.** Payload length is not physical disk usage. Let `A` be
`f_frsize` from the work filesystem (`f_bsize` when unavailable, at least 512
bytes), and define `F(x) = max(1, ceil(x/A)) * A + 1024`. Each file pays whole
data allocation units plus a separate 1 KiB allowance for inode/directory
metadata. Sixteen auxiliary job files/directories reserve `J = 16 * F(0)`.
These are conservative sizing allowances, not an exact filesystem bill or a
proof of every filesystem's metadata behavior. SQLite destination headroom is
still one v2 catalog maximum `C` = 288 MiB, not a proved database-growth bound;
PostgreSQL destination database storage is separate from the work volume.

For `O` bytes of unique originals and `n` originals, archive admission reserves
`J + F(C) + F(40n) + F(60 + C + 40n + O)`. The bounded catalog and inventory
records are counted twice, and original bytes once. The actual filesystem's
allocation unit is used without assuming it is at most 1 MiB. Artifact writes
charge only newly occupied blocks, with metadata charged once per file.

Capture writes the catalog and inventory spools **before** checking their
modeled cost against the reservation. A library that grew after admission can
therefore transiently exceed that reservation during capture. The logical spool
is bounded by `MAX_BYTES`; inventory records come from the same captured source
rows and are bounded by `MAX_ROWS * 40` before tighter catalog/cell bounds apply.
The spool check refuses with retryable `resource_limit` (job error `space`) and
removes both spools before creating an artifact. After that check, each artifact
growth is charged before writing; failure removes owned output and releases the
reservation. Per-write enforcement during capture is not claimed.

A restore upload initially reserves `2D + C` from its declared size `D`. Before
extraction, the worker reads uploaded framing without hashing/extracting and
re-admits `F(D) + C + J + F(catalog) + F(40n) + sum(F(original_length))`.
If larger, its reservation grows under writer admission, counting other jobs,
running imports and `LYTRONAUT_FREE_FLOOR`; otherwise it fails retryably as
`space` before journal preparation or extraction. Every extracted file is
charged against the same metadata-inclusive model before writing. The normal
library-shaped arithmetic fixture (about 33,800 captures, 6.5 files each) stays
at 0.100002% combined data-block/metadata overhead (under 0.11%); many tiny
files cost materially more. Its catalog fixture is within the actual v2 section bound.

The filesystem's finite free-inode count, when reported, must cover `n + 16 +
1024`. The existing single validation permit is held from inode re-admission
through extraction: two restore uploads may be pending, but their extraction
and inode checks are serialized. It does not hold a live database writer lock
through the restore. This is an advisory check against unrelated filesystem
writers, not a kernel inode reservation; later I/O failures retain the existing
owned cleanup behavior. Insufficient inodes use the real `StorageError::Inodes`
path and refuse before extraction.

**Admission.** An archive or restore is admitted when the work volume's available
space is at least its reservation plus every active export reservation plus every
running import's reservation plus `LYTRONAUT_FREE_FLOOR`. Import admission in turn
counts every export and restore reservation, under the same writer admission, so
neither can pass the check on space the other already reserved; an import short
of space is refused as retryable `429 resource_limit` with the same `shortfall`
(only a declaration beyond `LYTRONAUT_UPLOAD_BYTES` remains a permanent
`413 invalid_input`). Otherwise it is refused before any job, file or schema
exists with `429 resource_limit`, retryable, carrying
`shortfall {required_bytes, available_bytes}` (byte counts only, never a path).
Restore accepts any positive `Content-Length` whose reservation fits a signed
64-bit count (anything larger is invalid input); a declaration the volume cannot
hold simply fails admission. Archive and restore
rows are not counted against `LYTRONAUT_RESERVED_BYTES`, which keeps bounding
imports and flatten/Live Photo exports. Space consumed by other processes after
admission still surfaces as an ordinary retryable job failure with owned cleanup.

**Any number of originals, at constant memory.** The capture snapshot streams the
unique originals in bytewise ascending (SHA-256, length) order (PostgreSQL sorts
with the `"C"` collation, so database locale cannot change it) to an on-disk
inventory spool, and the bundle writes its records in that order. Restore accepts
only strictly ascending records, rejecting a repeat or disorder while remembering
just the previous record, spools them, and after the row import compares them
with the restored catalog's own ascending inventory in one streamed pass. Neither
side keeps a per-original structure in memory, and `LYTRONAUT_UPLOAD_FILES` (a
per-upload count) no longer bounds a bundle. The header's u32 count (about
4.29 billion unique originals) is the format's only count bound. Bundles written
before this change already carry ascending records. Version-1 bundles keep their
in-memory check, bounded by their 32 MiB catalog file (each original needs at
least its 64-character digest there), not by any deployment budget.

**Bounds that remain, and why they are not library-size limits.** Each original
must be at most `LYTRONAUT_FILE_BYTES`, the same per-file policy the store applies
when it verifies a blob for reading. The v2 logical catalog section is at most
288 MiB (codec bound), accepted as this release's format bound. Measured through
the real Desktop intake, adaptation and API mapping on a deliberately generous
synthetic library (21 picture and 30 metadata columns, seven companion files per
capture, album membership, events, bins, picture lists and import history per
capture; `logical_catalog_capacity_measured_on_realistic_library`), a capture
costs about 8.9 KB (8,937 bytes: 1,000 captures produced 8,936,813 catalog
bytes, fixed overhead included), so the section holds about 33,800 captures.
Seven files per capture is above the surveyed corpus's average of about 6.7, so a
real library likely fits slightly more. Owner decision (2026-10-13): this bound is
accepted for this release; splitting the catalog into chunks is a possible later
format change, not part of this release. The test fails if capacity drops below
30,000 captures. Exceeding the section fails
at backup time, before any artifact exists, with a non-retryable `resource_limit`
job error (`bundle-limit`), never a partial or corrupt bundle; its spools are
removed and its quota released. The version-1 compatibility path keeps its 32 MiB
catalog-file cap but no longer caps its originals.

**Proof beyond 4 GiB.** `just large-bundle-test` (CI job `large-bundle`) runs one
ignored test explicitly and requires exactly one pass: a sparse original of
4 GiB + 64 KiB plus two smaller originals are backed up and restored, so both a
single length and the stream offsets pass 4 GiB, and every restored original's
SHA-256 is recomputed. It needs about 9 GiB of real scratch space, so it is not
in the default suite.

## Destination ownership and publication

Restore creates a separate, independently startable result. SQLite has its own fresh
catalog file; PostgreSQL gets a fresh generated schema inside the explicitly configured
deployment database. It never imports into the active schema or replaces an existing
namespace. Deployment privileges require database CONNECT and CREATE-schema, plus
ownership/USAGE of the generated schema; CREATEDB and superuser are unnecessary.
`LYTRONAUT_DATABASE_SCHEMA` explicitly binds every PostgreSQL pool connection and
startup to the selected namespace. Deployment credentials remain deployment input;
the result descriptor contains only its backend and generated namespace.

A private journal is created with mode 0600 and fsynced, including its parent directory,
before any result directory, ownership marker, or namespace creation. A pre-existing final result fails before journal creation and remains unchanged. The pending name includes the unpredictable journal nonce. A crash after mkdir but before its marker leaves the prepared journal retained; cleanup never deletes an unmarked directory by guessing ownership from its name. It records the job, unpredictable ownership nonce,
generated namespace, result path relative to the owned export root, and reserved size.
The generated schema and trusted ownership marker are created in ONE transaction;
crashing before its commit leaves neither. A same-name existing schema fails creation
and is never removed. Subsequent cleanup verifies the exact namespace and nonce/job
marker in the explicitly configured database; it never enumerates by prefix.

Publication phases are prepared → schema-owned → rows-committed → files-ready →
files-published → job-succeeded. Every phase transition uses an atomic journal rewrite
and directory fsync. Rename success followed by parent fsync failure is an ambiguous durability result: the implementation reloads and validates the exact job, nonce, namespace and reserved amount before adopting the observed phase, and still returns an error. Missing or changed journal identity retains ownership and quota rather than pretending rollback succeeded. Original files and pending directories are fsynced before rename;
the publication parent is fsynced afterward. Database commit and filesystem rename
are distinct boundaries and cannot be described as a single atomic operation.

Central owned-result reconciliation handles startup BEFORE generic failed-job cleanup,
worker cancellation/failure, final job UPDATE failure, and late DELETE. Only a durable
successful job plus fully published result is retained. Everything else is cleanup
pending. It drops only a verified owned namespace, removes only the exclusively owned
result, and releases quota/removes the journal only after both cleanups are durable.
Namespace cleanup never uses CASCADE: it drops exactly the trusted application
tables and then the schema, both RESTRICT, in one transaction, so an unrelated object
outside the namespace that depends on a restored table makes cleanup fail closed
instead of being dropped with it.
If the database is unreachable, permissions deny access or DROP, a dependency
restricts DROP, or filesystem deletion fails, the journal and reservation remain. Repeated recovery is idempotent; no guessed
ownership, lost reservation, or unjournaled successful-looking result is allowed.

Blocked cleanup is per job and does not stop the service. A job whose owned cleanup
cannot be proved or completed is persisted as `failed` with error `cleanup-blocked`
(reported as a retryable `storage_unavailable` error), logged loudly with its job id,
and keeps its files, journal and quota. An in-flight reservation it still holds
keeps counting in free-space admission; the bytes of an already published
whole-library result are its `retained` quota, counted in no budget (archive and
restore are exempt from `LYTRONAUT_RESERVED_BYTES`) but still physically on the
work volume, so admission sees them through the filesystem's free space. Its report
also keeps one of the 128 retained-report slots and is never evicted.
`DELETE` and every startup retry the cleanup; once it succeeds the reservation is
released and the job reads `interrupted`. A previously ready result that no longer
verifies at startup (including a ready restore row whose journal is missing or never
recorded publication, since the database and export root are separate stores)
becomes `failed` with error `unverified-result` (also reported
as retryable `storage_unavailable`), keeps its files, namespace, journal and quota,
and is never cleaned automatically: only an explicit `DELETE`, with ownership proof,
removes the user's restored copy. Each startup retries publication/ownership and
PostgreSQL schema verification; if they pass, a retained `unverified-result` job
returns to `succeeded`. This verifies ownership and publication, not every restored
file's content hash. Missing/non-published journals remain unready. If either non-ready state cannot be persisted,
startup is refused rather than leaving the job in its previous state. Startup is
refused only for whole-store failures: an unreadable export root or restore journal,
or an unreachable database (checked after any per-job failure). A journal file name
that is not an application job id is foreign and is left untouched.
Without a journal, a `<job>.restored`, `<job>.pending-*` or `<job>.deleting-*` path is not provably
owned: cleanup leaves it untouched and keeps the job's reservation (no quota is
released while those bytes remain), exactly like the other fixed transfer names.

The worker's final status UPDATE can fail ambiguously (committed, but reported as
an error). The worker therefore re-reads the durable job state and acts only on it:
a committed success keeps its published result, a durable failure or cancellation
triggers owned cleanup, and any other state leaves ownership, files and quota to
startup reconciliation. The worker's progress monitor retries one failed state read at its next 25 ms
poll; any successful read resets that allowance. Two consecutive failed reads
stop work early, but that never decides the outcome: only a cancellation
recorded in the database (applied atomically by the final UPDATE) discards a
finished restore, and an unreadable state after success leaves the result, files
and quota for reconciliation. Recovery keeps an already recorded cancellation or failure
reason and marks only never-finished jobs `interrupted`.

A restore namespace is created fresh and exclusively owned by one journaled job, so
its catalog writes do not take the deployment-wide PostgreSQL writer key; a long row
import never stalls live intake, export/restore admission or job ordering.

No migration of older on-disk state is needed: fixed-name `<job>.artifact`,
`.snapshot` and `.upload` files without ownership records were only ever produced by
unreleased integration-branch history (introduced 2026-10-11, never on `main`, no
release or tag), and the current code creates them only inside a recorded transfer
owner directory.

## Startup reconciliation matrix

Restore jobs at startup, by database row state, restore journal phase and whether
the owned files (`<job>.restored` / pending store and, on PostgreSQL, the owned
schema) are present. "Unverified" means: persist `failed` / `unverified-result`
(reported as retryable `storage_unavailable`), retain files, schema, journal and
quota (re-counting the journaled reservation if the row lost it in the same atomic
UPDATE as the unverified status), unready for this recovery, and remove only on
an explicit `DELETE` with ownership proof. Subsequent startups retry verification. "Cleanup" means owned
cleanup with marker/nonce proof, releasing quota only after removal; an unprovable
or failed cleanup becomes `cleanup-blocked` (retained, retried on DELETE/startup).
Startup is refused only for whole-store failures or when the chosen state cannot be
persisted. Journal phases: never-ready = Prepared, SchemaOwned, RowsCommitted,
FilesReady; published = FilesPublished; once-ready = JobSucceeded; cleanup-intent =
CleanupPending, CleanupComplete (a cleanup was already decided and begun).

| Row state | Journal phase | Files | Action |
|---|---|---|---|
| any | cleanup-intent | any | Resume durable owned cleanup before considering readiness or unverified errors |
| `failed`, error `unverified-result` | published or once-ready | verified | Restore succeeded, clear error, keep accounting; journal → JobSucceeded |
| `failed`, error `unverified-result` | any | missing or not verified | Keep unverified and counted; retry next startup |
| `cancelled`/`cancelling` | any | any | Cleanup (recorded DELETE takes precedence, including unverified results) |
| `succeeded` | published or once-ready | present, verified (dir, private marker, PG schema marker) | Ready; journal → JobSucceeded |
| `succeeded` | published or once-ready | missing or not verified | Unverified |
| `succeeded` | never-ready | any | Unverified |
| `succeeded` | absent | any | Unverified (M1) |
| missing | once-ready | any | Re-create row as unverified with retained = journaled reservation |
| `queued`/`running` | once-ready | any | Unverified (database regressed; no recorded DELETE) |
| `failed` (not `cleanup-blocked`) | once-ready | any | Unverified |
| `cancelled`, or `failed`/`cleanup-blocked` | once-ready | any | Cleanup (recorded DELETE) |
| missing, `queued`/`running` | published | any | Unverified (no recorded outcome; may have been ready); missing row re-created |
| `cancelled` or `failed` | published | any | Cleanup (recorded terminal outcome) |
| any non-`succeeded`, or missing | never-ready | any | Mark interrupted (keep a recorded reason), Cleanup |
| `failed`/`cancelled` | absent | `<job>.restored` or `<job>.pending-*` present | Cleanup-blocked (unprovable; nothing deleted, quota kept) |
| `failed`/`cancelled` | absent | none present | Release quota (nothing owned remains) |
| `queued`/`running`/`cancelling` | absent | any | Mark interrupted, then as the `failed` rows above |

Intentional behaviour change: a published result (FilesPublished) whose job has no
recorded outcome used to be cleaned up at startup as an interrupted job. Because a
database restored from an older snapshot looks exactly the same, it is now kept as
unverified. After a crash between publication and the final status write the user
therefore sees a retained, unready restore. A subsequent startup can verify it
and restore readiness; `DELETE` requests owned cleanup instead. Recovery logs do
not claim a recreated row was previously ready or that verification was attempted
when the journal only proves publication.

Every cell that can delete data requires either a recorded cancellation/DELETE or
terminal failure (cancelled, failed, cleanup-blocked), a never-ready phase, or an
already recorded cleanup decision. Without a journal nothing restore-owned is
deleted because ownership cannot be proved.

Finished non-restore exports (archive, Flatten and Live Photo artifacts) are not
re-verified at startup. A `succeeded` row records job completion even if its
artifact was later lost. Download requires the succeeded row, verified transfer
ownership and an openable artifact; it does not rehash content. See the explicit
non-restore integrity-policy disposition below.

## Verification

The `disk-accounting validation` additions (metadata-inclusive sizing, oversized allocation
units, inode serialization and actual HTTP/1.1 upload frames) have separately
scoped evidence in [disk accounting validation](../validation/disk-accounting.md).
That source snapshot distinguishes local results from additional PostgreSQL/CI
execution required for the lifecycle handoff.


Executed on SQLite in the default suite and on PostgreSQL 18.6 through the strict
`just postgres-test` recipe (exact counts, restricted role, public sentinel):

- Independent SQLite→SQLite, SQLite→PostgreSQL, PostgreSQL→SQLite and
  PostgreSQL→PostgreSQL round trips comparing every portable table's logical
  values (signed64 MIN/negative/zero/MAX IDs and ordinals, Unicode/NUL keys and
  names, item errors, intake and source columns, opaque bytes, originals and
  SHA-256) and reopening the result through normal startup.
- The four exact version-1 legacy forms with synthetic credential rows (never
  restored) and near-32 MiB legacy files (one large cell; many small rows).
- A library whose originals exceed the former `LYTRONAUT_UPLOAD_BYTES` + 288 MiB
  ceiling (sparse synthetic originals, small configured budgets) backed up and
  restored losslessly bundle-to-bundle and through the HTTP archive/download/
  restore path (download frames asserted at most 64 KiB), free-space admission
  refusals that name required and available bytes, per-file block cost enforced
  on restore and archive (a 200,000-tiny-original bundle refused before extraction
  on both engines), import/export reservations counted in each other's admission,
  and a library with no originals.
- `restore_fault_tests.rs`: corruption, admission and mid-restore quota, cancel,
  graceful shutdown, a real child-process SIGKILL with restart recovery,
  owner-revoked cleanup, dependency-restricted DROP, failing and ambiguous final
  status UPDATE, late DELETE racing publication, journal crash cuts (prepared,
  mkdir, marker, rename, namespace created before SchemaOwned, files ready, files
  published with and without a committed success), unjournaled restore paths,
  live writes during a restore import, an exactly-at-limit source cell, per-job
  blocked cleanup that still starts the service, an unverifiable ready result
  that fails closed and is never auto-deleted, a monitor read error after success
  that keeps the result, and whole-store failures that still refuse startup.
  Every case checks unrelated SQL/filesystem sentinels and the live library.

The new `restore reconciliation` cases executed at source `47b4cb0`: SQLite restore-fault
matrix **22 passed**, active monitor retry **1 passed**, and strict PostgreSQL
**62 passed**, each with zero failures or ignored cases. They cover startup
re-verification after repaired ownership; published/missing-row and published/
cancelling cases; an SQL-trigger check that unverified status never commits with
lost accounting; tombstone intent/rename/deletion crash cuts and foreign-directory
substitution (including actual DELETE of a failed/unverified row and a failed
fallback status write); unjournaled tombstone preservation; and active monitor
retry at a deterministic render boundary in `export_control_tests.rs`.
Six subsequent mutation checks caught disabled accounting, retry and cleanup-intent
guards on both engines. The separate deterministic free-space assertion correction
then passed its affected SQLite and PostgreSQL cases. These are historical source-
specific results, not a claim that a future final SHA was already tested; see
[restore reconciliation validation](../validation/restore-reconciliation.md).

Not covered by an engine test: a database connection lost during cleanup, and
measured memory under concurrent admission. Coverage is gated separately.

Reconciliation follow-ups (restore reconciliation):

- Startup retries retained unverified restores and restores readiness on ownership/
  publication/schema verification. Missing-row recreation and zero-accounting
  recovery remain conservative, using the journaled reservation. Recorded
  `cancelling` is a DELETE even when the journal says FilesPublished/JobSucceeded.
- Before recursive restore cleanup, the journal durably records the verified
  directory's device/inode and pending/result slot, then the directory is renamed
  to a private nonce-qualified tombstone and the export root is synced. Recovery
  matches that exact directory identity before resuming deletion; it does not
  need an inner marker that partial deletion already removed. Unrelated,
  mismatched or symlink paths remain untouched and quota stays retained. Identity
  is tied to the original filesystem: moving a partially cleaned store to another
  filesystem may require operator resolution, and fails closed. Existing journals
  without this optional cleanup field remain readable.
- A single failed monitor state read is retried; two consecutive failures still
  stop work early. A durable DELETE or shutdown continues to stop work immediately.
- Finished non-restore archive, Flatten and Live Photo artifacts are deliberately
  **not re-verified at startup** (won't-fix for this task). Their `succeeded` status
  records the completed job, not ongoing artifact integrity. Download verifies
  transient ownership and opens the artifact; absent/conflicting ownership or a
  missing/unreadable file is refused. Download does not rehash artifact content,
  so corruption/truncation is not universally detected. A full startup scan could
  re-read multi-GiB whole-library archives and block service startup; adding a
  background integrity state/API is a separate product change. Users must retain
  their original library and verify portable bundle hashes during restore.

Transient ownership uses a separate durable nonce record before creating a private
job transfer directory and marker. Uploads, export artifacts and logical spools
live inside that directory. Startup, DELETE and finalization verify its marker
before deletion. A pre-existing fixed-name legacy file or unmarked transfer directory
is never deleted based on its filename; quota remains until ownership is resolved.
Successful upload crashes before restore journal creation are recoverable through
this earlier ownership record. Initial mkdir/marker crash cuts retain the record
and quota rather than guessing that an unmarked directory is safe to remove.

Recursive removal is not atomic. Restore results now use the journaled identity
and tombstone protocol above to resume partial deletion. Transient transfer-owner
cleanup still requires its inner marker: if interruption removes it before all
contents, recovery retains the durable record, remaining files and reservation
for ownership resolution. It never infers ownership from the directory name;
automatic resumable deletion of those transient directories is not claimed.

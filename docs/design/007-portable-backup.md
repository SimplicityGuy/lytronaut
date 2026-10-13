# Portable backup and isolated restore

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

Version-2 transport admission includes 288 MiB catalog bytes, a 60-byte header and
40 bytes for each admitted original, in addition to configured encoded payload.
Archive reservation includes both the logical spool and its archive copy plus
framing and original bytes. Restore reservation includes two declared archive
sizes plus the maximum logical spool. These staging reservations are deliberately
fixed upper bounds, not measured sizes: an archive job reserves its unique original
bytes plus 2 x 288 MiB (spool and archive copy at the v2 catalog maximum) plus
framing, and a restore reserves 2 x the declared upload plus 288 MiB, even for a
small catalog. Admission therefore needs that much free staging space. SQL database/index storage is deployment
storage too; these application reservations are not a physical bound on PostgreSQL
WAL/index growth. Cleanup retains reservations until every owned filesystem and
SQL namespace removal succeeds. Failed uploads use that same cleanup path rather
than releasing quota after a best-effort unlink.

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
and keeps its files, journal and quota; admission keeps counting that quota.
`DELETE` and every startup retry the cleanup; once it succeeds the reservation is
released and the job reads `interrupted`. A previously ready result that no longer
verifies at startup (including a ready restore row whose journal is missing or never
recorded publication, since the database and export root are separate stores)
becomes `failed` with error `unverified-result` (also reported
as retryable `storage_unavailable`), keeps its files, namespace, journal and quota,
and is never cleaned automatically: only an explicit `DELETE`, with ownership proof,
removes the user's restored copy. If either non-ready state cannot be persisted,
startup is refused rather than leaving the job in its previous state. Startup is
refused only for whole-store failures: an unreadable export root or restore journal,
or an unreachable database (checked after any per-job failure). A journal file name
that is not an application job id is foreign and is left untouched.
Without a journal, a `<job>.restored` or `<job>.pending-*` path is not provably
owned: cleanup leaves it untouched and keeps the job's reservation (no quota is
released while those bytes remain), exactly like the other fixed transfer names.

The worker's final status UPDATE can fail ambiguously (committed, but reported as
an error). The worker therefore re-reads the durable job state and acts only on it:
a committed success keeps its published result, a durable failure or cancellation
triggers owned cleanup, and any other state leaves ownership, files and quota to
startup reconciliation. The worker's progress monitor may stop work early when it
cannot read the job state, but that never decides the outcome: only a cancellation
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
quota (re-counting the journaled reservation if the row lost it), never ready, and
remove only on an explicit `DELETE` with ownership proof. "Cleanup" means owned
cleanup with marker/nonce proof, releasing quota only after removal; an unprovable
or failed cleanup becomes `cleanup-blocked` (retained, retried on DELETE/startup).
Startup is refused only for whole-store failures or when the chosen state cannot be
persisted. Journal phases: never-ready = Prepared, SchemaOwned, RowsCommitted,
FilesReady; published = FilesPublished; once-ready = JobSucceeded; cleanup-intent =
CleanupPending, CleanupComplete (a cleanup was already decided and begun).

| Row state | Journal phase | Files | Action |
|---|---|---|---|
| any, error `unverified-result` | any | any | Keep as is (retained until DELETE) |
| `succeeded` | published or once-ready | present, verified (dir, private marker, PG schema marker) | Ready; journal → JobSucceeded |
| `succeeded` | published or once-ready | missing or not verified | Unverified |
| `succeeded` | never-ready or cleanup-intent | any | Unverified |
| `succeeded` | absent | any | Unverified (M1) |
| missing | once-ready | any | Re-create row as unverified with retained = journaled reservation |
| `queued`/`running`/`cancelling` | once-ready | any | Unverified (database regressed; no recorded DELETE) |
| `failed` (not `cleanup-blocked`) | once-ready | any | Unverified |
| `cancelled`, or `failed`/`cleanup-blocked` | once-ready | any | Cleanup (recorded DELETE) |
| missing, `queued`/`running`/`cancelling` | published | any | Unverified (no recorded outcome; may have been ready); missing row re-created |
| `cancelled` or `failed` | published | any | Cleanup (recorded terminal outcome) |
| any non-`succeeded`, or missing | never-ready | any | Mark interrupted (keep a recorded reason), Cleanup |
| any non-`succeeded`, or missing | cleanup-intent | any | Cleanup (continue the recorded decision) |
| `failed`/`cancelled` | absent | `<job>.restored` or `<job>.pending-*` present | Cleanup-blocked (unprovable; nothing deleted, quota kept) |
| `failed`/`cancelled` | absent | none present | Release quota (nothing owned remains) |
| `queued`/`running`/`cancelling` | absent | any | Mark interrupted, then as the `failed` rows above |

Intentional behaviour change: a published result (FilesPublished) whose job has no
recorded outcome used to be cleaned up at startup as an interrupted job. Because a
database restored from an older snapshot looks exactly the same, it is now kept as
unverified. After a crash between publication and the final status write the user
therefore sees a retained, unready restore and removes it with `DELETE`.

Every cell that can delete data requires either a recorded cancellation/DELETE or
terminal failure (cancelled, failed, cleanup-blocked), a never-ready phase, or an
already recorded cleanup decision. Without a journal nothing restore-owned is
deleted because ownership cannot be proved.

Finished non-restore exports (archive and flatten artifacts) are not re-verified at
startup. A `succeeded` archive/flatten row keeps its status even if its artifact was
lost or the database regressed, so the status can be misleading; download still
refuses it unless the job is `succeeded` and its transfer-owner record and artifact
verify, so no unverified bytes are served.

## Verification

Executed on SQLite in the default suite and on PostgreSQL 18.6 through the strict
`just postgres-test` recipe (exact counts, restricted role, public sentinel):

- Independent SQLite→SQLite, SQLite→PostgreSQL, PostgreSQL→SQLite and
  PostgreSQL→PostgreSQL round trips comparing every portable table's logical
  values (signed64 MIN/negative/zero/MAX IDs and ordinals, Unicode/NUL keys and
  names, item errors, intake and source columns, opaque bytes, originals and
  SHA-256) and reopening the result through normal startup.
- The four exact version-1 legacy forms with synthetic credential rows (never
  restored) and near-32 MiB legacy files (one large cell; many small rows).
- `restore_fault_tests.rs`: corruption, admission and mid-restore quota, cancel,
  graceful shutdown, a real child-process SIGKILL with restart recovery,
  owner-revoked cleanup, dependency-restricted DROP, failing and ambiguous final
  status UPDATE, late DELETE racing publication, journal crash cuts (prepared,
  mkdir, marker, rename, namespace created before SchemaOwned, files ready, files
  published with and without a committed success), unjournaled restore paths,
  live writes during a restore import, an exactly-at-limit source cell, per-job
  blocked cleanup that still starts the service, an unverifiable ready result
  that fails closed and is never auto-deleted, a monitor read error after success
  that keeps the result, and whole-store failures that still refuse startup. Every case checks unrelated SQL/filesystem sentinels and
  the live library.

Not covered by an engine test: a database connection lost during cleanup, and
measured memory under concurrent admission. Coverage is gated separately.

Known follow-ups, deliberately not changed here:

- An `unverified-result` job stays unready until `DELETE`; re-verifying it at each
  startup and restoring readiness when it verifies again would be safe.
- Directory cleanup removes the owned result with `remove_dir_all` after checking
  its marker; renaming it to a private tombstone first would make a crash mid-removal
  recoverable instead of permanently blocked.
- The worker's progress monitor stops work early on a single failed state read; a
  short retry before stopping would avoid aborting healthy jobs on one transient
  error (outcomes are already decided only by durable state).

Transient ownership uses a separate durable nonce record before creating a private
job transfer directory and marker. Uploads, export artifacts and logical spools
live inside that directory. Startup, DELETE and finalization verify its marker
before deletion. A pre-existing fixed-name legacy file or unmarked transfer directory
is never deleted based on its filename; quota remains until ownership is resolved.
Successful upload crashes before restore journal creation are recoverable through
this earlier ownership record. Initial mkdir/marker crash cuts retain the record
and quota rather than guessing that an unmarked directory is safe to remove.

Recursive removal is not atomic. If interruption removes the ownership marker
before all directory contents, a subsequent recovery deliberately fails closed:
it retains the durable record, remaining files and reservation. It never resumes
deletion by inferring ownership from the directory name. This conservative state
requires ownership resolution; automatic resumable deletion is not claimed.

# Portable backup and isolated restore

Status: implementation design approved for source work; runtime validation pending.

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
transaction. Capture streams fixed table rows to an owned logical spool with a bounded wire budget; it does not retain all SQLx rows plus a second encoded buffer. The version-1 32 MiB limit is its SQLite FILE limit, not a new logical payload limit. Legacy extraction uses bounded read-only row cursors and must not acquire narrower cell/row acceptance limits. A conservative expansion bound must account for rowid aliases, UTF-16 conversion and defaulted trailing columns and be proved before imposing a conversion spool cap. Near-limit legacy acceptance is a required independent fixture gate. The transaction
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
before any namespace creation. It records the job, unpredictable ownership nonce,
generated namespace, result path relative to the owned export root, and reserved size.
The generated schema and trusted ownership marker are created in ONE transaction;
crashing before its commit leaves neither. A same-name existing schema fails creation
and is never removed. Subsequent cleanup verifies the exact namespace and nonce/job
marker in the explicitly configured database; it never enumerates by prefix.

Publication phases are prepared → schema-owned → rows-committed → files-ready →
files-published → job-succeeded. Every phase transition uses an atomic journal rewrite
and directory fsync. Original files and pending directories are fsynced before rename;
the publication parent is fsynced afterward. Database commit and filesystem rename
are distinct boundaries and cannot be described as a single atomic operation.

Central owned-result reconciliation handles startup BEFORE generic failed-job cleanup,
worker cancellation/failure, final job UPDATE failure, and late DELETE. Only a durable
successful job plus fully published result is retained. Everything else is cleanup
pending. It drops only a verified owned namespace, removes only the exclusively owned
result, and releases quota/removes the journal only after both cleanups are durable.
If the database is unreachable, permissions deny DROP, or filesystem deletion fails,
the journal and reservation remain. Repeated recovery is idempotent; no guessed
ownership, lost reservation, or unjournaled successful-looking result is allowed.

## Required verification (not executed yet)

Independent SQLite→SQLite, SQLite→PostgreSQL, PostgreSQL→SQLite and
PostgreSQL→PostgreSQL round trips compare logical IDs, all typed record bytes,
signed64 MIN/negative/zero/MAX IDs and ordinals, Unicode/NUL names and keys, album
ordering, originals and SHA-256. Each restored result must reopen through normal
startup. Legacy four-form fixtures and malicious schema/rows remain separate gates.

Deterministic interruption, cancellation and SQL/filesystem error injection covers
every publication phase, marker/CREATE transaction, failed final job UPDATE, DELETE
racing commit/publication, denied DROP, disconnected database, restart, repeated
cleanup and concurrent admission. Tests verify retained quota until cleanup succeeds,
active-store preservation, and unrelated-schema/file sentinels. Actual engine tests
must pass before submission; this design document is not evidence of runtime success.

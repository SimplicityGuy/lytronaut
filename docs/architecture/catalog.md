# Offline Desktop catalog

`lytronaut_storage::desktop::import_desktop(&catalog, &store, job)` adapts a
completed intake. `migrate` installs additive `catalog_` tables transactionally;
deployment and intake tables are untouched. `summary` reads bounded aggregate
counts without opening or hashing any source blob. These native functions do not
install HTTP routes or grant access. HTTP consumers remain inside the configured
Host/Origin deployment boundary and select `catalog_versions.state='ready'` explicitly.

An intake success is not a semantic import success. Every version retains a
logical `catalog_sources` reference to every completed intake entry, including
unknown files, JSON sidecars, backup databases and unsupported content. Physical
blob deduplication never collapses pictures, memberships or albums. Current
Desktop hash fields are opaque historical data, including empty hashes. Sidecar
bytes are retained exactly; this adapter does not parse or apply their edits.

Version identity hashes the complete sorted logical manifest, including paths,
lengths and verified physical digests. A common outer selection folder is omitted
from identity only when every entry shares it; original paths remain exact.
Identical reimports reuse one version. Different complete bytes sharing a source
library UUID produce a separate `quarantined` version with the earlier version
unchanged. Independent libraries remain separate even if their pictures' source
hash fields match. Reimport uses the first successful version's source job;
later intake jobs still retain their own original manifests.

`ready` means a supported catalog transaction completed; individual missing
required companions appear in `catalog_item_errors`. Missing `stacklq.lfp` is
optional, including the observed LFE 10 variant. `preserved_only` records a bounded
reason for unsupported/malformed databases, conflicting primary identities,
orphans, hostile schema forms or journal snapshots. `quarantined` versions have
valid semantic records but conflicting library identity. None implies image
renderability. Album `public` remains original historical metadata and never
changes deployment access.

Known table rows retain every physical column in `catalog_records.original` as
an explicitly tagged `OriginalRecord`: null; decimal-string signed 64-bit integer;
hexadecimal IEEE-754 64-bit float bits; exact SQLite text bytes; or exact blob
bytes. No integer passes through a JSON float. This preserves unknown columns,
dates, statuses, history, picture lists and metadata separately from normalized
pictures/albums/memberships. Album sort order and membership picture index are
independent. Unknown ordinary tables are preserved byte-exactly in the original
SQLite database, without assigning semantics or projecting their rows.

Imports admit at most two concurrent adapters. `Store::manifest` checks sealed
blobs on its bounded blocking worker; only the active database is opened again
with `Store::open_blob`, also on a bounded blocking task. A private create-new
0700 staging root contains a <=32 MiB read-only 0400 DB copy and a private 0600
SQLite semantic index. Copying hashes the actual copied bytes against the sealed
manifest; both passes read that same application-owned copy. Source libraries
are never opened writable. A task-owned worker drains SQLite, unlinks its files
and syncs staging before releasing its scratch reservation. SQLite uses read-only, immutable, query-only,
trusted-schema-off, ignore-check-constraints, disabled mmap and a 2 MiB cache.
No extension loading, attachments or source writes are enabled. WAL, SHM or
journal companions prohibit semantic import. Only the main `lytrolibrary.db`
filename is active; other databases and backups remain opaque preserved files.

Schema admission precedes string retrieval and integrity inspection: <=1024
objects, <=64 KiB definition, <=4 MiB aggregate definitions, <=256-byte names.
All tables must be ordinary physical tables without generated columns; source
views are never queried. Virtual tables are refused globally before integrity
hooks can run. CHECK expressions are not evaluated. Fixed table names and quoted,
doubled identifiers are the only interpolated SQL. Read admission caps each table
at 128 columns, each text/blob at 1 MiB, and each decoded row at 8 MiB (every
formerly admitted row already fit that old whole-catalog byte bound). It no
longer caps capture count, row count, table cells or aggregate decoded bytes.
SQLx buffers one source row at a time; validation and installation each scan the
same immutable private copy. Exact typed-ID uniqueness, UUIDs, foreign references
and membership ordering use indexed disk tables instead of all-catalog Rust maps.
Original records and missing-companion errors stream directly into the destination.
The manifest itself remains subject to the existing intake file/path budgets.

Source inspection retains the 20-million-instruction/ten-second safety budget.
Each row scan has a VM-work budget and renews its deadline when requesting the
next row, excluding downstream destination-write backpressure. These are parser
resource limits; support is not unlimited source size. No source schema expression
becomes application SQL, a migration or an extension hook.

`intake_scratch` is additive application-owned transient quota state, separate
from succeeded intake rows and retained export artifacts. It records a random
owner nonce, the locked staging directory's device/inode identity, source job and
reserved bytes. A short writer transaction counts all intake/export/scratch
reservations before creating files; source copy/hash/validation holds no target
SQL transaction. Intake, export and restore re-admission include scratch quota.
The SQLite index has an enforced 4096-byte page cap of `4 MiB + 16 * source_size`
rounded up; reservation costs both files using the staging filesystem's actual
allocation unit plus 1 KiB metadata per file. The full reservation remains charged
until cleanup, conservatively including materialized bytes, exactly once in each
admission sum. This does not reserve permanent destination catalog growth.

Startup holds exclusive root locks, deletes interrupted staging files and syncs
the directory before releasing only that root's reservations. The held directory
FD anchors identity; replacing the pathname fails closed. A live reservation's
foreign key prevents deleting its source job. Cancellation, caller drop and Store
shutdown request the worker to stop and wait for drain/cleanup. Cleanup errors
retain quota for startup reconciliation. Portable v2's fixed logical table list
excludes this transient table; exact older physical schemas without it remain
readable, and a physical snapshot containing live scratch reservations is refused.

The final installation is one SQLite or PostgreSQL transaction: assert intake still succeeded,
insert exactly one completed intake reference per source, then original records,
normalized associations and item errors, and finally publish the state. Fingerprint
uniqueness serializes concurrent/repeated imports. Failures roll back all new
catalog visibility. No catalog pointer can refer to an unfinished upload.

Synthetic MIT tests exercise Unicode, captions, ratings/flags, extreme integers,
blob/null/float unknown fields, quoted identifiers, cross-album membership and
ordering; dedup/reimport/restart/version conflicts; missing companions; orphans,
duplicate IDs, malformed DBs, unknown schemas, views, virtual/generated tables,
CHECK expressions, schema/cell/row budgets; and failed migration rollback retaining
auth/intake. Private `examples/private_catalog.rs` separately reconciles the
operator's source against its ignored manifest before and after intake, checks
all logical counts, album sizes, source digests and restart/reimport, and writes
only a mode-0600 aggregate report in its create-new mode-0700 scratch directory.
It is never an automatic CI dependency.

Upstream API references: [SQLx immutable/read-only options](https://docs.rs/sqlx/0.9.0/sqlx/sqlite/struct.SqliteConnectOptions.html),
[SQLx progress handler](https://docs.rs/sqlx/0.9.0/sqlx/sqlite/struct.LockedSqliteHandle.html#method.set_progress_handler).
The source adapter and fixtures are independently authored MIT code; no new crate
or license graph is introduced (`serde_json` was already in the locked graph).

Local authorized validation on 2026-10-11 passed: 728 files / 2,001,830,074 bytes;
109 pictures and 109 metadata rows; three albums with 109 ordered memberships
(sizes 12, 39, 58); two bins; three events; eleven picture lists; zero item errors.
All original source SHA-256, size and modification-time entries matched the
operator's durable private baseline before and after import. All completed
manifest hashes/lengths matched, and reimport/restart retained the same version.
The command used a command-local optimized SHA-256 development build for the
2 GiB validation; no global Cargo profile or product dependency was changed.
The private source and generated catalog/report remain outside Git and CI.

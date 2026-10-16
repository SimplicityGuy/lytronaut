# Offline Desktop catalog

`lytronaut_storage::desktop::import_desktop(&catalog, &store, job)` adapts a
completed intake. `migrate` installs additive `catalog_` tables transactionally;
authentication and intake tables are untouched. `summary` reads bounded aggregate
counts without opening or hashing any source blob. These native functions do not
install HTTP routes or grant access. API consumers must apply authentication and
select `catalog_versions.state='ready'` explicitly.

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
changes authentication.

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
0700 temporary directory contains a <=32 MiB read-only 0400 DB copy. Copies are
removed by an owned RAII guard. SQLite uses read-only, immutable, query-only,
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
at 128 columns, all known rows at 100,000, all cells at 100,000, each text/blob at
1 MiB, and aggregate cell bytes at 8 MiB before fetching records. A SQLx progress
handler interrupts after approximately 20 million VM instructions or ten seconds.
These deliberately conservative semantic limits yield preservation-only versions;
they do not discard originals or enlarge the intake budget. SQLx's 0.9 safe
`LockedSqliteHandle::set_progress_handler` API is used without unsafe application
code. Source schema expressions never become application SQL or migrations.

The final installation is one SQLite transaction: assert intake still succeeded,
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

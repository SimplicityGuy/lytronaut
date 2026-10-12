# Application database boundary

Bead: `lytronaut-e7y`, within the approved `lytronaut-p9o` molecule.

## Baseline and validation

The development seat began at `1211f70a7ea9601e47cbcad413e248767a9554c2`,
with no local changes. The application currently exposes `SqlitePool` directly
to ingestion, catalog adaptation, API reads, and export jobs. The original
Lytro Desktop database reader separately owns a bounded, immutable
`SqliteConnection`; that input reader stays SQLite regardless of the selected
application database. Existing backup snapshots use SQLite `VACUUM INTO` and
must retain that behavior until the portable backup bead replaces it.

The exclusive validation slot is currently held by packaging. This document
records a source inspection baseline, not a fresh passing execution. Before
moving application callers, validate the inherited SQLite behavior after the
approved login-removal change has landed. Do not use auth-era tests as evidence
for the final deployment access model.

Use strict validation because migrations, integer fidelity, publication and
quota transactions are data integrity boundaries: focused actual-engine tests
at each coherent transition, broader storage/service tests when callers move,
and the complete `just check` gate before submission. Both actual engines are
required. Skipping PostgreSQL because no test URL is supplied is not a passing
PostgreSQL acceptance result.

## North star

`Catalog` owns the selected application connection pool, backend identity,
bounded connection/statement waits, migrations, and writer admission. Consumers
use the application pool and its explicit transaction helper; they do not
construct PostgreSQL connections themselves. SQL remains parameterized and
owned by application source. No input library or inbound archive supplies SQL
to this boundary.

SQLite and PostgreSQL use portable static queries where semantics agree and
explicit engine-specific statements where they do not. This is deliberately
not a generic SQL transpiler. In particular:

- Preserve complete signed 64-bit values, binary record bytes and generated
  catalog identities; PostgreSQL uses BIGINT, BYTEA and generated identities.
- Preserve SQLite ASCII-only case folding for search and name sorting, literal
  `%`, `_` and escape-character queries, byte-based display truncation, and
  existing NULL sort order.
- Replace incidental SQLite rowid job ordering with a persistent monotonic job
  order, migrating existing rows in their previous rowid order.
- Reserve SQLite writers with `BEGIN IMMEDIATE`; PostgreSQL writer transactions
  take the same application advisory transaction lock before reading quotas or
  publishing competing catalog identities. Rollback releases ownership.
- Bound pools, acquisition and server statements. Connection configuration and
  errors never reveal database URLs, passwords or imported data.
- SQLite deployments upgrade additively. The approved login removal remains
  intact. Database credentials are deployment configuration, not app accounts.
- Uploaded originals remain immutable server files. Neither backend stores
  original capture blobs in database columns.

The persistence capability scores cohesion 2, coupling 1, state ownership 2,
port narrowness 1, replaceability 2, direction 2 and test closure 1. The storage
crate is the smallest existing coherent owner; a separate service or crate
would add a deployment boundary without solving current semantics. This is an
incremental persistence refactor rather than a claim that every SQL operation
is hidden behind a domain repository trait. Storage never imports the service.

## Movement sequence

1. Introduce backend configuration and transaction/connection ownership with
   isolated contract tests, preserving the SQLite input decoder.
2. Move application schema and queries to shared bindings plus explicit dialect
   choices. Validate SQLite migration, conflict handling, cancellation, recovery,
   concurrent quota/publication and literal search against both actual engines.
3. Wire deployment selection and redacted startup errors; retain SQLite snapshot
   compatibility and provide the next backup bead a consistent catalog read seam.
4. Integrate the landed packaging/access change, freeze source-bound evidence,
   refine and submit through Beadhive.

Portable same/cross-backend archive restore belongs to `lytronaut-1ga`;
PostgreSQL Compose and GitHub CI jobs belong to `lytronaut-55g`. These are initial
release requirements and are not disabled or represented as complete by this
bead. Final coverage includes every added first-party application path.

## Source text fidelity

The existing input validator accepts valid UTF-8 containing NUL in source
names, captions and identities. PostgreSQL TEXT cannot represent NUL. Rejecting
these inputs only on PostgreSQL would change imported-library support.

PostgreSQL source identities therefore require a reversible binary-safe
representation at the application boundary. Hex text preserves bytewise
identity comparison, including embedded NUL, without colliding with ordinary
source strings. Names and captions require exact binary original projections
plus derived display and search values. The existing SQLite implementation
terminates `substr` and `LIKE` text at the first NUL; its NOCASE comparison folds
ASCII bytes and then compares complete original byte lengths when prefixes
compare equal. PostgreSQL uses explicit prefix/display/length values to retain
these semantics. Unicode case folding is not a substitute.

The independent helper tests cover reversible keys, binary ordering, Unicode,
NUL and controls. Actual-engine tests still must verify the SQL integration.
Portable backup rows must carry logical source values and regenerate derived
engine projections. PostgreSQL storage encodings are never exposed as source
identities in API responses or treated as original library bytes.

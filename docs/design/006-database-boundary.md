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
PostgreSQL Compose and broader deployment/cross-restore CI belong to
`lytronaut-55g`. The scoped e7y actual-engine parity CI job is described below.
These are initial
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


## Source wiring checkpoint (runtime verification pending)

Application intake, adaptation, API mapping/browsing, export retention, recovery,
and deployment startup now select the application backend. The source Desktop
connection remains typed SQLite and read-only. The SQLite snapshot seam retains
its generated schema and immutable originals; snapshots remove transient job
order references alongside transient jobs. Restore accepts only the current exact
trusted schema or that exact schema minus the newly introduced order table, which
startup reconstructs from legacy SQLite rowids. Portable PostgreSQL bundles remain
assigned to lytronaut-1ga; this checkpoint does not claim that capability complete.

PostgreSQL generated catalog identity allocation follows the retained table maximum
under writer admission: max+1 for every retained signed64 maximum below MAX (including negative and zero),
1 for an empty table, or at most 100 collision-checked
positive candidates at signed64 MAX. This also survives a sequence exhausted by
rollback or an ignored conflict without a retained MAX row, avoiding an overflowing
nextval that aborts the transaction. SQLite keeps its native rowid allocation.
Existing maximum-valued identities are retained.
New job ordinal exhaustion is a distinct ordering concern assigned to the portable
restore design: a random identifier cannot silently satisfy monotonic order.

Source-written actual-engine tests cover integer/binary preservation, rollback,
transaction-scoped PostgreSQL admission, named SQLite memory pool identity,
legacy rowid backfill, NUL/Unicode/ASCII literal-search and display projections,
and maximum catalog identity collision handling. They have not been executed:
the packaging task retains the shared build and Docker validation slot. The final
login-removal integration, dependency lockfile update, both-engine execution and
submission gate must precede any completion claim.


The existing catalog schema has no positive-ID check, and strict bundle row
validation bounds schema, foreign keys, job states and original inventory rather
than narrowing catalog IDs. Negative and zero IDs therefore remain within the
accepted restored range. Added actual-engine tests exercise empty, signed64 MIN,
negative-only, -1, zero and positive retained tables, comparing PostgreSQL
allocation and rollback directly with the linked SQLite engine. These source
cases also await the shared validation slot; no runtime parity claim is made.


## Preliminary actual-engine results

The auth-era storage-only checkpoint passed 31 SQLite tests (12 unit, five Desktop,
14 intake) on the actual linked SQLite 3.51.3 and seven explicitly selected
PostgreSQL tests (six unit, one Desktop) on an actual PostgreSQL 18.6 / 180006
service. Both commands enforce nonzero expected counts and zero ignored tests;
PostgreSQL tests run serially under a role without superuser, create-role,
create-database or replication privileges. All four pooled PostgreSQL connections
assert the exact owned schema and Desktop migration checks that no app tables
appear in public. The original deployment URL is preserved when selecting the
owned schema because SQLx's lossy typed-options URL omits server options.

Actual execution exposed and corrected named SQLite in-memory connection isolation:
the shared name requires a file URI, and SQLx's lossy URL builder cannot handle its
relative colon. The stable opaque sqlite:file URL now roundtrips the named memory
identity through Any while retaining the typed snapshot options. Memory sharing,
independent-open isolation and snapshot visibility now pass. Initial failures were
retained rather than counted as successful runs. Earlier executables could not be
retrospectively frozen because their expected paths were absent; fresh green
executables and complete tested source manifests are frozen outside the worktree.

The PostgreSQL dependency closure adds 43 package identities without changing or
removing any of the 304 prior locked identities/checksums. Notice generation
retains ring's Apache-2.0 AND ISC obligations and each upstream legal text, selects
MIT where rustls/whoami explicitly offer that alternative, and retains the full
[CDLA-Permissive-2.0](https://cdla.dev/permissive-2-0/) agreement for webpki-roots
certificate data. These notices do not change the original project's MIT license.

This is preliminary evidence, not final submission: canonical login-removal
integration, service API/export/migration adverse-path testing, whole-application
checks, and the release coverage threshold remain required. Portable cross-backend
backup and deployment CI remain owned by their approved downstream beads.


## Canonical login-removal source integration

The database leaf integrates the canonical a0ab1e2 login-removal merge. Startup
retains deployment-selected database configuration and redacted connection errors,
removes only the app-owned obsolete bootstrap secret, and uses the approved
Host/Origin and X-Lytronaut-Intent mutation contract. Native and Rust WASM retain
automatic CPU/GPU selection. Historical obsolete-auth table removal runs only
against the owned application SQLite database under writer admission; PostgreSQL
starts with the approved login-free schema and source Desktop inputs remain
immutable.

SQLite bundle admission combines the two exact compatibility seams: current
trusted schema or that exact schema without the derived job-order table, each with
or without the three exact historical auth definitions. Unknown DDL is rejected;
input schema SQL is never executed. Verified legacy credential state is removed
only from the isolated owned copy, while retained order rows are scrubbed and
legacy missing order is reconstructed at startup. Actual cross-combination and
unknown-DDL rejection tests remain required after a validation-slot grant.

The source-only merge preserves all PostgreSQL/Any dependency versions and checksums
while removing the five unreachable auth-only packages. Lock, generated inventory
and notice package identities agree statically (342 locked identities, 336 external
packages). Cargo resolution and licence regeneration have not run on this merged
source. The earlier 31 SQLite and seven PostgreSQL passes remain explicitly bound
to the pre-integration preliminary checkpoint; they are not evidence for this tree.


A subsequent limited remote CI run rejected that offline reconstructed lock under
`cargo fetch --locked`. Authorized actual `cargo metadata --offline` resolution
kept every package identity/version/checksum and corrected feature edges after
auth removal (digest no longer enables its optional subtle edge), redundant
rand_core qualifiers and semantic package ordering. Actual locked offline metadata
now accepts the file, and the hash-verified frozen notice checker verifies all
336 external packages without compilation or downloads. This metadata evidence
also does not replace runtime tests or the final gate on the integrated source.


## Login-free actual-engine parity CI (source checkpoint)

The approved e7y `persistence-parity` job uses the official PostgreSQL 18.6
stable image, pinned to the multi-platform index digest
`sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873`.
The PostgreSQL [version policy](https://www.postgresql.org/support/versioning/)
still lists 18.6 as stable; 19 Beta is a prerelease. Fresh official registry
manifest metadata matched its `Docker-Content-Digest` by SHA-256 of the original
10,293 response bytes, with Linux amd64 and arm64 descriptors. No manifest
reserialization, image pull or local service startup was needed for this check.
Re-verified for `lytronaut-55g` on 2026-10-13: the versioning page and the
official source listing still show 18.6 as the newest stable release (19 beta 4
newest prerelease); the `18.6-alpine` tag and the pinned digest both return the
same 10,293-byte OCI index whose SHA-256 equals the digest; its linux/amd64 and
linux/arm64 manifests and configs hash to their descriptors and declare
`PG_VERSION=18.6`. The pin now lives once in `compose.postgres.yaml`, which the
fixture and deployment tests read.

The disposable database runs with one CPU, 1 GiB RAM, 256 PIDs and a bounded
TCP readiness wait. Only bootstrap creates the test role and public sentinel. The tested
application role has LOGIN, CONNECT and CREATE on the dedicated test database,
with NOSUPERUSER, NOCREATEDB, NOCREATEROLE and NOREPLICATION; it has no CREATE
privilege on `public`. Database CREATE permits generated private schemas and
does not grant the role permission to create databases. The public sentinel has
one synthetic marker and a SELECT-only grant. Passwords are generated per run;
the bootstrap password reaches Docker only through the client environment and
the connection URL only through the test command's environment. The recipe
redacts the URL and password from test output and nothing is uploaded.

`mise exec -- just postgres-test` requires `LYTRONAUT_TEST_POSTGRES_URL` rather
than falling back to SQLite or skipping tests. `mise exec -- just postgres-parity`
(used by the CI job, `lytronaut-55g`) provisions exactly this role and sentinel in
a disposable, uniquely named container and volume of the release pinned in
`compose.postgres.yaml`, with generated passwords, asserts `server_version_num`,
runs `postgres-test` and removes its own container and volume. The recipe runs
serially with exact result-count assertions; the expected total is computed from
its case list:

- Seven actual PostgreSQL storage unit contracts (including actual SQLite oracles).
- One actual Desktop intake/adaptation/originals/conflict/reopen contract.
- Three HTTP upload-boundary scenarios (streaming progress and Unicode names;
  stream overflow, content encoding and unsafe names; malformed declarations
  never echoed) on an owned schema, rechecking the restricted role and public
  sentinel at start and close.
- Six HTTP scenarios: browse/search/detail/assets/originals and restart,
  unsupported input/cancellation, ZIP publication, concurrent publication polling,
  PNG/JPEG export/cancellation/restart, and the lossless portable bundle
  backup/restore/tamper/conflict/interruption scenario (owned-schema restore).
- Four Live Photo export scenarios (run separately from the six HTTP scenarios
  above): create and download complete private-metadata-free pairs in all eight
  orientations; DELETE of a ready pair releasing its files and quota across
  restart; in-flight cancellation that never offers a half-pair; and an
  unavailable plane failing without output and surviving restart.
- Five portable backup cases with a PostgreSQL destination or source:
  SQLite→PostgreSQL, PostgreSQL→SQLite and PostgreSQL→PostgreSQL round trips, the
  four exact version-1 legacy forms, and near-32 MiB legacy files (one large cell;
  many small rows) restored onto PostgreSQL. SQLite-only counterparts run by default.
- Nineteen restore failure/cleanup scenarios on a PostgreSQL deployment: corrupt
  archives, quota admission and mid-restore quota, cancellation and shutdown, a
  real SIGKILL of a restoring child process with restart recovery, publish and
  DELETE, owner-revoked (denied) cleanup, restricted DROP refused by an unrelated
  dependent object, failing and ambiguous final status UPDATE, late DELETE racing
  publication, journal crash-cut matrix, unjournaled restore paths, live writes
  during a restore import, an exactly-at-limit source cell, an unverifiable ready
  result that fails closed without auto-deletion, a monitor read error after
  success that must not discard the result, ready restores without a published
  journal that stay unready and retained, once-ready restores whose database row
  regressed or vanished, and whole-store failures that
  refuse startup (per-job blocked cleanup starts the service). Unrelated SQL and
  filesystem sentinels, including restore-named decoys, must survive each case.

The HTTP fixture selects PostgreSQL only with the explicit
`LYTRONAUT_TEST_BACKEND=postgres` set by this recipe. The ordinary test recipe
continues to use SQLite, even if a test PostgreSQL URL is present. Each PG HTTP
fixture generates its own schema, preserves the selected URL/configuration
across restart, checks all four pooled connections together, verifies the public
sentinel before/after migrations and at cleanup, and removes only its own schema.
A failed assertion may leave a generated schema in the disposable CI database;
the service is job-scoped and never shares a user database.

No PostgreSQL case is excluded. The lossless portable bundle HTTP scenario
(`public_lossless_bundle_unknown_bytes_catalog_restore_tamper_conflict_and_interruption`)
runs on PostgreSQL in this recipe since `lytronaut-1ga`, and `lytronaut-55g`
added the upload-boundary group; the earlier staging exclusion no longer exists.
The recipe fails if any group reports a different count, any failed, ignored or
measured test, or no result. Deployment-level execution of both engines (Compose
services, external PostgreSQL, persistence, permissions and all four cross-backend
restores in Linux containers) is `just deployment-test`, described in
`docs/operations/linux.md`. Line coverage is gated separately by the release
acceptance task.

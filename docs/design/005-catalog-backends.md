# SQLite and PostgreSQL catalog backends

> Historical source- and date-bound record. See the [delivered application baseline](../validation/final-main-status.md) for current status.

## Decision

The operator requested SQLite and PostgreSQL support, explicitly confirmed that
both belong in the initial release, and requested the latest PostgreSQL release.
The existing release authorization covers dispatch and landing of this addition.
The catalog-backend work precedes final MVP acceptance in the filed delivery plan.

Keep SQLite available for deployments without a separate database server. Add
PostgreSQL for application catalog, import/job, and export persistence with the
same observable preservation, ordering, cancellation, quota, and recovery
semantics. Database credentials are deployment configuration; this decision does
not introduce application users or login. Imported Lytro Desktop SQLite files
remain immutable input regardless of the selected application backend.

## Filesystem storage

Uploaded originals live on the server filesystem, separate from either catalog
database engine. Compose must document configurable host bind mounts for
`/data/originals`, `/data/catalog`, and `/data/work`, including container UID/GID
permissions. PostgreSQL also requires its own persistent database storage when
provided through Compose. Keep private source libraries unchanged.

## Release and compatibility evidence

The official PostgreSQL [versioning page](https://www.postgresql.org/support/versioning/)
and [18.6 release notes](https://www.postgresql.org/docs/18/release-18-6.html)
were verified on 2026-10-11: 18.6 is the latest stable release; 19 beta 4 is
prerelease. Recheck the latest stable release before implementation and pin the
official multiarchitecture image by explicit release and immutable digest.
Rechecked for deployment parity validation on 2026-10-13 (18.6 stable; 19 beta 4 prerelease):
`compose.postgres.yaml` pins
`postgres:18.6-alpine@sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873`
(OCI index with linux/amd64 and linux/arm64), and every test fixture reads that
single pin. Operation, backend choice, backup/recovery, upgrades and limitations
are in `docs/operations/linux.md`.

The three tasks are backend persistence parity (persistence parity), lossless
portable backup/restore (portable backup implementation), and deployment plus CI validation
(deployment parity validation). Preserve existing SQLite stores and supported SQLite backup
bundles, and verify same-backend and cross-backend restoration without executing
untrusted backup SQL. Tests must run both actual database engines and cover
transaction failures, concurrent writes, corruption, quotas, cancellation, and
restart recovery. All added first-party Rust code remains in the initial
release's native plus actual-WASM application coverage gate of at least 95%.

## Dispatch boundary

Database implementation must integrate the login-free packaging changes before
final submission. Shared Cargo build/test artifacts are exclusive to one worker
at a time. The database work group lands into the release container, not directly
onto main; the assembled initial release still requires final QA and exact-main
GitHub workflow evidence. No image publication or visibility change is authorized.

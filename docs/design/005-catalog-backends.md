# SQLite and PostgreSQL catalog backends

## Decision

The operator requested SQLite and PostgreSQL support, explicitly confirmed that
both belong in the initial release, and requested the latest PostgreSQL release.
The existing release authorization covers dispatch and landing of this addition.
Beadhive molecule `lytronaut-p9o` is nested under `lytronaut-re8`; final acceptance
`lytronaut-cjq` depends on its completion.

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

The three tasks are backend persistence parity (`lytronaut-e7y`), lossless
portable backup/restore (`lytronaut-1ga`), and deployment plus CI validation
(`lytronaut-55g`). Preserve existing SQLite stores and supported SQLite backup
bundles, and verify same-backend and cross-backend restoration without executing
untrusted backup SQL. Tests must run both actual database engines and cover
transaction failures, concurrent writes, corruption, quotas, cancellation, and
restart recovery. All added first-party Rust code remains in the initial
release's native plus actual-WASM application coverage gate of at least 95%.

## Dispatch boundary

Database implementation must integrate the login-free packaging changes before
final submission. Shared Cargo build/test artifacts are exclusive to one worker
at a time. The database molecule lands into the release container, not directly
onto main; the assembled initial release still requires final QA and exact-main
GitHub workflow evidence. No image publication or visibility change is authorized.

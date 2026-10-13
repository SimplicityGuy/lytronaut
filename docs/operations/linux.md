# Private Linux operation

Build locally for your server architecture; no registry publication is configured:

```sh
docker build --platform linux/arm64 -t lytronaut:local \
  --build-arg SOURCE_REVISION="$(git rev-parse HEAD)" .
# Use linux/amd64 instead on an x86-64 server.
docker compose up -d --no-build
```

That starts the default SQLite deployment. To keep the catalog in PostgreSQL
instead, add one overlay (see [Choosing a catalog backend](#choosing-a-catalog-backend)):

```sh
docker compose -f compose.yaml -f compose.postgres.yaml up -d --no-build           # bundled server
docker compose -f compose.yaml -f compose.postgres-external.yaml up -d --no-build  # existing server
```

The image builds locked Rust 1.99.0 native and Rust/WASM code plus matching
wasm-bindgen 0.2.129 glue inside the target Linux architecture. An emulator may
build/run the other architecture. The runtime is CPU-only: it installs no Python,
C#, FFmpeg, browser or GPU devices. Optional browser GPU and WebCodecs APIs
are probed automatically and retain supported CPU/static fallback behavior.
The compiler, source checkout and network are needed for installation, not for
subsequent startup, import, browse, focus, export or restore. The image contains
the exact Cargo.lock, resolved Rust redistribution notices and Debian package
copyright files in `/usr/share/doc`; runtime proof records the binary/client/lock
SHA256s and complete installed Debian package/version inventory. Keep the local image
and volume backups when operating offline. Runtime traffic is only between your
browser and service; there is no cloud dependency.

Open `http://127.0.0.1:3000` directly, then select a copied library directory or
ZIP. No account or password is required. Keep access private. For remote browser
use, place an HTTPS reverse proxy in front of loopback/private ingress and set
`LYTRONAUT_ORIGIN=https://library.example.test`. Preserve that exact Host;
forward browser Origin unchanged. Configure your chosen ingress access policy
at the proxy. Do not expose the container directly to the public Internet based
on the application request guard. Forwarded user headers grant no application
privileges. HTTPS is required for remote secure-context browser APIs.

The service runs as UID/GID 10001 with all capabilities dropped, no new
privileges and a read-only container filesystem. Three persistent locations
are writable:

| Mount | Role | Environment | Host selection |
| --- | --- | --- | --- |
| `/data/catalog` | Application SQLite and sidecars (private service state with PostgreSQL) | `LYTRONAUT_DATA` | `LYTRONAUT_CATALOG_DIR` |
| `/data/originals` | Byte-exact sealed imported originals | `LYTRONAUT_ORIGINALS` | `LYTRONAUT_ORIGINALS_DIR` |
| `/data/work` | Upload staging, export artifacts, isolated restores/cache | `LYTRONAUT_STAGING=/data/work/staging` | `LYTRONAUT_WORK_DIR` |
| `/var/lib/postgresql` (bundled PostgreSQL only) | PostgreSQL cluster (`18/docker`) | `PGDATA` | `LYTRONAUT_POSTGRES_DIR` |

Uploaded originals always live on this filesystem, never in database columns,
whichever catalog backend you choose. If a host-selection variable is unset,
Compose uses a project named volume; the image creates private owned directories
so new named volumes inherit mode 0700 and ownership. To choose locations on your
Linux server, precreate dedicated directories with the exact owner and mode:

```sh
sudo install -d -m 0700 -o 10001 -g 10001 \
  /srv/lytronaut/catalog /mnt/photos/lytronaut-originals /srv/lytronaut/work
# Bundled PostgreSQL only: the official Alpine image's postgres user is 70:70.
sudo install -d -m 0700 -o 70 -g 70 /srv/lytronaut/postgres
```

Then set absolute paths in a private `.env` file beside `compose.yaml` (it is
ignored by git), or export them in the shell that runs Compose:

```sh
LYTRONAUT_CATALOG_DIR=/srv/lytronaut/catalog
LYTRONAUT_ORIGINALS_DIR=/mnt/photos/lytronaut-originals
LYTRONAUT_WORK_DIR=/srv/lytronaut/work
LYTRONAUT_POSTGRES_DIR=/srv/lytronaut/postgres
```

A `compose.override.yaml` that replaces a mount by its container destination
works as well. Check the resulting mounts with `docker compose config`, then start
the service. The service never changes ownership or modes of existing
directories; a catalog directory with group/world access makes startup fail, and
a directory it cannot write fails the corresponding startup or request. Files uploaded through the
browser are copied into the originals location; catalog metadata lives in the
catalog location (or PostgreSQL) and temporary upload/export work lives in the
work location. On hosts with SELinux, apply the appropriate private container
volume label to these dedicated directories. Never mount the source library as application data.
Imported source files are copied; originals are never edited. Staging and export
storage share the work mount so restore/upload scratch has an explicit writable
boundary. Do not launch two services against the same volume set or the same
PostgreSQL schema.

Compose caps CPU at 2, memory at 2 GiB and PIDs at 128; logs rotate after three
10 MiB files. Import permits two jobs/one decoder, a 2 GiB upload, a 512 MiB
individual file, 3 GiB encoded input and an 8 GiB aggregate reservation. At least
2 GiB must remain free. These are application admission budgets, not filesystem
quotas: use server volume quotas and capacity monitoring for durable originals
and catalog growth. Increase budgets deliberately for larger imports. These
budgets do not bound the portable backup bundle: a library of any size can be
backed up and restored when the work location has the free space it needs (see
[Backup bundle size and free space](#backup-bundle-size-and-free-space)).
Resource exhaustion rejects requests while preserving already stored originals.
No GPU device or hardware acceleration runtime mount is required.

The native `--healthcheck` probes `/healthz` with the configured Host using a
bounded local TCP request. Readiness requires a live application database pool;
startup must finish storage recovery, catalog/API migration and loading generated
client assets before the listener opens. Recovery rehashes sealed originals; it
does not skip integrity verification to meet a readiness deadline. Docker checks
every 10 seconds, with a 30-second initial grace period. Increase
`healthcheck.start_period` in your Compose override if measured recovery of your
existing library takes longer; synthetic smoke timings do not establish a bound
for a large library.
`LYTRONAUT_LISTEN` is a numeric IPv4/IPv6 socket address; wildcard probes use the
matching loopback family. SIGTERM stops admission, cancels intake and background
exports/restores, then drains accepted HTTP work and closes the database pool. Drain and
cleanup together are bounded to 20 seconds. A timeout exits with an error without
closing the database under surviving workers. Compose allows 30 seconds; after interrupted
shutdown durable recovery marks unfinished work failed on restart rather than
offering partial artifacts. Stop your service with `docker compose stop`; do not
remove volumes when restarting.

If the log reports `Lytronaut job <id>: owned cleanup is blocked`, the service is
running but that export/restore job holds its files and staging quota because the
application cannot prove it owns what remains (for example an unrelated file named
like the job under `staging.exports`, a permission change, or a database object
that depends on the job's private restore schema). Remove or fix the conflict
yourself, then `DELETE /exports/<id>` or restart; the job's quota is released only
after its owned files and schema are gone. A startup failure instead means the
whole store is unavailable (export work directory or a restore journal unreadable,
or the database unreachable, or a job's blocked/unverified state could not be
recorded).

If the log reports `Lytronaut job <id>: its published restore result failed
verification and is no longer ready`, a previously completed restore copy no longer
matches its ownership record. Startup checks only ownership, not the restored data
itself: the restore journal must exist and record publication; the
`staging.exports/<id>.restored` directory must exist with private permissions and
contain the matching private `restore-owner` marker; and on PostgreSQL the job's
`lytronaut_restore_*` schema must exist with a `restore_ownership` row naming this
job and nonce. Typical causes are changed directory permissions, a work volume or
database restored from an older snapshot, or a dropped or edited restore schema or
marker row. Restored tables and rows are not re-checked at startup. The job now reads
failed (`storage_unavailable`) and nothing it holds is deleted or released
automatically. Inspect `staging.exports/<id>.restored`; if the copy is still wanted,
take it out of the application's work area yourself, then `DELETE /exports/<id>`.
DELETE removes the copy and schema only when ownership can still be proved; without
a journal it refuses, and you remove the leftovers manually before deleting the job.

`Lytronaut job <id>: a previously ready restore has no database record` means the
database is older than the work volume. The job is re-created as failed
(`storage_unavailable`) with its staging quota counted and is handled the same way.

Use the workspace's **Back up originals and catalog** action and download its
private bundle. Restore uploads into a separate store inside the work volume;
verify that copy before choosing a stopped-service volume replacement. A bundle
contains all preserved originals and catalog associations, excludes pending jobs
and contains no credentials. Legacy authentication schemas are strictly validated
and removed from the isolated copy. Opening an older application catalog removes
its obsolete account/session tables. Startup also unlinks the former app-owned
`bootstrap.secret` in the application data directory, without following symlinks;
a directory at that fixed filename prevents startup. No source-library database
or original source bytes are changed. Keep bundles private and on a separate disk.
For a full operational volume backup, stop this Compose service and snapshot all
three volumes together. Retain originals/catalog consistency; never use an active
SQLite database file without its journal/sidecars. Restart the same volumes to
resume browsing. Do not delete originals merely to clear staging or free space.

## Choosing a catalog backend

| | SQLite (default `compose.yaml`) | PostgreSQL (`compose.postgres.yaml` or `compose.postgres-external.yaml`) |
| --- | --- | --- |
| Moving parts | One container, one file in the catalog location | A PostgreSQL server (bundled container or your existing server) |
| Credentials | None | One generated database password, mounted as a secret file |
| Backups | Portable bundle, or stopped-service snapshot of the three locations | Portable bundle, or `pg_dump` plus snapshot of originals/work |
| Choose when | A single private server; simplest operation | You already operate PostgreSQL, or want its tooling for backup/monitoring |

Both backends keep the same observable catalog, ordering, cancellation, quota and
recovery behavior, and a portable bundle restores into either. Original files
are on the filesystem in both cases. Database credentials are deployment
configuration only: there are no application accounts or logins.

The supported PostgreSQL release is **18.6**, the latest stable release on the
[official versioning page](https://www.postgresql.org/support/versioning/)
(verified 2026-10-13; PostgreSQL 19 is still in beta). The bundled service and all
tests use the official image pinned by release and immutable multi-architecture
index digest
`postgres:18.6-alpine@sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873`,
which includes linux/amd64 and linux/arm64. An external server must run
PostgreSQL 18: at startup the application reads `server_version_num` and refuses
any other major release with the fixed message "unsupported PostgreSQL server
version; Lytronaut requires PostgreSQL 18", before migrating anything.

### Database password secret

The application reads its PostgreSQL password from the file named by
`LYTRONAUT_DATABASE_PASSWORD_FILE`; the database URL (`LYTRONAUT_DATABASE_URL`)
then carries no password. Compose mounts the file as a secret, so the password is
not in `compose.yaml`, the image, the container environment, `docker inspect` or
`docker compose config`. Startup and connection errors are fixed messages that
never include the URL, password or file path. The file holds one line of 1 to
1024 bytes of UTF-8 without control characters (Unicode Cc, including multi-byte
C1 controls such as U+0085); a single final LF or CRLF is ignored. The application
and the bundled role bootstrap apply exactly these rules; a URL that already has a password
together with a password file is rejected as ambiguous. Compose outside Swarm
ignores secret `uid`/`gid`/`mode`, so set them on the host file itself.

Keep secrets in a private directory outside the checkout, for example:

```sh
sudo install -d -m 0700 /etc/lytronaut/secrets
python3 -c 'import secrets; print(secrets.token_urlsafe(32), end="")' |
  sudo install -m 0440 -o 10001 -g 70 /dev/stdin /etc/lytronaut/secrets/lytronaut-database-password
python3 -c 'import secrets; print(secrets.token_urlsafe(32), end="")' |
  sudo install -m 0400 -o 70 -g 70 /dev/stdin /etc/lytronaut/secrets/postgres-bootstrap-password
```

The application (10001) and the bundled PostgreSQL initialization (group 70) read
`lytronaut-database-password`; only PostgreSQL reads the superuser bootstrap
password, which the application never uses. For an external server, only the
first file is needed (owner 10001, mode 0400 is sufficient). Set
`LYTRONAUT_SECRETS_DIR=/etc/lytronaut/secrets` in `.env`.

### Bundled PostgreSQL service

```sh
docker compose -f compose.yaml -f compose.postgres.yaml up -d --no-build
```

The `postgres` service runs the pinned official image as UID/GID 70 with a
read-only root filesystem, all capabilities dropped, 1 CPU, 1 GiB memory and 256
PIDs, on an internal network with no published port and no outside route; only
the application joins that network. On first start with an empty
`LYTRONAUT_POSTGRES_DIR`, `deploy/postgres/initdb/10-lytronaut-role.sh` creates
the restricted login role `lytronaut` (no superuser, create-database, create-role,
replication or row-security bypass), revokes `CREATE` on `public`, grants
`CONNECT, CREATE, TEMPORARY` on database `lytronaut` (database `CREATE` lets the
application create isolated restore schemas), and creates schema `lytronaut`
owned by that role. PostgreSQL reads the secret file itself (`pg_read_file`), so
the password is never in a command line, environment variable or SQL text, and
statements are kept out of the server log. It never runs again on an
initialized cluster: changing the secret file afterwards does not change the role
password (use `ALTER ROLE lytronaut PASSWORD ...` as the superuser, then restart the
application).

Startup ordering: the PostgreSQL health check uses TCP, which the image only
opens after initialization completes, so the application starts only once the
role and schema exist. When Compose restarts PostgreSQL (`docker compose restart
postgres`, or `up` recreating it), Compose also restarts the application. If the
PostgreSQL server itself crashes, the engine's `restart: unless-stopped` policy
restarts it and the running application reconnects on its next database use;
both cases are exercised by `just deployment-test`. Both services use
`restart: unless-stopped`. PostgreSQL receives
SIGINT (fast shutdown) and has 60 seconds; the application has 30 seconds.

### External PostgreSQL server

Prepare a PostgreSQL 18 database once as an administrator, using a generated
password (psql prompts for it, so it is not in shell history):

```sql
CREATE ROLE lytronaut LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
\password lytronaut
CREATE DATABASE lytronaut;
\connect lytronaut
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON DATABASE lytronaut FROM PUBLIC;
GRANT CONNECT, CREATE, TEMPORARY ON DATABASE lytronaut TO lytronaut;
CREATE SCHEMA lytronaut AUTHORIZATION lytronaut;
```

Store the same password in `lytronaut-database-password` and set, in `.env`:

```sh
LYTRONAUT_DATABASE_URL=postgres://lytronaut@db.example.internal:5432/lytronaut?sslmode=verify-full
LYTRONAUT_SECRETS_DIR=/etc/lytronaut/secrets
```

then `docker compose -f compose.yaml -f compose.postgres-external.yaml up -d --no-build`.
Use `sslmode=verify-full` across any network you do not fully control;
`sslmode=disable` is only appropriate on a private host network. The URL accepts
only connection parameters (host, port, database, user, TLS options,
`application_name`, `options`); unknown parameters are rejected before connecting.
`LYTRONAUT_DATABASE_SCHEMA` (default `lytronaut`) selects the owned schema; the
application never falls back to `public`. The application container must be able
to reach the server (for a server on another Docker network, attach the
`lytronaut` service to that network in a `compose.override.yaml`).

### Verifying a restored copy

A restore never replaces the active catalog. It creates an isolated copy inside
the work location (`staging.exports/<job>.restored/store`), with the restored
catalog in a SQLite file there or, on PostgreSQL, in a new schema named in that
directory's `restore.json`. To inspect the copy, stop normal use and start the
same service on it with a temporary override, for example `verify.yaml`:

```yaml
services:
  lytronaut:
    environment:
      LYTRONAUT_DATA: /data/work/staging.exports/JOB.restored/store
      LYTRONAUT_ORIGINALS: /data/work/staging.exports/JOB.restored/store/originals
      LYTRONAUT_STAGING: /data/work/staging.exports/JOB.restored/store/staging
```

```sh
docker compose -f compose.yaml [-f compose.postgres.yaml] -f verify.yaml up -d --no-build --force-recreate lytronaut
# PostgreSQL: also set LYTRONAUT_DATABASE_SCHEMA to the "schema" in restore.json.
```

Browse it, then recreate the service without `verify.yaml` to return to the
active catalog. Keep or adopt the copy deliberately: with SQLite, stop the service
and replace the three locations with the verified copy; with PostgreSQL, keep
`LYTRONAUT_DATABASE_SCHEMA` pointing at the restored schema and use the copy's
originals. Otherwise `DELETE /exports/<job>` removes it. A bundle from either
backend restores into either backend.

### PostgreSQL backup, recovery and upgrades

The portable bundle is the backend-independent backup and the way to move a
library between SQLite and PostgreSQL or between PostgreSQL major versions, for a
library of any size given the [free work space](#backup-bundle-size-and-free-space)
it needs; `pg_dump`/`pg_restore` plus the file locations remains an alternative
between PostgreSQL major versions. For an operational backup of a PostgreSQL deployment, stop the
application (leave PostgreSQL running), then take a consistent dump and snapshot
originals and work together:

```sh
docker compose -f compose.yaml -f compose.postgres.yaml stop lytronaut
docker compose -f compose.yaml -f compose.postgres.yaml exec -T postgres \
  pg_dump -U postgres -d lytronaut --schema='lytronaut*' --format=custom > lytronaut-catalog.dump
# snapshot LYTRONAUT_ORIGINALS_DIR, LYTRONAUT_WORK_DIR and LYTRONAUT_CATALOG_DIR now
docker compose -f compose.yaml -f compose.postgres.yaml start lytronaut
```

`--schema='lytronaut*'` dumps exactly the application's schemas: `lytronaut` and
its isolated `lytronaut_restore_*` copies. Anything else in the database (for
example administrator-owned objects in `public`) is not application data and
could not be restored as the restricted role. To recover the bundled service onto
an empty `LYTRONAUT_POSTGRES_DIR` (same secrets), put back the matching file
snapshot, start only PostgreSQL so its one-time bootstrap recreates the role and
an empty `lytronaut` schema, drop that empty schema (plain `DROP SCHEMA` refuses if
it is not empty), restore the dump as the role through `exec -T` (the service has
no published port), then start the application:

```sh
C="docker compose -f compose.yaml -f compose.postgres.yaml"
$C up -d --no-build --wait postgres
$C exec -T postgres psql -U postgres -d lytronaut -v ON_ERROR_STOP=1 -c 'DROP SCHEMA lytronaut'
$C exec -T postgres pg_restore -U postgres -d lytronaut --no-owner --no-acl \
  --role=lytronaut --exit-on-error < lytronaut-catalog.dump
$C up -d --no-build
```

For an external server, dump the same `--schema='lytronaut*'` with your
administrator connection; because only the application's schemas are dumped,
objects that administrators or other tools own in `public` (or elsewhere) neither
enter the dump nor block the restore, so the procedure works on any PostgreSQL 18
server. Prepare an empty
database with the role and grants from [External PostgreSQL server](#external-postgresql-server)
but without creating the schema, run the same `pg_restore ... --role=lytronaut`
into it, and point `LYTRONAUT_DATABASE_URL` at that database. `just
deployment-test` runs both procedures literally and browses the recovered catalog.

A stopped-service copy of `LYTRONAUT_POSTGRES_DIR` is also consistent; never copy
it while PostgreSQL runs.

Minor PostgreSQL releases (18.x) keep the on-disk format: update the pinned
release and digest after verifying them from the official sources, then
`docker compose ... up -d --no-build`. A new major release cannot open an older
cluster directory, and the application refuses a server other than PostgreSQL 18
until a release supports it. Upgrade by creating a new deployment with an empty
PostgreSQL location and restoring a portable bundle or a
`pg_dump` into it;
keep the old location until the new deployment is verified. Application upgrades
migrate the catalog schema additively on startup on either backend.

### Backup bundle size and free space

A portable bundle carries every unique original plus a logical catalog of at most
288 MiB, in one `.lytronaut` file of any size (sizes are 64-bit; there is no 4 GiB
boundary). Creating, downloading, uploading and restoring it stream through a
fixed 64 KiB buffer, so memory use does not grow with the library. The
single-upload budgets (`LYTRONAUT_UPLOAD_BYTES`, `LYTRONAUT_ENCODED_BYTES`) and the
aggregate `LYTRONAUT_RESERVED_BYTES` do not limit it; only free space in the work
location does. For `N` bytes of unique originals:

- Creating a bundle needs about `N` + 576 MiB free, plus 80 bytes per unique
  original (the bundle itself plus the catalog and inventory spools beside it),
  in addition to the reservations of other running exports and
  `LYTRONAUT_FREE_FLOOR` (2 GiB with Compose).
- Restoring a bundle of `B` bytes needs about 2 x `B` + 288 MiB free (the upload
  and the extracted copy, plus room for a SQLite catalog), with the same additions.
- A refused job fails immediately with a retryable `resource_limit` error whose
  `shortfall` reports `required_bytes` and `available_bytes` (the browser shows
  both, for example "this needs 5.0 GiB and 1.0 GiB is available"); free space
  (or wait for other exports to finish) and retry. Nothing is created for a
  refused job.

Each original must still be at most `LYTRONAUT_FILE_BYTES` (512 MiB with Compose),
which the store also enforces when it reads an original; restore a bundle into a
deployment configured at least as large as the one that created it. The number of
unique originals is not limited by `LYTRONAUT_UPLOAD_FILES` or any other budget.
The logical catalog (every capture's records, albums, memberships and source
paths) must fit the bundle's 288 MiB catalog section: about 33,800 captures,
measured by backing up a generous synthetic Desktop library of 1,000 captures
with seven files each through the real import (about 8.9 KB per capture). A
larger catalog fails when the backup job runs, with a non-retryable
`resource_limit` job error and no bundle file, never a partial one; use
stopped-service volume snapshots (SQLite) or `pg_dump` plus file snapshots
(PostgreSQL) for such a library. Splitting the catalog into chunks is a possible
later change. A ready bundle or restore keeps its space until you `DELETE` it.
Interrupted transfers are not resumed; start the download or upload again.

### Limitations

- A single Lytro Desktop library database is read semantically only up to
  100,000 table cells (about 1,700 captures with the Desktop schema's usual
  columns); a larger one is preserved byte-exact but imported as preserved-only.
  This is a known limit of the Desktop adapter, tracked separately.
- A portable bundle's logical catalog holds about 33,800 captures; see
  [Backup bundle size and free space](#backup-bundle-size-and-free-space).
- One application instance per catalog: do not run two services against the same
  SQLite location or PostgreSQL schema.
- A portable bundle needs free work space of about its own size to create, and
  about twice its size to restore; see
  [Backup bundle size and free space](#backup-bundle-size-and-free-space).
- No application accounts or login: protect access at your proxy; database
  credentials are not user accounts.
- Application reservations bound staging files, not PostgreSQL storage: monitor
  database size and WAL growth (including isolated restore schemas) yourself.
- The bundled service publishes no port. Administer it with
  `docker compose exec postgres psql -U postgres -d lytronaut`.
- Images are built locally; nothing is published to a registry.

## Deployment validation

The Justfile runs the same synthetic deployment proof locally and in CI:

```sh
revision=$(git rev-parse HEAD)
mise exec -- just container-build arm64 lytronaut:local lytronaut:smoke-relay "$revision"
mise exec -- just synthetic-fixtures arm64 /tmp/lytronaut-fixtures
mise exec -- just deployment-test arm64 lytronaut:local bind /tmp/lytronaut-fixtures /tmp/lytronaut-evidence
mise exec -- just postgres-parity
```

`deployment-test` starts the default SQLite Compose path, the bundled PostgreSQL
path and an externally managed PostgreSQL configured only by URL and secret file;
imports a synthetic library; checks owners and modes, graceful stop, down/up and
restart-policy restarts on the same storage, that a Compose restart of PostgreSQL
restarts the application and that a PostgreSQL server crash is recovered, a
CRLF-terminated password secret, rejection of a multi-byte control character by
both the application and the role bootstrap, the documented `pg_dump`/`pg_restore` recovery
for the bundled and the external server (the external one beside an
administrator-owned `public` table; each browsed afterwards), fail-closed
redacted startup with a wrong or ambiguous password, and that no
generated password appears in Compose configuration, container configuration,
logs, the image or the report; and restores all four SQLite/PostgreSQL bundle
combinations, starting and browsing each restored copy through the override
above. `bind` prepares host directories with the documented owners and requires a
Linux host whose filesystem the engine uses directly; on a VM-backed engine (for
example Docker on macOS) use `volume`, which checks the same owners and modes
inside the engine's named volumes. The report records the host, engine and image
architectures and whether the image ran under emulation. Every container, volume,
network and directory it creates has a unique prefix (`LYTRONAUT_TEST_PREFIX`) and
is removed afterwards.

`postgres-parity` starts a disposable container of the pinned release with a
restricted role and a public sentinel, asserts `server_version_num`, runs the
strict `postgres-test` suite (exact per-group counts; zero failed or ignored
tests; no SQLite fallback) and removes the container and its volume.
`postgres-test` alone requires an explicit disposable `LYTRONAUT_TEST_POSTGRES_URL`.

`python3 scripts/container-smoke.py IMAGE PLATFORM` runs synthetic HTTP runtime
checks against a prebuilt image. Python is host test tooling, not image runtime.
First build `lytronaut:smoke-relay` with
`docker build --platform PLATFORM -t lytronaut:smoke-relay -f scripts/Dockerfile.smoke-relay scripts`.
This test-only Rust ingress relay connects the host runner to the service's
internal network. The application has no external route; the relay forwards only
to that fixed service and is absent from the application image.
See the bead's validation record for measured architecture and artifact evidence.

The Justfile shares the container build and proof commands with CI. On a native
Linux host, choose an architecture, local image names, exact source revision and
an evidence output directory explicitly:

```sh
mise exec -- cargo build --locked -p lytronaut-service --example synthetic_jpeg
revision=$(git rev-parse HEAD)
mise exec -- just container-build amd64 lytronaut:local lytronaut:smoke-relay "$revision"
mise exec -- just container-artifacts amd64 lytronaut:local /tmp/lytronaut-evidence
mise exec -- just container-smoke amd64 lytronaut:local lytronaut:smoke-relay "$revision" /tmp/lytronaut-evidence
mise exec -- just container-evidence amd64 "$revision" /tmp/lytronaut-evidence
mise exec -- just container-bundle /tmp/lytronaut-evidence /tmp/lytronaut-evidence.tar.gz
```

Use `arm64` on a native arm64 Linux host. Runtime smoke may also run under declared
emulation; the evidence verifier deliberately requires a matching native Linux
runner. CI stores only programs, the saved image, build logs and synthetic
aggregate results, with a 128 MiB archive cap and three-day artifact retention.
It retains bounded failed-run evidence separately from a verified passing report.

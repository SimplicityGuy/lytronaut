# Private Linux operation

Build locally for your server architecture; no registry publication is configured:

```sh
docker build --platform linux/arm64 -t lytronaut:local \
  --build-arg SOURCE_REVISION="$(git rev-parse HEAD)" .
# Use linux/amd64 instead on an x86-64 server.
docker compose up -d --no-build
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
privileges and a read-only container filesystem. Three persistent named volumes
are writable:

| Mount | Role | Environment |
| --- | --- | --- |
| `/data/catalog` | Application SQLite and sidecars | `LYTRONAUT_DATA` |
| `/data/originals` | Byte-exact sealed imported originals | `LYTRONAUT_ORIGINALS` |
| `/data/work` | Upload staging, export artifacts, isolated restores/cache | `LYTRONAUT_STAGING=/data/work/staging` |

The image creates private owned directories so new named volumes inherit mode
0700 and ownership. For bind mounts, precreate the three directories owned by
10001:10001 with mode 0700. To choose locations on your Linux server:

```sh
sudo install -d -m 0700 -o 10001 -g 10001 \
  /srv/lytronaut/catalog /mnt/photos/lytronaut-originals /srv/lytronaut/work
```

Create `compose.override.yaml` beside `compose.yaml`:

```yaml
services:
  lytronaut:
    volumes:
      - /srv/lytronaut/catalog:/data/catalog
      - /mnt/photos/lytronaut-originals:/data/originals
      - /srv/lytronaut/work:/data/work
```

Compose replaces each named mount by its matching container destination. Check
the resulting mounts with `docker compose config`, then start the service. Files
uploaded through the browser are copied into the originals location; catalog
metadata lives in the catalog location and temporary upload/export work lives in
the work location. On hosts with SELinux, apply the appropriate private container
volume label to these dedicated directories. Never mount the source library as application data.
Imported source files are copied; originals are never edited. Staging and export
storage share the work mount so SQLite restore/upload scratch has an explicit
writable boundary. Do not launch two services against the same volume set.

Compose caps CPU at 2, memory at 2 GiB and PIDs at 128; logs rotate after three
10 MiB files. Import permits two jobs/one decoder, a 2 GiB upload, a 512 MiB
individual file, 3 GiB encoded input and an 8 GiB aggregate reservation. At least
2 GiB must remain free. These are application admission budgets, not filesystem
quotas: use server volume quotas and capacity monitoring for durable originals
and catalog growth. Increase budgets deliberately for larger libraries.
Resource exhaustion rejects requests while preserving already stored originals.
No GPU device or hardware acceleration runtime mount is required.

The native `--healthcheck` probes `/healthz` with the configured Host using a
bounded local TCP request. Readiness requires a live application SQLite pool;
startup must finish storage recovery, catalog/API migration and loading generated
client assets before the listener opens. Recovery rehashes sealed originals; it
does not skip integrity verification to meet a readiness deadline. Docker checks
every 10 seconds, with a 30-second initial grace period. Increase
`healthcheck.start_period` in your Compose override if measured recovery of your
existing library takes longer; synthetic smoke timings do not establish a bound
for a large library.
`LYTRONAUT_LISTEN` is a numeric IPv4/IPv6 socket address; wildcard probes use the
matching loopback family. SIGTERM stops admission, cancels intake and background
exports/restores, then drains accepted HTTP work and closes SQLite. Drain and
cleanup together are bounded to 20 seconds. A timeout exits with an error without
closing SQLite under surviving workers. Compose allows 30 seconds; after interrupted
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
or the database unreachable).

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

# Login-free Linux deployment validation

Checkpoint status: **arm64 runtime complete; amd64 and pristine submission
pending**. The amd64 emulated build was cancelled when measured host free space
crossed the required 2 GiB floor. It produced no validated amd64 image or passing
runtime report. The owned builder was removed only after all compiler/build work
was terminal. The guest filesystem still had about 66 GiB available, while the
host had substantial swap usage; the peak's cause remains unproven. Rebuilding
requires verified host headroom. After recovery to 11 GiB free, a second build
started in a new isolated builder with verified 3 GiB memory, zero guest swap
and two CPU limits. The product Dockerfile is unchanged. This bead remains in
progress; that build and the amd64 runtime are not yet proven.

The application and container configuration were built from
`4950b8a4b9001c22eeb77f7d44b7d682104025dd`, based on integration/design commit
`1211f70a7ea9601e47cbcad413e248767a9554c2`. Later validation documentation does
not change those application sources. The architecture reports bind the image
revision, actual binary, generated Rust/WASM, glue and Cargo.lock SHA256s.
All committed fixtures and reports are independently generated synthetic data.
The [completed arm64 report](assets/lytronaut-j2u-arm64.json) records all 15
passing runtime checks. Final browser release-client binding, amd64 execution and
the pristine gate receipt must be added before submission.
The [source manifest](assets/lytronaut-j2u-source.json) binds application and
container configuration files. The [cancelled-build checkpoint](assets/lytronaut-j2u-cancelled-build.json)
records the resource observations separately from successful runtime evidence.

Rust 1.99.0, wasm-bindgen 0.2.129 and locked dependencies build inside each
Linux architecture. The local engine is Colima Docker 29.5.2 on aarch64 with
BuildKit 0.30. Arm64 runs natively; the pending amd64 run will use declared
emulation.
No registry publication is part of this validation. Compiled programs, raw and
generated WASM, glue, image archives and provenance are retained outside the
disposable developer worktree before handoff.

The runtime smoke uses an actual nonroot container with a read-only root,
three persistent mounted directories, dropped capabilities and no new
privileges. It checks enforced CPU, memory and PID limits, failed writes outside
mounts, successful writes inside them and the installed package inventory.
The service has no external network route. A separate test-only Rust ingress
relay forwards to its fixed service destination; it is absent from the product
image. Neither a login nor credentials are used for startup or HTTP operations.
Foreign Origin and invalid Host requests fail, and oversized input fails
admission without changing stored originals.

The smoke exercised on arm64 imports a 30-capture synthetic library, browses it, reads
byte-exact preserved originals and renders CPU focus output. It exports a
flattened PNG and a lossless catalog/original bundle, restores into an isolated
store, starts that restored store and browses it. Stopping and restarting with
the same volumes preserves the library. An interrupted private flat catalog
snapshot is removed by exclusive recovery. Tests observe a running archive
worker before SIGTERM, interrupt an upload with a stalled body and interrupt
a stalled JSON body; restart retains the existing library and recovers durable
work state. IPv6 wildcard listening and matching-family health probes run in
the container; malformed listeners, missing listeners and mismatched configured
Hosts fail the bounded probe.

The arm64 synthetic startup/recovery measurements, including container launch
and normal integrity verification, are 0.261–0.498 seconds. Its active archive
shutdown takes 0.178 seconds and stalled upload shutdown 0.134 seconds. A stalled
JSON body reaches the 20.104-second bounded interruption path, exits with code
1 rather than SIGKILL, then successfully reopens and browses the same volumes.
These synthetic timings do not bound recovery of a large existing library.

Earlier runtime iterations exposed a real read-only-root regression: desktop
SQLite snapshots used the system temporary directory. Snapshot files now live
inside the application's private staging mount. A native regression and the
actual container verify immutable originals, cleanup on normal drop and cleanup
after interrupted exclusive recovery. Preliminary harness failures from Docker
internal-network ingress, selecting the wrong sorted capture and assuming the
wrong progress response shape were corrected before the passing arm64 report;
those runs are not counted as successful validation.

Native tests cover legacy authentication migration, fixed obsolete bootstrap
artifact handling, strict untrusted backup schemas, bundle restore, request
boundaries and interrupted snapshot recovery. Earlier debug-WASM browser checks exercised direct
entry and import/browse/focus/export/restore without an authentication UI, plus
automatic GPU/static fallback, video joins, cancellation and owned URL cleanup.
Final release-client browser binding is still pending.
The operations guide documents concrete host bind mounts, UID/GID permissions,
reverse-proxy Host/Origin handling, explicit credentialless CLI mutation intent,
offline image retention and stopped-service backups.

This deployment proof does not establish the final release's 100 ms performance
or 95% native plus actual WASM coverage gates. The final acceptance bead owns
those source-inclusive checks after the remaining initial-release integrations.

# Login-free Linux deployment validation

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

Checkpoint status: **native amd64 and arm64 runtime and release-client browser
checks complete; updated CI and pristine submission pending**. The first amd64
emulated build was cancelled when measured host free space
crossed the required 2 GiB floor. It produced no validated amd64 image or passing
runtime report. The owned builder was removed only after all compiler/build work
was terminal. The guest filesystem still had about 66 GiB available, while the
host had substantial swap usage; the peak's cause remains unproven. Rebuilding
requires verified host headroom. After recovery to 11 GiB free, a second build
started in a new isolated builder with verified 3 GiB memory, zero guest swap
and two CPU limits. That build failed while installing wasm-bindgen: GCC crashed
with SIGSEGV compiling ring's public x86_64 assembly. Its cgroup recorded no OOM
or OOM kill. The exact compiler command subsequently passed three times in an
isolated container with the same pinned base and limits; the original failure's
cause remains unproven. No emulated amd64 runtime pass is claimed. The product
Dockerfile is unchanged. Native amd64 and arm64 GitHub runners subsequently
built and passed all runtime checks through a private draft PR, without registry
publication. Their independently verified reports supersede the failed local
amd64 attempts as successful architecture evidence.

The application and container configuration were built from
`4950b8a4b9001c22eeb77f7d44b7d682104025dd`, based on integration/design commit
`1211f70a7ea9601e47cbcad413e248767a9554c2`. Later validation documentation does
not change those application sources. The architecture reports bind the image
revision, actual binary, generated Rust/WASM, glue and Cargo.lock SHA256s.
All committed fixtures and reports are independently generated synthetic data.
The [completed arm64 report](assets/lytronaut-j2u-arm64.json) records all 15
passing runtime checks. Native CI at `821727e46fa2ccda5c68618ff99d1551eed01411`
passed on both architectures in
[run 38204634077](https://github.com/SimplicityGuy/lytronaut/actions/runs/38204634077):
[amd64](assets/lytronaut-j2u-native-amd64.json) and
[arm64](assets/lytronaut-j2u-native-arm64.json), with 15 checks each. Both artifacts
are retained outside the worktree, with independently verified GitHub ZIP
digests, 166 source hashes, every program hash, native ELF architecture and saved
image identity plus every filesystem layer. The final pristine submission
receipt and updated harness CI remain pending.
The [artifact identities](assets/lytronaut-j2u-native-identities.json) retain the
job/artifact IDs, ZIP SHA256s, image hashes, exercised program hashes and exact
source hashes independently of GitHub's three-day artifact retention.
The [source manifest](assets/lytronaut-j2u-source.json) binds application and
container configuration files. The [cancelled-build checkpoint](assets/lytronaut-j2u-cancelled-build.json)
records the resource observations separately from successful runtime evidence.

Rust 1.99.0, wasm-bindgen 0.2.129 and locked dependencies build inside each
Linux architecture. The initial local engine inventory was Colima Docker
29.5.2 on aarch64 with BuildKit 0.30; the later isolated bounded
builder reports BuildKit 0.33.1. The historical arm64 smoke runs natively;
failed local amd64 builds used declared emulation. The replacement CI matrix
uses native Ubuntu runners for each architecture.
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

The smoke exercised on each architecture imports a 30-capture synthetic library,
browses it, reads
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

The earlier local arm64 synthetic startup/recovery measurements, including container launch
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
The final functional browser checks passed CPU and GPU modes for each distinct
Linux release WASM/glue artifact with a macOS debug service compiled from identical
application sources. Their reports bind the actual native service, WASM and
generated glue hashes: arm64-built [CPU](assets/lytronaut-j2u-browser-cpu.json) and
[GPU](assets/lytronaut-j2u-browser-gpu.json), and amd64-built
[CPU](assets/lytronaut-j2u-browser-amd64-cpu.json) and
[GPU](assets/lytronaut-j2u-browser-amd64-gpu.json). All eight GPU orientation comparisons
have zero pixel difference, injected failures recover, video joins pass and
owned Blob URLs are released. These mixed-profile checks have the performance
gate disabled; their focus p95 measurements exceed 100 ms. The fresh native
arm64 CI client is byte-identical to the first exercised client. The amd64 CI
client has different WASM/glue bytes and therefore received its own actual CPU
and GPU browser checks. No cause for the cross-host output difference is claimed.
The amd64 browser checks record committed harness revision `24ebdfd`, including
the portable temp-directory correction; the application sources remain identical
to native CI revision `821727e`.

At clean source `821727e46fa2ccda5c68618ff99d1551eed01411`, the canonical
`bh work check` completed successfully: native and WASM lint, native tests,
workspace builds, generated debug WASM, dependency boundaries and notices for
292 resolved packages. Its exact programs, raw/generated WASM and log are frozen
outside the worktree. This check is not the final pristine submission receipt.
The first private container workflow failed parsing before any jobs ran because
`runner.temp` was used in job-level environment configuration. The corrected
workflow initializes the directory from `RUNNER_TEMP` in its first step and
passes actionlint; native CI run `38204634077` exercises the corrected source.
The separate application CI browser job in
[run 38204634036](https://github.com/SimplicityGuy/lytronaut/actions/runs/38204634036)
failed before application execution because the video harness assumed macOS's
`/private/tmp`. The harness now creates its private scratch directory using
Node's `os.tmpdir()`; syntax and a real temp-directory permission probe passed,
and the corrected harness ran both amd64-client browser checks above. No shim
or skipped CI check is used. A fresh Ubuntu CI run remains required.
The operations guide documents concrete host bind mounts, UID/GID permissions,
reverse-proxy Host/Origin handling, explicit credentialless CLI mutation intent,
offline image retention and stopped-service backups.

This deployment proof does not establish the final release's 100 ms performance
or 95% native plus actual WASM coverage gates. The final acceptance bead owns
those source-inclusive checks after the remaining initial-release integrations.

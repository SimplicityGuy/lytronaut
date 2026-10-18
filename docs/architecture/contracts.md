# Workspace and version 1 contracts

Implementation epic `lytronaut-re8` completed delivery, review, and landing.
The contracts below describe the current application; historical foundation
policy is retained in the dated design records.

| Crate | Responsibility | Target |
|---|---|---|
| lytronaut-contracts | Versioned serde DTOs and input invariants | Native and WASM |
| lytronaut-format | Independent bounded format implementations | Platform neutral |
| lytronaut-storage | SQLite/PostgreSQL catalog abstraction and durable jobs | Native |
| lytronaut-service | Axum HTTP and redacted diagnostics | Native |
| lytronaut-client | Leptos CSR application entry point | WASM |
| xtask | Dependency boundaries and license evidence | Development host |

Native default members exclude the client. Client platform dependencies are
WASM-specific; database drivers are native-specific. `just dependencies` checks actual
normal/build dependency trees for the service and client. CI independently
compiles/tests the native service and compiles and binds the WASM artifact.
Do not use `--workspace --target wasm32-unknown-unknown`: the native service is
intentionally a separate target. Optional GPU and WebCodecs dependencies belong
only to the client and retain their runtime probes and fallbacks.

## Contract rules

JSON `version` is numeric `1`. Unsupported versions and enum variants fail
closed; adding incompatible fields requires a new wire version. Unknown fields
are rejected in DTOs, independently of source formats where unknown fields must
be preserved. `OpaqueId` permits 1–128 ASCII alphanumeric, hyphen or underscore
characters and is never a filesystem path or camera identity. `ContentDigest`
is exactly 64 lowercase hexadecimal SHA-256 characters; identifiers/digests
from private sources must not appear in routine logs.

`ImportRequest` declares ZIP-library or directory intake, nonzero file count and
bytes. Declarations do not establish trust: storage must enforce actual streamed
sizes against explicit admin configuration (defaults: 20 GiB and 100,000 files).
Those configurable admission limits are separate from structural DTO validity.

`JobReport` represents queued/running/cancelling/cancelled/succeeded/failed states.
Completed bytes cannot exceed total; success requires completion, failure requires
an `ApiError`, and other states cannot carry an error. This is a snapshot contract;
durable transitions, cancellation and persistence belong to storage.

`AssetManifest` carries opaque ID, SHA-256, representation, dimensions, color
space, encoded/decoded sizes, checked contiguous chunk ranges and capabilities.
Viewer JPEGs are capped at 2048 × 2048, 48 MiB encoded and exact RGBA allocation.
Viewer JPEGs require nonempty chunk coverage. Chunks are at most 8 MiB, checked for overflow, ordered without gaps/overlap and
cover the full offered asset. At most 1024 ranges are allowed. Empty ranges permit
large preserved originals to remain outside the bounded viewer. API handlers must
also enforce the 256 KiB serialized manifest budget and concurrent-transfer
limits; these are transport rules, not permissions granted by a valid DTO.

Deserialization validates these invariants. Public struct fields allow construction
by Rust producers; producers must call `validate()` before persisting/sending.
`Dimensions` describes encoded geometry; decoder-specific allocation limits must
be checked before decompression even when a generic asset has valid dimensions.

`CapabilityState` distinguishes available, preserved-only, unverified, disabled
and unavailable with a normalized reason. Optional renderer/codec states are
independent; the current API reports them from actual catalog and runtime probes.

`ApiError` contains only version, code and retryability, plus an optional
`shortfall` (`required_bytes`, `available_bytes`) on a `resource_limit` refusal
caused by insufficient free work space. The field is additive and
backward-compatible: it is omitted whenever absent, so every other error
(including every job report) is byte-identical to the original V1 shape, and it
appears on 429 free-space admission responses of `POST /imports`,
`POST /exports` (archive) and `POST /restores`. Clients that ignore unknown fields parse it unchanged; earlier
Lytronaut browser clients never decode a 429 body. Services map internal
errors to codes and keep filesystem paths, credentials and serials out of the
wire and routine logs. Public `GET /healthz` performs an actual catalog query and returns 503 on
catalog failure, with no source metadata. The service authentication boundary
protects every application route and fallback; see
[authentication and extension contracts](authentication.md).

## Reproducible toolchain and provenance

Rust 1.99.0, Axum 0.8.9, Leptos 0.8.22 CSR, SQLx 0.9.0 (SQLite and PostgreSQL), Tokio 1.53.2,
and wasm-bindgen crate/CLI 0.2.129 are pinned. Cargo.lock records the resolved
graph. The bare `just check` entry point invokes `mise exec` itself before
running every check, so clean Beadhive review/merge checkouts do not depend on
the caller having wasm-bindgen or other pinned tools on PATH. Install mise and
the configured tools first; individual development recipes run in `mise exec`.
`CARGO_TARGET_DIR` may point to an absolute shared Cargo artifact cache for
command-local validation; `wasm-build` resolves its wasm-bindgen input through
that directory, falling back to `target` when unset. Source checkouts remain
pristine and all locked native/WASM, boundary and license checks still run.
Keep `CARGO_BUILD_JOBS=2` on constrained shared hosts.
SQLx uses bundled SQLite (a public-domain database engine); no native image
or video codec is included. The service binds loopback and uses only its own application database.
The authentication contract defines private local bootstrap state and the
exact-origin/TLS reverse-proxy deployment policy.

API sources checked against upstream crate code/docs:
[Axum Router/serve](https://docs.rs/axum/0.8.9/axum/),
[Leptos mount](https://docs.rs/leptos/0.8.22/leptos/mount/),
[SQLx SQLite](https://docs.rs/sqlx/0.9.0/sqlx/sqlite/), and
[wasm-bindgen tooling](https://wasm-bindgen.github.io/wasm-bindgen/reference/cli.html).
The dependency source manifests and successful target builds provide exact-version
evidence; no reference-project implementation is translated.

Run `just licenses-update` only after auditing graph changes. `just licenses`
checks every external resolved workspace package, including browser, build and
test dependencies, against the permissive SPDX policy and committed upstream
license/notice texts. Missing/unknown licenses or missing evidence fail closed. The three upstream
packages with no separate license file retain their explicit original manifest
declarations and standard MIT terms without inventing copyright notices;
see notices/README.md and its pinned retrieval provenance.
SPDX expressions and complete texts are retained in DEPENDENCY_LICENSES.md and
THIRD_PARTY_NOTICES.md. Development tool binaries are not redistributed with the
application. Fixture provenance lives in fixtures/synthetic/PROVENANCE.md.

The authenticated browsing, semantic import phases, typed original roles, bounded
search/pagination and immutable transfer contracts are documented in [api.md](api.md).

`FocusStack` supplies the finite ordered stored lambdas and source geometry for
explicit discrete selection. `nearest()` clamps requests to the stored endpoints;
exact midpoint ties choose the lower-index plane, and nonfinite requests fail.
Lambdas are relative vendor values, never metres. `FocusDerivative` is the typed
selection/orientation/extent/content-hash/provenance response for a newly encoded
lossy JPEG. A consumer must validate it against its selected stack and verify its
bytes and decoded geometry. GPU presentation and flattened export can consume
this contract without source keys or replaying an imported recipe. Original
fields, scalar types, recipes and bytes remain immutable.

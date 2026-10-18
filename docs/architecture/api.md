# Catalog and asset API

Compose `uploads::routes(store).merge(api.routes())` before the final
`router_with_routes` / `router_with_client_assets` Host/Origin boundary.
`Api::open(catalog, store)` runs additive API/catalog migrations and recovers
completed intakes. The delivered Rust/WASM browser uses the versioned DTOs in
`lytronaut-contracts` across these routes.

All product requests require the configured Host. Mutations also require the exact
configured Origin and `X-Lytronaut-Intent: mutation`. There are no session cookies
or authentication endpoints. See [deployment access](authentication.md).

## Intake and semantic readiness

Directory upload uses the existing `POST /imports`, `PUT /imports/{id}/file`,
and `POST /imports/{id}/finish` sequence. The path header is lowercase hexadecimal
UTF-8 in `X-Lytronaut-Path-Hex`, with `X-Lytronaut-Entry-Type: file`. ZIP upload
uses `PUT /imports/{id}/zip`; successful ZIP completion also adapts the catalog.
Raw upload bodies retain storage's streaming quotas; buffered JSON is at most
4096 bytes. Cancellation/retry retain the existing storage contracts.

`GET /imports/{id}` returns `ImportProgress`: the previous `JobReport` fields
remain at top level, with typed `phase`, optional application `library` ID,
`catalog_state`, and `catalog_error`. Intake `state=succeeded` means immutable
intake completed; it does **not** mean the library can be browsed. Phases are:

| Phase | Meaning |
| --- | --- |
| intake | Queued/running intake |
| catalog_pending | Completed intake awaiting semantic adaptation |
| catalog_complete | Catalog result committed, including preservation-only/quarantine |
| catalog_failed | Adaptation failed, with durable fixed retryable error |
| failed / cancelling / cancelled | Corresponding intake state |

Ready catalogs permit semantic browsing. Unsupported/malformed schemas produce
`preserved_only`; conflicting source identities produce `quarantined`. Both
retain all downloadable originals through the library files endpoint. A corrupt
optional JPEG produces a per-capture preview error while valid originals remain
accessible. Missing optional `stacklq.lfp` is normal.

`POST /imports/{id}/finish` is idempotent for completed intake and retries
semantic adaptation without resending originals. Startup retries pending/failed
adaptations. One source verification/adapter failure is isolated only if its
redacted failure status can be saved durably and an application health query
succeeds. Global migration/storage failures prevent startup. Interrupted adaptation
reuses an already committed catalog version; reimport retains stable IDs and
associations. Cancelled intake never creates a browseable library. Cancellation
after immutable intake completion does not remove a committed library.

## Browsing and bounds

| GET route | Response |
| --- | --- |
| `/libraries` | `Page<LibrarySummary>`; ready/preserved-only/quarantined counts |
| `/libraries/{id}/albums` | `Page<AlbumSummary>` for a ready library |
| `/libraries/{id}/captures` | `Page<CaptureSummary>` for a ready library |
| `/albums/{id}/captures` | `Page<CaptureSummary>` with retained picture_index |
| `/captures/{id}` | `CaptureDetail`; caption, capabilities, preview/error, originals and typed missing-companion errors |
| `/libraries/{id}/files` | `Page<PreservedFile>` for any catalog state |

Pages accept `limit=1..100` (default 50), `offset=0..100000` (default zero),
`search` (at most 128 UTF-8 bytes), and `sort=order|name|rating`. Unknown/duplicate
keys, invalid UTF-8/percent escapes, control characters, and invalid numbers
return fixed 400 errors. All URI queries are at most 1024 bytes. Library and file
lists allow only order with empty search; albums allow order/name. Searches match
literal substrings in album names or capture names/captions: `%`, `_` and `~`
are escaped, not wildcard operators. SQLite case folding is ASCII; Unicode text
is retained and non-ASCII search matching is exact.

Album order uses `sort_order` then stable application ID. Default membership
order uses `picture_index`, then stable ID; library capture/file order uses the
stable application ID. Name order uses SQLite NOCASE with the same stable
secondary keys; rating uses descending rating and stable ID. Imported versions
are immutable, so offsets remain consistent while browsing one version. Responses
contain version, items, offset and optional next_offset (no cursor inferred from
source identifiers).

Serialized pages/details/manifests are at most 256 KiB. Names are at most 256
UTF-8 bytes and captions at most 1024, with explicit truncation flags. Exact
native records remain private and unchanged for backup. Capture details admit at
most 256 associated files; larger details fail with resource_limit and still
permit paginated library file downloads. `PreservedFile` carries only application
ID, size and typed role (raw/depth/refocus_stack/jpeg_focus_stack/view_stack/
thumbnail/legacy_edits/catalog/other), never source filename/path. Missing item
errors include that role and a normalized error. Focus, codec, and rendering
capability states reflect the current catalog and runtime probes. Validated
static/JPEG focus and original preservation are available from the native API;
browser H.264 and GPU presentation remain conditional and retain static/CPU fallbacks.

## Immutable assets and transfers

| GET route | Behavior |
| --- | --- |
| `/assets/{id}/manifest` | Validated `AssetManifest` with application ID, whole-content SHA-256, representation, dimensions, color policy, byte sizes and chunk ranges |
| `/assets/{id}/chunks/{index}` | Exact checked viewer chunk selected by zero-based manifest index |
| `/assets/{id}/download` | Streaming original, generic application-ID attachment filename |

Manifest IDs are saved OS-random application identifiers, never source UUIDs,
paths or camera identities. Lookup SQL binds parameters and resolves exact sealed
`catalog_sources` rows. Whole-library verification occurs at intake/adapter/
recovery boundaries, never on each browse query. A descriptor cache admits at
most 128 records and holds verified read-only original handles, not whole asset
buffers. Cache keys distinguish original download from preview validation: a
malformed thumbnail still has a downloadable original. Immutable application
storage and its exclusive root locks are the trust boundary for cached handles.

JPEG headers and actual pure-Rust decode establish geometry; catalog zero geometry
is never substituted. Viewer JPEG limits are 48 MiB encoded and 2048² decoded
RGBA, with conservative counted working admission of 96 MiB including encoded
staging; this can reject an image below the geometric maximum. Validation is
serialized, including when a caller cancels while its blocking worker continues.
A JPEG's archived color profile is not analyzed here, so `color_space=unknown`
is honest; no sRGB conversion or imported-edit rendering is claimed.

Manifest chunks cover the entire viewer asset contiguously and each is at most
8 MiB. The chunk route validates index/range against the descriptor and sets exact
Content-Length. Arbitrary HTTP Range headers are rejected with fixed 400.
Preserved originals use an empty chunk list and stream up to storage's 512 MiB
file default without whole-file buffering. Zero-byte originals are valid downloads
and file descriptors; they cannot be represented by the nonempty AssetManifest
contract and manifest requests return unsupported_format.

At most two actual response bodies may transfer by default. A third returns 429
resource_limit. Each body owns its semaphore permit through the final data frame,
until EOF/error/drop; cancellation releases the body. Positional 64 KiB bounded
reads prevent cloned-file offsets from interfering across concurrent streams.
The browser should use at most two requests, abort stale requests, validate
received length/hash and manifest limits, and separately enforce its viewer
working allocations. Server admission does not grant a browser memory budget.

## Local aggregate diagnostics and evidence

`mise exec -- cargo run --locked -p lytronaut-service -- inspect <source-directory>`
inspects a local owner-authorized directory read-only. `import` with the same
argument copies originals into `LYTRONAUT_DATA` (default data), then adapts the
catalog. It accepts regular files only, rejects symlinks/hardlinks/devices,
rechecks file identity/size/mtime before intake, and applies configured storage
path/depth/count/byte bounds, including aggregate retained path bytes during inspect. It is an owner CLI, not a hostile multiuser
filesystem traversal sandbox. Do not concurrently change the selected source
snapshot. Output is one aggregate support-report JSON object: operation/status,
file/byte/capture/album/membership/item-error counts. Failures are fixed redacted
JSON; no paths, names, source UUIDs, individual hashes or credentials are printed.

Public tests use independent schema-23 records and encoder-generated JPEG pixels.
They exercise real loopback credentialless Host/Origin intent checks, directory and ZIP streaming,
readiness, ordered pagination/search, direct manifests/chunks/originals,
malformed previews, missing companions, empty originals, unknown schemas,
restart/retry/cancellation and response bounds. Body tests check final-frame/EOF/
drop permit lifetime and concurrent offset independence. Local private validation
uses `cargo run -p lytronaut-service --example private_api -- <retained-scratch>`;
it reuses the existing catalog/originals and prints aggregate counts only. No private captures,
source values, manifests or credentials belong in Git or public CI.

## Discrete JPEG focus derivatives

`GET /assets/{file-id}/focus` validates the ready capture's `jpeg_focus_stack`
original and returns `FocusStack`: V1, opaque application file ID, strictly
increasing finite lambdas, source dimensions, and orientation policy. Availability
comes from this payload validation, never the LFE version alone. Only one uniform
JPEG block with at most 64 stored planes and dimensions at most 512 × 512 is
admitted. LFE 10 without a verified JPEG stack remains static with disabled focus;
H.264 stays separately gated.

`GET /assets/{file-id}/focus/{plane-index}/{exif-code}` produces a newly encoded
lossy JPEG of exactly that stored plane. It uses the
same whole-router Host/Origin policy as existing asset routes. It returns
`X-Lytronaut-Derivative` containing typed `FocusDerivative` JSON: V1,
`jpeg_focus_plane`, opaque file ID, plane index/lambda, source/output dimensions,
explicit EXIF 1–8, encoded length, SHA256 of the **generated** bytes, unknown color
space and processing provenance. This header is part of the response contract,
not a hash of a source file. The browser admits the metadata, verifies identity,
selection, geometry, length and generated hash, then actually decodes the JPEG
and checks its dimensions. Output is at most 2 MiB in one bounded transfer;
metadata and focus manifests stay below the existing 256 KiB JSON limit.

The default transform is identity. Existing processed recipes may already be
baked into pixels. Sidecar, embedded recipe and catalog orientation remain
preserved; this endpoint does not choose or combine them implicitly. Catalog 0
is not EXIF. User-selected EXIF transforms apply only to these verified derivative
pixels. Neither complete recipes nor unsupported aperture/perspective/tone/
denoise/animation are replayed. This flow does not establish Desktop pixel parity.

The shared two-transfer permit stays owned by the response body through EOF/drop.
Native parsing/decoding is serialized with preview validation; there is no plane
pixel cache. Original descriptors retain the existing 128-record bounded cache.
Focus admission reserves at most 8 MiB source bytes, 64 MiB conservative codec/
transform/encoding working memory, 8 MiB metadata/slack, and 16 MiB transfer
reserve within the common 96 MiB CPU limit. The encoding writer caps allocation
and writes before crossing its output limit. Browser GPU operations reserve 24
MiB inside the shared 64 MiB client transient admission, while GPU allocation
also has its own explicit 64 MiB cap; see [GPU presentation](gpu.md). No
generated derivative is written into original storage.

## Verified export and lossless bundles

`POST /exports` accepts V1 `ExportRequest`: `kind=flatten` selects the opaque
asset, `format=jpeg|png`, optional stored `plane_index`, explicit EXIF 1–8 and
an empty `operations` array. Identity is the default browser selection. Unsupported
aperture, perspective, tone, denoise, animation and synthesis requests fail with
400; they are never claimed applied. A static selection uses a verified JPEG;
a focus selection authenticates the original uniform JPEG stack, plane extent
and lambda. PNG directly encodes decoded/oriented stored pixels without an
intervening JPEG encode. JPEG uses explicit quality 90. Generated pixels have
unknown color, no color conversion and no copied ICC/EXIF metadata.

`kind=live_photo` selects a `JpegFocusStack` original, a stored `plane_index`,
and explicit EXIF 1–8. The native HTTP path decodes the selected JPEG plane in
independent Rust and returns a ZIP containing the paired JPEG, MOV, and
provenance. The browser path instead handles archived AVC through WebCodecs;
its comparator scope is documented in the
[final private report](../validation/lytronaut-f5a-private.md).

`GET /exports/{id}` returns typed `ExportProgress`
(flatten/archive/restore/live-photo plus durable JobReport).
`DELETE /exports/{id}` cancels pending work or removes a
completed artifact. `GET /exports/{id}/download` streams a completed artifact
with an opaque filename; `/recipe` separately returns the V1 Lytronaut record
for flattened pixels: source variant, plane/lambda, source/output dimensions,
full visible geometry, explicit orientation, generated extent/SHA256, color
policy, processing loss, refocus loss and preserved-only imported adjustments.
Legacy sidecars remain original downloads, byte-exact and separate.

Two pending jobs share the existing serialized codec admission and two streaming
transfer permits. Static JPEG staging is at most 16 MiB, focus source at most
8 MiB; counted codec/transform/encoder work is at most 80 MiB with source bytes
included in decoder admission. JPEG output is capped at 4 MiB and PNG at 8 MiB
before encoder writes/allocations; source staging is released before encoding.
No export overlaps a second native viewer decoder. Browser downloads and restore
File uploads stream through platform APIs; WASM never accumulates bundle bytes.
Existing 64 MiB transient/32 MiB persistent viewer admission remains separate.
Jobs reserve disk bytes and the free-space floor before admission. Whole-library
archive and restore jobs are bounded only by free work space (not by the upload
budgets or `LYTRONAUT_RESERVED_BYTES`, which keeps bounding imports and
flatten/Live Photo exports); a refusal is a retryable 429 `resource_limit` whose
optional, additive `shortfall` names the required and available bytes, shown
by the browser as a sentence with binary units (docs/design/007). `POST /imports`
uses the same free-space `shortfall` shape because its admission also counts
export/restore reservations; permanent declaration-limit failures stay distinct.
An archive
whose logical catalog exceeds the 288 MiB v2 section fails before any artifact
exists with a non-retryable `resource_limit` job error.
At most 128 reports/artifacts remain; cancelled/failed reports are evicted oldest
first when another job is admitted. Remove completed exports to make room.

`kind=archive_originals` produces a private `.lytronaut` bundle of the complete
application catalog snapshot plus each unique immutable original blob. Logical
paths, source SQLite databases, unknown backup databases/files, legacy sidecars,
H.264, exact typed native records, stable application IDs, albums, order, events
and metadata survive. Original databases are opaque blobs and remain byte-exact.
Current V2 archives stream fixed logical application tables in a
backend-neutral form. Legacy V1 archives contain a validated physical SQLite
snapshot produced with `VACUUM`; both retain the represented semantic values.
A consistent snapshot is taken before selecting its blob inventory, so concurrent
new intake cannot create a catalog/blob mismatch. Export jobs and unfinished intake jobs are deleted from the generated snapshot.
Application catalogs and current bundles contain no authentication state.

`POST /restores` streams a private bundle of any declared Content-Length into an
owned upload file, then validates/restores it as a durable cancellable job.
Checks include supported V1/V2 framing, sizes/counts, duplicate blobs, exact
SHA256s, no trailing bytes, empty transient jobs, and exact source/blob
inventories. V2 decodes only the fixed bounded logical tables and row/value
types; it never executes bundled DDL. Legacy V1 validates its known SQLite schema
with read-only/query-only/trusted_schema=OFF, a 2 MiB cache, the
20-million-VM-instruction/10-second deadline, integrity and foreign keys before
obsolete credential tables are removed from the isolated copy. Recomputed
checksums never bypass either version's schema and association checks.

Restore exclusively creates `staging.exports/{job-id}.restored/store` beside the
application staging root. It never opens source libraries or the running catalog
writable, overwrites an existing target, or activates restored data. The store
becomes visible only after all validation passes; cancellation/failure removes
only its exclusively owned temporary target. Interrupted jobs recover as fixed,
retryable failed reports on restart and their incomplete owned outputs are removed.
If a job's owned cleanup cannot be proved or completed (foreign collision, denied
permission, a dependent database object, a marker lost mid-removal), that one job
is reported `failed` with a retryable `storage_unavailable` error; its files,
journal and quota stay reserved, it is never offered as ready, and `DELETE` or the
next startup retries cleanup. A ready restore whose result no longer verifies
at startup (including a missing restore journal or one that never recorded
publication) is likewise reported `failed`/`storage_unavailable` and retained, but
is removed only by an explicit `DELETE`. The service still starts. Startup is
refused only when the whole store is unavailable (unreadable export root or
restore journal, or an unreachable database), or when a job's blocked/unverified
state cannot be persisted, so that no job is left looking ready.
Logs and errors never contain source paths, row values or individual source hashes.

# Authenticated catalog and asset API

Compose `uploads::routes(store).merge(api.routes())` before the final
`router_with_routes` / `router_with_client_assets` authentication boundary.
`Api::open(catalog, store)` runs additive API/catalog migrations and recovers
completed intakes. The browser application is a later bead; these routes expose
Rust DTOs in `lytronaut-contracts` for its WASM client.

All product requests require the session cookie and configured Host. Mutations
also require the exact configured Origin and `X-Lytronaut-Csrf` from
`GET /auth/session`. Login/setup retain their intent-header exceptions described
in [authentication.md](authentication.md). Legacy `public=1` grants no access.
Errors are fixed `ApiError` JSON, without underlying exception messages. Every
response inherits no-store, nosniff and no-referrer. No CORS or server-side local
filesystem import endpoint exists.

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
errors include that role and a normalized error. Optional focus/codec/rendering
capabilities remain disabled/unverified until their own beads satisfy gates;
this API claims only validated static JPEG previews and original preservation.

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
They exercise real loopback setup/login/CSRF, directory and ZIP streaming,
readiness, ordered pagination/search, authorized manifests/chunks/originals,
malformed previews, missing companions, empty originals, unknown schemas,
restart/retry/cancellation and response bounds. Body tests check final-frame/EOF/
drop permit lifetime and concurrent offset independence. Local private validation
uses `cargo run -p lytronaut-service --example private_api -- <retained-scratch>`;
it reuses the existing catalog/originals, saves validation-only auth state in
private mode-0600 files, and prints aggregate counts only. No private captures,
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
lossy JPEG of exactly that stored plane. It requires authentication under the
same whole-router origin/CSRF policy as existing asset routes. It returns
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
and writes before crossing its output limit. GPU work remains a separate future
64 MiB cap. No generated derivative is written into original storage.

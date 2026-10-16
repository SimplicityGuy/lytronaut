# Bounded immutable intake and durable jobs

Storage intake preserves regular-file bytes and logical UTF-8 names, including
unknown files. `JobState::Succeeded` means **intake complete**, with a sealed,
verified source manifest. It does not mean semantic catalog import or library
readiness. The catalog adapter must independently transact its records against
`Store::manifest` and `Store::open_blob`; it must never use unfinished sources.
Source entries have redacted Debug output. HTTP job reports omit paths/digests.
No routine intake logging or telemetry is emitted.

## HTTP transport

Build `uploads::routes(store)` as product routes before `router_with_routes`
(or `router_with_client_assets`). The final authentication guard applies to
all routes, including raw streaming requests. Every mutation requires the
session cookie, exact configured Origin and `X-Lytronaut-Csrf`.

`POST /imports` accepts the version-1 `ImportRequest` JSON, capped at 4096 bytes.
`declared_bytes` and `declared_files` are positive admission **upper bounds**;
actual reads/files are counted independently. The response is 201
`{version:1,id:"<opaque generated job ID>"}`. Missing/false declarations never
increase the admitted budget. Empty files are preserved within a nonzero job
budget. Empty libraries are rejected.

For `kind:directory`, upload each regular file with `PUT /imports/{id}/file`.
The body is its raw bytes. Supply `X-Lytronaut-Entry-Type: file` and
`X-Lytronaut-Path-Hex`: the canonical lowercase hex encoding of the original
relative path's UTF-8 bytes. For example the relative path `a/b` is `612f62`.
Unicode is preserved byte-exactly without relying on non-ASCII HTTP headers.
The encoded path length is checked before allocation; invalid UTF-8, uppercase
hex, odd lengths and unsafe paths fail closed. This protocol accepts only files,
never a server filesystem path, links or device nodes. A client walking a local
directory must reject links/devices rather than dereference them. There is no
server-side directory walker or inferred symlink creation.

`POST /imports/{id}/finish` explicitly seals a directory manifest containing
exactly the successfully received files; declarations do not imply missing
files were received. Until this explicit finish, partial directory progress is
not a usable catalog manifest. The browser should declare its complete selection
and call finish only after every selected file succeeds.

For `kind:zip_library`, `PUT /imports/{id}/zip` sends one raw ZIP archive.
Success (204) seals intake automatically only after the complete archive has
been framed, decoded and checked. ZIP jobs cannot be sealed early via finish,
and directory/file and ZIP kinds cannot be interchanged. A malformed later
entry never turns earlier preserved blobs into a completed source manifest.
Both uploads stream into a private encoded spool, count bytes before writes,
and reject every `Content-Encoding`. `Content-Length` is optional for chunked
streams, but when supplied must match actual bytes and fit admission. A 30-second
idle read limit bounds abandoned streams. Bodies bypass the buffered 4096-byte
extractor limit only through manually counted raw streaming handlers.

`GET /imports/{id}` returns a version-1 `JobReport`. Committed file bytes are
durable progress; bytes in a partial incoming file are not reported complete.
Upon success `total_bytes` becomes the actual committed total (including zero
bytes for empty files). `POST /imports/{id}/cancel` stops work and removes its
staging spools. `POST /imports/{id}/retry` creates a new job with the old upper
bounds; clients resend input. The old job retains durable progress/error state,
and verified content deduplicates on retry. Malformed/quota-failed uploads become
explicit failed jobs with normalized `InvalidInput` and `retryable:false`.
I/O failures and interruption remain `StorageUnavailable`/retryable. Clients
can explicitly retry after correcting input or policy; blindly retrying hostile
input is never recommended.

## Limits and admission

Defaults: 20 GiB actual uncompressed/upload, 100,000 entries, 512 MiB/file,
4096 path bytes, depth 32, 32 MiB aggregate path/ZIP central metadata,
21 GiB maximum encoded ZIP, two active jobs, one ZIP decoder, 84 GiB aggregate
reservation ceiling and 64 MiB free-space floor. Native format decoding has its
separate bounded admission policy; intake performs no image/sensor decoding.
ZIP framing metadata, decoder output and transport bytes are separate budgets.
Path bytes are counted cumulatively for directory intake; collision checks use
indexed canonical/prefix lookups rather than collecting every prior filename.
Case aliases and file/directory prefix collisions are rejected; stored names
are never case-folded or rewritten.

Each directory job reserves actual declared bytes plus two capped per-file
buffers on disk (one encoded spool and one publication temporary). A one-byte
declaration reserves three bytes, rather than the global encoded maximum.
ZIP encoded admission is the minimum of its configured cap and declared bytes
plus 1% compression expansion and twice the bounded metadata allowance plus
1024 bytes for fixed/end framing. Its reservation adds declared originals and
one capped per-file publication temporary. Pathological ZIP header overhead
beyond this explicit encoded budget is rejected. Reservations are summed across
active jobs and checked conservatively on both volumes against actual available
space; on the staging volume the in-flight reservations of export and restore jobs
are added, under the same writer admission, so an import cannot claim space a
backup or restore already reserved (export admission counts running imports in
turn). A shortage is a retryable `StorageError::Space` naming the required and
available bytes (HTTP 429 `resource_limit` with a `shortfall`); a declaration
beyond the upload budget remains `Quota` (HTTP 413). One encoded spool per job prevents concurrent requests exceeding that
reservation. Process-exclusive locks own both roots; reservation/terminal changes
are durable SQLite updates serialized with intake operations. Other processes
can still exhaust a volume; write failure produces an explicit retryable job,
with no partial original publication.

Administrators can override numeric limits through `LYTRONAUT_UPLOAD_BYTES`,
`LYTRONAUT_UPLOAD_FILES`, `LYTRONAUT_FILE_BYTES`, `LYTRONAUT_PATH_BYTES`,
`LYTRONAUT_PATH_DEPTH`, `LYTRONAUT_ZIP_METADATA_BYTES`, `LYTRONAUT_ENCODED_BYTES`,
`LYTRONAUT_IMPORT_JOBS`, `LYTRONAUT_IMPORT_DECODERS`,
`LYTRONAUT_RESERVED_BYTES`, and `LYTRONAUT_FREE_FLOOR`. Invalid/zero/overflowing
policies fail startup. The service defaults to private `originals` and `staging`
directories under `LYTRONAUT_DATA`; `Store::open` accepts distinct administrator
roots, including separate filesystems. Unix private permissions are required.

## Archive and publication safety

The independently implemented framing subset accepts single-volume classic
ZIP/ZIP64 stored or DEFLATE regular files and directories, UTF-8 names, bounded
ZIP64/time/UID/NTFS extra fields, and verified data descriptors. It rejects
preambles, unreferenced gaps, overlaps, encryption, alternate compression,
unknown/Unix-link extra fields, nonregular Unix types, absolute/traversal/NUL/
backslash/drive paths, normalized collisions, recursive archive signatures,
CRC/length mismatch and actual decompression overflow. ZIP64 end headers,
counts, offsets and total metadata are bounded before allocating entries or
constructing any decoder; no generic `ZipArchive` allocation/extraction occurs.
Decoder buffers and 64 KiB copy chunks are bounded, with cancellation checked
between chunks. Synthetic ZIP64 fixtures test framing without enormous files.

Only server-generated digest/length and random temporary names touch physical
storage. SHA-256 **and** actual length select a candidate; existing candidates
are then fully hashed and byte-compared against the incoming completed temporary.
No existing original is replaced. A same-volume publication temporary is
written, fsynced, made read-only, fsynced again, and atomically hard-linked with
no replacement into the originals namespace. Parent-directory fsync makes the
publication durable; then the temporary link is removed and the parent fsynced.
This is a no-replace atomic publication, avoiding `rename`'s replacement race.
The temporary is always on the originals volume, so staging/original EXDEV never
triggers a partial copying fallback. Publication races deduplicate only after
byte verification. The sole original API returns a verified read-only File.

Only after publication does a SQLite transaction install the logical source
entry and update progress. Finish re-verifies all references on blocking workers,
polls cancellation and rechecks before the terminal update. Cancellation is
requested through an atomic flag before waiting for the serialized operation;
finish wins once it passes its last cancellation check into its terminal update.
A concurrent later cancel never rewrites succeeded intake. Active encoded
spools prevent finish. Immutable orphan blobs from an interrupted transaction
remain verified and deduplicate on retry; storage does not delete preserved
originals as cancellation cleanup.

Restart with exclusive roots marks every unfinished job failed/retryable,
releases reservations, removes pending publication/spool files, and checks
completed manifests. Corrupt/missing/writable blobs turn their jobs into explicit
retryable failures. Recovery never trusts an unfinished blob or resumes an
arbitrary partial decoder. Authentication tables are preserved; intake uses only
`intake_` tables. Fault injection covers before/after write, fsync, publication
and job transactions, including terminal transactions and real abandoned files.

Upstream APIs checked: [Axum body framing](https://docs.rs/axum/0.8.9/axum/body/struct.Body.html),
[flate2 DEFLATE reader](https://docs.rs/flate2/1.1.10/flate2/read/struct.DeflateDecoder.html),
[fs2 space/locks](https://docs.rs/fs2/0.4.3/fs2/), and
[PKWARE ZIP framing](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT).
The framing code and synthetic inputs were independently authored; no reference
implementation was translated. The added resolved graph is permissive: flate2,
crc32fast, fs2, miniz_oxide, adler2, simd-adler32 and target-only winapi packages.
The complete license texts/inventory are retained, including winapi import-library
manifest declarations whose archives omit the shared upstream license files.

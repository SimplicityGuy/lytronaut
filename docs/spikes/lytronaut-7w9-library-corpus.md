# Desktop library fidelity and authorized corpus

> Historical source- and date-bound record. See the [delivered application baseline](../validation/final-main-status.md) for current status.

Bead: **lytronaut-7w9**. Observed: 2026-10-11. This is a research result;
there is no importer implementation yet.

## Question

Can Lytronaut preserve the supplied Desktop library faithfully, and which
camera/library variants have enough authorized evidence to begin implementation?

## Method

The operator authorized read-only inspection of their local Desktop library.
No redistribution permission was supplied. All captures, identifiers, names,
timestamps, catalog rows, individual hashes and paths remain local.

The probe opened `lytrolibrary.db` using Python's standard-library SQLite driver
with a `file:` URI ending `?mode=ro&immutable=1`, then set
`PRAGMA query_only=ON`. It queried table definitions and aggregate counts, ran
`PRAGMA integrity_check` and `PRAGMA foreign_key_check`, and joined pictures to
their metadata and album memberships. No source database was opened writable.
An immutable connection is appropriate only for an offline, unchanged snapshot;
an implementation must first stage uploads and refuse a live/incomplete SQLite
snapshot with unhandled journal or WAL files.

The path mapping tested for every catalog row was
`<library>/<imagebin_uuid>/<picture_uuid>/<companion filename>`. Directory walking
checked extra raw-capture directories, absent associations, symlinks, file sizes,
and the shape/version of JSON edit sidecars. SHA-256 was streamed in 1 MiB chunks
over **every file twice**, before and after the read-only probes. Relative paths,
bytes, modification times and hashes were compared exactly.

A mode-0600 private manifest is retained durably, outside Git, at
`fixtures/private/library-corpus-manifest.json` in the operator's main clone
(`/Users/Robert/workspaces/github/SimplicityGuy/lytronaut`). It contains the source
path, observation time, provenance, rights restriction, all file hashes and
sanitized aggregate results. Its original scratch copy and the disposable
`corpus_probe.py` remain under `/private/tmp/lytronaut-provisioning/`; temporary
probe files may be removed by OS cleanup. The durable manifest is ignored and
must never be attached to public reports or CI. Neither artifact is a product
dependency. The committed tables below are the sanitized support matrix.

The exact disposable command was
`python3 /private/tmp/lytronaut-provisioning/corpus_probe.py`; the probe's SHA-256
is `56cf6387c0ea6995bb78fcf982752812890f1c1f0a26e1300064ee6201ab0864`.
The manifest stores the verified source hashes; later additive observations
record empty catalog hashes and the low-quality-stack version split.

For reproduction on an authorized offline copy, use the same read-only URI,
then run these aggregate queries (they disclose no personal row values):

```sql
PRAGMA query_only=ON;
PRAGMA integrity_check;
PRAGMA foreign_key_check;
SELECT version FROM library_metadata;
SELECT count(*) FROM picture;
SELECT count(*) FROM picture_metadata;
SELECT count(*) FROM album;
SELECT count(*), min(picture_index), max(picture_index),
       count(DISTINCT picture_index)
FROM album_to_picture GROUP BY album_uuid;
SELECT count(*) FROM picture GROUP BY imagebin_uuid;
SELECT count(*) FROM picture GROUP BY event_uuid;
SELECT lfe_major, lfe_minor, count(*)
FROM picture_metadata GROUP BY lfe_major, lfe_minor;
SELECT length(hash), count(*) FROM picture GROUP BY length(hash);
```

Follow the path mapping above and stream each file's SHA-256 before and after;
do not include database rows or individual hashes in public test reports.

## Evidence

### Catalog and preservation

| Property | Observed result | Fidelity implication |
|---|---|---|
| Active Desktop schema | `library_metadata.version=23`; SQLite `user_version=0` | Detect the library metadata version explicitly; `user_version` alone is insufficient |
| Pictures / metadata | 109 / 109; zero pictures without metadata | Preserve one-to-one association through source UUIDs in the private catalog |
| Albums / memberships | 3 / 109; no orphan membership or repeated picture membership | Import all memberships, not only capture files |
| Album order | Sizes 12, 39, 58; indices respectively 0–11, 0–38, 0–57, all distinct | Preserve `picture_index` and album `sort_order` independently; filesystem order is irrelevant |
| Image bins | 2, containing 9 and 100 pictures | Preserve bin association; do not flatten paths before reconciliation |
| Events | 3, containing 12, 39 and 58 pictures | Preserve event association, title and time separately from albums |
| Picture lists | 11; filter codes 0–10, all sort code 1 | Retain codes opaquely until independently interpreted; they are not 11 albums |
| Ratings / flags | All 109 have rating 0 and flag status 0 | Field mapping is observed; nonzero semantics and round-trip fidelity need synthetic fixtures |
| Other status fields | `broken_state=0`, `protected=0`; `lfe_state=0` for 101 and 2 for 8 | Retain raw codes without inventing meaning for state 2 |
| Names / captions | 109 nonempty names; zero nonempty captions | Preserve Unicode names/captions; nonempty caption coverage needs synthetic fixtures |
| Catalog dimensions | All image/cropped widths/heights are 0 | Determine derivative dimensions from verified file headers; zero is not a valid decoded image size |
| Album sharing | All 3 have legacy `public=1` | Preserve as historical metadata; never infer authorization to publish or disable authentication |
| Integrity | SQLite integrity `ok`; no foreign-key violations | This snapshot is internally consistent; malformed libraries still need negative fixtures |
| Dedup fields | All 109 `picture.hash` values empty; history has 109 distinct nonempty hashes, none joining the picture hash field | Compute independent SHA-256; never collapse pictures because legacy hash fields match |
| Source preservation | 728 files, 2,001,830,074 bytes; both SHA-256/size/mtime maps identical | No observed byte or modification-time changes; access times were not asserted unchanged |

Dates, captions, names, source identifiers, serial numbers, exposure/ISO/lens,
white balance, orientation, rotation, focus, depth score, capabilities and other
`picture_metadata` fields exist and must be retained losslessly. Their presence
does not prove that every value has been interpreted correctly. The importer
must store both normalized fields and the original typed record/sidecar so later
interpretation does not lose information.

### Capture companions and version coverage

| Variant or artifact | Coverage | Result / support boundary |
|---|---:|---|
| Catalog camera model identifier `1` | 109 | Catalog evidence only; decode spike independently verifies actual camera metadata |
| LFE major/minor 10/0 | 51 | Version-23 catalog and directory mapping verified |
| LFE major/minor 11/0 | 58 | Version-23 catalog and directory mapping verified |
| `raw.lfp` | 109 | All catalog pictures have their original container; no extra raw directory |
| `dm.lfp` | 109 | Association verified; depth semantics belong to processed interoperability spike |
| `stack.lfp` | 109 | Association verified; encoding/renderability belong to interoperability spike |
| `stack.view.lfp` | 109 | Retain separately; do not infer identical semantics to `stack.lfp` |
| `stacklq.lfp` | 58 | Present for all LFE 11 captures; absent for all 51 LFE 10 captures; optional in this corpus |
| `thumbnail.jpg` | 109 | Available for initial browsing; dimensions/orientation must be verified by image decoding |
| `view_parameters.json` | 109, all version string `"4"` | Valid JSON; lossless sidecar retention is supported, edit reproduction is not established here |
| `animation.json` | 2 | Preserve opaquely; animation playback semantics unverified |
| Legacy snapshots | 2 other `.db` files; 11 `.bckup` files | Retain bytes; do not silently import these as additional active libraries |
| Other Desktop schema versions | 0 active samples | Unsupported for semantic import until a versioned adapter and fixture are verified |
| Confirmed Illum corpus | 0 | NO-GO for claiming Illum compatibility; require authorized capture, Desktop export and matching calibration |

The entire tree contains 494 LFPs, 109 JPEGs, 111 JSON files, 3 SQLite files and
11 backup files. All expected raw/depth/standard-stack/view-stack/thumbnail/edit
companions resolve for all 109 catalog pictures. There are zero absent capture
directories, extra raw directories or source symlinks. This proves association
and preservation, not successful decoding of every companion.

Edit sidecars have 34 keys spanning crop, focus, aperture, perspective/tilt,
stereo, exposure, white balance, tone/color, denoise/sharpen, pan/zoom and
orientation. Focus has 25 distinct stored values; orientation is `1` for 108 and
`8` for one. All other keys each have a single stored value in this corpus.
Catalog metadata orientation is instead `0` for all 109: retain both sources and
resolve precedence through the interoperability spike rather than assuming
these integer enums are interchangeable. Preserve JSON bytes and unknown keys,
even when a control is not supported by Lytronaut.

Calibration files were located in the operator's local Lytro application-support
directory during the decoder spike. The directory matches the capture camera
serial locally (only the match boolean is disclosed). This evidence is outside
the supplied library and would need explicit import as a separate calibration
bundle; it must not be assumed present in an uploaded library. Calibration
completeness, white-image/lens-position semantics and calibrated rendering remain
that spike's verdict. No calibration files are redistributed here.

## Verdict GO | NO-GO

**GO** for implementing a preservation-first, schema-23 Desktop import/catalog
adapter against this authorized private corpus. All 109 records and their core
companions reconcile, and source preservation is measured.

**NO-GO** for an unqualified "all Lytro libraries" claim, Illum compatibility,
unknown-schema semantic import, nonzero rating/flag interpretation, animation
playback or Desktop edit parity. These need verified additional evidence.
Original-camera identity, native decoding and calibration quality are governed
by [the decoding spike](lytronaut-4li-decode-calibration.md), and derivative/edit
behavior by [the interoperability spike](lytronaut-w6z-processed-interop.md).

## Recommendation

Begin with schema-23 import, original byte preservation, albums/events/order,
metadata and JPEG browsing. Make capability status visible per capture, with
independent states for original available, preview available, processed stack,
depth and calibrated rendering. Preserve originals even when semantic import or
decoding is unavailable; report unknown versions explicitly and never run a
newer schema through the version-23 adapter by guesswork.

Require the following implementation acceptance checks before enabling uploads:

1. **Fidelity:** Import exactly 109 pictures, 3 albums and 109 ordered memberships
   from this corpus; preserve both bins, all event associations, names/captions,
   typed metadata, status codes, edit bytes and every unknown file. Compare the
   source SHA-256 manifest unchanged after import, restart and export/backup.
   Use authorized synthetic schema-23 fixtures for nonzero ratings/flags,
   nonempty captions, Unicode, repeated cross-album membership and unknown keys.
2. **Missing and malformed data:** Test absent raw, absent thumbnail, absent
   depth/stack, invalid JSON, orphan FK, duplicate UUID, absent bin, truncated
   database and unsupported metadata version. Preserve what can be preserved,
   report per-item errors, and never mislabel missing optional LFE-10 low-quality
   stacks as source corruption. A duplicate identifier with different bytes
   must be quarantined/reported, never overwrite an original.
3. **Bounded intake:** Proposed initial defaults are 20 GiB uncompressed per
   upload, 100,000 files, 512 MiB per file, 4 KiB per relative path and path
   depth 32; administrators may raise quotas deliberately. Enforce streaming
   aggregate limits, disk reservation, actual decompressed size and worker
   concurrency before extraction. These are proposed guardrails, not measured
   support claims; revise them against large authorized libraries. Reject
   traversal, absolute paths, symlinks/hardlinks, devices, archive collisions,
   embedded NULs, recursive archives and SQLite extension execution. Never trust
   uploaded album `public` metadata as an access-control instruction.
4. **Crash recovery:** Stage immutable files and journal import progress in the
   destination catalog, not the source. Inject interruption before/after each
   file write, fsync/rename and catalog transaction commit; on restart recover
   to complete verified files or an explicit retryable failure. No orphan
   committed catalog rows or silently missing originals; clean abandoned staging
   under a bounded retention policy. This remains untested until implementation.
5. **Deduplication:** Compute SHA-256 plus byte length independently and verify
   any candidate match before reuse. Reimporting identical source bytes must
   create no duplicate capture or album membership while allowing explicit
   separate logical imports. One altered file must produce a new version/conflict
   record without changing earlier originals; empty legacy hashes must never
   deduplicate unrelated pictures. Test restart at every dedup commit boundary.
6. **Coverage expansion:** Acquire operator-authorized Illum raw/processed
   captures, an associated Desktop catalog and exact matching calibration,
   plus older/newer active-schema fixtures. Record provenance and redistribution
   rights per sample. Keep private fixtures out of Git and CI; construct lawful
   synthetic fixtures for public automated tests and report private corpus
   validation separately.

These requirements do not authorize publication of source data or implementation
dispatch; the scope decision bead must incorporate all four spike results first.

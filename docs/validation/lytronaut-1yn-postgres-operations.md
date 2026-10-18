# PostgreSQL operations follow-up: lytronaut-1yn

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

## Change and scope

Based on the implementation container at
`3f7d5b82bfb294e1ed93ed6957ca65bc74fe7afc` (including the canonical
lytronaut-80j disk-accounting fixes). This follow-up changes deployment test
orchestration and operations documentation only; application code, bootstrap
validation rules and resource limits are unchanged. All deployment data and
passwords are generated synthetic fixtures, never operator credentials.

The earlier lytronaut-cjq reconciliation already removed obsolete bounded
whole-library backup sizing. The lytronaut-80j implementation subsequently added
metadata-inclusive filesystem costs. This follow-up documents those current
rules instead of restoring the obsolete configured-upload-file-limit formula.

| Review item | Resolution |
|---|---|
| Archive framing and file count | Document actual unique-original count, 60-byte header, two 40-byte inventory/framing records, separately rounded spools/artifact and auxiliary allowance. Formula follows `bundle::archive_need` and `file_cost`. |
| Retained exports | Distinguish real free-space observation from the per-asset aggregate budget, which counts both reserved and retained outputs. Archives/restores are excluded from that aggregate. |
| Recovery table counts | Compare every table in the dumped `lytronaut*` schemas before/after bundled and external `pg_dump` recovery. Both samples are taken while the application is stopped, before recovered startup can reconcile bookkeeping. Table/count maps are saved in `deployment.json`. The public administrator-owned sentinel remains outside the dump and is still checked independently. |
| Portable recovery semantics | Preserve public library/capture identity and exact-original hash checks. Portable bundles intentionally omit runtime/export-job state, so whole-database row counts are not an equivalent assertion. The strict PostgreSQL suite separately checks every portable table's logical values across all four backend combinations. |
| Valid non-ASCII secret | Add U+20AC to the random punctuation-containing CRLF-terminated password used by the actual bundled bootstrap and app. Healthy import/restarts prove both consumers accept it; existing redaction audits remain. |
| Crash restart count | Require `RestartCount` to increase from the immediate pre-event value for both induced application exits and the database crash; health alone or a historical restart cannot pass. |

## Evidence and remaining execution

Local lightweight source checks passed: Python syntax compilation; isolated
execution of the table-count helper with a quoted identifier, alternate database
and an empty schema; and both restart predicates with unchanged/increased counts
and healthy/starting health states. `git diff --check` passed. These checks use
mocked Docker responses and do **not** prove actual database/Compose behavior.

No local Cargo or container build was run during implementation: the dispatcher
reserved the build lease for restore-reconciliation work. Canonical
`mise exec -- just check` remains required before submission. The exact-source
Linux container workflow must execute both amd64 and arm64 deployment suites to
prove real non-ASCII bootstrap/application acceptance and the table-count
comparisons; record the resulting run IDs and statuses at handoff. CI and human
review are still pending at this source checkpoint.

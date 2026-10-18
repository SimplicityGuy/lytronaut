# Documentation audit — 2026-10-18

Baseline: delivered application commit `a3d0482896e6ca3361d7f900f0f0417cda20e4ac`.
Every tracked Markdown file at that baseline was reviewed. “Historical” means the
record remains intentionally bound to its stated source and date; it was not
rewritten as current runtime evidence. Generated legal inventories were checked
through the repository license gate rather than edited by hand.

## Root, policy, legal, and fixture documents

| File | Disposition and evidence |
| --- | --- |
| `AGENTS.md` | Reviewed; managed AGF block and privacy/legal rules retained |
| `CLAUDE.md` | Reviewed; generated AGF pointer retained; missing `docs/AGF.md` supplied |
| `README.md` | Updated from pending to delivered baseline; Acknowledgement and legal text retained |
| `DEPENDENCY_LICENSES.md` | Generated; validated against `Cargo.lock` by `just licenses` |
| `THIRD_PARTY_NOTICES.md` | Generated; validated against `Cargo.lock` by `just licenses` |
| `notices/README.md` | Reviewed; retrieval provenance and exception notes retained |
| `fixtures/synthetic/PROVENANCE.md` | Reviewed; synthetic authorship/provenance remains current |
| `plans/compatibility.yaml` | Historical filed spike specification; existing historical header retained |
| `plans/implementation.yaml` | Historical filed implementation specification; clarified current delivered-baseline pointer |

## Current architecture and operations guides

| File | Disposition and evidence |
| --- | --- |
| `docs/architecture/api.md` | Reviewed against routes/contracts; current |
| `docs/architecture/authentication.md` | Reviewed against Host/Origin/mutation-intent guard; current |
| `docs/architecture/browser.md` | Updated for passed coverage, FIFO admission, final focus and Live Photo evidence |
| `docs/architecture/catalog.md` | Reviewed against adapter, scratch ownership, and dual-backend installation; current |
| `docs/architecture/contracts.md` | Updated from foundation/SQLite-only language to delivered dual-backend contracts |
| `docs/architecture/formats.md` | Reviewed against independent bounded parsers; current |
| `docs/architecture/gpu.md` | Updated from pending coverage to final measured coverage and retained exception |
| `docs/architecture/storage.md` | Corrected removed cookie/CSRF-token guidance to Host + Origin + mutation intent |
| `docs/operations/coverage.md` | Reviewed against `coverage.rs` and recipes; current |
| `docs/operations/linux.md` | Corrected restore ownership/adoption, shell portability, and transport-frame wording |
| `docs/operations/required-checks.md` | Reviewed; accurately says enforcement is not enabled |
| `docs/operations/supported-variants.md` | Closed delivered focus/grid/HRD/PostgreSQL items; retained accepted limits |

## Historical design records

| File | Disposition and evidence |
| --- | --- |
| `docs/design/001-lytronaut.md` | Historical scope/gates; retained, linked to current final-main status |
| `docs/design/002-deployment-access.md` | Historical ADR; implemented access boundary remains consistent |
| `docs/design/003-automatic-rendering.md` | Historical ADR; runtime-probe/fallback decision remains consistent |
| `docs/design/004-live-photo-export.md` | Historical design/owner decisions retained without relabeling |
| `docs/design/004-live-photo-release.md` | Historical release design retained without relabeling |
| `docs/design/005-catalog-backends.md` | Historical ADR; SQLite/PostgreSQL implementation remains consistent |
| `docs/design/006-database-boundary.md` | Historical ADR; database boundary remains consistent |
| `docs/design/007-portable-backup.md` | Historical design; current sizing/ownership clarifications live in Linux guide |

## Historical spike reports

| File | Disposition and evidence |
| --- | --- |
| `docs/spikes/lytronaut-0mz-rust-gpu.md` | Historical experiment; concept PNG classified as UI concept evidence, not schematic |
| `docs/spikes/lytronaut-4li-decode-calibration.md` | Historical source/date scope retained |
| `docs/spikes/lytronaut-7w9-library-corpus.md` | Historical aggregate corpus research retained; no private locators added |
| `docs/spikes/lytronaut-w6z-processed-interop.md` | Historical interoperability observations retained |

## Historical validation reports

| File | Disposition and evidence |
| --- | --- |
| `docs/validation/lytronaut-1yn-postgres-operations.md` | Source-bound PostgreSQL receipt retained |
| `docs/validation/lytronaut-25u-restore-reconciliation.md` | Source-bound restore receipt retained |
| `docs/validation/lytronaut-5ld-desktop-streaming.md` | Source-bound streaming receipt retained |
| `docs/validation/lytronaut-80j-disk-accounting.md` | Historical failures/pending state retained; current guide reconciled separately |
| `docs/validation/lytronaut-b0e-focus-acceptance.md` | Full source-bound focus/FIFO acceptance report retained |
| `docs/validation/lytronaut-cjq-final.md` | Earlier acceptance checkpoint retained; final-main status added separately |
| `docs/validation/lytronaut-f5a-private.md` | Corrected sealed-copy disposition after review; comparator limits retained |
| `docs/validation/lytronaut-hyy-video.md` | Historical video receipt retained; no universal conformance claim |
| `docs/validation/lytronaut-j2u-deployment.md` | Source-bound deployment receipt retained |
| `docs/validation/lytronaut-m5h-focus.md` | Historical focus checkpoint retained |
| `docs/validation/lytronaut-q1o-instrumented.md` | Historical diagnostic receipt retained |
| `docs/validation/lytronaut-re8-3-acceptance.md` | Historical reconciliation retained; current status is separate |
| `docs/validation/lytronaut-re8-3-hrd.md` | Narrow reviewed HRD result retained without universal claim |
| `docs/validation/lytronaut-u8i-export.md` | Historical export receipt retained |
| `docs/validation/lytronaut-us7-gpu.md` | Physical-host GPU evidence retained with timing limitations |

## Non-Markdown evidence assets

All tracked JSON and PNG assets under `docs/validation/assets/` were inventoried
and preserved byte-for-byte. They are historical evidence, not current diagrams.
`docs/spikes/assets/lytronaut-0mz-concept.png` is historical UI concept art.
Before this refresh the repository contained no Mermaid, SVG, or HTML schematic.
The new original schematics and their provenance are listed in
[`docs/diagrams/STYLE.md`](diagrams/STYLE.md).

## New navigation and reconciliation documents

The refresh added `docs/README.md`, `docs/AGF.md`, `docs/quick-start.md`,
`docs/configuration.md`, `docs/architecture/overview.md`, and
`docs/validation/final-main-status.md`. These concise entry points link to the
existing detailed guides rather than duplicating them.

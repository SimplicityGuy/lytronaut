# Lytronaut documentation

Current operator and developer guides, architecture contracts, and preserved delivery evidence.

[Back to main](../README.md) | [AGF workflow](AGF.md) | [Supported variants](operations/supported-variants.md)

## Start here

| Document | Purpose |
| --- | --- |
| [Linux operation](operations/linux.md) | Build, configure, back up, restore, and recover a self-hosted instance |
| [Supported variants](operations/supported-variants.md) | Verified capabilities and explicit limits |
| [Authentication](architecture/authentication.md) | Host, origin, mutation-intent, and reverse-proxy boundary |
| [API](architecture/api.md) | Import, browse, export, and restore contracts |

## Architecture

| Document | Purpose |
| --- | --- |
| [System overview](architecture/overview.md) | Components and trust/data boundaries |
| [Contracts](architecture/contracts.md) | Workspace, wire, toolchain, and provenance rules |
| [Storage](architecture/storage.md) | Immutable intake, quotas, and durable jobs |
| [Catalog](architecture/catalog.md) | Desktop adaptation and catalog ownership |
| [Browser](architecture/browser.md) | Rust/WASM client behavior and runtime probes |
| [GPU](architecture/gpu.md) | Optional WebGPU path, fallback, and evidence limits |
| [Formats](architecture/formats.md) | Independently implemented bounded formats |

## Operations and evidence

| Area | Documents |
| --- | --- |
| Quality | [Coverage](operations/coverage.md), [required checks](operations/required-checks.md) |
| Current acceptance | [Final-main status](validation/final-main-status.md) |
| Historical design | [001](design/001-lytronaut.md), [002](design/002-deployment-access.md), [003](design/003-automatic-rendering.md), [004 export](design/004-live-photo-export.md), [004 release](design/004-live-photo-release.md), [005](design/005-catalog-backends.md), [006](design/006-database-boundary.md), [007](design/007-portable-backup.md) |
| Historical experiments | [GPU/WebUI](spikes/lytronaut-0mz-rust-gpu.md), [decode/calibration](spikes/lytronaut-4li-decode-calibration.md), [corpus](spikes/lytronaut-7w9-library-corpus.md), [processed interop](spikes/lytronaut-w6z-processed-interop.md) |
| Source-bound receipts | [PostgreSQL](validation/lytronaut-1yn-postgres-operations.md), [restore](validation/lytronaut-25u-restore-reconciliation.md), [streaming](validation/lytronaut-5ld-desktop-streaming.md), [disk](validation/lytronaut-80j-disk-accounting.md), [focus](validation/lytronaut-b0e-focus-acceptance.md), [checkpoint](validation/lytronaut-cjq-final.md), [private](validation/lytronaut-f5a-private.md), [video](validation/lytronaut-hyy-video.md), [deployment](validation/lytronaut-j2u-deployment.md), [focus checkpoint](validation/lytronaut-m5h-focus.md), [instrumentation](validation/lytronaut-q1o-instrumented.md), [acceptance](validation/lytronaut-re8-3-acceptance.md), [HRD](validation/lytronaut-re8-3-hrd.md), [export](validation/lytronaut-u8i-export.md), [GPU](validation/lytronaut-us7-gpu.md) |
| Documentation audit | [2026-10-18 inventory](documentation-audit-2026-10-18.md) |

Design, spike, and validation documents are historical records. Their dates,
source revisions, failures, and limitations remain authoritative for that
record; use current architecture and operations guides for present behavior.

## Contributing to documentation

- Keep user-facing names lowercase with hyphens and use relative links.
- State whether evidence is current or bound to a historical source/date.
- Never commit private paths, identifiers, captures, calibration bundles, or manifests.
- Keep generated dependency inventories generated; validate them with `just licenses`.
- Use the pinned [diagram style and provenance](diagrams/STYLE.md) for schematics.
- Run `mise exec -- just check` before handoff.

Last reviewed: 2026-10-18.

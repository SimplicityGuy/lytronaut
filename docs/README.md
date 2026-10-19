# Lytronaut documentation

Current operator and developer guides, architecture contracts, and preserved delivery evidence.

[Back to main](../README.md) | [AGF workflow](AGF.md) | [Supported variants](operations/supported-variants.md)

## Getting started

| Document | Purpose |
| --- | --- |
| [Quick start](quick-start.md) | Run the synthetic local demo and verify a checkout |
| [Configuration map](configuration.md) | Find the main runtime settings and exact references |
| [Linux operation](operations/linux.md) | Build, configure, back up, restore, and recover a self-hosted instance |
| [Supported variants](operations/supported-variants.md) | Verified capabilities and explicit limits |
| [AGF workflow](AGF.md) | Plan, implement, review, and merge repository work |

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
| [Authentication](architecture/authentication.md) | Host, origin, mutation-intent, and reverse-proxy boundary |
| [API](architecture/api.md) | Import, browse, export, and restore contracts |
| [Diagram style](diagrams/STYLE.md) | Pinned upstream editorial tokens and provenance |

## Operations

| Area | Documents |
| --- | --- |
| Quality | [Coverage](operations/coverage.md), [required checks](operations/required-checks.md) |
| Current acceptance | [Final-main status](validation/final-main-status.md) |
| Documentation audit | [2026-10-18 inventory](documentation-audit-2026-10-18.md) |

## Design decisions

[001 scope](design/001-lytronaut.md) · [002 access](design/002-deployment-access.md) · [003 rendering](design/003-automatic-rendering.md) · [004 export](design/004-live-photo-export.md) · [004 release](design/004-live-photo-release.md) · [005 catalog](design/005-catalog-backends.md) · [006 database](design/006-database-boundary.md) · [007 backup](design/007-portable-backup.md)

## Research

[GPU/WebUI](spikes/rust-gpu-feasibility.md) · [decode/calibration](spikes/decode-calibration.md) · [corpus](spikes/library-corpus.md) · [processed interop](spikes/processed-interop.md)

## Validation

[PostgreSQL](validation/postgres-operations.md) · [restore](validation/restore-reconciliation.md) · [streaming](validation/desktop-streaming.md) · [disk](validation/disk-accounting.md) · [focus](validation/focus-acceptance.md) · [checkpoint](validation/mvp-validation.md) · [private](validation/private-corpus-validation.md) · [video](validation/video-validation.md) · [deployment](validation/deployment-validation.md) · [focus checkpoint](validation/focus-validation.md) · [instrumentation](validation/instrumented-request-validation.md) · [acceptance](validation/release-acceptance.md) · [HRD](validation/hrd-validation.md) · [export](validation/export-validation.md) · [GPU](validation/gpu-validation.md)

Design, spike, and validation documents are historical records. Their dates,
source revisions, failures, and limitations remain authoritative for that
record; use current architecture and operations guides for present behavior.

## Reading paths

### Operators

1. [Quick start](quick-start.md)
1. [Configuration map](configuration.md)
1. [Linux operation](operations/linux.md)
1. [Supported variants](operations/supported-variants.md)

### Developers

1. [Architecture overview](architecture/overview.md)
1. [Contracts](architecture/contracts.md)
1. [AGF workflow](AGF.md)
1. [Coverage](operations/coverage.md)

### Evidence reviewers

1. [Final-main status](validation/final-main-status.md)
1. [Documentation audit](documentation-audit-2026-10-18.md)
1. [Design scope](design/001-lytronaut.md)
1. [Source-bound validation](validation/release-acceptance.md)

## Contributing to documentation

- Keep user-facing names lowercase with hyphens and use relative links.
- State whether evidence is current or bound to a historical source/date.
- Never commit private paths, identifiers, captures, calibration bundles, or manifests.
- Keep generated dependency inventories generated; validate them with `just licenses`.
- Use the pinned [diagram style and provenance](diagrams/STYLE.md) for schematics.
- Run `mise exec -- just check` before handoff.

Last reviewed: 2026-10-18.

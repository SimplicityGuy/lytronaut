# Selected configuration map

Operational configuration, Compose overlays, secrets, and backup procedures live
in [Linux operation](operations/linux.md). This page is a navigation map for the
settings readers most often need; the source and Compose files remain the exact reference.

| Concern | Settings | Rule |
| --- | --- | --- |
| Application state | `LYTRONAUT_DATA`, `LYTRONAUT_ORIGINALS`, `LYTRONAUT_STAGING` | Never point them at a source library |
| Catalog | `LYTRONAUT_DATABASE_URL`, `LYTRONAUT_DATABASE_PASSWORD_FILE`, `LYTRONAUT_DATABASE_SCHEMA` | SQLite is default; PostgreSQL 18 is supported |
| Request boundary | `LYTRONAUT_ORIGIN` | Mutations require matching Host/Origin and `X-Lytronaut-Intent: mutation` |
| Intake | `LYTRONAUT_UPLOAD_BYTES`, `LYTRONAUT_UPLOAD_FILES`, `LYTRONAUT_FILE_BYTES`, `LYTRONAUT_IMPORT_JOBS` | Invalid or zero policies fail startup |
| Disk admission | `LYTRONAUT_RESERVED_BYTES`, `LYTRONAUT_FREE_FLOOR` | Shared across intake/export/restore accounting |

Use the Compose defaults and variable tables in the Linux guide as the
machine-readable starting point. Keep password files outside the checkout with
the documented modes.

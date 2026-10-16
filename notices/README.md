# License evidence

`just licenses` verifies every external package in the complete Cargo metadata
workspace graph, including WASM and host build/test dependencies. Committed
`DEPENDENCY_LICENSES.md` records exact versions/SPDX expressions;
`THIRD_PARTY_NOTICES.md` reproduces package-shipped notice and license files.
SPDX alternatives are resolved to a permissive option; r-efi is used under MIT,
not LGPL. Conjunctive license obligations remain in effect.

Some crate archives omit their root license files. `licenses/SOURCES.txt` records
upstream retrieval URLs pinned to revisions. Files beneath `licenses/` are legal
evidence, not vendored application implementations. Where a published VCS revision
is unavailable, the record explicitly identifies that fact and the frozen
upstream revision used for its authoritative license text. Unrelated example
and node_modules licenses are excluded.

convert_case_extras 0.2.0, erased 0.1.2 and drain_filter_polyfill 0.1.3 explicitly
declare MIT (or MIT/Apache alternatives) in their original Cargo manifests, but
supply no separate license/copyright file in their archives or recorded upstream
trees. Their original declarations are retained alongside the standard MIT
permission/warranty terms. No copyright holder or copyright year is inferred.
r-efi supplies its complete licensing terms and authors in the archive's AUTHORS
file, which is included verbatim. Original Lytronaut licensing is unchanged.

A dependency update must be audited before `just licenses-update`. Unknown SPDX
expressions and missing license evidence fail closed. No GPL-only dependency or
unlicensed port is permitted. Build-tool binaries installed by mise are not
redistributed in the product. Notices remain checked without any network lookup
beyond Cargo's normal locked package resolution.

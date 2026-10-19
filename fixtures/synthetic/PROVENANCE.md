# Synthetic fixture provenance

`asset-v1.json` was independently authored for Lytronaut on 2026-10-11 by the
foundation implementer. It is licensed under this repository's MIT license.
All identifiers, byte counts and status values are invented. The zero digest is
a schema example, not a claim to hash actual source bytes. There are no images,
camera identifiers, calibration bundles or copied reference-project source.
No private library was accessed to author these fixtures.

Future format vectors must record their independent construction and license
here; private corpus evidence is never a public fixture.

`chrome-macos-library-package.hex` is the hexadecimal encoding of a 729-byte
archive produced on 2026-10-18 when Google Chrome 155.0.8059.82 on macOS 27.0.1
selected an independently authored synthetic Finder package. The package held
only a 42-byte invented text file and a 21-byte invented binary file. Its source
payload SHA-256 values are
`70cf00fea36e4459db9b65e1b2fedf3cf25614d0a2834482429da47e4ddf10b0` and
`1457c648d708a4b69a38dc85f1c5d137367e46975eddea9cd21543cacb2c2b06`;
the archive SHA-256 is
`a04150890b4043e1e0decae07b346b257274b0565854ba151a3c322e8c239dbe`.
The fixture records Chrome's independently generated ZIP framing and is licensed
under this repository's MIT license. It contains no photograph, camera identity,
calibration data, private library content or reference-project source.

`crates/lytronaut-format/tests/formats.rs` independently constructs MIT test
vectors in memory: the packed bytes `ab cd ef`, invented minimal F01/unknown
metadata, synthetic SHA1-framed sections, two tiny RGB JPEG pixels encoded by
`image`, and explicit finite/NaN/infinity scalar arrays. They are mathematical
examples with no camera or capture identifiers. Malformed fixtures mutate only
these invented bytes. No photographic pixels or reference implementation code
were reused. `examples/fuzz_input.rs` accepts bounded synthetic stdin seeds.

`crates/lytronaut-storage/tests/intake.rs`, its publication race unit test, and
`crates/lytronaut-service/tests/uploads.rs` independently create MIT preservation
fixtures in memory and temporary private test directories: invented unknown
file bytes and Unicode logical names, empty files, classic/ZIP64 framing, stored
and DEFLATE data, path/type/count/size/CRC mutations, interrupted publication
files, and synthetic credentials. ZIP framing follows the linked PKWARE
specification without copying its text or another implementation. No private
library, photographic image, camera ID, capture ID, calibration or reference
project implementation was used. Random test object/job identifiers are generated
only at runtime and are not committed as fixtures.

Service API/diagnostics tests independently author schema-23 Unicode records,
nonzero ratings/flags and synthetic preservation bytes. JPEG pixels are simple
red/green RGB samples encoded at test time with the audited pure-Rust image crate.
ZIP framing is independently authored stored-entry framing; no personal capture,
GPL port or third-party image is used. All original test code/data is MIT.

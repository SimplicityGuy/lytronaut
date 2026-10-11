# Independent bounded formats

`lytronaut-format` is a native/WASM neutral independent Rust implementation.
`Container::parse` borrows payloads, validates package/section versions, SHA1
identities, duplicate sections, checked payload boundaries and zero-filled
16-byte alignment from each payload end. It never searches payloads for magic.
A `Section` exposes its checked payload slice and byte offset for downstream
catalog/stream delivery. Caller-owned immutable originals remain authoritative.

Default admission is 128 MiB encoded container, 1024 sections, 2 MiB JSON,
64 nesting levels, 100,000 lexical JSON nodes, 8192 per dimension, 16 million
pixels, 64 MiB decoded output and 128 MiB conservative JPEG working allocation.
JPEG encoded input is at most 48 MiB. Callers should lower these budgets for
viewer assets and independently bound body/file reads before constructing a
slice. JSON lexical preflight runs before allocating serde values and rejects
duplicate object keys. Reference traversal also caps pointer length at 4096
bytes and total retained pointer/identity text at 2 MiB to prevent path
amplification. Errors are stable enums without private strings.

Raw directory major/minor 1/0 with matching referenced F01 metadata, explicit
12-bit big-endian packing and top-left blue Bayer produces `RawAsset::F01`.
Dimensions drive checked payload/sample lengths; `unpack12`/`pack12` are byte
exact. Unknown camera, packing/CFA, or directory versions produce preserved-only.
Unpack is sensor extraction, with no demosaic/calibration/refocus claim.

`ReferenceMode::Raw` requires every Ref locally. Processed mode permits missing
`picture.frameArray[N].frame` source image/public/private metadata references
as `ExternalOriginal`; all acceleration payload refs are required locally.
Syntax is always validated. External identity resolution belongs to catalog
import; this parser does not assert that an external source was supplied.

`processed_asset` preserves the complete bounded directory and offers typed
JPEG/H264 `Plane`s with source acceleration index, frame index, dimensions,
finite lambda, representation, identity and borrowed payload. JPEG stack lambdas
must increase strictly and metadata dimensions must match the encoded JPEG.
H264 payloads are preserved for a separate codec gate, with no decode claim.
Unknown acceleration representations remain in the original directory.
Map descriptors distinguish relative depth/LUT from confidence; length derives
from metadata. Little-endian f32 inspection rejects nonfinite values without
clamping confidence above 1 or claiming metres or correct depth picking.
`Recipe` retains original scalar types and unknown fields without rendering edits.

JPEG SOF/segment preflight bounds dimensions and conservative scratch/output
allocation before even constructing `image`'s decoder. The JPEG-only `image`
0.25.9 path uses `zune-jpeg` 0.5.15 and no native codec. The working estimate is
64 bytes per pixel plus twice encoded input. `image::Limits` additionally guards
dimensions/allocations, but its best-effort allocation limit is not relied on
as a strict allocator quota. Resource admission bounds individual operations;
callers must limit concurrent decodes and apply cancellation/deadline policies.

Public tests synthesize their inputs under MIT. The bounded stdin fuzz target
runs container, reference, JSON, JPEG, float and packed-sample paths; build with
`cargo build -p lytronaut-format --example fuzz_input` and point a mutational
fuzzer at the resulting binary. The decoder's own JPEG robustness remains an
upstream responsibility; malformed inputs fail as `Codec`.

The opt-in private gate is `cargo run --release --locked -p lytronaut-format
--example private_corpus -- <explicit-source-root> <private-manifest>`. It checks
every manifest size/SHA256/mtime before and after decoding, and reports aggregate
counts only. It never writes private source files or outputs images. Do not run
it in public CI or attach the manifest/photos to a review. Compile the crate
independently for `wasm32-unknown-unknown` to validate platform neutrality.

Local implementation evidence (2026-10-11): the optimized gate passed all 109
F01 roundtrips (1,172,665,600 samples), 109 thumbnails and 420 JPEG planes
(90,075,600 pixels), 385 processed containers and 385 finite maps. All 728
manifest files totaling 2,001,830,074 bytes retained identical SHA256/size/mtime
before and after. The crate compiled separately for wasm32-unknown-unknown.
These counts establish this authorized F01 corpus only; Illum remains
preserved-only and no matched calibrated synthesis is available.

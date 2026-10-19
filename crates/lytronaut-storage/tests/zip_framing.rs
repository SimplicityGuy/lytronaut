//! Independently written ZIP framing fixtures (PKWARE APPNOTE structure, MIT):
//! accepted streaming/data-descriptor and directory archives preserve exact
//! bytes, while every hostile framing variant fails closed before any original
//! is published.
use lytronaut_contracts::{ImportKind, ImportRequest, WireVersion};
use lytronaut_storage::{Catalog, Limits, SourceEntry, StorageError, Store};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    store: Store,
}
impl Fixture {
    async fn new() -> Self {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-zip-{}-{:x}",
            std::process::id(),
            u128::from_le_bytes(b)
        ));
        fs::create_dir(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog,
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                upload_bytes: 1024,
                files: 8,
                file_bytes: 512,
                encoded_bytes: 4096,
                reserved_bytes: 32768,
                free_floor: 0,
                metadata_bytes: 4096,
                path_bytes: 64,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        Self { root, store }
    }
    fn published(&self) -> usize {
        fs::read_dir(self.root.join("originals"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != "store.lock")
            .count()
    }
    async fn import(&self, bytes: &[u8]) -> Result<Vec<(String, u64)>, StorageError> {
        Ok(self
            .import_entries(bytes)
            .await?
            .into_iter()
            .map(|e| (e.logical_path, e.length))
            .collect())
    }
    async fn import_entries(&self, bytes: &[u8]) -> Result<Vec<SourceEntry>, StorageError> {
        let s = &self.store;
        let id = s
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: ImportKind::ZipLibrary,
                declared_bytes: 1024,
                declared_files: 8,
            })
            .await?;
        let mut spool = s.spool(&id).await?;
        spool.write(bytes).await?;
        spool.sync().await?;
        match s.import_zip(&id, spool).await {
            Ok(()) => s.manifest(&id).await,
            Err(e) => {
                let _ = s.cancel(&id).await;
                Err(e)
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Descriptor {
    None,
    Signed,
    Unsigned,
    /// ZIP64 sizes in the central extra and a 64-bit descriptor.
    Wide,
}
#[derive(Clone)]
struct Item {
    name: String,
    data: Vec<u8>,
    mode: u32,
    flags: u16,
    method: u16,
    descriptor: Descriptor,
    central_extra: Vec<u8>,
    local_extra: Vec<u8>,
    comment: Vec<u8>,
}
fn file(name: &str, data: &[u8]) -> Item {
    Item {
        name: name.into(),
        data: data.into(),
        mode: 0o100644,
        flags: 0,
        method: 0,
        descriptor: Descriptor::None,
        central_extra: vec![],
        local_extra: vec![],
        comment: vec![],
    }
}
fn directory(name: &str) -> Item {
    Item {
        mode: 0o040755,
        ..file(name, b"")
    }
}
fn extra(tag: u16, value: &[u8]) -> Vec<u8> {
    let mut out = tag.to_le_bytes().to_vec();
    out.extend((value.len() as u16).to_le_bytes());
    out.extend(value);
    out
}
/// Assemble locals, central directory and end record; `gap` inserts bytes
/// between the last payload and the central directory.
fn build_with(items: &[Item], gap: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for item in items {
        let crc = crc32fast::hash(&item.data);
        let size = item.data.len() as u32;
        let descriptor = item.descriptor != Descriptor::None;
        let flags = item.flags | if descriptor { 8 } else { 0 };
        let local = out.len() as u32;
        out.extend(b"PK\x03\x04");
        out.extend(20u16.to_le_bytes());
        out.extend(flags.to_le_bytes());
        out.extend(item.method.to_le_bytes());
        out.extend([0; 4]);
        for v in if descriptor {
            [0, 0, 0]
        } else {
            [crc, size, size]
        } {
            out.extend(v.to_le_bytes());
        }
        out.extend((item.name.len() as u16).to_le_bytes());
        out.extend((item.local_extra.len() as u16).to_le_bytes());
        out.extend(item.name.as_bytes());
        out.extend(&item.local_extra);
        out.extend(&item.data);
        match item.descriptor {
            Descriptor::None => {}
            Descriptor::Signed | Descriptor::Unsigned => {
                if item.descriptor == Descriptor::Signed {
                    out.extend(b"PK\x07\x08");
                }
                out.extend(crc.to_le_bytes());
                out.extend(size.to_le_bytes());
                out.extend(size.to_le_bytes());
            }
            Descriptor::Wide => {
                out.extend(b"PK\x07\x08");
                out.extend(crc.to_le_bytes());
                out.extend(u64::from(size).to_le_bytes());
                out.extend(u64::from(size).to_le_bytes());
            }
        }
        let mut central_extra = item.central_extra.clone();
        let central_size = if item.descriptor == Descriptor::Wide {
            central_extra.extend(extra(
                1,
                &[u64::from(size).to_le_bytes(), u64::from(size).to_le_bytes()].concat(),
            ));
            u32::MAX
        } else {
            size
        };
        central.extend(b"PK\x01\x02");
        central.extend(0x0314u16.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend(flags.to_le_bytes());
        central.extend(item.method.to_le_bytes());
        central.extend([0; 4]);
        central.extend(crc.to_le_bytes());
        central.extend(central_size.to_le_bytes());
        central.extend(central_size.to_le_bytes());
        central.extend((item.name.len() as u16).to_le_bytes());
        central.extend((central_extra.len() as u16).to_le_bytes());
        central.extend((item.comment.len() as u16).to_le_bytes());
        central.extend([0; 4]);
        central.extend((item.mode << 16).to_le_bytes());
        central.extend(local.to_le_bytes());
        central.extend(item.name.as_bytes());
        central.extend(&central_extra);
        central.extend(&item.comment);
    }
    out.extend(gap);
    let offset = out.len() as u32;
    let size = central.len() as u32;
    out.extend(central);
    out.extend(b"PK\x05\x06");
    out.extend([0; 4]);
    out.extend((items.len() as u16).to_le_bytes());
    out.extend((items.len() as u16).to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(offset.to_le_bytes());
    out.extend([0; 2]);
    out
}
fn build(items: &[Item]) -> Vec<u8> {
    build_with(items, &[])
}

fn hex_fixture(value: &str) -> Vec<u8> {
    let digits: Vec<u8> = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    assert_eq!(digits.len() % 2, 0);
    digits
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => panic!("invalid fixture hex"),
            };
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect()
}

#[tokio::test]
async fn chrome_macos_package_archive_preserves_files_and_rejects_placeholder_lies() {
    let archive = hex_fixture(include_str!(
        "../../../fixtures/synthetic/chrome-macos-library-package.hex"
    ));
    assert_eq!(archive.len(), 729);
    let f = Fixture::new().await;
    let mut entries = f.import_entries(&archive).await.unwrap();
    entries.sort_by(|a, b| a.logical_path.cmp(&b.logical_path));
    let manifest: Vec<_> = entries
        .iter()
        .map(|entry| (entry.logical_path.clone(), entry.length))
        .collect();
    assert_eq!(
        manifest,
        [
            ("Synthetic.lytrolibrary/catalog.txt".to_owned(), 42),
            ("Synthetic.lytrolibrary/nested/asset.bin".to_owned(), 21),
        ]
    );
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.digest.as_str())
            .collect::<Vec<_>>(),
        [
            "70cf00fea36e4459db9b65e1b2fedf3cf25614d0a2834482429da47e4ddf10b0",
            "1457c648d708a4b69a38dc85f1c5d137367e46975eddea9cd21543cacb2c2b06",
        ]
    );

    // Every local record has an inert 16-byte ZIP64 placeholder. Nonzero data
    // remains invalid rather than being ignored.
    let local = archive
        .windows(4)
        .position(|bytes| bytes == b"PK\x03\x04")
        .unwrap();
    let name = u16::from_le_bytes(archive[local + 26..local + 28].try_into().unwrap()) as usize;
    let placeholder = local + 30 + name + 4;
    let mut nonzero = archive.clone();
    nonzero[placeholder] = 1;
    assert_eq!(f.import(&nonzero).await, Err(StorageError::Invalid));

    let locals: Vec<usize> = archive
        .windows(4)
        .enumerate()
        .filter_map(|(at, bytes)| (bytes == b"PK\x03\x04").then_some(at))
        .collect();
    let centrals: Vec<usize> = archive
        .windows(4)
        .enumerate()
        .filter_map(|(at, bytes)| (bytes == b"PK\x01\x02").then_some(at))
        .collect();
    let directory = locals[2];
    let directory_name =
        u16::from_le_bytes(archive[directory + 26..directory + 28].try_into().unwrap()) as usize;
    let directory_extra =
        u16::from_le_bytes(archive[directory + 28..directory + 30].try_into().unwrap()) as usize;
    let mut invalid_stream = archive.clone();
    invalid_stream[directory + 30 + directory_name + directory_extra] ^= 1;
    assert_eq!(f.import(&invalid_stream).await, Err(StorageError::Invalid));
    for compressed in [1_u32, 3] {
        let mut wrong_length = archive.clone();
        wrong_length[directory + 18..directory + 22].copy_from_slice(&compressed.to_le_bytes());
        wrong_length[centrals[2] + 20..centrals[2] + 24].copy_from_slice(&compressed.to_le_bytes());
        assert_eq!(f.import(&wrong_length).await, Err(StorageError::Invalid));
    }
    let mut false_crc = archive.clone();
    false_crc[directory + 14] = 1;
    false_crc[centrals[2] + 16] = 1;
    assert_eq!(f.import(&false_crc).await, Err(StorageError::Invalid));
    assert_eq!(f.published(), 2);
}

#[tokio::test]
async fn streaming_descriptors_directories_and_metadata_extras_are_preserved() {
    let f = Fixture::new().await;
    let timestamp = extra(0x5455, &[1, 0, 0, 0, 0]);
    let archive = build(&[
        directory("library/"),
        Item {
            descriptor: Descriptor::Signed,
            central_extra: timestamp.clone(),
            local_extra: timestamp,
            comment: b"synthetic comment".to_vec(),
            ..file("library/signed", b"signed descriptor bytes")
        },
        Item {
            descriptor: Descriptor::Unsigned,
            flags: 0x0800,
            ..file("library/unsigned", b"unsigned descriptor")
        },
        Item {
            descriptor: Descriptor::Wide,
            ..file("library/wide", b"zip64 descriptor")
        },
    ]);
    let mut manifest = f.import(&archive).await.unwrap();
    manifest.sort();
    assert_eq!(
        manifest,
        [
            ("library/signed".to_owned(), 23),
            ("library/unsigned".to_owned(), 19),
            ("library/wide".to_owned(), 16),
        ],
        "directories are structure only; every file keeps its exact length"
    );
    assert_eq!(f.published(), 3);
}

#[tokio::test]
async fn descriptor_and_extra_field_lies_are_rejected_before_publication() {
    let f = Fixture::new().await;
    // Descriptor CRC disagrees with the central directory.
    let mut lying = build(&[Item {
        descriptor: Descriptor::Signed,
        ..file("lying", b"descriptor crc")
    }]);
    let at = lying.windows(4).position(|w| w == b"PK\x07\x08").unwrap();
    lying[at + 4] ^= 1;
    assert_eq!(f.import(&lying).await, Err(StorageError::Invalid));
    for (central_extra, local_extra) in [
        (extra(0x4242, b"unknown"), vec![]),
        (vec![], extra(0x4242, b"unknown")),
        (vec![1, 0, 8], vec![]),
        (vec![], vec![1, 0, 8]),
        (
            [extra(0x000a, &[0; 4]), vec![0x0a, 0, 200, 0]].concat(),
            vec![],
        ),
        (extra(1, &[0; 8]), vec![]),
        (vec![], [extra(1, &[]), extra(1, &[])].concat()),
    ] {
        let archive = build(&[Item {
            central_extra,
            local_extra,
            ..file("extras", b"x")
        }]);
        assert_eq!(f.import(&archive).await, Err(StorageError::Invalid));
    }
    assert_eq!(f.published(), 0);
}

#[tokio::test]
async fn hostile_headers_flags_methods_and_layout_fail_closed() {
    let f = Fixture::new().await;
    let invalid = [
        // Encryption flag, unsupported method.
        build(&[Item {
            flags: 1,
            ..file("encrypted", b"x")
        }]),
        build(&[Item {
            method: 12,
            ..file("bzip2", b"x")
        }]),
        // Bytes between payloads and the central directory are not framed.
        build_with(&[file("gap", b"x")], b"PREAMBLE"),
        // Directory mode on a regular name and a file mode on a directory name.
        build(&[Item {
            mode: 0o040755,
            ..file("mismatch", b"")
        }]),
        build(&[Item {
            mode: 0o100644,
            ..directory("dir/")
        }]),
    ];
    for archive in invalid {
        assert_eq!(f.import(&archive).await, Err(StorageError::Invalid));
    }
    // Local and central names must agree byte for byte.
    let mut renamed = build(&[file("local", b"x")]);
    renamed[30] = b'L';
    assert_eq!(f.import(&renamed).await, Err(StorageError::Invalid));
    // Multi-disk end records and a comment length overrunning the tail.
    let mut disks = build(&[file("disk", b"x")]);
    let end = disks.len() - 22;
    disks[end + 4] = 1;
    assert_eq!(f.import(&disks).await, Err(StorageError::Invalid));
    let mut comment = build(&[file("comment", b"x")]);
    let n = comment.len();
    comment[n - 2] = 5;
    assert_eq!(f.import(&comment).await, Err(StorageError::Invalid));
    // Stored entries whose central sizes disagree, oversize names and too-short files.
    let mut sizes = build(&[file("sizes", b"xy")]);
    let central = sizes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    sizes[central + 20] = 1;
    assert_eq!(f.import(&sizes).await, Err(StorageError::Quota));
    assert_eq!(
        f.import(&build(&[file(&"n".repeat(80), b"x")])).await,
        Err(StorageError::Quota)
    );
    assert_eq!(f.import(b"PK\x05\x06").await, Err(StorageError::Quota));
    // A directory carrying data is a quota violation, not structure.
    let mut full_dir = build(&[directory("full/")]);
    let central = full_dir
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .unwrap();
    full_dir[central + 24] = 1;
    assert_eq!(f.import(&full_dir).await, Err(StorageError::Quota));
    assert_eq!(f.published(), 0);
}

#[tokio::test]
async fn names_are_case_insensitively_unique_and_never_both_file_and_directory() {
    let f = Fixture::new().await;
    assert_eq!(
        f.import(&build(&[file("Album/A", b"1"), file("album/a", b"2")]))
            .await,
        Err(StorageError::Conflict)
    );
    assert_eq!(
        f.import(&build(&[file("album", b"1"), file("album/inner", b"2")]))
            .await,
        Err(StorageError::Conflict)
    );
    // More entries than the configured file limit.
    let many: Vec<Item> = (0..9).map(|i| file(&format!("f{i}"), b"x")).collect();
    assert_eq!(f.import(&build(&many)).await, Err(StorageError::Quota));
    assert_eq!(f.published(), 0);
}

#[tokio::test]
async fn zip64_end_locator_must_be_exact() {
    let f = Fixture::new().await;
    let base = build(&[file("z", b"zip64 locator")]);
    let central = base.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let end = base.len() - 22;
    // Rewrite the end record as ZIP64 with a locator that names a second disk.
    let mut out = base[..end].to_vec();
    let zoffset = out.len() as u64;
    out.extend(b"PK\x06\x06");
    out.extend(44u64.to_le_bytes());
    out.extend([45, 0, 45, 0]);
    out.extend([0; 8]);
    out.extend(1u64.to_le_bytes());
    out.extend(1u64.to_le_bytes());
    out.extend(((end - central) as u64).to_le_bytes());
    out.extend((central as u64).to_le_bytes());
    out.extend(b"PK\x06\x07");
    out.extend([0; 4]);
    out.extend(zoffset.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(b"PK\x05\x06");
    out.extend([0; 4]);
    out.extend([255; 12]);
    out.extend([0; 2]);
    assert_eq!(f.import(&out).await.unwrap(), [("z".to_owned(), 13)]);
    let locator = out.len() - 22 - 20;
    let mut two_disks = out.clone();
    two_disks[locator + 16] = 2;
    assert_eq!(f.import(&two_disks).await, Err(StorageError::Invalid));
    let mut misplaced = out.clone();
    misplaced[locator + 8] = misplaced[locator + 8].wrapping_sub(1);
    assert_eq!(f.import(&misplaced).await, Err(StorageError::Invalid));
    assert_eq!(f.published(), 1);
}

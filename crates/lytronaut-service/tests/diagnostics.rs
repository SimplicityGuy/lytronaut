use lytronaut_service::diagnostics;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, symlink},
    path::PathBuf,
};
fn root() -> PathBuf {
    let mut b = [0; 16];
    getrandom::fill(&mut b).unwrap();
    let root = std::env::temp_dir().join(format!(
        "lytronaut-diagnostics-test-{:x}",
        u128::from_le_bytes(b)
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    root
}
#[tokio::test]
async fn aggregate_inspection_rejects_links_and_import_preserves_unknown_files() {
    let root = root();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("synthetic-sensitive-source-name"),
        b"preservation data",
    )
    .unwrap();
    let report = diagnostics::run("inspect", &source, &root.join("data"))
        .await
        .unwrap();
    let json = serde_json::to_string(&report).unwrap();
    assert_eq!(report.files, 1);
    assert_eq!(report.bytes, 17);
    assert!(!json.contains("synthetic-sensitive"));
    assert!(!json.contains(root.to_str().unwrap()));
    let imported = diagnostics::run("import", &source, &root.join("data"))
        .await
        .unwrap();
    assert_eq!(imported.status, "preserved_only");
    assert_eq!(
        fs::read(source.join("synthetic-sensitive-source-name")).unwrap(),
        b"preservation data"
    );
    symlink(
        source.join("synthetic-sensitive-source-name"),
        source.join("link"),
    )
    .unwrap();
    assert!(
        diagnostics::run("inspect", &source, &root.join("data"))
            .await
            .is_err()
    );
    fs::remove_file(source.join("link")).unwrap();
    fs::hard_link(
        source.join("synthetic-sensitive-source-name"),
        source.join("hard"),
    )
    .unwrap();
    assert!(
        diagnostics::run("inspect", &source, &root.join("data"))
            .await
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

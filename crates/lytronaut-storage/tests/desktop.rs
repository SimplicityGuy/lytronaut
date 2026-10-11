//! Independently authored MIT synthetic Desktop data; never private captures.
use lytronaut_contracts::{ImportKind, ImportRequest, WireVersion};
use lytronaut_storage::{
    Catalog, Limits, Store,
    desktop::{self, CatalogState, OriginalRecord, TypedValue},
};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    fs,
    io::Cursor,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
};
struct Fixture {
    root: PathBuf,
    catalog: Catalog,
    store: Store,
}
impl Fixture {
    async fn new() -> Self {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-catalog-test-{:x}",
            u128::from_le_bytes(b)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                upload_bytes: 4 * 1024 * 1024,
                file_bytes: 2 * 1024 * 1024,
                files: 32,
                free_floor: 0,
                reserved_bytes: 16 * 1024 * 1024,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        Self {
            root,
            catalog,
            store,
        }
    }
    async fn source(&self, change: &str) -> Vec<u8> {
        let path = self.root.join("synthetic.db");
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        let mut c = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        sqlx::raw_sql("CREATE TABLE library_metadata(id INTEGER,version INTEGER);INSERT INTO library_metadata VALUES(1,23);
CREATE TABLE picture(id INTEGER,uuid TEXT,hash TEXT,imagebin_uuid TEXT,event_uuid TEXT,name TEXT,caption TEXT,rating INTEGER,flag_status INTEGER,capture_date TEXT,orientation INTEGER,unknown_blob BLOB,unknown_int INTEGER,unknown_null,unknown_real REAL,\"quote\"\"field\" TEXT);
INSERT INTO picture VALUES(1,'p1','','bin','ev','雪 🌌','Caption with café',4,2,'2024-01-02',0,X'00ff01',9223372036854775807,NULL,0.125,'quoted');
INSERT INTO picture VALUES(2,'p2','','bin','ev','Second','Other caption',1,3,'2024-02-03',0,X'02',-9223372036854775808,NULL,1.5,'q');
CREATE TABLE picture_metadata(id INTEGER,uuid TEXT,lfe_major INTEGER,lfe_minor INTEGER,orientation INTEGER);INSERT INTO picture_metadata VALUES(1,'p1',10,0,8),(2,'p2',11,0,1);
CREATE TABLE album(id INTEGER,uuid TEXT,name TEXT,sort_order INTEGER,public INTEGER);INSERT INTO album VALUES(1,'a','Album α',8,1),(2,'b','Album β',3,0);
CREATE TABLE album_to_picture(id INTEGER,album_uuid TEXT,picture_uuid TEXT,picture_index INTEGER);INSERT INTO album_to_picture VALUES(1,'a','p2',0),(2,'a','p1',1),(3,'b','p1',0);
CREATE TABLE event(id INTEGER,uuid TEXT,library_uuid TEXT,title TEXT);INSERT INTO event VALUES(1,'ev','library-id','Event');
CREATE TABLE image_bin(uuid TEXT);INSERT INTO image_bin VALUES('bin');
CREATE TABLE picture_list(id INTEGER,uuid TEXT,filter INTEGER,sort_order INTEGER);INSERT INTO picture_list VALUES(1,'list',10,1);
CREATE TABLE imported_picture_history(id INTEGER,hash TEXT);INSERT INTO imported_picture_history VALUES(1,'not-current-hash');
CREATE TABLE future_table(secret BLOB);INSERT INTO future_table VALUES(X'ffff');").execute(&mut c).await.unwrap();
        if !change.is_empty() {
            sqlx::raw_sql(sqlx::AssertSqlSafe(change.to_string()))
                .execute(&mut c)
                .await
                .unwrap();
        }
        c.close().await.unwrap();
        fs::read(path).unwrap()
    }
    async fn intake(&self, db: Vec<u8>, missing: bool, journal: bool) -> String {
        let id = self
            .store
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: ImportKind::Directory,
                declared_bytes: 4 * 1024 * 1024,
                declared_files: 32,
            })
            .await
            .unwrap();
        self.store
            .append_directory_file(&id, "lytrolibrary.db", Cursor::new(db))
            .await
            .unwrap();
        for p in ["p1", "p2"] {
            for c in [
                "raw.lfp",
                "dm.lfp",
                "stack.lfp",
                "stack.view.lfp",
                "thumbnail.jpg",
                "view_parameters.json",
            ] {
                if missing && p == "p2" && c == "raw.lfp" {
                    continue;
                }
                let bytes = if c.ends_with("json") {
                    br#"{"version":"4","orientation":8,"unknown":{"large":9223372036854775807,"null":null}}"#.to_vec()
                } else {
                    b"synthetic preserved bytes".to_vec()
                };
                self.store
                    .append_directory_file(&id, &format!("bin/{p}/{c}"), Cursor::new(bytes))
                    .await
                    .unwrap();
            }
        }
        self.store
            .append_directory_file(
                &id,
                "backups/catalog.db",
                Cursor::new(b"opaque backup".to_vec()),
            )
            .await
            .unwrap();
        self.store
            .append_directory_file(&id, "unknown.bin", Cursor::new(vec![0, 255, 13]))
            .await
            .unwrap();
        if journal {
            self.store
                .append_directory_file(&id, "lytrolibrary.db-wal", Cursor::new(vec![0; 32]))
                .await
                .unwrap();
        }
        self.store.finish(&id).await.unwrap();
        id
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[tokio::test]
async fn lossless_types_order_cross_album_versioning_reimport_and_restart() {
    let f = Fixture::new().await;
    let db = f.source("").await;
    let id = f.intake(db.clone(), false, false).await;
    let first = desktop::import_desktop(&f.catalog, &f.store, &id)
        .await
        .unwrap();
    assert_eq!(first.state, CatalogState::Ready);
    assert_eq!(
        (
            first.pictures,
            first.metadata,
            first.albums,
            first.memberships,
            first.item_errors
        ),
        (2, 2, 2, 3, 0)
    );
    assert_eq!(first.files, 15); // 1 DB + 12 companions + backup + unknown
    let rows = sqlx::query(
        "SELECT original FROM catalog_records WHERE version=? AND kind='picture' ORDER BY ordinal",
    )
    .bind(first.version_id)
    .fetch_all(f.catalog.pool())
    .await
    .unwrap();
    let r: OriginalRecord = serde_json::from_slice(&rows[0].get::<Vec<u8>, _>(0)).unwrap();
    assert_eq!(
        r.fields["unknown_int"],
        TypedValue::Integer(i64::MAX.to_string())
    );
    assert_eq!(r.fields["unknown_blob"], TypedValue::Blob(vec![0, 255, 1]));
    assert_eq!(r.fields["unknown_null"], TypedValue::Null);
    assert_eq!(
        r.fields["unknown_real"],
        TypedValue::Real(format!("{:016x}", 0.125f64.to_bits()))
    );
    assert_eq!(
        r.fields["quote\"field"],
        TypedValue::Text(b"quoted".to_vec())
    );
    assert_eq!(
        r.fields["caption"],
        TypedValue::Text("Caption with café".as_bytes().to_vec())
    );
    let order:Vec<String>=sqlx::query_scalar("SELECT picture_uuid FROM catalog_memberships WHERE version=? AND album_uuid='a' ORDER BY picture_index").bind(first.version_id).fetch_all(f.catalog.pool()).await.unwrap();
    assert_eq!(order, ["p2", "p1"]);
    let sort: i64 =
        sqlx::query_scalar("SELECT sort_order FROM catalog_albums WHERE version=? AND uuid='a'")
            .bind(first.version_id)
            .fetch_one(f.catalog.pool())
            .await
            .unwrap();
    assert_eq!(sort, 8);
    let re = f.intake(db, false, false).await;
    assert_eq!(
        desktop::import_desktop(&f.catalog, &f.store, &re)
            .await
            .unwrap()
            .version_id,
        first.version_id
    );
    let wrapped = f
        .store
        .begin(&ImportRequest {
            version: WireVersion::V1,
            kind: ImportKind::Directory,
            declared_bytes: 4 * 1024 * 1024,
            declared_files: 32,
        })
        .await
        .unwrap();
    for entry in f.store.manifest(&id).await.unwrap() {
        f.store
            .append_directory_file(
                &wrapped,
                &format!("Selected/{}", entry.logical_path),
                f.store.open_blob(&entry).unwrap(),
            )
            .await
            .unwrap();
    }
    f.store.finish(&wrapped).await.unwrap();
    assert_eq!(
        desktop::import_desktop(&f.catalog, &f.store, &wrapped)
            .await
            .unwrap()
            .version_id,
        first.version_id
    );
    let changed = f
        .source("UPDATE picture SET caption='changed' WHERE uuid='p1'")
        .await;
    let altered = f.intake(changed, false, false).await;
    let second = desktop::import_desktop(&f.catalog, &f.store, &altered)
        .await
        .unwrap();
    assert_eq!(second.state, CatalogState::Quarantined);
    assert!(second.conflicting_identity);
    assert_ne!(first.version_id, second.version_id);
    let restarted = Catalog::open_application_database(&f.root.join("app.db"))
        .await
        .unwrap();
    assert_eq!(
        desktop::summary(&restarted, first.version_id)
            .await
            .unwrap()
            .memberships,
        3
    );
    assert_eq!(
        desktop::import_desktop(&restarted, &f.store, &id)
            .await
            .unwrap()
            .version_id,
        first.version_id
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_pictures")
        .fetch_one(restarted.pool())
        .await
        .unwrap();
    assert_eq!(count, 4);
}
#[tokio::test]
async fn malformed_or_hostile_inputs_preserve_all_originals_without_semantic_visibility() {
    for change in [
        "UPDATE library_metadata SET version=24",
        "UPDATE picture SET rating='malformed'",
        "UPDATE picture SET name=X'ff'",
        "UPDATE album SET sort_order='malformed'",
        "UPDATE picture SET imagebin_uuid='orphan'",
        "UPDATE album_to_picture SET picture_uuid='orphan' WHERE id=1",
        "INSERT INTO picture_metadata SELECT * FROM picture_metadata WHERE id=1",
        "UPDATE picture SET id=1 WHERE uuid='p2'",
        "DROP TABLE picture; CREATE VIEW picture AS SELECT * FROM picture_metadata",
        "DROP TABLE picture; CREATE VIRTUAL TABLE picture USING fts5(uuid)",
        "ALTER TABLE picture ADD COLUMN generated TEXT GENERATED ALWAYS AS (name || caption) VIRTUAL",
        "ALTER TABLE future_table ADD COLUMN generated BLOB GENERATED ALWAYS AS (zeroblob(4096)) VIRTUAL",
        "CREATE VIRTUAL TABLE hostile_unknown USING fts5(body)",
        "UPDATE picture SET caption=zeroblob(1048577) WHERE uuid='p1'",
    ] {
        let f = Fixture::new().await;
        let db = f.source(change).await;
        let id = f.intake(db, false, false).await;
        let s = desktop::import_desktop(&f.catalog, &f.store, &id)
            .await
            .unwrap();
        assert_eq!(s.state, CatalogState::PreservedOnly, "case {change}");
        assert_eq!(s.files, 15);
        assert_eq!(s.pictures, 0);
        assert_eq!(f.store.manifest(&id).await.unwrap().len(), 15);
    }
    for (db, journal) in [
        (b"not sqlite".to_vec(), false),
        (Fixture::new().await.source("").await, true),
    ] {
        let f = Fixture::new().await;
        let id = f.intake(db, false, journal).await;
        let s = desktop::import_desktop(&f.catalog, &f.store, &id)
            .await
            .unwrap();
        assert_eq!(s.state, CatalogState::PreservedOnly);
    }
}
#[tokio::test]
async fn missing_companion_is_item_error_optional_lfe10_stack_is_normal() {
    let f = Fixture::new().await;
    let db = f.source("").await;
    let id = f.intake(db, true, false).await;
    let s = desktop::import_desktop(&f.catalog, &f.store, &id)
        .await
        .unwrap();
    assert_eq!(s.state, CatalogState::Ready);
    assert_eq!(s.item_errors, 1);
    assert_eq!(s.pictures, 2);
}
#[tokio::test]
async fn migration_rollback_preserves_auth_intake_and_rejects_incomplete_sources() {
    let f = Fixture::new().await;
    let preserved = f
        .store
        .begin(&ImportRequest {
            version: WireVersion::V1,
            kind: ImportKind::Directory,
            declared_bytes: 1024,
            declared_files: 1,
        })
        .await
        .unwrap();
    f.store
        .append_directory_file(
            &preserved,
            "unknown.bin",
            Cursor::new(b"already preserved".to_vec()),
        )
        .await
        .unwrap();
    f.store.finish(&preserved).await.unwrap();
    let before = f.store.manifest(&preserved).await.unwrap();
    sqlx::raw_sql("CREATE TABLE auth_account(singleton INTEGER PRIMARY KEY,password_hash TEXT);INSERT INTO auth_account VALUES(1,'synthetic-auth');CREATE VIEW catalog_records AS SELECT 1;").execute(f.catalog.pool()).await.unwrap();
    // An incompatible existing object causes the additive migration to roll back.
    assert!(desktop::migrate(&f.catalog).await.is_err());
    let tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name='catalog_versions'")
            .fetch_one(f.catalog.pool())
            .await
            .unwrap();
    assert_eq!(tables, 0);
    assert_eq!(f.store.manifest(&preserved).await.unwrap(), before);
    let auth: String = sqlx::query_scalar("SELECT password_hash FROM auth_account")
        .fetch_one(f.catalog.pool())
        .await
        .unwrap();
    assert_eq!(auth, "synthetic-auth");
    sqlx::query("DROP VIEW catalog_records")
        .execute(f.catalog.pool())
        .await
        .unwrap();
    desktop::migrate(&f.catalog).await.unwrap();
    desktop::migrate(&f.catalog).await.unwrap();
    let id = f
        .store
        .begin(&ImportRequest {
            version: WireVersion::V1,
            kind: ImportKind::Directory,
            declared_bytes: 1024,
            declared_files: 1,
        })
        .await
        .unwrap();
    assert!(
        desktop::import_desktop(&f.catalog, &f.store, &id)
            .await
            .is_err()
    );
    assert_eq!(f.store.job(&id).await.unwrap().completed_bytes, 0);
}

#[tokio::test]
async fn schema_admission_and_check_expressions_are_bounded() {
    let f = Fixture::new().await;
    let change = format!(
        "CREATE TABLE oversized_definition(v TEXT DEFAULT '{}')",
        "x".repeat(65537)
    );
    let db = f.source(&change).await;
    let id = f.intake(db, false, false).await;
    assert_eq!(
        desktop::import_desktop(&f.catalog, &f.store, &id)
            .await
            .unwrap()
            .state,
        CatalogState::PreservedOnly
    );
    let f = Fixture::new().await;
    // This false CHECK is deliberately present without evaluating it at creation.
    // If source CHECK expressions ran during quick_check it would fail import.
    let db=f.source("PRAGMA ignore_check_constraints=ON; ALTER TABLE picture ADD COLUMN check_expression INTEGER CHECK(length(hex(zeroblob(4096)))=1)").await;
    let id = f.intake(db, false, false).await;
    assert_eq!(
        desktop::import_desktop(&f.catalog, &f.store, &id)
            .await
            .unwrap()
            .state,
        CatalogState::Ready
    );
    let f = Fixture::new().await;
    let db=f.source("WITH RECURSIVE n(x) AS (VALUES(2) UNION ALL SELECT x+1 FROM n WHERE x<30000) INSERT INTO picture_list SELECT x,'list-'||x,0,1 FROM n").await;
    let id = f.intake(db, false, false).await;
    assert_eq!(
        desktop::import_desktop(&f.catalog, &f.store, &id)
            .await
            .unwrap()
            .state,
        CatalogState::PreservedOnly
    );
}

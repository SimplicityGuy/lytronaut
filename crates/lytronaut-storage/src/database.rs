//! Application connection ownership. Imported Desktop SQLite never enters this module.
use sqlx::{
    Any, AnyPool, ConnectOptions, Connection, SqliteConnection, Transaction,
    any::{AnyConnectOptions, AnyPoolOptions},
    postgres::PgConnectOptions,
    sqlite::SqliteConnectOptions,
};
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

/// Deployment credentials deliberately have no Debug representation.
#[derive(Clone)]
pub enum DatabaseConfig {
    Sqlite(PathBuf),
    Postgres(String),
    PostgresSchema { url: String, schema: String },
}
impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(_) => f.write_str("DatabaseConfig::Sqlite(<application path>)"),
            Self::Postgres(_) => f.write_str("DatabaseConfig::Postgres(<redacted>)"),
            Self::PostgresSchema { .. } => {
                f.write_str("DatabaseConfig::PostgresSchema(<redacted>)")
            }
        }
    }
}
impl DatabaseConfig {
    /// Explicit deployment namespace; never accept a name supplied by an archive.
    /// A generated isolated-restore descriptor can name its owned namespace, but
    /// deployment credentials must still be supplied independently at startup.
    pub fn with_schema(self, schema: &str) -> Result<Self, DatabaseError> {
        if !valid_schema(schema) {
            return Err(DatabaseError::Configuration);
        }
        match self {
            Self::Postgres(url) | Self::PostgresSchema { url, .. } => Ok(Self::PostgresSchema {
                url,
                schema: schema.to_owned(),
            }),
            Self::Sqlite(_) => Err(DatabaseError::Configuration),
        }
    }
    /// Supply the PostgreSQL password separately from a credential-free deployment
    /// URL (for example from a mounted secret file), so the URL can live in
    /// non-secret configuration. A URL that already carries a password is
    /// ambiguous and rejected; neither value ever enters a diagnostic.
    pub fn with_password(self, password: &str) -> Result<Self, DatabaseError> {
        if password.is_empty() || password.len() > 1024 || password.chars().any(char::is_control) {
            return Err(DatabaseError::Configuration);
        }
        let add = |url: String| -> Result<String, DatabaseError> {
            let mut parsed = AnyConnectOptions::from_str(&url)
                .map_err(|_| DatabaseError::Configuration)?
                .database_url;
            if parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query_pairs().any(|(key, _)| key == "password")
            {
                return Err(DatabaseError::Configuration);
            }
            // The driver percent-decodes the userinfo password. Encode every byte
            // except ASCII alphanumerics so `%`, `@`, `:` and `/` survive exactly.
            let encoded: String = password
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() {
                        char::from(b).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            parsed
                .set_password(Some(&encoded))
                .map_err(|_| DatabaseError::Configuration)?;
            let url = parsed.to_string();
            Self::from_deployment(Path::new("unused"), Some(&url))?;
            Ok(url)
        };
        match self {
            Self::Postgres(url) => Ok(Self::Postgres(add(url)?)),
            Self::PostgresSchema { url, schema } => Ok(Self::PostgresSchema {
                url: add(url)?,
                schema,
            }),
            Self::Sqlite(_) => Err(DatabaseError::Configuration),
        }
    }
    pub fn from_deployment(sqlite_path: &Path, url: Option<&str>) -> Result<Self, DatabaseError> {
        match url {
            None => Ok(Self::Sqlite(sqlite_path.to_owned())),
            Some(value)
                if value.starts_with("postgres://") || value.starts_with("postgresql://") =>
            {
                if value.len() > 8192 || value.chars().any(char::is_control) {
                    return Err(DatabaseError::Configuration);
                }
                // Parse now; never include either the parser error or URL in a diagnostic.
                let parsed =
                    AnyConnectOptions::from_str(value).map_err(|_| DatabaseError::Configuration)?;
                // SQLx warns with the value of unrecognized URL parameters. Reject
                // them before the driver parser can emit deployment credentials.
                for (key, _) in parsed.database_url.query_pairs() {
                    if !matches!(
                        key.as_ref(),
                        "sslmode"
                            | "ssl-mode"
                            | "sslrootcert"
                            | "ssl-root-cert"
                            | "ssl-ca"
                            | "sslcert"
                            | "ssl-cert"
                            | "sslkey"
                            | "ssl-key"
                            | "statement-cache-capacity"
                            | "host"
                            | "hostaddr"
                            | "port"
                            | "dbname"
                            | "user"
                            | "password"
                            | "application_name"
                            | "options"
                    ) && !(key.starts_with("options[") && key.ends_with(']'))
                    {
                        return Err(DatabaseError::Configuration);
                    }
                }
                PgConnectOptions::from_str(value).map_err(|_| DatabaseError::Configuration)?;
                Ok(Self::Postgres(value.to_owned()))
            }
            Some(_) => Err(DatabaseError::Configuration),
        }
    }
}

fn valid_schema(schema: &str) -> bool {
    !schema.is_empty()
        && schema.len() <= 63
        && schema
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c == b'_' || (i > 0 && c.is_ascii_digit()))
        && !schema.starts_with("pg_")
        && schema != "information_schema"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseError {
    Configuration,
    Connection,
    Migration,
}
impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid application database configuration; use a PostgreSQL connection URL or omit LYTRONAUT_DATABASE_URL for SQLite",
            Self::Connection => "application database unavailable; check deployment connectivity, credentials and database permissions",
            Self::Migration => "application database migration failed; check permissions, schema compatibility and concurrent writers",
        })
    }
}
impl std::error::Error for DatabaseError {}

#[derive(Clone)]
pub(crate) struct ApplicationDatabase {
    pub(crate) pool: AnyPool,
    pub(crate) backend: Backend,
    sqlite_options: Option<SqliteConnectOptions>,
    sqlite_memory: bool,
    postgres_url: Option<String>,
    admission: WriteAdmission,
}
/// Which writers a PostgreSQL write transaction must serialize against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WriteAdmission {
    /// The live deployment: every writer shares one database-wide advisory key.
    Deployment,
    /// A freshly created, exclusively owned restore namespace. Only its single
    /// restore job writes there, so it must not hold the deployment's key for
    /// the length of a row import and stall live intake/export admission.
    ExclusiveRestore,
}
impl ApplicationDatabase {
    pub(crate) async fn open(config: DatabaseConfig) -> Result<Self, DatabaseError> {
        sqlx::any::install_default_drivers();
        let sqlite_memory =
            matches!(&config, DatabaseConfig::Sqlite(path) if path == Path::new(":memory:"));
        let postgres_url = match &config {
            DatabaseConfig::Postgres(url) | DatabaseConfig::PostgresSchema { url, .. } => {
                Some(url.clone())
            }
            _ => None,
        };
        let namespace = match &config {
            DatabaseConfig::PostgresSchema { schema, .. } if valid_schema(schema) => {
                Some(schema.clone())
            }
            DatabaseConfig::PostgresSchema { .. } => return Err(DatabaseError::Configuration),
            _ => None,
        };
        let (backend, options, sqlite_options) = match config {
            DatabaseConfig::Sqlite(path) => {
                // The previous typed SQLite connector also rejects non-UTF8 paths.
                // Reject before URL roundtripping so no lossy alias can be opened.
                if path.to_str().is_none() {
                    return Err(DatabaseError::Configuration);
                }
                let mut options = SqliteConnectOptions::new()
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_secs(5));
                if path == Path::new(":memory:") {
                    let mut random = [0; 16];
                    getrandom::fill(&mut random).map_err(|_| DatabaseError::Configuration)?;
                    options = options
                        .filename(format!(
                            "file:lytronaut-memory-{:x}",
                            u128::from_le_bytes(random)
                        ))
                        .in_memory(true)
                        .shared_cache(true);
                } else {
                    options = options.filename(&path);
                }
                // SQLx's filename-to-URL helper treats a relative file: URI as
                // an authority and panics on its colon. An opaque sqlite: URL
                // preserves the named memory URI across Any driver reparsing.
                let url = if path == Path::new(":memory:") {
                    format!(
                        "sqlite:{}?mode=memory&cache=shared",
                        options.get_filename().display()
                    )
                } else {
                    options.to_url_lossy().to_string()
                };
                let any_options =
                    AnyConnectOptions::from_str(&url).map_err(|_| DatabaseError::Configuration)?;
                (Backend::Sqlite, any_options, Some(options))
            }
            DatabaseConfig::Postgres(url) | DatabaseConfig::PostgresSchema { url, .. } => {
                DatabaseConfig::from_deployment(Path::new("unused"), Some(&url))?;
                let options =
                    AnyConnectOptions::from_str(&url).map_err(|_| DatabaseError::Configuration)?;
                (Backend::Postgres, options, None)
            }
        };
        let pool = AnyPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(60))
            .after_connect(move |connection, _| {
                let namespace = namespace.clone();
                Box::pin(async move {
                match backend {
                    Backend::Sqlite => {
                        sqlx::raw_sql("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA trusted_schema=OFF;")
                            .execute(connection).await?;
                    }
                    Backend::Postgres => {
                        sqlx::raw_sql("SET statement_timeout='10s'; SET lock_timeout='5s'; SET idle_in_transaction_session_timeout='10s';")
                            .execute(&mut *connection).await?;
                        if let Some(namespace) = namespace {
                            // A bound value avoids identifier interpolation. pg_catalog
                            // remains implicitly searched; public is never a fallback.
                            sqlx::query("SELECT pg_catalog.set_config('search_path',$1,false)")
                                .bind(&namespace).execute(&mut *connection).await?;
                            let selected: Option<String> = sqlx::query_scalar("SELECT current_schema()::text")
                                .fetch_one(&mut *connection).await?;
                            if selected.as_deref() != Some(namespace.as_str()) {
                                return Err(sqlx::Error::Protocol("configured application schema is unavailable".into()));
                            }
                        }
                    }
                }
                Ok(())
            })})
            .connect_with(options.disable_statement_logging())
            .await
            .map_err(|_| DatabaseError::Connection)?;
        Ok(Self {
            pool,
            backend,
            sqlite_options,
            sqlite_memory,
            postgres_url,
            admission: WriteAdmission::Deployment,
        })
    }
    pub(crate) fn with_admission(mut self, admission: WriteAdmission) -> Self {
        self.admission = admission;
        self
    }
    pub(crate) fn namespace_config(&self, schema: &str) -> Result<DatabaseConfig, DatabaseError> {
        DatabaseConfig::Postgres(
            self.postgres_url
                .clone()
                .ok_or(DatabaseError::Configuration)?,
        )
        .with_schema(schema)
    }

    /// All writers that inspect quotas or publish catalog identities use this admission.
    /// The lock is transaction scoped, so cancellation/error/drop cannot strand it.
    pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Any>, sqlx::Error> {
        match self.backend {
            Backend::Sqlite => self.pool.begin_with("BEGIN IMMEDIATE").await,
            Backend::Postgres => {
                let mut tx = self.pool.begin().await?;
                if self.admission == WriteAdmission::Deployment {
                    // Fixed namespace for this application database, not user-controlled data.
                    sqlx::query("SELECT pg_advisory_xact_lock(1280922706, 1)")
                        .execute(&mut *tx)
                        .await?;
                }
                Ok(tx)
            }
        }
    }

    /// Reproduce SQLite rowid allocation under writer admission. PostgreSQL's
    /// nontransactional nextval can be exhausted even after a rollback or ignored
    /// conflicting insert, so generated catalog IDs use the retained table maximum
    /// directly. A retained signed64 MAX uses a bounded positive-rowid search.
    pub(crate) async fn catalog_insert_id(
        &self,
        tx: &mut Transaction<'_, Any>,
    ) -> Result<Option<i64>, sqlx::Error> {
        if self.backend == Backend::Sqlite {
            return Ok(None); // Preserve SQLite's native allocation algorithm.
        }
        let maximum: Option<i64> = sqlx::query_scalar("SELECT max(id) FROM catalog_versions")
            .fetch_one(&mut **tx)
            .await?;
        if maximum != Some(i64::MAX) {
            return Ok(Some(maximum.map_or(1, |id| id + 1)));
        }
        unused_positive_catalog_id(tx, || {
            let mut random = [0; 8];
            getrandom::fill(&mut random).map_err(|_| {
                sqlx::Error::Protocol("catalog identity entropy unavailable".into())
            })?;
            Ok((i64::from_le_bytes(random) & i64::MAX).max(1))
        })
        .await
        .map(Some)
    }

    /// Temporary SQLite snapshot seam for the existing lossless bundle implementation.
    /// No PostgreSQL connection can reach SQLite-specific backup SQL through this method.
    pub(crate) async fn sqlite_snapshot_connection(&self) -> Result<SqliteConnection, sqlx::Error> {
        let options = self.sqlite_options.as_ref().ok_or(sqlx::Error::Protocol(
            "SQLite snapshot requested for a different application backend".to_owned(),
        ))?;
        // Typed connection preserves this worker-channel bound. Any's URL
        // roundtrip drops it; portable snapshots must not use that path.
        // These settings belong to the fresh typed connection; pool after_connect
        // PRAGMAs are not inherited. Named shared-memory fixtures must keep
        // mode=memory, but file snapshots never create a missing database.
        let mut snapshot = options
            .clone()
            .create_if_missing(false)
            .row_buffer_size(1)
            .pragma("query_only", "ON")
            .pragma("trusted_schema", "OFF")
            .pragma("cache_size", "-2048")
            .pragma("mmap_size", "0")
            .pragma("temp_store", "MEMORY");
        if !self.sqlite_memory {
            snapshot = snapshot.read_only(true);
        }
        SqliteConnection::connect_with(&snapshot).await
    }

    pub(crate) async fn migrate_job_order(&self) -> Result<(), sqlx::Error> {
        let mut tx = self.begin_write().await?;
        let ddl = match self.backend {
            Backend::Sqlite => {
                "CREATE TABLE IF NOT EXISTS application_job_order(ordinal INTEGER PRIMARY KEY,kind TEXT NOT NULL,job TEXT NOT NULL,UNIQUE(kind,job))"
            }
            Backend::Postgres => {
                "CREATE TABLE IF NOT EXISTS application_job_order(ordinal BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,kind TEXT NOT NULL,job TEXT NOT NULL,UNIQUE(kind,job))"
            }
        };
        sqlx::query(ddl).execute(&mut *tx).await?;
        tx.commit().await
    }

    /// Derived order allocation follows retained rows, not PostgreSQL's
    /// nontransactional identity sequence. At signed64 exhaustion, rebuild only
    /// auxiliary ordinals inside the already-admitted write transaction.
    pub(crate) async fn record_job_order(
        &self,
        tx: &mut Transaction<'_, Any>,
        kind: &str,
        job: &str,
    ) -> Result<(), sqlx::Error> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM application_job_order WHERE kind=$1 AND job=$2",
        )
        .bind(kind)
        .bind(job)
        .fetch_one(&mut **tx)
        .await?;
        if exists != 0 {
            return Ok(());
        }
        let mut maximum: Option<i64> =
            sqlx::query_scalar("SELECT max(ordinal) FROM application_job_order")
                .fetch_one(&mut **tx)
                .await?;
        if maximum == Some(i64::MAX) {
            // A fixed session-local temporary table avoids in-place uniqueness
            // collisions at MIN/negative/zero/MAX. All original IDs and observable
            // relative order survive; rollback restores the complete old table.
            sqlx::query("CREATE TEMPORARY TABLE lytronaut_order_reindex AS SELECT ROW_NUMBER() OVER(ORDER BY ordinal) AS ordinal,kind,job FROM application_job_order")
                .execute(&mut **tx).await?;
            sqlx::query("DELETE FROM application_job_order")
                .execute(&mut **tx)
                .await?;
            sqlx::query("INSERT INTO application_job_order(ordinal,kind,job) SELECT ordinal,kind,job FROM lytronaut_order_reindex ORDER BY ordinal")
                .execute(&mut **tx).await?;
            sqlx::query("DROP TABLE lytronaut_order_reindex")
                .execute(&mut **tx)
                .await?;
            maximum = sqlx::query_scalar("SELECT max(ordinal) FROM application_job_order")
                .fetch_one(&mut **tx)
                .await?;
        }
        let next = maximum
            .map_or(Some(1), |value| value.checked_add(1))
            .ok_or_else(|| sqlx::Error::Protocol("application job ordering exhausted".into()))?;
        sqlx::query("INSERT INTO application_job_order(ordinal,kind,job) VALUES($1,$2,$3)")
            .bind(next)
            .bind(kind)
            .bind(job)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Called by each job owner's additive migration only after its table exists.
    /// Static table names are selected here, never interpolated from imported data.
    pub(crate) async fn backfill_job_order(&self, kind: JobKind) -> Result<(), sqlx::Error> {
        let mut tx = self.begin_write().await?;
        let statement = match (self.backend, kind) {
            (Backend::Sqlite, JobKind::Intake) => {
                "INSERT INTO application_job_order(kind,job) SELECT 'intake',j.id FROM intake_jobs j WHERE NOT EXISTS(SELECT 1 FROM application_job_order o WHERE o.kind='intake' AND o.job=j.id) ORDER BY j.rowid ON CONFLICT(kind,job) DO NOTHING"
            }
            (Backend::Sqlite, JobKind::Export) => {
                "INSERT INTO application_job_order(kind,job) SELECT 'export',j.id FROM export_jobs j WHERE NOT EXISTS(SELECT 1 FROM application_job_order o WHERE o.kind='export' AND o.job=j.id) ORDER BY j.rowid ON CONFLICT(kind,job) DO NOTHING"
            }
            (Backend::Postgres, JobKind::Intake) => {
                "INSERT INTO application_job_order(kind,job) SELECT 'intake',j.id FROM intake_jobs j WHERE NOT EXISTS(SELECT 1 FROM application_job_order o WHERE o.kind='intake' AND o.job=j.id) ORDER BY j.id ON CONFLICT(kind,job) DO NOTHING"
            }
            (Backend::Postgres, JobKind::Export) => {
                "INSERT INTO application_job_order(kind,job) SELECT 'export',j.id FROM export_jobs j WHERE NOT EXISTS(SELECT 1 FROM application_job_order o WHERE o.kind='export' AND o.job=j.id) ORDER BY j.id ON CONFLICT(kind,job) DO NOTHING"
            }
        };
        // A supported PostgreSQL store always inserts its order record with its job;
        // this backfill repairs only legacy SQLite stores before deployment conversion.
        sqlx::query(statement).execute(&mut *tx).await?;
        tx.commit().await
    }
}

/// The random candidate source is injectable only for deterministic collision
/// tests. Production admission excludes competing application identity writers.
async fn unused_positive_catalog_id(
    tx: &mut Transaction<'_, Any>,
    mut candidate: impl FnMut() -> Result<i64, sqlx::Error>,
) -> Result<i64, sqlx::Error> {
    for _ in 0..100 {
        let id = candidate()?;
        if id <= 0 {
            continue;
        }
        let occupied: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_versions WHERE id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
        if occupied == 0 {
            return Ok(id);
        }
    }
    Err(sqlx::Error::Protocol(
        "catalog identity allocation exhausted".into(),
    ))
}

#[derive(Clone, Copy)]
pub(crate) enum JobKind {
    Intake,
    Export,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[test]
    fn explicit_schema_configuration_cannot_select_a_fallback_or_log_credentials() {
        let postgres = || DatabaseConfig::Postgres("postgres://user:secret@localhost/app".into());
        let config = postgres().with_schema("lytronaut_restore_123abc").unwrap();
        assert!(!format!("{config:?}").contains("secret"));
        assert!(!format!("{config:?}").contains("123abc"));
        for invalid in [
            "",
            "a,public",
            "a;DROP SCHEMA public",
            "pg_catalog",
            "information_schema",
            "1schema",
            "雪",
            "A",
            &"a".repeat(64),
        ] {
            assert!(postgres().with_schema(invalid).is_err());
        }
        assert!(
            DatabaseConfig::Sqlite("catalog.db".into())
                .with_schema("isolated")
                .is_err()
        );
    }

    #[test]
    fn deployment_config_is_explicit_and_redacted() {
        let path = Path::new("application.db");
        assert!(matches!(
            DatabaseConfig::from_deployment(path, None).unwrap(),
            DatabaseConfig::Sqlite(_)
        ));
        let config =
            DatabaseConfig::from_deployment(path, Some("postgres://user:secret@localhost/app"))
                .unwrap();
        assert!(!format!("{config:?}").contains("secret"));
        assert!(!format!("{config:?}").contains("localhost"));
        for invalid in [
            "",
            "mysql://user:secret@localhost/app",
            "sqlite://app",
            "postgres://user:\nsecret@localhost/app",
            "postgres://localhost/app?unknown=secret",
            "postgres://localhost/app?sslmode=invalid",
        ] {
            let error = DatabaseConfig::from_deployment(path, Some(invalid)).unwrap_err();
            assert_eq!(error, DatabaseError::Configuration);
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn separate_password_is_exact_encoded_and_never_ambiguous() {
        fn decode(encoded: &str) -> String {
            let bytes = encoded.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    out.push(u8::from_str_radix(&encoded[i + 1..i + 3], 16).unwrap());
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).unwrap()
        }
        let password = "p%41@:/?#[] \\'\"雪+=&;x";
        let base = "postgres://lytronaut@postgres:5432/lytronaut?sslmode=disable";
        let configured = DatabaseConfig::Postgres(base.into())
            .with_password(password)
            .unwrap();
        assert!(!format!("{configured:?}").contains("p%41"));
        let DatabaseConfig::Postgres(url) = configured else {
            panic!("backend changed")
        };
        let parsed = AnyConnectOptions::from_str(&url).unwrap().database_url;
        assert_eq!(decode(parsed.password().unwrap()), password);
        assert_eq!(parsed.username(), "lytronaut");
        assert_eq!(parsed.host_str(), Some("postgres"));
        assert_eq!(parsed.port(), Some(5432));
        assert_eq!(parsed.path(), "/lytronaut");
        assert_eq!(parsed.query(), Some("sslmode=disable"));
        PgConnectOptions::from_str(&url).unwrap();
        let schema = DatabaseConfig::Postgres(base.into())
            .with_schema("lytronaut")
            .unwrap()
            .with_password("generated")
            .unwrap();
        assert!(matches!(
            schema,
            DatabaseConfig::PostgresSchema { ref schema, .. } if schema == "lytronaut"
        ));
        for (url, password) in [
            (
                "postgres://lytronaut:inline@postgres/lytronaut",
                "generated",
            ),
            ("postgres://postgres/lytronaut?user=lytronaut", "generated"),
            (
                "postgres://lytronaut@postgres/lytronaut?password=inline",
                "generated",
            ),
            (base, ""),
            (base, "line\nbreak"),
            (base, &"x".repeat(1025)),
        ] {
            let error = DatabaseConfig::Postgres(url.into())
                .with_password(password)
                .unwrap_err();
            assert_eq!(error, DatabaseError::Configuration);
            assert!(!error.to_string().contains("inline"));
            assert!(!error.to_string().contains("generated"));
        }
        assert!(
            DatabaseConfig::Sqlite("catalog.db".into())
                .with_password("generated")
                .is_err()
        );
    }

    async fn engine_contract(db: &ApplicationDatabase) {
        let version: String = sqlx::query_scalar(match db.backend {
            Backend::Sqlite => "SELECT sqlite_version()",
            Backend::Postgres => "SHOW server_version",
        })
        .fetch_one(&db.pool)
        .await
        .unwrap();
        println!("actual application engine {:?}: {version}", db.backend);
        let ddl = match db.backend {
            Backend::Sqlite => {
                "CREATE TABLE database_contract(id INTEGER PRIMARY KEY,value INTEGER NOT NULL,payload BLOB NOT NULL)"
            }
            Backend::Postgres => {
                "CREATE TABLE database_contract(id BIGINT PRIMARY KEY,value BIGINT NOT NULL,payload BYTEA NOT NULL)"
            }
        };
        sqlx::query(ddl).execute(&db.pool).await.unwrap();
        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("INSERT INTO database_contract VALUES($1,$2,$3)")
            .bind(1_i64)
            .bind(i64::MIN)
            .bind(vec![0_u8, 255, 128, 0])
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let row = sqlx::query("SELECT value,payload FROM database_contract WHERE id=$1")
            .bind(1_i64)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>("value"), i64::MIN);
        assert_eq!(row.get::<Vec<u8>, _>("payload"), [0, 255, 128, 0]);

        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("UPDATE database_contract SET value=$1 WHERE id=$2")
            .bind(i64::MAX)
            .bind(1_i64)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        let value: i64 = sqlx::query_scalar("SELECT value FROM database_contract WHERE id=$1")
            .bind(1_i64)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(value, i64::MIN);

        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("UPDATE database_contract SET value=$1 WHERE id=$2")
            .bind(i64::MAX)
            .bind(1_i64)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let value: i64 = sqlx::query_scalar("SELECT value FROM database_contract WHERE id=$1")
            .bind(1_i64)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(value, i64::MAX);
        sqlx::query("DROP TABLE database_contract")
            .execute(&db.pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn sqlite_contract_preserves_integer_bytes_and_rollback() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-database-contract-{:x}",
            u128::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("application.db");
        let db = ApplicationDatabase::open(DatabaseConfig::Sqlite(path))
            .await
            .unwrap();
        engine_contract(&db).await;
        // Check every pooled connection, not only the one used for opening the database.
        let mut connections = Vec::new();
        for _ in 0..4 {
            let mut connection = db.pool.acquire().await.unwrap();
            let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            assert_eq!(foreign_keys, 1);
            connections.push(connection);
        }
        drop(connections);
        db.pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database; deployment CI runs this explicitly"]
    async fn postgres_contract_preserves_integer_bytes_and_rollback() {
        let fixture = PgFixture::new().await;
        engine_contract(&fixture.db).await;
        fixture.close().await;
    }

    struct PgFixture {
        db: ApplicationDatabase,
        admin: sqlx::PgConnection,
        schema: String,
    }
    impl PgFixture {
        async fn new() -> Self {
            let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL").expect(
                "LYTRONAUT_TEST_POSTGRES_URL must identify a disposable synthetic test database",
            );
            let options = PgConnectOptions::from_str(&url)
                .unwrap()
                .disable_statement_logging();
            let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
            let restricted: bool = sqlx::query_scalar("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication) FROM pg_roles WHERE rolname=current_user")
                .fetch_one(&mut admin).await.unwrap();
            assert!(
                restricted,
                "parity tests require a restricted application role"
            );
            let public_create: bool =
                sqlx::query_scalar("SELECT has_schema_privilege(current_user,'public','CREATE')")
                    .fetch_one(&mut admin)
                    .await
                    .unwrap();
            assert!(
                !public_create,
                "test role must not create objects in public"
            );
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let schema = format!("lytronaut_test_{:x}", u128::from_le_bytes(random));
            // Identifier contains only a fixed prefix and generated hexadecimal digits.
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
                .execute(&mut admin)
                .await
                .unwrap();
            // Preserve the original deployment URL: PgConnectOptions::to_url_lossy
            // drops server options. Explicitly append our owned schema selection.
            let mut selected = AnyConnectOptions::from_str(&url).unwrap().database_url;
            selected
                .query_pairs_mut()
                .append_pair("options", &format!("-c search_path={schema}"));
            let config =
                DatabaseConfig::from_deployment(Path::new("unused"), Some(selected.as_str()))
                    .unwrap();
            let db = ApplicationDatabase::open(config).await.unwrap();
            let mut connections = Vec::new();
            for _ in 0..4 {
                let mut connection = db.pool.acquire().await.unwrap();
                let actual: String = sqlx::query_scalar("SELECT current_schema()::text")
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap();
                assert_eq!(
                    actual, schema,
                    "each pooled connection must select only the owned schema"
                );
                connections.push(connection);
            }
            drop(connections);
            Self { db, admin, schema }
        }
        async fn close(mut self) {
            self.db.pool.close().await;
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA \"{}\" CASCADE",
                self.schema
            )))
            .execute(&mut self.admin)
            .await
            .unwrap();
            self.admin.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn named_memory_pool_is_shared_and_separate_opens_are_isolated() {
        let first = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE memory_contract(value INTEGER NOT NULL)")
            .execute(&first.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memory_contract VALUES(17)")
            .execute(&first.pool)
            .await
            .unwrap();
        let mut leases = Vec::new();
        for _ in 0..4 {
            let mut connection = first.pool.acquire().await.unwrap();
            let value: i64 = sqlx::query_scalar("SELECT value FROM memory_contract")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            assert_eq!(value, 17);
            leases.push(connection);
        }
        let second = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        assert!(
            sqlx::query("SELECT value FROM memory_contract")
                .execute(&second.pool)
                .await
                .is_err()
        );
        let mut snapshot = first.sqlite_snapshot_connection().await.unwrap();
        let value: i64 = sqlx::query_scalar("SELECT value FROM memory_contract")
            .fetch_one(&mut snapshot)
            .await
            .unwrap();
        assert_eq!(value, 17);
        snapshot.close().await.unwrap();
        drop(leases);
        first.pool.close().await;
        second.pool.close().await;
    }

    #[tokio::test]
    async fn sqlite_legacy_job_order_retains_rowid_order_and_backfill_is_idempotent() {
        let db = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        db.migrate_job_order().await.unwrap();
        for &statement in crate::sqlite_schema::INTAKE {
            sqlx::query(statement).execute(&db.pool).await.unwrap();
        }
        for id in ["z-last-lexically", "a-first-lexically"] {
            sqlx::query("INSERT INTO intake_jobs(id,kind,state,total,budget,max_files,encoded_limit,reserved) VALUES($1,'directory','succeeded',0,0,1,0,0)")
                .bind(id).execute(&db.pool).await.unwrap();
        }
        for _ in 0..2 {
            db.backfill_job_order(JobKind::Intake).await.unwrap();
            let jobs: Vec<String> = sqlx::query_scalar(
                "SELECT job FROM application_job_order WHERE kind='intake' ORDER BY ordinal",
            )
            .fetch_all(&db.pool)
            .await
            .unwrap();
            assert_eq!(jobs, ["z-last-lexically", "a-first-lexically"]);
        }
        db.pool.close().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database; deployment CI runs this explicitly"]
    async fn postgres_admission_lock_is_transaction_scoped_and_rolls_back() {
        let fixture = PgFixture::new().await;
        let first = fixture.db.begin_write().await.unwrap();
        let mut second = fixture.db.pool.begin().await.unwrap();
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(1280922706,1)")
            .fetch_one(&mut *second)
            .await
            .unwrap();
        assert!(!acquired);
        first.rollback().await.unwrap();
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(1280922706,1)")
            .fetch_one(&mut *second)
            .await
            .unwrap();
        assert!(acquired);
        second.rollback().await.unwrap();
        fixture.close().await;
    }

    async fn order_exhaustion_contract(db: &ApplicationDatabase) {
        db.migrate_job_order().await.unwrap();
        let original = [i64::MIN, -1, 0, i64::MAX];
        let mut tx = db.begin_write().await.unwrap();
        for (index, ordinal) in original.into_iter().enumerate() {
            sqlx::query(
                "INSERT INTO application_job_order(ordinal,kind,job) VALUES($1,'intake',$2)",
            )
            .bind(ordinal)
            .bind(format!("source-{index}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        let mut tx = db.begin_write().await.unwrap();
        db.record_job_order(&mut tx, "export", "new-job")
            .await
            .unwrap();
        let ordered: Vec<String> =
            sqlx::query_scalar("SELECT job FROM application_job_order ORDER BY ordinal")
                .fetch_all(&mut *tx)
                .await
                .unwrap();
        assert_eq!(
            ordered,
            ["source-0", "source-1", "source-2", "source-3", "new-job"]
        );
        tx.rollback().await.unwrap();
        let restored: Vec<i64> =
            sqlx::query_scalar("SELECT ordinal FROM application_job_order ORDER BY ordinal")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(restored, original);
        let mut tx = db.begin_write().await.unwrap();
        db.record_job_order(&mut tx, "export", "new-job")
            .await
            .unwrap();
        db.record_job_order(&mut tx, "export", "new-job")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM application_job_order")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 5);
        let ordered: Vec<String> =
            sqlx::query_scalar("SELECT job FROM application_job_order ORDER BY ordinal")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(
            ordered,
            ["source-0", "source-1", "source-2", "source-3", "new-job"]
        );
    }
    #[tokio::test]
    async fn sqlite_derived_order_exhaustion_preserves_order_and_rolls_back() {
        let db = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        order_exhaustion_contract(&db).await;
        db.pool.close().await;
    }
    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database; deployment CI runs this explicitly"]
    async fn postgres_derived_order_exhaustion_preserves_order_and_rolls_back() {
        let fixture = PgFixture::new().await;
        order_exhaustion_contract(&fixture.db).await;
        fixture.close().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database; deployment CI runs this explicitly"]
    async fn postgres_binary_text_projection_matches_actual_sqlite_nul_unicode_and_literals() {
        let sqlite = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        let fixture = PgFixture::new().await;
        sqlx::query("CREATE TABLE projection_contract(id INTEGER PRIMARY KEY,name TEXT NOT NULL)")
            .execute(&sqlite.pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE projection_contract(id BIGINT PRIMARY KEY,name BYTEA NOT NULL,display TEXT NOT NULL,fold BYTEA NOT NULL,bytes BIGINT NOT NULL)")
            .execute(&fixture.db.pool).await.unwrap();
        let mut names: Vec<String> = [
            "a\0b",
            "a\0c",
            "A\0bb",
            "a",
            "a\0",
            "ab",
            "%_~",
            "café",
            "Café",
            "CAFÉ",
            "CAFé",
            "雪",
            "🌌",
            "\0hidden",
            "prefix\0after",
            "literal 100%_~done",
            "\u{1}\u{e000}",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        names.extend([
            "雪".repeat(100),
            "x".repeat(257),
            format!("{}\0ignored", "é".repeat(256)),
        ]);
        for (id, name) in names.iter().enumerate() {
            sqlx::query("INSERT INTO projection_contract VALUES($1,$2)")
                .bind(id as i64)
                .bind(name)
                .execute(&sqlite.pool)
                .await
                .unwrap();
            let projection = crate::source_text::TextProjection::new(name, 256);
            sqlx::query("INSERT INTO projection_contract VALUES($1,$2,$3,$4,$5)")
                .bind(id as i64)
                .bind(projection.original)
                .bind(projection.display)
                .bind(projection.folded_prefix)
                .bind(projection.original_bytes)
                .execute(&fixture.db.pool)
                .await
                .unwrap();
            let raw: Vec<u8> =
                sqlx::query_scalar("SELECT name FROM projection_contract WHERE id=$1")
                    .bind(id as i64)
                    .fetch_one(&fixture.db.pool)
                    .await
                    .unwrap();
            assert_eq!(raw, name.as_bytes());
        }
        let expected: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM projection_contract ORDER BY name COLLATE NOCASE,id",
        )
        .fetch_all(&sqlite.pool)
        .await
        .unwrap();
        let actual: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM projection_contract ORDER BY fold,bytes,id")
                .fetch_all(&fixture.db.pool)
                .await
                .unwrap();
        assert_eq!(actual, expected);
        for literal in [
            "", "%", "_", "~", "CAF", "É", "é", "hidden", "after", "🌌", "done",
        ] {
            let pattern = format!(
                "%{}%",
                literal
                    .replace('~', "~~")
                    .replace('%', "~%")
                    .replace('_', "~_")
            );
            let expected: Vec<i64> = sqlx::query_scalar(
                "SELECT id FROM projection_contract WHERE name LIKE $1 ESCAPE '~' ORDER BY id",
            )
            .bind(pattern)
            .fetch_all(&sqlite.pool)
            .await
            .unwrap();
            let actual: Vec<i64> = sqlx::query_scalar(
                "SELECT id FROM projection_contract WHERE position($1 in fold)>0 ORDER BY id",
            )
            .bind(literal.as_bytes().to_ascii_lowercase())
            .fetch_all(&fixture.db.pool)
            .await
            .unwrap();
            assert_eq!(actual, expected, "literal search parity");
        }
        let expected = sqlx::query("SELECT id,substr(name,1,256) AS display,CASE WHEN length(CAST(name AS BLOB))>256 THEN 1 ELSE 0 END AS truncated FROM projection_contract ORDER BY id")
            .fetch_all(&sqlite.pool).await.unwrap();
        let actual = sqlx::query("SELECT id,display,CASE WHEN bytes>256 THEN 1 ELSE 0 END AS truncated FROM projection_contract ORDER BY id")
            .fetch_all(&fixture.db.pool).await.unwrap();
        for (expected, actual) in expected.iter().zip(&actual) {
            assert_eq!(actual.get::<i64, _>("id"), expected.get::<i64, _>("id"));
            assert_eq!(
                actual.get::<String, _>("display"),
                expected.get::<String, _>("display")
            );
            assert_eq!(
                actual.get::<i64, _>("truncated"),
                expected.get::<i64, _>("truncated")
            );
        }
        sqlite.pool.close().await;
        fixture.close().await;
    }

    async fn collision_and_rollback_contract(db: &ApplicationDatabase) {
        sqlx::query("INSERT INTO catalog_versions(id) VALUES($1),($2)")
            .bind(i64::MAX)
            .bind(7_i64)
            .execute(&db.pool)
            .await
            .unwrap();
        let mut tx = db.begin_write().await.unwrap();
        let mut candidates = [i64::MAX, 7, 0, -1, 11].into_iter();
        let id = unused_positive_catalog_id(&mut tx, || Ok(candidates.next().unwrap()))
            .await
            .unwrap();
        assert_eq!(id, 11);
        assert!(unused_positive_catalog_id(&mut tx, || Ok(7)).await.is_err());
        // A bounded allocation failure never issues an invalid nextval or write:
        // the transaction is still usable and rollback preserves all old IDs.
        sqlx::query("INSERT INTO catalog_versions(id) VALUES($1)")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM catalog_versions ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        assert_eq!(ids, [7, i64::MAX]);
    }

    #[tokio::test]
    async fn sqlite_exhausted_rowid_native_allocation_and_collision_rollback() {
        let db = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE catalog_versions(id INTEGER PRIMARY KEY)")
            .execute(&db.pool)
            .await
            .unwrap();
        collision_and_rollback_contract(&db).await;
        let mut tx = db.begin_write().await.unwrap();
        assert_eq!(db.catalog_insert_id(&mut tx).await.unwrap(), None);
        let native: i64 =
            sqlx::query_scalar("INSERT INTO catalog_versions DEFAULT VALUES RETURNING id")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(native > 0 && native < i64::MAX && native != 7);
        tx.rollback().await.unwrap();
        db.pool.close().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable actual PostgreSQL service via LYTRONAUT_TEST_POSTGRES_URL"]
    async fn postgres_exhausted_identity_uses_positive_collision_checked_fallback() {
        let fixture = PgFixture::new().await;
        sqlx::query(
            "CREATE TABLE catalog_versions(id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY)",
        )
        .execute(&fixture.db.pool)
        .await
        .unwrap();
        collision_and_rollback_contract(&fixture.db).await;
        // Restore reseeds the identity to its exact maximum; do not call nextval.
        sqlx::query("SELECT setval(pg_get_serial_sequence('catalog_versions','id'),$1,true)")
            .bind(i64::MAX)
            .execute(&fixture.db.pool)
            .await
            .unwrap();
        let mut tx = fixture.db.begin_write().await.unwrap();
        let id = fixture
            .db
            .catalog_insert_id(&mut tx)
            .await
            .unwrap()
            .unwrap();
        assert!(id > 0 && id < i64::MAX && id != 7);
        sqlx::query("INSERT INTO catalog_versions(id) VALUES($1)")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM catalog_versions ORDER BY id")
            .fetch_all(&fixture.db.pool)
            .await
            .unwrap();
        assert_eq!(ids, [7, i64::MAX]);
        fixture.close().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable actual PostgreSQL service via LYTRONAUT_TEST_POSTGRES_URL"]
    async fn postgres_catalog_allocation_survives_consumed_max_sequence_without_max_row() {
        let fixture = PgFixture::new().await;
        sqlx::query("CREATE TABLE catalog_versions(id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,fingerprint TEXT UNIQUE)")
            .execute(&fixture.db.pool).await.unwrap();
        sqlx::query("INSERT INTO catalog_versions(id,fingerprint) VALUES(7,'existing')")
            .execute(&fixture.db.pool)
            .await
            .unwrap();
        sqlx::query("SELECT setval(pg_get_serial_sequence('catalog_versions','id'),$1,true)")
            .bind(i64::MAX - 1)
            .execute(&fixture.db.pool)
            .await
            .unwrap();
        let mut tx = fixture.db.begin_write().await.unwrap();
        let last: i64 = sqlx::query_scalar(
            "INSERT INTO catalog_versions(fingerprint) VALUES('rolled-back') RETURNING id",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert_eq!(last, i64::MAX);
        tx.rollback().await.unwrap();
        let mut tx = fixture.db.begin_write().await.unwrap();
        let id = fixture
            .db
            .catalog_insert_id(&mut tx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, 8);
        sqlx::query("INSERT INTO catalog_versions(id,fingerprint) VALUES($1,'new')")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        // A conflicting fingerprint can consume MAX without retaining that row.
        sqlx::query("SELECT setval(pg_get_serial_sequence('catalog_versions','id'),$1,true)")
            .bind(i64::MAX - 1)
            .execute(&fixture.db.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO catalog_versions(fingerprint) VALUES('existing') ON CONFLICT(fingerprint) DO NOTHING")
            .execute(&fixture.db.pool).await.unwrap();
        let mut tx = fixture.db.begin_write().await.unwrap();
        let id = fixture
            .db
            .catalog_insert_id(&mut tx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, 8);
        sqlx::query("INSERT INTO catalog_versions(id,fingerprint) VALUES($1,'new')")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM catalog_versions ORDER BY id")
            .fetch_all(&fixture.db.pool)
            .await
            .unwrap();
        assert_eq!(ids, [7, 8]);
        fixture.close().await;
    }

    async fn prepare_retained_id(db: &ApplicationDatabase, retained: Option<i64>) {
        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("DELETE FROM catalog_versions")
            .execute(&mut *tx)
            .await
            .unwrap();
        if let Some(id) = retained {
            sqlx::query("INSERT INTO catalog_versions(id) VALUES($1)")
                .bind(id)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
    }

    async fn allocate_and_rollback(db: &ApplicationDatabase) -> i64 {
        let mut tx = db.begin_write().await.unwrap();
        let allocated = match db.catalog_insert_id(&mut tx).await.unwrap() {
            None => sqlx::query_scalar("INSERT INTO catalog_versions DEFAULT VALUES RETURNING id")
                .fetch_one(&mut *tx)
                .await
                .unwrap(),
            Some(id) => {
                sqlx::query_scalar("INSERT INTO catalog_versions(id) VALUES($1) RETURNING id")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap()
            }
        };
        tx.rollback().await.unwrap();
        allocated
    }

    #[tokio::test]
    async fn sqlite_native_rowid_empty_negative_zero_and_rollback_contract() {
        let db = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE catalog_versions(id INTEGER PRIMARY KEY)")
            .execute(&db.pool)
            .await
            .unwrap();
        // No existing app schema or trusted bundle row validation excludes these
        // signed64 catalog IDs; derive behavior from the actual linked engine.
        for (retained, expected) in [
            (None, 1),
            (Some(i64::MIN), i64::MIN + 1),
            (Some(-5), -4),
            (Some(-1), 0),
            (Some(0), 1),
            (Some(7), 8),
        ] {
            prepare_retained_id(&db, retained).await;
            assert_eq!(allocate_and_rollback(&db).await, expected);
            assert_eq!(allocate_and_rollback(&db).await, expected);
            let actual: Option<i64> = sqlx::query_scalar("SELECT max(id) FROM catalog_versions")
                .fetch_one(&db.pool)
                .await
                .unwrap();
            assert_eq!(actual, retained);
        }
        db.pool.close().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable actual PostgreSQL service via LYTRONAUT_TEST_POSTGRES_URL"]
    async fn postgres_catalog_empty_negative_zero_allocation_matches_actual_sqlite() {
        let sqlite = ApplicationDatabase::open(DatabaseConfig::Sqlite(":memory:".into()))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE catalog_versions(id INTEGER PRIMARY KEY)")
            .execute(&sqlite.pool)
            .await
            .unwrap();
        let fixture = PgFixture::new().await;
        sqlx::query(
            "CREATE TABLE catalog_versions(id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY)",
        )
        .execute(&fixture.db.pool)
        .await
        .unwrap();
        for retained in [None, Some(i64::MIN), Some(-5), Some(-1), Some(0), Some(7)] {
            prepare_retained_id(&sqlite, retained).await;
            prepare_retained_id(&fixture.db, retained).await;
            let native = allocate_and_rollback(&sqlite).await;
            assert_eq!(allocate_and_rollback(&fixture.db).await, native);
            assert_eq!(allocate_and_rollback(&fixture.db).await, native);
            let actual: Option<i64> = sqlx::query_scalar("SELECT max(id) FROM catalog_versions")
                .fetch_one(&fixture.db.pool)
                .await
                .unwrap();
            assert_eq!(actual, retained);
        }
        sqlite.pool.close().await;
        fixture.close().await;
    }
}

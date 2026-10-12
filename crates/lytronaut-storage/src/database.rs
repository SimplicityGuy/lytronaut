//! Application connection ownership. Imported Desktop SQLite never enters this module.
use sqlx::{
    Any, AnyPool, ConnectOptions, Connection, SqliteConnection, Transaction,
    any::{AnyConnectOptions, AnyPoolOptions},
    sqlite::SqliteConnectOptions, postgres::PgConnectOptions,
};
use std::{fmt, path::{Path, PathBuf}, str::FromStr, time::Duration};

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
}
impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(_) => f.write_str("DatabaseConfig::Sqlite(<application path>)"),
            Self::Postgres(_) => f.write_str("DatabaseConfig::Postgres(<redacted>)"),
        }
    }
}
impl DatabaseConfig {
    pub fn from_deployment(sqlite_path: &Path, url: Option<&str>) -> Result<Self, DatabaseError> {
        match url {
            None => Ok(Self::Sqlite(sqlite_path.to_owned())),
            Some(value) if value.starts_with("postgres://") || value.starts_with("postgresql://") => {
                if value.len() > 8192 || value.chars().any(char::is_control) {
                    return Err(DatabaseError::Configuration);
                }
                // Parse now; never include either the parser error or URL in a diagnostic.
                let parsed = AnyConnectOptions::from_str(value)
                    .map_err(|_| DatabaseError::Configuration)?;
                // SQLx warns with the value of unrecognized URL parameters. Reject
                // them before the driver parser can emit deployment credentials.
                for (key, _) in parsed.database_url.query_pairs() {
                    if !matches!(key.as_ref(),
                        "sslmode" | "ssl-mode" | "sslrootcert" | "ssl-root-cert" | "ssl-ca"
                        | "sslcert" | "ssl-cert" | "sslkey" | "ssl-key"
                        | "statement-cache-capacity" | "host" | "hostaddr" | "port"
                        | "dbname" | "user" | "password" | "application_name" | "options")
                        && !(key.starts_with("options[") && key.ends_with(']'))
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
    sqlite_path: Option<PathBuf>,
}
impl ApplicationDatabase {
    pub(crate) async fn open(config: DatabaseConfig) -> Result<Self, DatabaseError> {
        sqlx::any::install_default_drivers();
        let (backend, options, sqlite_path) = match config {
            DatabaseConfig::Sqlite(path) => {
                let options = SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_secs(5));
                let url = options.to_url_lossy();
                let options = AnyConnectOptions::from_str(url.as_str())
                    .map_err(|_| DatabaseError::Configuration)?;
                (Backend::Sqlite, options, Some(path))
            }
            DatabaseConfig::Postgres(url) => {
                DatabaseConfig::from_deployment(Path::new("unused"), Some(&url))?;
                let options = AnyConnectOptions::from_str(&url)
                    .map_err(|_| DatabaseError::Configuration)?;
                (Backend::Postgres, options, None)
            }
        };
        let pool = AnyPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(60))
            .after_connect(move |connection, _| Box::pin(async move {
                match backend {
                    Backend::Sqlite => {
                        sqlx::raw_sql("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA trusted_schema=OFF;")
                            .execute(connection).await?;
                    }
                    Backend::Postgres => {
                        sqlx::raw_sql("SET statement_timeout='10s'; SET lock_timeout='5s'; SET idle_in_transaction_session_timeout='10s';")
                            .execute(connection).await?;
                    }
                }
                Ok(())
            }))
            .connect_with(options.disable_statement_logging())
            .await
            .map_err(|_| DatabaseError::Connection)?;
        Ok(Self { pool, backend, sqlite_path })
    }

    /// All writers that inspect quotas or publish catalog identities use this admission.
    /// The lock is transaction scoped, so cancellation/error/drop cannot strand it.
    pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Any>, sqlx::Error> {
        match self.backend {
            Backend::Sqlite => self.pool.begin_with("BEGIN IMMEDIATE").await,
            Backend::Postgres => {
                let mut tx = self.pool.begin().await?;
                // Fixed namespace for this application database, not user-controlled data.
                sqlx::query("SELECT pg_advisory_xact_lock(1280922706, 1)")
                    .execute(&mut *tx).await?;
                Ok(tx)
            }
        }
    }

    /// Temporary SQLite snapshot seam for the existing lossless bundle implementation.
    /// No PostgreSQL connection can reach SQLite-specific backup SQL through this method.
    pub(crate) async fn sqlite_snapshot_connection(&self) -> Result<SqliteConnection, sqlx::Error> {
        let path = self.sqlite_path.as_ref().ok_or(sqlx::Error::Protocol(
            "SQLite snapshot requested for a different application backend".to_owned(),
        ))?;
        SqliteConnection::connect_with(&SqliteConnectOptions::new()
            .filename(path)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5)))
            .await
    }

    pub(crate) async fn migrate_job_order(&self) -> Result<(), sqlx::Error> {
        let mut tx = self.begin_write().await?;
        let ddl = match self.backend {
            Backend::Sqlite => "CREATE TABLE IF NOT EXISTS application_job_order(ordinal INTEGER PRIMARY KEY,kind TEXT NOT NULL,job TEXT NOT NULL,UNIQUE(kind,job))",
            Backend::Postgres => "CREATE TABLE IF NOT EXISTS application_job_order(ordinal BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,kind TEXT NOT NULL,job TEXT NOT NULL,UNIQUE(kind,job))",
        };
        sqlx::query(ddl).execute(&mut *tx).await?;
        tx.commit().await
    }

    /// Called by each job owner's additive migration only after its table exists.
    /// Static table names are selected here, never interpolated from imported data.
    pub(crate) async fn backfill_job_order(&self, kind: JobKind) -> Result<(), sqlx::Error> {
        let mut tx = self.begin_write().await?;
        let statement = match (self.backend, kind) {
            (Backend::Sqlite, JobKind::Intake) => "INSERT INTO application_job_order(kind,job) SELECT 'intake',id FROM intake_jobs WHERE 1 ORDER BY rowid ON CONFLICT(kind,job) DO NOTHING",
            (Backend::Sqlite, JobKind::Export) => "INSERT INTO application_job_order(kind,job) SELECT 'export',id FROM export_jobs WHERE 1 ORDER BY rowid ON CONFLICT(kind,job) DO NOTHING",
            (Backend::Postgres, JobKind::Intake) => "INSERT INTO application_job_order(kind,job) SELECT 'intake',id FROM intake_jobs ORDER BY id ON CONFLICT(kind,job) DO NOTHING",
            (Backend::Postgres, JobKind::Export) => "INSERT INTO application_job_order(kind,job) SELECT 'export',id FROM export_jobs ORDER BY id ON CONFLICT(kind,job) DO NOTHING",
        };
        // A supported PostgreSQL store always inserts its order record with its job;
        // this backfill repairs only legacy SQLite stores before deployment conversion.
        sqlx::query(statement).execute(&mut *tx).await?;
        tx.commit().await
    }
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
    fn deployment_config_is_explicit_and_redacted() {
        let path = Path::new("application.db");
        assert!(matches!(DatabaseConfig::from_deployment(path, None).unwrap(), DatabaseConfig::Sqlite(_)));
        let config = DatabaseConfig::from_deployment(path, Some("postgres://user:secret@localhost/app")).unwrap();
        assert!(!format!("{config:?}").contains("secret"));
        assert!(!format!("{config:?}").contains("localhost"));
        for invalid in ["", "mysql://user:secret@localhost/app", "sqlite://app", "postgres://user:\nsecret@localhost/app", "postgres://localhost/app?unknown=secret", "postgres://localhost/app?sslmode=invalid"] {
            let error = DatabaseConfig::from_deployment(path, Some(invalid)).unwrap_err();
            assert_eq!(error, DatabaseError::Configuration);
            assert!(!error.to_string().contains("secret"));
        }
    }

    async fn engine_contract(db: &ApplicationDatabase) {
        let ddl = match db.backend {
            Backend::Sqlite => "CREATE TABLE database_contract(id INTEGER PRIMARY KEY,value INTEGER NOT NULL,payload BLOB NOT NULL)",
            Backend::Postgres => "CREATE TABLE database_contract(id BIGINT PRIMARY KEY,value BIGINT NOT NULL,payload BYTEA NOT NULL)",
        };
        sqlx::query(ddl).execute(&db.pool).await.unwrap();
        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("INSERT INTO database_contract VALUES($1,$2,$3)")
            .bind(1_i64).bind(i64::MIN).bind(vec![0_u8, 255, 128, 0])
            .execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let row = sqlx::query("SELECT value,payload FROM database_contract WHERE id=$1")
            .bind(1_i64).fetch_one(&db.pool).await.unwrap();
        assert_eq!(row.get::<i64, _>("value"), i64::MIN);
        assert_eq!(row.get::<Vec<u8>, _>("payload"), [0, 255, 128, 0]);

        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("UPDATE database_contract SET value=$1 WHERE id=$2")
            .bind(i64::MAX).bind(1_i64).execute(&mut *tx).await.unwrap();
        tx.rollback().await.unwrap();
        let value: i64 = sqlx::query_scalar("SELECT value FROM database_contract WHERE id=$1")
            .bind(1_i64).fetch_one(&db.pool).await.unwrap();
        assert_eq!(value, i64::MIN);

        let mut tx = db.begin_write().await.unwrap();
        sqlx::query("UPDATE database_contract SET value=$1 WHERE id=$2")
            .bind(i64::MAX).bind(1_i64).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let value: i64 = sqlx::query_scalar("SELECT value FROM database_contract WHERE id=$1")
            .bind(1_i64).fetch_one(&db.pool).await.unwrap();
        assert_eq!(value, i64::MAX);
        sqlx::query("DROP TABLE database_contract").execute(&db.pool).await.unwrap();
    }

    #[tokio::test]
    async fn sqlite_contract_preserves_integer_bytes_and_rollback() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!("lytronaut-database-contract-{:x}", u128::from_le_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("application.db");
        let db = ApplicationDatabase::open(DatabaseConfig::Sqlite(path)).await.unwrap();
        engine_contract(&db).await;
        // Check every pooled connection, not only the one used for opening the database.
        let mut connections = Vec::new();
        for _ in 0..4 {
            let mut connection = db.pool.acquire().await.unwrap();
            let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *connection).await.unwrap();
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
        let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL")
            .expect("LYTRONAUT_TEST_POSTGRES_URL must identify a disposable synthetic test database");
        let config = DatabaseConfig::from_deployment(Path::new("unused"), Some(&url)).unwrap();
        let db = ApplicationDatabase::open(config).await.unwrap();
        engine_contract(&db).await;
        db.pool.close().await;
    }
}

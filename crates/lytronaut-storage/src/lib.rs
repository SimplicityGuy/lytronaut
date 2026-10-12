//! Native catalog and durable job boundary. Foundation opens only the application's DB.
#![cfg(not(target_arch = "wasm32"))]
mod database;
pub mod desktop;
pub mod intake;
mod postgres_schema;
mod source_text;
mod sqlite_schema;
mod zip;
use database::{ApplicationDatabase, JobKind};
pub use database::{Backend, DatabaseConfig, DatabaseError};
pub use intake::{FaultPoint, Limits, SourceEntry, StorageError, Store};
use sqlx::{Any, AnyPool, SqliteConnection, Transaction};
use std::path::Path;

#[derive(Clone)]
pub struct Catalog {
    database: ApplicationDatabase,
}

#[derive(Clone, Copy)]
pub enum SchemaComponent {
    Intake,
    Catalog,
    Api,
    Export,
}
impl Catalog {
    /// This path must refer to the application's catalog, never an imported source library.
    pub async fn open_application_database(path: &Path) -> Result<Self, sqlx::Error> {
        Self::open(DatabaseConfig::Sqlite(path.to_owned()))
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))
    }
    pub async fn open(config: DatabaseConfig) -> Result<Self, DatabaseError> {
        let database = ApplicationDatabase::open(config).await?;
        database
            .migrate_job_order()
            .await
            .map_err(|_| DatabaseError::Migration)?;
        Ok(Self { database })
    }
    pub fn pool(&self) -> &AnyPool {
        &self.database.pool
    }
    pub fn backend(&self) -> Backend {
        self.database.backend
    }
    pub fn dialect(&self, sqlite: &'static str, postgres: &'static str) -> &'static str {
        match self.backend() {
            Backend::Sqlite => sqlite,
            Backend::Postgres => postgres,
        }
    }
    pub fn schema_statements(&self, component: SchemaComponent) -> &'static [&'static str] {
        match (self.backend(), component) {
            (Backend::Sqlite, SchemaComponent::Intake) => sqlite_schema::INTAKE,
            (Backend::Sqlite, SchemaComponent::Catalog) => sqlite_schema::CATALOG,
            (Backend::Sqlite, SchemaComponent::Api) => sqlite_schema::API,
            (Backend::Sqlite, SchemaComponent::Export) => sqlite_schema::EXPORT,
            (Backend::Postgres, SchemaComponent::Intake) => postgres_schema::INTAKE,
            (Backend::Postgres, SchemaComponent::Catalog) => postgres_schema::CATALOG,
            (Backend::Postgres, SchemaComponent::Api) => postgres_schema::API,
            (Backend::Postgres, SchemaComponent::Export) => postgres_schema::EXPORT,
        }
    }
    pub async fn begin_write(&self) -> Result<Transaction<'static, Any>, sqlx::Error> {
        self.database.begin_write().await
    }
    pub(crate) async fn catalog_insert_id(
        &self,
        tx: &mut Transaction<'_, Any>,
    ) -> Result<Option<i64>, sqlx::Error> {
        self.database.catalog_insert_id(tx).await
    }
    /// A consistent read transaction for portable catalog snapshots. Never execute
    /// input archive SQL or source Desktop schema statements through this connection.
    pub async fn begin_snapshot(&self) -> Result<Transaction<'static, Any>, sqlx::Error> {
        match self.backend() {
            Backend::Sqlite => self.pool().begin().await,
            Backend::Postgres => {
                self.pool()
                    .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    .await
            }
        }
    }
    pub async fn sqlite_snapshot_connection(&self) -> Result<SqliteConnection, sqlx::Error> {
        self.database.sqlite_snapshot_connection().await
    }
    pub async fn backfill_job_order(&self, component: SchemaComponent) -> Result<(), sqlx::Error> {
        let kind = match component {
            SchemaComponent::Intake => JobKind::Intake,
            SchemaComponent::Export => JobKind::Export,
            _ => {
                return Err(sqlx::Error::Protocol(
                    "invalid job ordering component".into(),
                ));
            }
        };
        self.database.backfill_job_order(kind).await
    }
    pub async fn record_job_order(
        &self,
        tx: &mut Transaction<'_, Any>,
        component: SchemaComponent,
        job: &str,
    ) -> Result<(), sqlx::Error> {
        let kind = match component {
            SchemaComponent::Intake => "intake",
            SchemaComponent::Export => "export",
            _ => {
                return Err(sqlx::Error::Protocol(
                    "invalid job ordering component".into(),
                ));
            }
        };
        sqlx::query("INSERT INTO application_job_order(kind,job) VALUES($1,$2) ON CONFLICT(kind,job) DO NOTHING")
            .bind(kind).bind(job).execute(&mut **tx).await?;
        Ok(())
    }
    /// PostgreSQL source identities are binary-safe encoded; generated app IDs are not.
    pub fn encode_source_key(&self, value: &str) -> String {
        match self.backend() {
            Backend::Sqlite => value.to_owned(),
            Backend::Postgres => source_text::encode_key(value),
        }
    }
    pub fn decode_source_key(&self, value: String) -> Result<String, sqlx::Error> {
        match self.backend() {
            Backend::Sqlite => Ok(value),
            Backend::Postgres => source_text::decode_key(&value).map_err(|_| {
                sqlx::Error::Protocol("invalid application source-key encoding".into())
            }),
        }
    }
    pub async fn healthy(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(self.pool()).await?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sqlite_is_live_with_foreign_keys() {
        let catalog = Catalog::open_application_database(Path::new(":memory:"))
            .await
            .unwrap();
        catalog.healthy().await.unwrap();
        let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(enabled, 1);
    }
}

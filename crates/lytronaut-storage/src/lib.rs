//! Native catalog and durable job boundary. Foundation opens only the application's DB.
#![cfg(not(target_arch = "wasm32"))]
pub mod desktop;
pub mod intake;
mod zip;
pub use intake::{FaultPoint, Limits, SourceEntry, StorageError, Store};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions};
use std::path::Path;

#[derive(Clone)]
pub struct Catalog {
    pool: SqlitePool,
}
impl Catalog {
    /// This path must refer to the application's catalog, never an imported source library.
    pub async fn open_application_database(path: &Path) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        // Only application databases enter this path. Remove obsolete credential state
        // transactionally; immutable imported databases and originals are never opened here.
        let mut tx = pool.begin().await?;
        sqlx::raw_sql("PRAGMA secure_delete=ON; DROP TABLE IF EXISTS auth_sessions; DROP TABLE IF EXISTS auth_attempts; DROP TABLE IF EXISTS auth_account;")
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self { pool })
    }
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
    pub async fn healthy(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
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

//! Exact owned PostgreSQL restore namespaces. Never enumerate or drop by prefix.
//! SQL audit: the interpolated schema is re-derived by `validate()` (called first
//! in every entry point) from a 64-hex nonce, so it is always
//! `lytronaut_restore_[0-9a-f]{32}`; table names come only from static
//! application DDL (`owned_tables`); job/nonce values are bound.
use crate::{Backend, Catalog, StorageError as E};
use sqlx::{AssertSqlSafe, Row};

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RestoreNamespace {
    job: String,
    nonce: String,
    schema: String,
}
impl std::fmt::Debug for RestoreNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RestoreNamespace(<owned identity redacted>)")
    }
}
impl RestoreNamespace {
    pub fn new(job: &str, nonce: &str) -> Result<Self, E> {
        if !job_identity(job) || !identity(nonce) {
            return Err(E::Invalid);
        }
        Ok(Self {
            job: job.to_owned(),
            nonce: nonce.to_owned(),
            schema: format!("lytronaut_restore_{}", &nonce[..32]),
        })
    }
    pub fn validate(&self) -> Result<(), E> {
        if Self::new(&self.job, &self.nonce)? != *self {
            return Err(E::Invalid);
        }
        Ok(())
    }
    pub fn schema(&self) -> &str {
        &self.schema
    }
    pub fn job(&self) -> &str {
        &self.job
    }
    pub async fn verify(&self, catalog: &Catalog) -> Result<(), E> {
        self.validate()?;
        if catalog.backend() != Backend::Postgres {
            return Err(E::Invalid);
        }
        let statement = format!(
            "SELECT job,nonce FROM \"{}\".restore_ownership WHERE singleton=1",
            self.schema
        );
        let marker = sqlx::query(AssertSqlSafe(statement.as_str()))
            .fetch_optional(catalog.pool())
            .await?
            .ok_or(E::Conflict)?;
        if marker.try_get::<String, _>(0)? != self.job
            || marker.try_get::<String, _>(1)? != self.nonce
        {
            return Err(E::Conflict);
        }
        Ok(())
    }

    /// Caller must durably prepare its private journal BEFORE entering here.
    /// CREATE plus marker commit together, so an interrupted transaction cannot
    /// leave an unmarked namespace. Existing same-name schemas always conflict.
    pub async fn create(&self, catalog: &Catalog) -> Result<(), E> {
        self.validate()?;
        if catalog.backend() != Backend::Postgres {
            return Err(E::Invalid);
        }
        let mut tx = catalog.begin_write().await?;
        let create = format!("CREATE SCHEMA \"{}\"", self.schema);
        sqlx::query(AssertSqlSafe(create.as_str()))
            .execute(&mut *tx)
            .await?;
        let marker = format!(
            "CREATE TABLE \"{}\".restore_ownership(singleton BIGINT PRIMARY KEY CHECK(singleton=1),job TEXT NOT NULL,nonce TEXT NOT NULL)",
            self.schema
        );
        sqlx::query(AssertSqlSafe(marker.as_str()))
            .execute(&mut *tx)
            .await?;
        let insert = format!(
            "INSERT INTO \"{}\".restore_ownership(singleton,job,nonce) VALUES(1,$1,$2)",
            self.schema
        );
        sqlx::query(AssertSqlSafe(insert.as_str()))
            .bind(&self.job)
            .bind(&self.nonce)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    /// Absence is idempotent success; presence without the exact ownership marker
    /// fails closed. SQL failure leaves the caller's journal/reservation intact.
    pub async fn cleanup(&self, catalog: &Catalog) -> Result<(), E> {
        self.validate()?;
        if catalog.backend() != Backend::Postgres {
            return Err(E::Invalid);
        }
        let mut tx = catalog.begin_write().await?;
        let exists: i64 = sqlx::query_scalar(
            "SELECT CAST(count(*) AS BIGINT) FROM pg_catalog.pg_namespace WHERE nspname=$1",
        )
        .bind(&self.schema)
        .fetch_one(&mut *tx)
        .await?;
        if exists == 0 {
            tx.commit().await?;
            return Ok(());
        }
        let check = format!(
            "SELECT job,nonce FROM \"{}\".restore_ownership WHERE singleton=1",
            self.schema
        );
        let marker = sqlx::query(AssertSqlSafe(check.as_str()))
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(E::Conflict)?;
        if marker.try_get::<String, _>(0)? != self.job
            || marker.try_get::<String, _>(1)? != self.nonce
        {
            return Err(E::Conflict);
        }
        // Never CASCADE: an unrelated object elsewhere that depends on a restored
        // table must make cleanup fail closed (journal and quota retained), not be
        // silently dropped. Drop exactly the trusted application tables together
        // (RESTRICT), then the namespace (RESTRICT), so anything unexpected inside
        // it, or depending on it from outside, aborts the whole transaction.
        let tables = owned_tables()
            .map(|table| format!("\"{}\".{table}", self.schema))
            .collect::<Vec<_>>()
            .join(",");
        let drop_tables = format!("DROP TABLE IF EXISTS {tables} RESTRICT");
        sqlx::query(AssertSqlSafe(drop_tables.as_str()))
            .execute(&mut *tx)
            .await?;
        let drop = format!("DROP SCHEMA \"{}\" RESTRICT", self.schema);
        sqlx::query(AssertSqlSafe(drop.as_str()))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
/// Tables created in a restore namespace: the trusted PostgreSQL application DDL
/// plus job order and the ownership marker. Names come only from static DDL.
fn owned_tables() -> impl Iterator<Item = &'static str> {
    use crate::postgres_schema::{API, CATALOG, EXPORT, INTAKE};
    [INTAKE, CATALOG, API, EXPORT]
        .into_iter()
        .flatten()
        .filter_map(|statement| statement.strip_prefix("CREATE TABLE IF NOT EXISTS "))
        .map(|rest| {
            // The whole identifier up to the column list; unit-tested to be a
            // plain lowercase SQL identifier for every static DDL statement.
            let end = rest
                .find(|c: char| c == '(' || c.is_whitespace())
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .chain(["application_job_order", "restore_ownership"])
}
fn job_identity(value: &str) -> bool {
    lytronaut_contracts::OpaqueId::try_from(value.to_owned()).is_ok()
}
fn identity(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_tables_cover_every_portable_table_and_only_static_identifiers() {
        let owned: Vec<_> = owned_tables().collect();
        for table in crate::portable::TABLES {
            assert!(owned.contains(&table.name), "{} not owned", table.name);
        }
        for name in ["export_jobs", "catalog_item_errors", "restore_ownership"] {
            assert!(owned.contains(&name));
        }
        assert!(owned.iter().all(|name| {
            name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }));
    }
    #[test]
    fn journal_identity_cannot_redirect_cleanup_to_an_arbitrary_schema() {
        let owned = RestoreNamespace::new(&"1".repeat(64), &"a".repeat(64)).unwrap();
        assert_eq!(owned.schema().len(), 50);
        owned.validate().unwrap();
        let mut untrusted = owned.clone();
        untrusted.schema = "public".into();
        assert!(untrusted.validate().is_err());
        for value in ["", "../other", &"a".repeat(129)] {
            assert!(RestoreNamespace::new(value, &"a".repeat(64)).is_err());
        }
        let json = serde_json::to_string(&owned).unwrap();
        let decoded: RestoreNamespace = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, owned);
        assert!(!format!("{owned:?}").contains("aaaa"));
    }
}

//! Deployment-selected application persistence; credentials never enter HTTP or diagnostics.
use lytronaut_storage::{Catalog, DatabaseConfig, DatabaseError};
use std::path::Path;

pub async fn open(data: &Path) -> Result<Catalog, DatabaseError> {
    let url = std::env::var_os("LYTRONAUT_DATABASE_URL")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| DatabaseError::Configuration)
        })
        .transpose()?;
    let config = match url.as_deref() {
        Some(url) => DatabaseConfig::from_deployment(Path::new("unused"), Some(url))?,
        None => {
            let path = crate::local::application_database(data)
                .map_err(|_| DatabaseError::Configuration)?;
            crate::local::database_file(&path).map_err(|_| DatabaseError::Configuration)?;
            DatabaseConfig::Sqlite(path)
        }
    };
    let schema = std::env::var_os("LYTRONAUT_DATABASE_SCHEMA")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| DatabaseError::Configuration)
        })
        .transpose()?;
    let config = match schema {
        Some(schema) => config.with_schema(&schema)?,
        None => config,
    };
    Catalog::open(config).await
}

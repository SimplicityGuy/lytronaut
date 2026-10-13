//! Deployment-selected application persistence; credentials never enter HTTP or diagnostics.
use lytronaut_storage::{Catalog, DatabaseConfig, DatabaseError};
use std::{ffi::OsString, io::Read, path::Path};

/// Largest accepted password secret file, including an optional final newline.
const PASSWORD_FILE_BYTES: u64 = 1026;

pub async fn open(data: &Path) -> Result<Catalog, DatabaseError> {
    let config = configured(
        data,
        std::env::var_os("LYTRONAUT_DATABASE_URL"),
        std::env::var_os("LYTRONAUT_DATABASE_PASSWORD_FILE"),
        std::env::var_os("LYTRONAUT_DATABASE_SCHEMA"),
    )?;
    Catalog::open(config).await
}

/// Resolve deployment configuration. The PostgreSQL password may come from a
/// mounted secret file so neither the image, Compose file nor container
/// environment holds it; the URL itself then carries no password.
fn configured(
    data: &Path,
    url: Option<OsString>,
    password_file: Option<OsString>,
    schema: Option<OsString>,
) -> Result<DatabaseConfig, DatabaseError> {
    let text = |value: OsString| {
        value
            .into_string()
            .map_err(|_| DatabaseError::Configuration)
    };
    let url = url.map(text).transpose()?;
    let config = match url.as_deref() {
        Some(url) => DatabaseConfig::from_deployment(Path::new("unused"), Some(url))?,
        None => {
            // A password without a PostgreSQL URL would silently select SQLite.
            if password_file.is_some() {
                return Err(DatabaseError::Configuration);
            }
            let path = crate::local::application_database(data)
                .map_err(|_| DatabaseError::Configuration)?;
            crate::local::database_file(&path).map_err(|_| DatabaseError::Configuration)?;
            DatabaseConfig::Sqlite(path)
        }
    };
    let config = match password_file {
        Some(path) => config.with_password(&read_password(Path::new(&path))?)?,
        None => config,
    };
    match schema.map(text).transpose()? {
        Some(schema) => config.with_schema(&schema),
        None => Ok(config),
    }
}

/// Read one bounded UTF-8 secret, removing a single trailing line ending. The
/// path and contents never appear in an error.
fn read_password(path: &Path) -> Result<String, DatabaseError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(PASSWORD_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| DatabaseError::Configuration)?;
    if bytes.len() as u64 > PASSWORD_FILE_BYTES {
        return Err(DatabaseError::Configuration);
    }
    let mut password = String::from_utf8(bytes).map_err(|_| DatabaseError::Configuration)?;
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "lytronaut-password-file-{:x}",
                u128::from_le_bytes(random)
            ));
            crate::local::private_directory(&path).unwrap();
            Self(path)
        }
        fn secret(&self, name: &str, bytes: &[u8]) -> OsString {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path.into_os_string()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const URL: &str = "postgres://lytronaut@postgres:5432/lytronaut?sslmode=disable";

    fn url_of(config: DatabaseConfig) -> String {
        match config {
            DatabaseConfig::Postgres(url) | DatabaseConfig::PostgresSchema { url, .. } => url,
            DatabaseConfig::Sqlite(_) => panic!("unexpected SQLite"),
        }
    }

    #[test]
    fn password_file_supplies_exactly_one_line_without_echoing_it() {
        let scratch = Scratch::new();
        for (name, contents) in [
            ("plain", &b"generated-secret"[..]),
            ("newline", b"generated-secret\n"),
            ("crlf", b"generated-secret\r\n"),
        ] {
            let config = configured(
                &scratch.0,
                Some(URL.into()),
                Some(scratch.secret(name, contents)),
                Some("lytronaut".into()),
            )
            .unwrap();
            assert!(!format!("{config:?}").contains("generated-secret"));
            assert!(matches!(
                &config,
                DatabaseConfig::PostgresSchema { schema, .. } if schema == "lytronaut"
            ));
            assert!(url_of(config).contains(":generated%2Dsecret@postgres"));
        }
        // Without a password file the URL is used exactly as configured.
        let config = configured(&scratch.0, Some(URL.into()), None, None).unwrap();
        assert_eq!(url_of(config), URL);
    }

    #[test]
    fn invalid_password_sources_fail_closed_and_redacted() {
        let scratch = Scratch::new();
        let missing = scratch.0.join("missing").into_os_string();
        let cases = [
            (Some(URL), Some(scratch.secret("empty", b""))),
            (Some(URL), Some(scratch.secret("blank-line", b"\n"))),
            (
                Some(URL),
                Some(scratch.secret("two-lines", b"generated\nsecret\n")),
            ),
            (Some(URL), Some(scratch.secret("binary", b"generated\xff"))),
            (Some(URL), Some(scratch.secret("large", &[b'x'; 1027]))),
            (Some(URL), Some(missing)),
            (Some(URL), Some(scratch.0.clone().into_os_string())),
            (
                Some("postgres://lytronaut:inline-secret@postgres/lytronaut"),
                Some(scratch.secret("ambiguous", b"generated")),
            ),
            // A password must never silently fall back to SQLite.
            (None, Some(scratch.secret("orphan", b"generated"))),
        ];
        for (url, password) in cases {
            let error = configured(&scratch.0, url.map(Into::into), password, None).unwrap_err();
            assert_eq!(error, DatabaseError::Configuration);
            let message = error.to_string();
            assert!(!message.contains("generated") && !message.contains("inline-secret"));
            assert!(!message.contains(&*scratch.0.to_string_lossy()));
        }
        // The maximum accepted secret is 1024 characters plus a CRLF ending.
        let mut largest = vec![b'x'; 1024];
        largest.extend_from_slice(b"\r\n");
        assert!(
            configured(
                &scratch.0,
                Some(URL.into()),
                Some(scratch.secret("largest", &largest)),
                None
            )
            .is_ok()
        );
        let sqlite = configured(&scratch.0, None, None, None).unwrap();
        assert!(matches!(sqlite, DatabaseConfig::Sqlite(_)));
        assert!(configured(&scratch.0, None, None, Some("lytronaut".into())).is_err());
    }
}

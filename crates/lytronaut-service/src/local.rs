//! Local bootstrap file management. Paths and secrets never enter diagnostic errors.
use crate::auth::{AuthError, secret, token_valid};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

/// Require a private real directory; fail rather than silently chmod an existing directory.
pub fn private_directory(path: &Path) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        match fs::create_dir(path) {
            Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| AuthError)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(AuthError),
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| AuthError)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(AuthError);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(AuthError)
    }
}
/// Create once with O_EXCL and mode 0600. Never replace a pending bootstrap on restart.
/// An interrupted/empty write fails closed on the next start; local operator may remove it
/// only while the application is stopped and before credentials exist.
pub fn bootstrap_file(path: &Path) -> Result<String, AuthError> {
    #[cfg(unix)]
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        match options.open(path) {
            Ok(mut file) => {
                let token = secret()?;
                file.write_all(token.as_bytes()).map_err(|_| AuthError)?;
                file.sync_all().map_err(|_| AuthError)?;
                Ok(token)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(path).map_err(|_| AuthError)?;
                if !metadata.is_file()
                    || metadata.permissions().mode() & 0o077 != 0
                    || metadata.len() != 64
                {
                    return Err(AuthError);
                }
                let mut token = String::new();
                fs::File::open(path)
                    .map_err(|_| AuthError)?
                    .take(65)
                    .read_to_string(&mut token)
                    .map_err(|_| AuthError)?;
                if !token_valid(&token) {
                    return Err(AuthError);
                }
                Ok(token)
            }
            Err(_) => Err(AuthError),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(AuthError)
    }
}
/// Secure database file before SQLite opens it. SQLite sidecars inherit private-directory protection.
pub fn database_file(path: &Path) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        match options.open(path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(AuthError),
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| AuthError)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(AuthError);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(AuthError)
    }
}

/// Resolve only an administrator-selected filename inside the private data directory.
pub fn application_database(data: &Path) -> Result<std::path::PathBuf, AuthError> {
    let name = std::env::var("LYTRONAUT_DATABASE").unwrap_or_else(|_| "catalog.db".into());
    database_path(data, &name)
}
pub fn database_path(data: &Path, name: &str) -> Result<std::path::PathBuf, AuthError> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(AuthError);
    }
    Ok(data.join(name))
}
#[cfg(test)]
mod browser_config_tests {
    use super::*;
    #[test]
    fn database_filename_stays_inside_private_root() {
        assert_eq!(
            database_path(Path::new("data"), "app.db").unwrap(),
            Path::new("data/app.db")
        );
        for name in ["", "../a", "/a", "a/b", "a\\b", ".", ".."] {
            assert!(database_path(Path::new("data"), name).is_err());
        }
    }
}

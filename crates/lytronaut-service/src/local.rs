//! Private application file management. Paths never enter diagnostic errors.
use crate::ServiceError;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, OpenOptions},
    path::Path,
};

/// Require a private real directory; fail rather than silently chmod an existing directory.
pub fn private_directory(path: &Path) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        match fs::create_dir(path) {
            Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| ServiceError)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ServiceError),
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| ServiceError)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(ServiceError);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(ServiceError)
    }
}
/// Secure database file before SQLite opens it. SQLite sidecars inherit private-directory protection.
pub fn database_file(path: &Path) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        match options.open(path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ServiceError),
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| ServiceError)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(ServiceError);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(ServiceError)
    }
}

/// Resolve only an administrator-selected filename inside the private data directory.
pub fn application_database(data: &Path) -> Result<std::path::PathBuf, ServiceError> {
    let name = std::env::var("LYTRONAUT_DATABASE").unwrap_or_else(|_| "catalog.db".into());
    database_path(data, &name)
}
pub fn database_path(data: &Path, name: &str) -> Result<std::path::PathBuf, ServiceError> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(ServiceError);
    }
    Ok(data.join(name))
}
/// Unlink the obsolete, fixed application-owned credential artifact. Never follow links.
/// Only invoked for the private application state directory, not imported source libraries.
pub fn remove_obsolete_bootstrap(data: &Path) -> Result<(), ServiceError> {
    let path = data.join("bootstrap.secret");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            fs::remove_file(path).map_err(|_| ServiceError)
        }
        Ok(_) => Err(ServiceError),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ServiceError),
    }
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

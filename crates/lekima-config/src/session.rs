//! Cookie persistence shared by the daemon and CLI login flows.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use lekima_domain::{CredentialsStore, DomainError, Result};

use crate::private::open_private;

/// A single-line cookie header stored with `0600` permissions.
#[derive(Debug, Clone)]
pub struct FileCredentials {
    path: PathBuf,
}

impl FileCredentials {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl CredentialsStore for FileCredentials {
    fn load(&self) -> Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(raw) if raw.trim().is_empty() => Ok(None),
            Ok(raw) => Ok(Some(raw.trim().to_owned())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(DomainError::Unavailable(format!(
                "failed to read session file {}: {error}",
                self.path.display()
            ))),
        }
    }

    fn save(&self, cookie: &str) -> Result<()> {
        let mut file = open_private(&self.path).map_err(|error| {
            DomainError::Unavailable(format!("failed to open {}: {error}", self.path.display()))
        })?;
        file.write_all(cookie.trim().as_bytes()).map_err(|error| {
            DomainError::Unavailable(format!("failed to write {}: {error}", self.path.display()))
        })
    }

    fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(DomainError::Unavailable(format!(
                "failed to remove {}: {error}",
                self.path.display()
            ))),
        }
    }
}

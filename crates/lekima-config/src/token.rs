//! Bearer token used by the HTTP control server.

use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use crate::{paths::PathError, private::open_private};

/// A single-line bearer token stored with `0600` permissions.
#[derive(Debug, Clone)]
pub struct HttpToken {
    path: PathBuf,
}

impl HttpToken {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the stored token; an empty or missing file means "not generated".
    ///
    /// # Errors
    ///
    /// Propagates read failures other than a missing file.
    pub fn load(&self) -> Result<Option<String>, PathError> {
        match fs::read_to_string(&self.path) {
            Ok(raw) if raw.trim().is_empty() => Ok(None),
            Ok(raw) => Ok(Some(raw.trim().to_owned())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(source) => Err(PathError::Read {
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// Writes the token, creating it with `0600` permissions when new.
    ///
    /// # Errors
    ///
    /// Propagates filesystem failures.
    pub fn save(&self, token: &str) -> Result<(), PathError> {
        let mut file = open_private(&self.path).map_err(|source| PathError::Write {
            path: self.path.clone(),
            source,
        })?;
        file.write_all(token.trim().as_bytes())
            .map_err(|source| PathError::Write {
                path: self.path.clone(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::HttpToken;

    #[test]
    fn round_trips_a_token_and_creates_private_directories() {
        let dir = tempfile::tempdir().unwrap();
        let token = HttpToken::new(dir.path().join("nested/http.token"));

        assert_eq!(token.load().unwrap(), None);
        token.save("abc123").unwrap();
        assert_eq!(token.load().unwrap().as_deref(), Some("abc123"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn empty_files_are_not_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let token = HttpToken::new(dir.path().join("http.token"));
        std::fs::write(token.path(), "  \n").unwrap();
        assert_eq!(token.load().unwrap(), None);
    }
}

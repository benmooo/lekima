//! Filesystem locations owned by lekima.

use std::{
    env,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;

use crate::Config;

const ENV_CONFIG_DIR: &str = "LEKIMA_CONFIG_DIR";
const ENV_SOCKET: &str = "LEKIMA_SOCKET";

#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// Resolves the state root, honouring `LEKIMA_CONFIG_DIR` first.
    ///
    /// # Errors
    ///
    /// Fails when no home directory can be determined.
    pub fn resolve() -> Result<Self, PathError> {
        if let Some(root) = env::var_os(ENV_CONFIG_DIR) {
            return Ok(Self {
                root: PathBuf::from(root),
            });
        }
        let dirs = ProjectDirs::from("dev", "lekima", "lekima").ok_or(PathError::NoHome)?;
        Ok(Self {
            root: dirs.config_dir().to_path_buf(),
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    #[must_use]
    pub fn session_file(&self) -> PathBuf {
        self.root.join("session.cookies")
    }

    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    #[must_use]
    pub fn log_file(&self) -> PathBuf {
        self.root.join("lekima.log")
    }

    /// Unix socket the daemon listens on.
    ///
    /// `LEKIMA_SOCKET` wins; otherwise the XDG runtime directory is preferred
    /// so the socket disappears on logout.
    #[must_use]
    pub fn socket(&self) -> PathBuf {
        if let Some(path) = env::var_os(ENV_SOCKET) {
            return PathBuf::from(path);
        }
        env::var_os("XDG_RUNTIME_DIR").map_or_else(
            || self.cache_dir().join("lekima.sock"),
            |dir| PathBuf::from(dir).join("lekima.sock"),
        )
    }

    /// Creates the directories lekima writes to.
    ///
    /// # Errors
    ///
    /// Propagates filesystem failures.
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.cache_dir())?;
        Ok(())
    }

    /// Loads the configuration for these paths.
    ///
    /// # Errors
    ///
    /// Propagates parse and I/O failures of an existing config file.
    pub fn config(&self) -> Result<Config, PathError> {
        crate::load_config(&self.config_file())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("could not determine the home directory")]
    NoHome,
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("invalid configuration structure: {0}")]
    Structure(String),
}

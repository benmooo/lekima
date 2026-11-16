//! Private state files: created `0600` on Unix with missing parents.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

/// Opens `path` for a full rewrite, creating private parent directories.
///
/// Session cookies and the HTTP bearer token must not inherit the process
/// umask. On Unix the file is created `0600`; existing modes are preserved.
pub(crate) fn open_private(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

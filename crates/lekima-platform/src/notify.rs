//! Desktop notifications.
//!
//! Notifications are best-effort and must never fail the daemon: a missing
//! helper, a denied permission or a headless host all degrade to
//! [`NoopNotifier`] instead of an error. Delivery is synchronous and may block
//! for up to [`HELPER_TIMEOUT`], so callers on an async runtime must run it off
//! the executor (the daemon uses a dedicated thread).
//!
//! Backends shell out to the platform helper so this crate needs no unsafe code
//! and no extra dependency:
//!
//! | OS | Helper |
//! | --- | --- |
//! | macOS | `osascript` (`display notification`) |
//! | Linux | `notify-send` (libnotify) |
//! | other | none |
//!
//! Callers sanitize provider text with `lekima_domain::sanitize_line` before
//! building a [`Notification`]; this module only decides how to deliver it.

use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

/// How long a helper may run before it is killed.
///
/// A first notification can wait on a permission prompt, and a wedged
/// notification daemon can block forever; neither may accumulate.
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(2);

const POLL: Duration = Duration::from_millis(50);

/// A user-visible notification payload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// A best-effort desktop notification sink.
pub trait Notifier: Send + Sync {
    /// Shows `notification`.
    ///
    /// Blocks until the helper exits or [`HELPER_TIMEOUT`] passes. Returns
    /// `false` when the helper is missing, timed out or exited unsuccessfully;
    /// the caller only logs that, it never retries.
    fn notify(&self, notification: &Notification) -> bool;

    /// Short backend name for `doctor` and logs.
    fn backend(&self) -> &'static str;
}

/// Discards notifications where the OS offers no helper.
#[derive(Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify(&self, _notification: &Notification) -> bool {
        false
    }

    fn backend(&self) -> &'static str {
        "none"
    }
}

/// AppleScript that takes the title and body as arguments, so no value ever
/// needs to be escaped into the script text.
#[cfg(target_os = "macos")]
const MACOS_SCRIPT: [&str; 3] = [
    "on run argv",
    "display notification (item 2 of argv) with title (item 1 of argv)",
    "end run",
];

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacNotifier {
    /// Absolute path resolved at startup: a daemon started without a login shell
    /// often has a `PATH` that cannot find the helper.
    helper: PathBuf,
}

#[cfg(target_os = "macos")]
impl Notifier for MacNotifier {
    fn notify(&self, notification: &Notification) -> bool {
        let mut command = Command::new(&self.helper);
        for line in MACOS_SCRIPT {
            command.args(["-e", line]);
        }
        command
            .arg("--")
            .arg(&notification.title)
            .arg(&notification.body);
        run(command)
    }

    fn backend(&self) -> &'static str {
        "osascript"
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct NotifySend {
    helper: PathBuf,
}

#[cfg(target_os = "linux")]
impl Notifier for NotifySend {
    fn notify(&self, notification: &Notification) -> bool {
        let mut command = Command::new(&self.helper);
        command
            .args(["--app-name=lekima", "--icon=audio-x-generic", "--"])
            .arg(&notification.title)
            .arg(&notification.body);
        run(command)
    }

    fn backend(&self) -> &'static str {
        "notify-send"
    }
}

/// Creates the notification backend for this OS.
///
/// Detection happens once, at startup, so a missing helper costs nothing per
/// track change.
#[must_use]
pub fn notifier() -> Arc<dyn Notifier> {
    detect().unwrap_or_else(|| Arc::new(NoopNotifier))
}

/// Name of the notification backend `notifier` would pick on this host.
#[must_use]
pub fn backend_name() -> &'static str {
    detect().map_or("none", |notifier| notifier.backend())
}

/// Single source of truth for backend selection, shared by [`notifier`] and
/// [`backend_name`] so `doctor` cannot report a backend that is not used.
fn detect() -> Option<Arc<dyn Notifier>> {
    #[cfg(target_os = "macos")]
    {
        helper("osascript").map(|helper| Arc::new(MacNotifier { helper }) as Arc<dyn Notifier>)
    }

    #[cfg(target_os = "linux")]
    {
        helper("notify-send").map(|helper| Arc::new(NotifySend { helper }) as Arc<dyn Notifier>)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Runs a helper under [`HELPER_TIMEOUT`] without inheriting or leaking output.
fn run(mut command: Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        tracing::debug!("notification helper could not be started");
        return false;
    };
    let deadline = Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() >= deadline => {
                tracing::debug!("notification helper timed out");
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(error) => {
                tracing::debug!(%error, "notification helper could not be awaited");
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Looks a helper up on `PATH`, then in the usual install locations.
///
/// Daemons are frequently started from a context without a login shell, so
/// `PATH` alone would report a working desktop as `none`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn helper(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    find_in_path(name, &path).or_else(|| {
        ["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"]
            .into_iter()
            .map(|directory| PathBuf::from(directory).join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Searches one `PATH` value for an executable file named `name`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn find_in_path(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    env::split_paths(path)
        .filter(|directory| !directory.as_os_str().is_empty())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, fs};

    use super::{NoopNotifier, Notification, Notifier, find_in_path};

    #[test]
    fn noop_never_reports_success() {
        let notifier = NoopNotifier;
        assert!(!notifier.notify(&Notification::default()));
        assert_eq!(notifier.backend(), "none");
    }

    #[test]
    fn notification_defaults_are_empty() {
        let notification = Notification::default();
        assert!(notification.title.is_empty());
        assert!(notification.body.is_empty());
    }

    #[test]
    fn find_in_path_skips_empty_entries() {
        let found = find_in_path("lekima-missing-helper", &OsString::from(":"));
        assert!(found.is_none());
    }

    #[test]
    fn find_in_path_returns_the_first_match() {
        let root = std::env::temp_dir().join(format!("lekima-notify-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("notify-helper"), b"#!/bin/sh\n").unwrap();
        fs::write(second.join("notify-helper"), b"#!/bin/sh\n").unwrap();

        let path = OsString::from(format!("{}:{}", first.display(), second.display()));
        let found = find_in_path("notify-helper", &path);

        assert_eq!(found, Some(first.join("notify-helper")));
        fs::remove_dir_all(&root).unwrap();
    }
}

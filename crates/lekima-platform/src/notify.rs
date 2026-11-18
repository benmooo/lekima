//! Desktop notifications.
//!
//! Notifications are best-effort and must never block or fail the daemon: a
//! missing helper, a denied permission or a headless host all degrade to
//! [`NoopNotifier`] instead of an error.
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
};

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
    /// Returns `false` when the helper is missing or exits unsuccessfully; the
    /// caller only logs that, it never retries.
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
struct MacNotifier;

#[cfg(target_os = "macos")]
impl Notifier for MacNotifier {
    fn notify(&self, notification: &Notification) -> bool {
        let mut command = Command::new("osascript");
        for line in MACOS_SCRIPT {
            command.args(["-e", line]);
        }
        command.arg(&notification.title).arg(&notification.body);
        run(command)
    }

    fn backend(&self) -> &'static str {
        "osascript"
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct NotifySend;

#[cfg(target_os = "linux")]
impl Notifier for NotifySend {
    fn notify(&self, notification: &Notification) -> bool {
        let mut command = Command::new("notify-send");
        command
            .args(["--app-name=lekima", "--icon=audio-x-generic"])
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
/// Detection happens once, at daemon start, so a missing helper costs nothing
/// per track change.
#[must_use]
pub fn notifier() -> Arc<dyn Notifier> {
    #[cfg(target_os = "macos")]
    {
        if helper("osascript").is_some() {
            return Arc::new(MacNotifier);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if helper("notify-send").is_some() {
            return Arc::new(NotifySend);
        }
    }

    Arc::new(NoopNotifier)
}

/// Name of the notification backend `notifier` would pick on this host.
#[must_use]
pub fn backend_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if helper("osascript").is_some() {
            return "osascript";
        }
    }

    #[cfg(target_os = "linux")]
    {
        if helper("notify-send").is_some() {
            return "notify-send";
        }
    }

    "none"
}

/// Runs a helper without inheriting or leaking its output.
fn run(mut command: Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.status() {
        Ok(status) => status.success(),
        Err(error) => {
            tracing::debug!(%error, "notification helper failed");
            false
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

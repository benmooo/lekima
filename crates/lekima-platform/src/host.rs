//! Daemon side of the macOS Now Playing helper.
//!
//! MediaPlayer only exposes a process to the system when an `NSApplication`
//! with a run loop owns it on the main thread. A daemon's main thread belongs to
//! the async runtime, so the OS-facing half runs in a small helper process
//! (`lekima media-host`) and the two exchange newline-delimited JSON: state
//! snapshots on stdin, media commands on stdout.

use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
};

use crate::{MediaCommand, MediaControls, NowPlaying};

const HOST_ARGUMENT: &str = "media-host";

/// Starts the helper and returns the daemon-side handle.
pub fn spawn() -> Arc<dyn MediaControls> {
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            tracing::warn!(%error, "cannot locate the lekima binary for Now Playing");
            return Arc::new(crate::NoopControls);
        }
    };
    let child = Command::new(executable)
        .arg(HOST_ARGUMENT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, "failed to start the Now Playing helper");
            return Arc::new(crate::NoopControls);
        }
    };
    let Some(stdin) = child.stdin.take() else {
        tracing::warn!("the Now Playing helper has no stdin");
        let _ = child.kill();
        return Arc::new(crate::NoopControls);
    };
    let Some(stdout) = child.stdout.take() else {
        tracing::warn!("the Now Playing helper has no stdout");
        let _ = child.kill();
        return Arc::new(crate::NoopControls);
    };

    let commands = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::clone(&commands);
    std::thread::Builder::new()
        .name("lekima-nowplaying".to_owned())
        .spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(command) = serde_json::from_str::<MediaCommand>(&line)
                    && let Ok(mut queue) = queue.lock()
                {
                    queue.push(command);
                }
            }
        })
        .ok();

    Arc::new(HostControls {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        commands,
        last: Mutex::new(NowPlaying::default()),
    })
}

struct HostControls {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    commands: Arc<Mutex<Vec<MediaCommand>>>,
    last: Mutex<NowPlaying>,
}

impl MediaControls for HostControls {
    fn update(&self, state: &NowPlaying) {
        {
            let Ok(mut last) = self.last.lock() else {
                return;
            };
            if *last == *state {
                return;
            }
            *last = state.clone();
        }
        let Ok(line) = serde_json::to_string(state) else {
            return;
        };
        if let Ok(mut stdin) = self.stdin.lock()
            && writeln!(stdin, "{line}").is_err()
        {
            tracing::debug!("the Now Playing helper stopped reading");
        }
    }

    fn poll(&self) -> Vec<MediaCommand> {
        self.commands
            .lock()
            .map_or_else(|_| Vec::new(), |mut queue| std::mem::take(&mut *queue))
    }

    fn backend(&self) -> &'static str {
        "now-playing"
    }
}

impl Drop for HostControls {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HOST_ARGUMENT;

    #[test]
    fn the_helper_argument_matches_the_cli() {
        assert_eq!(HOST_ARGUMENT, "media-host");
    }
}

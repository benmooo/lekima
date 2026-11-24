//! External hook processes.
//!
//! A hook is a client with a lifecycle. The daemon starts it, writes a hello
//! frame followed by sequenced events to its stdin, and reads commands from its
//! stdout. Nothing runs inside the daemon process: a hook that hangs, crashes or
//! talks nonsense can only lose its own connection, never playback.
//!
//! Contract (version [`PROTOCOL`]):
//!
//! ```text
//! daemon → hook (stdin), one JSON object per line
//!   {"type":"hello","protocol":1,"version":"0.10.1","events":["status"],"capabilities":["pause"]}
//!   {"seq":42,"type":"status","state":{…}}
//!
//! hook → daemon (stdout), one JSON object per line, only declared capabilities
//!   {"type":"pause"}
//! ```
//!
//! Hooks are **action-only**: a command stream carries no replies, so a hook
//! reads state from the event stream or from the public state files. A `resync`
//! frame means it lagged behind and may have missed events. Hook stderr is
//! forwarded to the log through the shared sanitizer, and hook output lines are
//! capped like socket frames.

use std::{
    collections::{BTreeMap, HashSet},
    process::Stdio,
    sync::Arc,
};

use lekima_config::HookConfig;
use lekima_domain::sanitize_line;
use lekima_ipc::{
    Command, Event, EventFrame, EventKind, HOOK_CAPABILITIES, MAX_FRAME_BYTES, is_hook_capability,
};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStderr, ChildStdin, ChildStdout, Command as Process},
    sync::{broadcast, mpsc},
};

/// Version of the hook contract carried in the hello frame.
pub const PROTOCOL: u16 = 1;

/// How many commands may wait for the daemon before a hook's sends are dropped.
pub const COMMAND_QUEUE: usize = 64;

/// Longest accepted output line, matching the socket frame limit.
const MAX_LINE: usize = MAX_FRAME_BYTES;

/// A validated hook configuration.
#[derive(Debug, Clone)]
pub struct HookSpec {
    pub name: String,
    pub command: Vec<String>,
    /// `None` means every event kind.
    pub events: Option<HashSet<EventKind>>,
    pub capabilities: HashSet<String>,
}

impl HookSpec {
    /// Validates one `[hooks.<name>]` table.
    ///
    /// An invalid hook is skipped with a warning at daemon start rather than
    /// stopping the player; `lekima doctor` shows the same error.
    ///
    /// # Errors
    ///
    /// Returns a message naming the problem: empty command, unknown event kind,
    /// or a capability a hook may not request.
    pub fn from_config(name: &str, config: &HookConfig) -> Result<Self, String> {
        let command = config.command.clone();
        if command.is_empty() || command[0].trim().is_empty() {
            return Err("`command` must name an executable".to_owned());
        }

        let events = if config.events.is_empty() {
            None
        } else {
            let mut kinds = HashSet::with_capacity(config.events.len());
            for event in &config.events {
                let kind = EventKind::from_name(event.trim()).ok_or_else(|| {
                    format!("unknown event `{event}` (expected one of status, queue, like, resync)")
                })?;
                kinds.insert(kind);
            }
            Some(kinds)
        };

        let mut capabilities = HashSet::with_capacity(config.capabilities.len());
        for capability in &config.capabilities {
            let capability = capability.trim();
            if !is_hook_capability(capability) {
                return Err(format!(
                    "`{capability}` is not a hook capability; allowed: {}",
                    HOOK_CAPABILITIES.join(", ")
                ));
            }
            capabilities.insert(capability.to_owned());
        }

        Ok(Self {
            name: name.to_owned(),
            command,
            events,
            capabilities,
        })
    }

    /// Whether this hook subscribes to `event`.
    #[must_use]
    pub fn wants(&self, event: &Event) -> bool {
        self.events
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&event.kind()))
    }

    /// Whether the hook declared the capability `command` needs.
    #[must_use]
    pub fn allows(&self, command: &Command) -> bool {
        self.capabilities.contains(command.tag())
    }

    /// Declared event names for the hello frame and `doctor`.
    #[must_use]
    pub fn event_names(&self) -> Vec<&'static str> {
        match &self.events {
            None => EventKind::ALL.iter().map(|kind| kind.as_str()).collect(),
            Some(kinds) => EventKind::ALL
                .iter()
                .filter(|kind| kinds.contains(kind))
                .map(|kind| kind.as_str())
                .collect(),
        }
    }

    /// Declared capability names, sorted for stable output.
    #[must_use]
    pub fn capability_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.capabilities.iter().cloned().collect();
        names.sort();
        names
    }
}

/// Whether the configured program can be found.
///
/// `doctor` reports this separately from configuration validity: a valid table
/// pointing at a missing binary is a different problem from a typo in a
/// capability name.
#[must_use]
pub fn is_executable(spec: &HookSpec) -> bool {
    let program = &spec.command[0];
    if program.contains('/') {
        return std::path::Path::new(program).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

/// A running hook process.
///
/// Dropping it aborts the waiter, which drops the child; `kill_on_drop` then
/// terminates the process. Only the direct child is killed: a hook that leaves
/// children of its own is expected to `exec` its final process.
#[derive(Debug)]
pub struct RunningHook {
    name: String,
    job: tokio::task::JoinHandle<()>,
}

impl Drop for RunningHook {
    fn drop(&mut self) {
        self.job.abort();
        tracing::debug!(hook = %self.name, "hook stopped");
    }
}

/// Starts every valid hook and returns the handles the daemon keeps alive.
///
/// Invalid configurations are logged and skipped, so one typo cannot keep the
/// player from starting.
pub fn start(
    configs: &BTreeMap<String, HookConfig>,
    events: broadcast::Sender<EventFrame>,
    commands: mpsc::Sender<(String, Command)>,
) -> Vec<RunningHook> {
    let mut running = Vec::with_capacity(configs.len());
    for (name, config) in configs {
        let spec = match HookSpec::from_config(name, config) {
            Ok(spec) => spec,
            Err(error) => {
                tracing::warn!(hook = %name, %error, "ignoring an invalid hook");
                continue;
            }
        };
        match spawn(spec, events.clone(), commands.clone()) {
            Ok(hook) => {
                // Only the program, never the full argv: arguments can carry
                // secrets and this line goes to a log file.
                tracing::info!(hook = %name, program = %config.command[0], "hook started");
                running.push(hook);
            }
            Err(error) => tracing::warn!(hook = %name, %error, "failed to start hook"),
        }
    }
    running
}

fn spawn(
    spec: HookSpec,
    events: broadcast::Sender<EventFrame>,
    commands: mpsc::Sender<(String, Command)>,
) -> std::io::Result<RunningHook> {
    let mut child = Process::new(&spec.command[0])
        .args(&spec.command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("hook stdin was not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("hook stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("hook stderr was not piped"))?;

    let spec = Arc::new(spec);
    tokio::spawn(write_events(Arc::clone(&spec), stdin, events.subscribe()));
    tokio::spawn(read_commands(spec.clone(), stdout, commands));
    tokio::spawn(pump_stderr(spec.clone(), stderr));

    let hook_name = spec.name.clone();
    let job = tokio::spawn(async move {
        let mut child = child;
        match child.wait().await {
            Ok(status) => tracing::info!(hook = %hook_name, %status, "hook process exited"),
            Err(error) => tracing::debug!(hook = %hook_name, %error, "hook wait failed"),
        }
    });

    Ok(RunningHook {
        name: spec.name.clone(),
        job,
    })
}

async fn write_events(
    spec: Arc<HookSpec>,
    mut stdin: ChildStdin,
    mut events: broadcast::Receiver<EventFrame>,
) {
    let hello = json!({
        "type": "hello",
        "protocol": PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        "events": spec.event_names(),
        "capabilities": spec.capability_names(),
    });
    if write_line(&mut stdin, &hello.to_string()).await.is_err() {
        return;
    }

    let mut last_seq = 0;
    let mut lag_warned = false;
    loop {
        match events.recv().await {
            Ok(frame) => {
                lag_warned = false;
                if !spec.wants(&frame.event) {
                    continue;
                }
                let Ok(line) = serde_json::to_string(&frame) else {
                    continue;
                };
                if write_line(&mut stdin, &line).await.is_err() {
                    break;
                }
                last_seq = frame.seq;
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                // One warning per lag episode, one resync unless the hook's
                // filter excludes it; otherwise a stuck hook floods the log.
                if lag_warned {
                    tracing::debug!(hook = %spec.name, skipped, "hook still lagging");
                } else {
                    tracing::warn!(hook = %spec.name, skipped, "hook lagged behind the event ring");
                    lag_warned = true;
                }
                let resync = Event::Resync;
                if !spec.wants(&resync) {
                    continue;
                }
                let frame = EventFrame {
                    seq: last_seq,
                    event: resync,
                };
                if write_line(&mut stdin, &frame_json(&frame)).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    tracing::debug!(hook = %spec.name, "hook input closed");
}

async fn read_commands(
    spec: Arc<HookSpec>,
    stdout: ChildStdout,
    commands: mpsc::Sender<(String, Command)>,
) {
    let mut reader = BufReader::new(stdout);
    let mut chunk = [0_u8; 8192];
    let mut lines = LineBuffer::default();
    let mut complete = Vec::new();
    let mut utf8_warned = false;
    let mut rejected = 0;

    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                complete.clear();
                lines.feed(&chunk[..read], &mut complete);
                if lines.rejected > rejected {
                    rejected = lines.rejected;
                    tracing::warn!(
                        hook = %spec.name,
                        limit = MAX_LINE,
                        "hook sent a line over the frame limit; it was dropped"
                    );
                }
                for raw in complete.drain(..) {
                    let Ok(text) = std::str::from_utf8(&raw) else {
                        if !utf8_warned {
                            tracing::warn!(hook = %spec.name, "hook output is not valid utf-8");
                            utf8_warned = true;
                        }
                        continue;
                    };
                    handle_line(&spec, text.trim(), &commands);
                }
            }
            Err(error) => {
                tracing::warn!(hook = %spec.name, %error, "hook output failed");
                break;
            }
        }
    }

    tracing::info!(hook = %spec.name, "hook output closed");
}

fn handle_line(spec: &HookSpec, line: &str, commands: &mpsc::Sender<(String, Command)>) {
    if line.is_empty() {
        return;
    }
    let command: Command = match serde_json::from_str(line) {
        Ok(command) => command,
        Err(error) => {
            tracing::debug!(hook = %spec.name, %error, "hook sent invalid json");
            return;
        }
    };
    if !spec.allows(&command) {
        tracing::warn!(
            hook = %spec.name,
            command = command.tag(),
            "hook command denied: capability not declared"
        );
        return;
    }
    match commands.try_send((spec.name.clone(), command)) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::debug!(hook = %spec.name, "hook command queue is full; dropping a command");
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

/// Forwards a hook's stderr into the daemon log, one sanitized line at a time.
async fn pump_stderr(spec: Arc<HookSpec>, stderr: ChildStderr) {
    let mut reader = BufReader::new(stderr);
    let mut chunk = [0_u8; 4096];
    let mut lines = LineBuffer::default();
    let mut complete = Vec::new();

    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                complete.clear();
                lines.feed(&chunk[..read], &mut complete);
                for raw in complete.drain(..) {
                    let text = String::from_utf8_lossy(&raw);
                    let line = sanitize_line(text.trim_end_matches('\r'));
                    if !line.is_empty() {
                        tracing::warn!(hook = %spec.name, "{line}");
                    }
                }
            }
            Err(_) => break,
        }
    }
}

/// Splits a byte stream into newline-delimited lines with a hard size cap.
///
/// A line that exceeds the cap is dropped whole, and the rest of it is skipped
/// up to the next newline, so one oversized write cannot exhaust memory or
/// desynchronize the stream.
#[derive(Debug, Default)]
struct LineBuffer {
    line: Vec<u8>,
    discarding: bool,
    rejected: u64,
}

impl LineBuffer {
    fn feed(&mut self, chunk: &[u8], complete: &mut Vec<Vec<u8>>) {
        for &byte in chunk {
            if self.discarding {
                if byte == b'\n' {
                    self.discarding = false;
                }
                continue;
            }
            if byte == b'\n' {
                if !self.line.is_empty() {
                    complete.push(std::mem::take(&mut self.line));
                }
                continue;
            }
            self.line.push(byte);
            if self.line.len() > MAX_LINE {
                self.rejected += 1;
                self.line.clear();
                self.discarding = true;
            }
        }
    }
}

fn frame_json(frame: &EventFrame) -> String {
    serde_json::to_string(frame).unwrap_or_default()
}

async fn write_line(stdin: &mut ChildStdin, line: &str) -> std::io::Result<()> {
    stdin.write_all(line.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await
}

#[cfg(test)]
mod tests {
    use lekima_config::HookConfig;
    use lekima_ipc::{Command, Event, MAX_FRAME_BYTES};

    use super::{HookSpec, LineBuffer};

    fn config(command: &[&str], events: &[&str], capabilities: &[&str]) -> HookConfig {
        HookConfig {
            command: command.iter().map(|value| (*value).to_owned()).collect(),
            events: events.iter().map(|value| (*value).to_owned()).collect(),
            capabilities: capabilities
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    #[test]
    fn a_command_is_required() {
        assert!(HookSpec::from_config("bad", &config(&[], &[], &[])).is_err());
        assert!(HookSpec::from_config("bad", &config(&["  "], &[], &[])).is_err());
        assert!(HookSpec::from_config("ok", &config(&["/bin/cat"], &[], &[])).is_ok());
    }

    #[test]
    fn unknown_events_and_capabilities_are_rejected() {
        assert!(
            HookSpec::from_config("bad", &config(&["/bin/cat"], &["track_change"], &[])).is_err()
        );
        assert!(HookSpec::from_config("bad", &config(&["/bin/cat"], &[], &["shutdown"])).is_err());
        assert!(HookSpec::from_config("bad", &config(&["/bin/cat"], &[], &["login"])).is_err());
        assert!(HookSpec::from_config("bad", &config(&["/bin/cat"], &[], &["status"])).is_err());
        assert!(
            HookSpec::from_config("ok", &config(&["/bin/cat"], &["status"], &["pause"])).is_ok()
        );
    }

    #[test]
    fn no_events_means_every_kind_and_no_capability_means_read_only() {
        let spec = HookSpec::from_config("all", &config(&["/bin/cat"], &[], &[])).unwrap();

        assert!(spec.wants(&Event::Resync), "every kind is subscribed");
        assert_eq!(
            spec.event_names(),
            Vec::from(["status", "queue", "like", "resync"])
        );
        assert!(!spec.allows(&Command::Pause));
        assert!(!spec.allows(&Command::Next));
        assert!(spec.capability_names().is_empty());
    }

    #[test]
    fn declared_events_and_capabilities_filter_both_directions() {
        let spec = HookSpec::from_config(
            "nowplaying",
            &config(&["/bin/cat"], &["status"], &["pause"]),
        )
        .unwrap();

        assert!(spec.wants(&Event::Status {
            state: Box::default()
        }));
        assert!(
            !spec.wants(&Event::Like {
                id: 1.into(),
                liked: true
            }),
            "an undeclared event is not delivered"
        );
        assert!(!spec.wants(&Event::Resync), "resync follows the filter too");
        assert_eq!(spec.event_names(), vec!["status"]);

        assert!(spec.allows(&Command::Pause));
        assert!(!spec.allows(&Command::Next));
        assert_eq!(spec.capability_names(), vec!["pause".to_owned()]);
    }

    #[test]
    fn lines_are_reassembled_across_chunks() {
        let mut buffer = LineBuffer::default();
        let mut complete = Vec::new();

        buffer.feed(b"{\"type\":\"pa", &mut complete);
        assert!(complete.is_empty());
        buffer.feed(b"use\"}\n{\"type\":\"next\"}\n", &mut complete);

        assert_eq!(complete.len(), 2);
        assert_eq!(complete[0], b"{\"type\":\"pause\"}");
        assert_eq!(complete[1], b"{\"type\":\"next\"}");
        assert_eq!(buffer.rejected, 0);
    }

    #[test]
    fn an_oversized_line_is_dropped_and_the_stream_recovers() {
        let mut buffer = LineBuffer::default();
        let mut complete = Vec::new();
        let oversized = vec![b'x'; MAX_FRAME_BYTES + 1];

        buffer.feed(&oversized, &mut complete);
        assert!(complete.is_empty());
        assert_eq!(buffer.rejected, 1);

        buffer.feed(b"\n{\"type\":\"pause\"}\n", &mut complete);
        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0], b"{\"type\":\"pause\"}");
    }

    #[test]
    fn empty_lines_are_ignored() {
        let mut buffer = LineBuffer::default();
        let mut complete = Vec::new();

        buffer.feed(b"\n\n \n", &mut complete);

        assert_eq!(complete.len(), 1, "a whitespace line is still a line");
        assert_eq!(complete[0], b" ");
    }
}

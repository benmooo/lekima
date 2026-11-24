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
//!   {"type":"hello","protocol":1,"version":"0.10.0","events":["status"],"capabilities":["pause"]}
//!   {"seq":42,"type":"status","state":{…}}
//!
//! hook → daemon (stdout), one JSON object per line, only declared capabilities
//!   {"type":"pause"}
//! ```
//!
//! A `resync` frame means the hook lagged behind the event ring and should
//! re-read whatever it cares about. Hooks are read-only unless `capabilities`
//! names the commands they may send.

use std::{
    collections::{BTreeMap, HashSet},
    process::Stdio,
    sync::Arc,
};

use lekima_config::HookConfig;
use lekima_ipc::{Command, Event, EventFrame, EventKind, HOOK_CAPABILITIES, is_hook_capability};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{ChildStdin, ChildStdout, Command as Process},
    sync::{broadcast, mpsc},
};

/// Version of the hook contract carried in the hello frame.
pub const PROTOCOL: u16 = 1;

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

/// A running hook process. Dropping it kills the child and reaps it.
#[derive(Debug)]
pub struct RunningHook {
    name: String,
    /// Owns the child and awaits its exit, so a hook that exits on its own is
    /// reaped instead of lingering as a zombie.
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
    commands: mpsc::UnboundedSender<(String, Command)>,
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
                tracing::info!(hook = %name, command = ?config.command, "hook started");
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
    commands: mpsc::UnboundedSender<(String, Command)>,
) -> std::io::Result<RunningHook> {
    let mut child = Process::new(&spec.command[0])
        .args(&spec.command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
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

    let spec = Arc::new(spec);
    tokio::spawn(write_events(Arc::clone(&spec), stdin, events.subscribe()));
    tokio::spawn(read_commands(spec.clone(), stdout, commands));

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
    loop {
        match events.recv().await {
            Ok(frame) => {
                last_seq = frame.seq;
                if !spec.wants(&frame.event) {
                    continue;
                }
                let Ok(line) = serde_json::to_string(&frame) else {
                    continue;
                };
                if write_line(&mut stdin, &line).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(hook = %spec.name, skipped, "hook lagged; sending resync");
                let frame = EventFrame {
                    seq: last_seq,
                    event: Event::Resync,
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
    commands: mpsc::UnboundedSender<(String, Command)>,
) {
    let mut lines: Lines<BufReader<ChildStdout>> = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => handle_line(&spec, line.trim(), &commands),
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(hook = %spec.name, %error, "hook output failed");
                break;
            }
        }
    }
    tracing::info!(hook = %spec.name, "hook exited");
}

fn handle_line(spec: &HookSpec, line: &str, commands: &mpsc::UnboundedSender<(String, Command)>) {
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
    let _ = commands.send((spec.name.clone(), command));
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
    use lekima_ipc::{Command, Event};

    use super::HookSpec;

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
        assert!(!spec.wants(&Event::Like {
            id: 1.into(),
            liked: true
        }));
        assert_eq!(spec.event_names(), vec!["status"]);

        assert!(spec.allows(&Command::Pause));
        assert!(!spec.allows(&Command::Next));
        assert_eq!(spec.capability_names(), vec!["pause".to_owned()]);
    }
}

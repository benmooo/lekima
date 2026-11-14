//! Minimal MCP (Model Context Protocol) server over stdio.
//!
//! A thin adapter: every tool maps to one IPC command on the daemon. No state,
//! no secrets, no second player. stdout is reserved for JSON-RPC; logs go to
//! stderr.

use lekima_domain::TrackId;
use lekima_ipc::{Command, FmAction, PlayTarget, Reply};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{
    commands::daemon_call,
    output::{Failure, reply_data},
};

const PROTOCOL: &str = "2025-06-18";

pub async fn serve(
    socket: std::path::PathBuf,
    paths: &lekima_config::Paths,
    config: &lekima_config::Config,
) -> Result<(), Failure> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| Failure::internal(error.to_string()))?
    {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            // Notifications (initialized, cancelled, …) need no reply.
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();

        let response = match method {
            "initialize" => success(
                id,
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "lekima", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
            "ping" => success(id, json!({})),
            "tools/list" => success(id, json!({ "tools": tools() })),
            "tools/call" => tools_call(id, &request, &socket, paths, config).await,
            other => failure(id, -32601, format!("unknown method `{other}`")),
        };

        let encoded = serde_json::to_string(&response)
            .map_err(|error| Failure::internal(error.to_string()))?;
        stdout
            .write_all(encoded.as_bytes())
            .await
            .map_err(|error| Failure::internal(error.to_string()))?;
        stdout
            .write_all(b"\n")
            .await
            .map_err(|error| Failure::internal(error.to_string()))?;
        stdout
            .flush()
            .await
            .map_err(|error| Failure::internal(error.to_string()))?;
    }
    Ok(())
}

async fn tools_call(
    id: Value,
    request: &Value,
    socket: &std::path::Path,
    paths: &lekima_config::Paths,
    config: &lekima_config::Config,
) -> Value {
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let command = match build_command(name, &args) {
        Ok(command) => command,
        Err(message) => return tool_error(id, "bad_request", &message),
    };

    match daemon_call(socket, paths, config, command).await {
        Ok(reply) => {
            let data = reply_data(&reply);
            success(
                id,
                json!({
                    "content": [{ "type": "text", "text": summary(&reply) }],
                    "structuredContent": data,
                    "isError": false,
                }),
            )
        }
        Err(error) => tool_error(id, "backend", &error.message),
    }
}

fn build_command(name: &str, args: &Value) -> Result<Command, String> {
    Ok(match name {
        "player_status" => Command::Status,
        "search" => Command::Search {
            query: string_arg(args, "query")?,
            kind: lekima_domain::SearchKind::Track,
            limit: number_arg(args, "limit").unwrap_or(25.0).clamp(1.0, 100.0) as u32,
            offset: 0,
        },
        "play" => Command::Play {
            target: play_target(args)?,
        },
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "next" => Command::Next,
        "previous" => Command::Prev,
        "set_volume" => Command::SetVolume {
            level: (number_arg(args, "level").unwrap_or(70.0) / 100.0).clamp(0.0, 1.0) as f32,
        },
        "queue_list" => Command::Queue,
        "queue_add" => Command::Enqueue {
            tracks: id_list(args)?,
            next: args.get("next").and_then(Value::as_bool).unwrap_or(false),
        },
        "lyrics" => Command::Lyrics {
            id: args.get("id").and_then(Value::as_u64).map(TrackId),
        },
        "daily" => Command::Daily {
            play: args.get("play").and_then(Value::as_bool).unwrap_or(true),
        },
        "fm" => Command::Fm {
            action: match args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("start")
            {
                "next" => FmAction::Next { trash: false },
                "trash" => FmAction::Next { trash: true },
                "stop" => FmAction::Stop,
                _ => FmAction::Start,
            },
        },
        other => return Err(format!("unknown tool `{other}`")),
    })
}

fn play_target(args: &Value) -> Result<PlayTarget, String> {
    if let Some(query) = args.get("query").and_then(Value::as_str)
        && !query.trim().is_empty()
    {
        return Ok(PlayTarget::Query {
            query: query.to_owned(),
        });
    }
    if args.get("ids").is_some() {
        return Ok(PlayTarget::Tracks {
            ids: id_list(args)?,
        });
    }
    if let Some(id) = args.get("playlist").and_then(Value::as_u64) {
        return Ok(PlayTarget::Playlist { id: id.into() });
    }
    Err("pass `query`, `ids` or `playlist`".to_owned())
}

fn id_list(args: &Value) -> Result<Vec<TrackId>, String> {
    let ids = args
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "`ids` must be an array of track ids".to_owned())?;
    ids.iter()
        .map(|id| {
            id.as_u64()
                .map(TrackId)
                .ok_or_else(|| "track ids must be integers".to_owned())
        })
        .collect()
}

fn string_arg(args: &Value, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("`{name}` is required"))
}

fn number_arg(args: &Value, name: &str) -> Option<f64> {
    args.get(name).and_then(Value::as_f64)
}

fn summary(reply: &Reply) -> String {
    match reply {
        Reply::Status { state } => match &state.track {
            Some(track) => format!(
                "{}{} {} — {}",
                if state.loading { "loading " } else { "" },
                state.status.as_str(),
                track.name,
                track.artist_names()
            ),
            None => state.status.as_str().to_owned(),
        },
        Reply::Tracks { tracks } => match tracks.first() {
            Some(first) => format!("{} tracks · first: {}", tracks.len(), first.name),
            None => "no tracks".to_owned(),
        },
        Reply::Search { results } => format!("{} results", results.tracks.len()),
        Reply::Playlists { playlists } => format!("{} playlists", playlists.len()),
        Reply::Toplists { toplists } => format!("{} charts", toplists.len()),
        Reply::Queue { tracks, index } => {
            format!("queue of {} (index {index:?})", tracks.len())
        }
        Reply::Like { id, liked } => {
            format!("{} {id}", if *liked { "liked" } else { "unliked" })
        }
        Reply::Lyrics { lyrics, .. } => format!("{} lyric lines", lyrics.lines.len()),
        Reply::User { user } => user
            .as_ref()
            .map_or_else(|| "signed out".to_owned(), |user| user.nickname.clone()),
        Reply::Ack | Reply::Pong => "ok".to_owned(),
        Reply::Welcome { protocol, .. } => format!("protocol {protocol}"),
        Reply::Subscribed { seq, .. } => format!("subscribed at seq {seq}"),
    }
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn tool_error(id: Value, code: &str, message: &str) -> Value {
    success(
        id,
        json!({
            "content": [{ "type": "text", "text": format!("! {message}") }],
            "structuredContent": { "error": { "code": code, "message": message } },
            "isError": true,
        }),
    )
}

fn tools() -> Vec<Value> {
    fn tool(name: &str, description: &str, schema: Value, read_only: bool) -> Value {
        json!({
            "name": name,
            "description": description,
            "inputSchema": schema,
            "annotations": { "readOnlyHint": read_only, "openWorldHint": false },
        })
    }
    let none = json!({ "type": "object", "properties": {}, "additionalProperties": false });

    vec![
        tool(
            "player_status",
            "Current track, state, position, volume and queue.",
            none.clone(),
            true,
        ),
        tool(
            "search",
            "Search NetEase Cloud Music tracks.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 100 } },
                "required": ["query"]
            }),
            true,
        ),
        tool(
            "play",
            "Play a search query, track ids or a playlist.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "ids": { "type": "array", "items": { "type": "integer" } },
                    "playlist": { "type": "integer" }
                }
            }),
            false,
        ),
        tool("pause", "Pause playback.", none.clone(), false),
        tool("resume", "Resume playback.", none.clone(), false),
        tool("next", "Play the next track.", none.clone(), false),
        tool("previous", "Play the previous track.", none.clone(), false),
        tool(
            "set_volume",
            "Set volume (0-100).",
            json!({
                "type": "object",
                "properties": { "level": { "type": "number", "minimum": 0, "maximum": 100 } },
                "required": ["level"]
            }),
            false,
        ),
        tool("queue_list", "List the playback queue.", none.clone(), true),
        tool(
            "queue_add",
            "Append tracks to the queue, or insert after the current track.",
            json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "integer" } },
                    "next": { "type": "boolean" }
                },
                "required": ["ids"]
            }),
            false,
        ),
        tool(
            "lyrics",
            "Timed lyrics for the current or a specific track.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer" } }
            }),
            true,
        ),
        tool(
            "daily",
            "Daily recommendations; plays by default.",
            json!({
                "type": "object",
                "properties": { "play": { "type": "boolean" } }
            }),
            false,
        ),
        tool(
            "fm",
            "Personal FM: start, next, trash or stop.",
            json!({
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["start", "next", "trash", "stop"] } }
            }),
            false,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{build_command, tools};
    use lekima_ipc::Command;

    #[test]
    fn tool_list_includes_the_core_surface() {
        let names: Vec<String> = tools()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        for expected in ["player_status", "play", "pause", "search", "lyrics", "fm"] {
            assert!(names.iter().any(|name| name == expected), "{names:?}");
        }
    }

    #[test]
    fn play_prefers_query_then_ids() {
        let command = build_command("play", &serde_json::json!({"query": "mota"})).unwrap();
        assert!(matches!(command, Command::Play { .. }));
        let command = build_command("play", &serde_json::json!({"ids": [1, 2]})).unwrap();
        assert!(matches!(command, Command::Play { .. }));
        assert!(build_command("play", &serde_json::json!({})).is_err());
    }
}

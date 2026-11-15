//! Contract tests for the agent-facing CLI: JSON shape and exit codes.

use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lekima"))
}

fn hermetic(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lekima-it-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn capabilities_are_machine_readable() {
    let dir = hermetic("capabilities");
    let output = binary()
        .env("LEKIMA_CONFIG_DIR", &dir)
        .args(["capabilities", "--json"])
        .output()
        .expect("run lekima");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid json envelope");
    assert_eq!(json["ok"], true);
    let names: Vec<_> = json["data"]["subcommands"]
        .as_array()
        .expect("subcommands")
        .iter()
        .filter_map(|command| command["name"].as_str())
        .collect();
    for expected in ["play", "search", "queue", "daemon", "tui"] {
        assert!(
            names.contains(&expected),
            "missing `{expected}` in {names:?}"
        );
    }
}

#[test]
fn status_without_a_daemon_exits_seven_with_a_daemon_error() {
    let dir = hermetic("status");
    fs::write(dir.join("config.toml"), "[daemon]\nautostart = false\n").expect("config");

    let output = binary()
        .env("LEKIMA_CONFIG_DIR", &dir)
        .args(["status", "--json"])
        .output()
        .expect("run lekima");

    assert_eq!(output.status.code(), Some(7));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid json envelope");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "daemon");
}

#[test]
fn themes_are_listed_for_agents() {
    let dir = hermetic("themes");
    let output = binary()
        .env("LEKIMA_CONFIG_DIR", &dir)
        .args(["themes", "--json"])
        .output()
        .expect("run lekima");
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let themes = json["data"]["themes"].as_array().expect("themes");
    assert_eq!(themes.len(), 12);
    assert!(themes.iter().any(|theme| theme == "dracula"));
    assert!(themes.iter().any(|theme| theme == "catppuccin"));
    assert!(themes.iter().any(|theme| theme == "light"));
}

#[test]
fn mcp_speaks_initialize_and_tools_list() {
    let dir = hermetic("mcp");
    let mut child = binary()
        .env("LEKIMA_CONFIG_DIR", &dir)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mcp");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
        )
        .unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
        )
        .unwrap();
    }

    let output = child.wait_with_output().expect("mcp exits on stdin close");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "lekima");
    let tools = lines[1]["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in ["player_status", "play", "search", "lyrics"] {
        assert!(names.contains(&expected), "{names:?}");
    }
}

#[test]
fn doctor_always_returns_a_report() {
    let dir = hermetic("doctor");
    let output = binary()
        .env("LEKIMA_CONFIG_DIR", &dir)
        .args(["doctor", "--json"])
        .output()
        .expect("run lekima");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid json envelope");
    assert_eq!(json["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(json["data"]["session"]["present"].is_boolean());
}

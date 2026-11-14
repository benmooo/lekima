//! Contract tests for the agent-facing CLI: JSON shape and exit codes.

use std::{fs, path::PathBuf, process::Command};

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
        assert!(names.contains(&expected), "missing `{expected}` in {names:?}");
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

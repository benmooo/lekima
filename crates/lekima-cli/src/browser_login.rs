//! Assisted web login through a locally launched browser.
//!
//! NetEase sometimes rejects QR logins created by API clients. The official web
//! player is a normal browser session, so this frontend opens it with a
//! dedicated profile, reads the resulting `MUSIC_U` cookie over the DevTools
//! protocol and hands it back to the CLI. The user's own browser profile is
//! never touched.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use lekima_config::FileCredentials;
use lekima_domain::CredentialsStore;

/// How long the user has to finish signing in before we give up.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long Chrome may take to write its debugging port and open a page.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const LOGIN_URL: &str = "https://music.163.com/";
const BROWSER_ENV: &str = "LEKIMA_BROWSER";

/// Outcome of an assisted web login attempt.
pub(crate) enum SignIn {
    SignedIn,
    /// No Chrome-family browser was found, so the caller should fall back.
    NoBrowser,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BrowserError {
    #[error("failed to start the browser: {0}")]
    Spawn(#[source] io::Error),
    #[error("the browser did not expose a debugging port")]
    NoDebugPort,
    #[error("the browser closed before the session was saved")]
    Closed,
    #[error("timed out waiting for the web login")]
    Timeout,
    #[error("browser protocol error: {0}")]
    Protocol(String),
    #[error("failed to save the session: {0}")]
    Save(#[source] lekima_domain::DomainError),
}

/// Opens the web player, waits for a confirmed session and stores its cookies.
///
/// Returns [`SignIn::NoBrowser`] when nothing launchable is installed; every
/// other failure is reported so the caller can explain and fall back.
pub(crate) async fn sign_in(session_file: &Path, profile: &Path) -> Result<SignIn, BrowserError> {
    let Some(browser) = discover_browser() else {
        return Ok(SignIn::NoBrowser);
    };

    fs::create_dir_all(profile).map_err(BrowserError::Spawn)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(profile, fs::Permissions::from_mode(0o700));
    }
    // A stale port file from a killed browser would point at nothing.
    let _ = fs::remove_file(profile.join("DevToolsActivePort"));

    let mut child = BrowserChild::spawn(&browser, profile)?;
    let port = wait_for_port(profile, &mut child).await?;
    let websocket = wait_for_login_page(port, &mut child).await?;
    // The outer timeout also covers a DevTools socket that stops answering.
    let cookies = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_session(&websocket, &mut child))
        .await
        .map_err(|_| BrowserError::Timeout)??;

    FileCredentials::new(session_file)
        .save(&cookies)
        .map_err(BrowserError::Save)?;
    Ok(SignIn::SignedIn)
}

/// Kills the child on every exit path, including errors and cancellation.
struct BrowserChild(Child);

impl BrowserChild {
    fn spawn(browser: &Path, profile: &Path) -> Result<Self, BrowserError> {
        let child = Command::new(browser)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-session-crashed-bubble",
                "--remote-debugging-port=0",
                LOGIN_URL,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BrowserError::Spawn)?;
        Ok(Self(child))
    }

    fn exited(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(Some(_)))
    }
}

impl Drop for BrowserChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Locates a Chromium-family browser without starting it.
#[must_use]
pub(crate) fn discover_browser() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(BROWSER_ENV) {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }

    const BUNDLES: [&str; 4] = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    for bundle in BUNDLES {
        let path = PathBuf::from(bundle);
        if path.is_file() {
            return Some(path);
        }
    }

    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
    ] {
        if let Some(path) = find_in_path(name) {
            return Some(path);
        }
    }
    None
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

async fn wait_for_port(profile: &Path, child: &mut BrowserChild) -> Result<u16, BrowserError> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(port) = read_debug_port(profile) {
            return Ok(port);
        }
        if child.exited() {
            return Err(BrowserError::Closed);
        }
        if Instant::now() >= deadline {
            return Err(BrowserError::NoDebugPort);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn read_debug_port(profile: &Path) -> Option<u16> {
    let raw = fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    raw.lines().next()?.trim().parse().ok()
}

async fn wait_for_login_page(port: u16, child: &mut BrowserChild) -> Result<String, BrowserError> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| BrowserError::Protocol(error.to_string()))?;
    let url = format!("http://127.0.0.1:{port}/json/list");
    let deadline = Instant::now() + STARTUP_TIMEOUT;

    loop {
        if child.exited() {
            return Err(BrowserError::Closed);
        }
        if let Ok(response) = client.get(&url).send().await
            && let Ok(body) = response.text().await
            && let Ok(targets) = serde_json::from_str::<Vec<Value>>(&body)
            && let Some(websocket) = targets.iter().find_map(login_target)
        {
            return Ok(websocket);
        }
        if Instant::now() >= deadline {
            return Err(BrowserError::Protocol(
                "no music.163.com page appeared".to_owned(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn login_target(target: &Value) -> Option<String> {
    let is_page = target["type"].as_str() == Some("page");
    let on_music = target["url"]
        .as_str()
        .is_some_and(|url| url.contains("music.163.com"));
    (is_page && on_music)
        .then(|| target["webSocketDebuggerUrl"].as_str().map(str::to_owned))
        .flatten()
}

async fn wait_for_session(
    websocket: &str,
    child: &mut BrowserChild,
) -> Result<String, BrowserError> {
    let (mut socket, _) = connect_async(websocket)
        .await
        .map_err(|error| BrowserError::Protocol(error.to_string()))?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let mut id = 0_u64;

    loop {
        if child.exited() {
            return Err(BrowserError::Closed);
        }
        if Instant::now() >= deadline {
            return Err(BrowserError::Timeout);
        }

        id += 1;
        let request = json!({"id": id, "method": "Network.getAllCookies"});
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        let cookies = read_cookies(&mut socket, id).await?;
        if let Some(header) = cookie_header(&cookies) {
            return Ok(header);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn read_cookies(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
) -> Result<Vec<Value>, BrowserError> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| BrowserError::Protocol(error.to_string()))?;
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        if value["id"].as_u64() != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(BrowserError::Protocol(error.to_string()));
        }
        return Ok(value["result"]["cookies"]
            .as_array()
            .cloned()
            .unwrap_or_default());
    }
    Err(BrowserError::Closed)
}

/// Builds the cookie header from DevTools cookie objects.
///
/// Only cookies belonging to NetEase are kept, and the header is returned only
/// once `MUSIC_U` is present, so a half-finished login is never persisted.
fn cookie_header(cookies: &[Value]) -> Option<String> {
    let mut pairs = BTreeMap::new();
    let mut authenticated = false;

    for cookie in cookies {
        let name = cookie["name"].as_str().unwrap_or_default();
        let value = cookie["value"].as_str().unwrap_or_default();
        let domain = cookie["domain"].as_str().unwrap_or_default();
        if name.is_empty() || value.is_empty() {
            continue;
        }
        let is_session = name == "MUSIC_U" || name == "MUSIC_A";
        let on_netease = domain == "music.163.com"
            || domain.ends_with(".music.163.com")
            || domain.ends_with(".163.com");
        if !is_session && !on_netease {
            continue;
        }
        authenticated |= name == "MUSIC_U";
        pairs.insert(name.to_owned(), value.to_owned());
    }

    authenticated.then(|| {
        pairs
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{cookie_header, read_debug_port};

    #[test]
    fn session_header_keeps_netease_cookies_only() {
        let cookies = vec![
            json!({"name": "MUSIC_U", "value": "abc", "domain": ".music.163.com"}),
            json!({"name": "__csrf", "value": "csrf", "domain": "music.163.com"}),
            json!({"name": "NMTID", "value": "device", "domain": ".music.163.com"}),
            json!({"name": "GA", "value": "tracker", "domain": ".google.com"}),
        ];

        let header = cookie_header(&cookies).expect("MUSIC_U must produce a header");

        assert!(header.contains("MUSIC_U=abc"));
        assert!(header.contains("__csrf=csrf"));
        assert!(header.contains("NMTID=device"));
        assert!(!header.contains("GA=tracker"));
    }

    #[test]
    fn header_waits_until_music_u_exists() {
        let cookies = vec![json!({"name": "__csrf", "value": "csrf", "domain": "music.163.com"})];
        assert!(cookie_header(&cookies).is_none());
    }

    #[test]
    fn empty_cookie_values_are_skipped() {
        let cookies = vec![
            json!({"name": "MUSIC_U", "value": "abc", "domain": ".music.163.com"}),
            json!({"name": "empty", "value": "", "domain": ".music.163.com"}),
        ];
        let header = cookie_header(&cookies).unwrap();
        assert!(!header.contains("empty="));
    }

    #[test]
    fn debug_port_is_read_from_the_first_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("DevToolsActivePort"),
            "49152\n/devtools/browser/abc\n",
        )
        .unwrap();
        assert_eq!(read_debug_port(dir.path()), Some(49152));
    }

    #[test]
    fn missing_debug_port_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_debug_port(dir.path()), None);
    }
}

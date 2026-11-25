//! HTTP control surface for phones and remote scripts.
//!
//! The server is a thin frontend over the daemon's JSON protocol: it owns no
//! playback state and never talks to the provider. Every request except
//! discovery requires the bearer token stored in `<config>/http.token`.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Query, State, rejection::JsonRejection, rejection::QueryRejection,
    },
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY, WWW_AUTHENTICATE,
            X_CONTENT_TYPE_OPTIONS,
        },
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use lekima_config::{HttpToken, Paths};
use lekima_ipc::{ApiError, Command, ErrorCode, IpcError, MAX_FRAME_BYTES, PlayTarget, call};
use serde_json::{Value, json};

use crate::output::{self, Failure};

/// Generated tokens are 32 random bytes, so 64 hex characters.
const TOKEN_BYTES: usize = 32;
/// A hand-supplied token shorter than this is rejected as guessable.
const MIN_TOKEN_CHARS: usize = 16;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const ENDPOINTS: [&str; 8] = [
    "GET /",
    "GET /v1/health",
    "GET /v1/capabilities",
    "GET /v1/status",
    "GET /v1/queue",
    "GET /v1/history?limit=N",
    "GET /v1/hooks",
    "POST /v1/command",
];
const REMOTE_HTML: &str = include_str!("../assets/remote.html");
/// The page is self-contained, so only same-origin requests and inline code.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; \
     script-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; \
     form-action 'none'; frame-ancestors 'none'";
/// Command tags accepted by `POST /v1/command`.
const COMMAND_TYPES: [&str; 41] = [
    "status",
    "play",
    "jump",
    "pause",
    "resume",
    "toggle",
    "stop",
    "next",
    "prev",
    "seek",
    "set_volume",
    "toggle_mute",
    "cycle_repeat",
    "set_repeat",
    "set_shuffle",
    "queue",
    "queue_remove",
    "queue_move",
    "enqueue",
    "clear_queue",
    "daily",
    "fm",
    "toplists",
    "cloud",
    "like",
    "comments",
    "history",
    "hooks",
    "liked",
    "sleep",
    "set_quality",
    "search",
    "playlists",
    "playlist_tracks",
    "lyrics",
    "whoami",
    "artist",
    "artist_albums",
    "album",
    "queue_album",
    "queue_playlist",
];

#[derive(Clone)]
struct AppState {
    socket: PathBuf,
    token: Arc<str>,
}

/// Starts the HTTP server and blocks until `Ctrl-C`.
///
/// # Errors
///
/// Fails when the token cannot be read or the address cannot be bound.
pub(crate) async fn http(
    paths: &Paths,
    socket: PathBuf,
    bind: SocketAddr,
    json: bool,
) -> Result<(), Failure> {
    let token = load_token(paths)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| Failure::internal(format!("failed to bind {bind}: {error}")))?;
    let local = listener
        .local_addr()
        .map_err(|error| Failure::internal(format!("failed to inspect the listener: {error}")))?;

    if json {
        output::emit_json(
            true,
            Some(json!({
                "bind": local.to_string(),
                "page": format!("http://{local}/"),
                "api": format!("http://{local}/v1"),
                "token_file": paths.http_token_file(),
                "endpoints": ENDPOINTS,
            })),
            None,
        );
    } else {
        crate::say!("lekima HTTP control listening on http://{local}/v1");
        crate::say!("web remote: http://{local}/  (paste the token once)");
        crate::say!(
            "authorization: Bearer token from {}",
            paths.http_token_file().display()
        );
        if !local.ip().is_loopback() {
            crate::say!("warning: this is reachable from your network; keep the token private");
        }
        crate::say!("press ctrl-c to stop");
    }
    tracing::info!(address = %local, "http control listening");

    let state = AppState { socket, token };
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|error| Failure::internal(format!("HTTP server failed: {error}")))
}

fn load_token(paths: &Paths) -> Result<Arc<str>, Failure> {
    if let Ok(value) = std::env::var("LEKIMA_HTTP_TOKEN") {
        let token = value.trim();
        if token.len() < MIN_TOKEN_CHARS {
            return Err(Failure::usage(format!(
                "LEKIMA_HTTP_TOKEN must be at least {MIN_TOKEN_CHARS} characters"
            )));
        }
        return Ok(Arc::from(token));
    }

    let store = HttpToken::new(paths.http_token_file());
    if let Some(token) = store.load()? {
        return Ok(Arc::from(token));
    }
    let token = generate_token()?;
    store.save(&token)?;
    Ok(Arc::from(token))
}

fn generate_token() -> Result<String, Failure> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| Failure::internal(format!("failed to generate a token: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(remote))
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/status", get(status))
        .route("/v1/queue", get(queue))
        .route("/v1/history", get(history))
        .route("/v1/hooks", get(hooks))
        .route("/v1/command", post(command))
        .layer(DefaultBodyLimit::max(MAX_FRAME_BYTES))
        .with_state(state)
}

async fn remote() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    (headers, REMOTE_HTML).into_response()
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true, "data": {"status": "ok"}}))
}

async fn capabilities() -> Json<Value> {
    Json(json!({
        "ok": true,
        "data": {
            "version": env!("CARGO_PKG_VERSION"),
            "auth": "bearer",
            "page": "/",
            "endpoints": ENDPOINTS,
            "commands": COMMAND_TYPES.as_slice(),
        }
    }))
}

async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize(&headers, &state) {
        return response;
    }
    forward(&state, Command::Status).await
}

async fn queue(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize(&headers, &state) {
        return response;
    }
    forward(&state, Command::Queue).await
}

#[derive(Debug, serde::Deserialize)]
struct HistoryQuery {
    limit: Option<usize>,
}

async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    if let Some(response) = authorize(&headers, &state) {
        return response;
    }
    // A malformed query is answered only after authorisation, so an
    // unauthenticated caller learns nothing and every error keeps the
    // documented JSON envelope.
    let Ok(Query(query)) = query else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::BadRequest,
            "`limit` must be a whole number".to_owned(),
        );
    };
    forward(&state, Command::History { limit: query.limit }).await
}

async fn hooks(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize(&headers, &state) {
        return response;
    }
    forward(&state, Command::Hooks).await
}

async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<Command>, JsonRejection>,
) -> Response {
    if let Some(response) = authorize(&headers, &state) {
        return response;
    }
    let Json(command) = match payload {
        Ok(payload) => payload,
        Err(rejection) => {
            return error(
                StatusCode::BAD_REQUEST,
                ErrorCode::BadRequest,
                rejection.body_text(),
            );
        }
    };
    if !permitted(&command) {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unsupported,
            "that command is not exposed over HTTP".to_owned(),
        );
    }
    forward(&state, command).await
}

async fn forward(state: &AppState, command: Command) -> Response {
    match tokio::time::timeout(COMMAND_TIMEOUT, call(&state.socket, command)).await {
        Ok(Ok(reply)) => (
            StatusCode::OK,
            Json(json!({"ok": true, "data": output::reply_data(&reply)})),
        )
            .into_response(),
        Ok(Err(IpcError::Remote { code, message })) => error(status_for(code), code, message),
        Ok(Err(IpcError::NotRunning { path })) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Daemon,
            format!(
                "the player daemon is not running at {}; start it with `lekima daemon start`",
                path.display()
            ),
        ),
        Ok(Err(IpcError::Timeout(duration))) => error(
            StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Daemon,
            format!("the daemon did not answer within {duration:?}"),
        ),
        Ok(Err(IpcError::Protocol(message))) => {
            error(StatusCode::BAD_GATEWAY, ErrorCode::Daemon, message)
        }
        Ok(Err(IpcError::Io(source))) => error(
            StatusCode::BAD_GATEWAY,
            ErrorCode::Daemon,
            source.to_string(),
        ),
        Err(_) => error(
            StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Daemon,
            "the daemon did not answer in time".to_owned(),
        ),
    }
}

/// Returns a response only when the request is not authorized.
fn authorize(headers: &HeaderMap, state: &AppState) -> Option<Response> {
    let presented = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if presented.is_some_and(|token| constant_time_eq(token.as_bytes(), state.token.as_bytes())) {
        return None;
    }
    Some(
        (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))],
            Json(json!({
                "ok": false,
                "error": ApiError::new(
                    ErrorCode::Unauthorized,
                    "missing or invalid bearer token",
                ),
            })),
        )
            .into_response(),
    )
}

fn error(status: StatusCode, code: ErrorCode, message: String) -> Response {
    (
        status,
        Json(json!({"ok": false, "error": ApiError::new(code, message)})),
    )
        .into_response()
}

fn status_for(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
        ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Unsupported => StatusCode::NOT_IMPLEMENTED,
        ErrorCode::Backend => StatusCode::BAD_GATEWAY,
        ErrorCode::Playback => StatusCode::CONFLICT,
        ErrorCode::Daemon => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Default-deny allowlist: commands added later stay private until listed.
fn permitted(command: &Command) -> bool {
    // Album playback resolves tracks and rewrites the queue, so it stays local
    // until a deployment opts in explicitly (ADR-004).
    if matches!(
        command,
        Command::QueueAlbum { .. }
            | Command::QueuePlaylist { .. }
            | Command::Play {
                target: PlayTarget::Album { .. }
            }
    ) {
        return false;
    }
    matches!(
        command,
        Command::Status
            | Command::Play { .. }
            | Command::Jump { .. }
            | Command::Pause
            | Command::Resume
            | Command::Toggle
            | Command::Stop
            | Command::Next
            | Command::Prev
            | Command::Seek { .. }
            | Command::SetVolume { .. }
            | Command::ToggleMute
            | Command::CycleRepeat
            | Command::SetRepeat { .. }
            | Command::SetShuffle { .. }
            | Command::Queue
            | Command::QueueRemove { .. }
            | Command::QueueMove { .. }
            | Command::Enqueue { .. }
            | Command::ClearQueue
            | Command::Daily { .. }
            | Command::Fm { .. }
            | Command::Toplists
            | Command::Cloud { .. }
            | Command::Like { .. }
            | Command::Comments { .. }
            | Command::History { .. }
            | Command::Hooks
            | Command::Liked { .. }
            | Command::Sleep { .. }
            | Command::SetQuality { .. }
            | Command::Search { .. }
            | Command::Playlists
            | Command::PlaylistTracks { .. }
            | Command::Lyrics { .. }
            | Command::Whoami
            | Command::Artist { .. }
            | Command::ArtistAlbums { .. }
            | Command::Album { .. }
    )
}

fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    if presented.len() != expected.len() {
        return false;
    }
    presented
        .iter()
        .zip(expected)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{AppState, constant_time_eq, permitted, router};
    use lekima_ipc::Command;

    fn state() -> AppState {
        AppState {
            socket: PathBuf::from("/nonexistent/lekima-test.sock"),
            token: Arc::from("test-token-0123456789"),
        }
    }

    async fn call(request: Request<Body>) -> (StatusCode, Value) {
        let response = router(state()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn the_remote_page_is_public_and_hardened() {
        let response = router(state())
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(response.headers().contains_key("content-security-policy"));

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let page = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(page.contains("/v1/"));
        assert!(page.contains("lekima.http.token"));
    }

    #[tokio::test]
    async fn health_is_public() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
    }

    #[tokio::test]
    async fn capabilities_describe_the_auth_scheme() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["auth"], "bearer");
        assert!(body["data"]["commands"].as_array().unwrap().len() >= 20);
    }

    #[tokio::test]
    async fn status_requires_a_bearer_token() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");
    }

    #[tokio::test]
    async fn a_valid_token_reaches_the_daemon_error_path() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/status")
                .header("authorization", "Bearer test-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "daemon");
    }

    #[tokio::test]
    async fn history_requires_a_bearer_token() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/history?limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");
    }

    #[tokio::test]
    async fn history_is_permitted_with_a_token() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/history?limit=5")
                .header("authorization", "Bearer test-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        // No daemon in this test, so reaching the forward path is the assertion.
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "daemon");
    }

    #[tokio::test]
    async fn a_malformed_history_limit_is_rejected_after_authorisation() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/history?limit=abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");

        let (status, body) = call(
            Request::builder()
                .uri("/v1/history?limit=abc")
                .header("authorization", "Bearer test-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "bad_request");
    }

    #[tokio::test]
    async fn hooks_requires_a_bearer_token() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/hooks")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");
    }

    #[tokio::test]
    async fn hooks_is_permitted_with_a_token() {
        let (status, body) = call(
            Request::builder()
                .uri("/v1/hooks")
                .header("authorization", "Bearer test-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "daemon");
    }

    #[tokio::test]
    async fn shutdown_is_not_exposed() {
        let (status, body) = call(
            Request::builder()
                .method("POST")
                .uri("/v1/command")
                .header("authorization", "Bearer test-token-0123456789")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"type":"shutdown"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["code"], "unsupported");
    }

    #[tokio::test]
    async fn malformed_commands_are_rejected_before_the_daemon() {
        let (status, body) = call(
            Request::builder()
                .method("POST")
                .uri("/v1/command")
                .header("authorization", "Bearer test-token-0123456789")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"type":"not_a_command"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "bad_request");
    }

    #[test]
    fn allowlist_opens_playback_but_not_shutdown_or_login() {
        assert!(permitted(&Command::Pause));
        assert!(permitted(&Command::Play {
            target: lekima_ipc::PlayTarget::Query {
                query: "mota".to_owned()
            }
        }));
        assert!(!permitted(&Command::Shutdown));
        assert!(!permitted(&Command::Login {
            cookie: "MUSIC_U=secret".to_owned()
        }));
        assert!(!permitted(&Command::Logout));
    }

    #[test]
    fn catalogue_reads_are_permitted_but_album_playback_is_local() {
        assert!(permitted(&Command::Artist {
            id: lekima_domain::ArtistId(6452)
        }));
        assert!(permitted(&Command::ArtistAlbums {
            id: lekima_domain::ArtistId(6452),
            limit: 20,
            offset: 0
        }));
        assert!(permitted(&Command::Album {
            id: lekima_domain::AlbumId(9)
        }));
        assert!(!permitted(&Command::QueueAlbum {
            id: lekima_domain::AlbumId(9),
            next: false
        }));
        assert!(!permitted(&Command::Play {
            target: lekima_ipc::PlayTarget::Album {
                id: lekima_domain::AlbumId(9)
            }
        }));
    }

    #[test]
    fn token_comparison_is_length_safe() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}

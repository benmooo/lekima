//! Client side of the daemon protocol.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::timeout,
};

use crate::protocol::{ApiError, Command, ErrorCode, MAX_FRAME_BYTES, Reply, Request, Response};

/// How long a single request may take, including connection setup.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("daemon is not running at {path}")]
    NotRunning { path: PathBuf },
    #[error("daemon request failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon sent a malformed frame: {0}")]
    Protocol(String),
    #[error("{message}")]
    Remote { code: ErrorCode, message: String },
    #[error("daemon did not answer within {0:?}")]
    Timeout(Duration),
}

impl IpcError {
    /// True when the failure means "no daemon is listening".
    #[must_use]
    pub fn is_not_running(&self) -> bool {
        matches!(self, Self::NotRunning { .. })
    }
}

impl From<ApiError> for IpcError {
    fn from(error: ApiError) -> Self {
        Self::Remote {
            code: error.code,
            message: error.message,
        }
    }
}

/// Sends one command to the daemon at `socket` and waits for the reply.
///
/// # Errors
///
/// Returns [`IpcError::NotRunning`] when nothing is listening, and
/// [`IpcError::Remote`] when the daemon reports a domain failure.
pub async fn call(socket: &Path, command: Command) -> Result<Reply, IpcError> {
    let stream = match UnixStream::connect(socket).await {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(IpcError::NotRunning {
                path: socket.to_path_buf(),
            });
        }
        Err(error) => return Err(IpcError::Io(error)),
    };
    call_on(stream, command).await
}

/// Sends one command over an already-connected socket.
///
/// # Errors
///
/// Fails on I/O, framing or timeout problems, or when the daemon reports an
/// application error.
pub async fn call_on(stream: UnixStream, command: Command) -> Result<Reply, IpcError> {
    timeout(REQUEST_TIMEOUT, exchange(stream, command))
        .await
        .map_err(|_| IpcError::Timeout(REQUEST_TIMEOUT))?
}

async fn exchange(stream: UnixStream, command: Command) -> Result<Reply, IpcError> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let request = Request::new(1, command);
    let mut payload =
        serde_json::to_string(&request).map_err(|error| IpcError::Protocol(error.to_string()))?;
    payload.push('\n');
    writer.write_all(payload.as_bytes()).await?;
    writer.flush().await?;

    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        return Err(IpcError::Protocol(
            "daemon closed the connection".to_owned(),
        ));
    }
    if line.len() > MAX_FRAME_BYTES {
        return Err(IpcError::Protocol("response frame too large".to_owned()));
    }
    let response: Response =
        serde_json::from_str(&line).map_err(|error| IpcError::Protocol(error.to_string()))?;
    match (response.result, response.error) {
        (_, Some(error)) => Err(error.into()),
        (Some(reply), None) => Ok(reply),
        (None, None) => Err(IpcError::Protocol("empty response".to_owned())),
    }
}

/// Checks whether a daemon answers a ping within a short deadline.
pub async fn is_running(socket: &Path) -> bool {
    matches!(
        timeout(Duration::from_millis(750), call(socket, Command::Ping)).await,
        Ok(Ok(Reply::Pong))
    )
}

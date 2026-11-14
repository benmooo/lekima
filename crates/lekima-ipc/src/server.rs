//! Server side of the daemon protocol.
//!
//! The transport is deliberately boring: accept a line, hand the decoded
//! request to the application, write one line back. All policy lives in the
//! daemon, so this module stays free of business rules.

use std::future::Future;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::UnixStream,
};

use crate::protocol::{ApiError, ErrorCode, MAX_FRAME_BYTES, Request, Response};

/// Serves requests on one accepted connection until the peer disconnects.
///
/// A malformed frame is answered with an error and the connection is closed:
/// a desynchronised stream cannot be trusted.
///
/// # Errors
///
/// Returns an I/O error when the socket itself fails.
pub async fn serve_connection<F, Fut>(stream: UnixStream, handler: F) -> std::io::Result<()>
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(());
        }
        if read > MAX_FRAME_BYTES {
            let response = Response::err(
                0,
                ApiError::new(ErrorCode::BadRequest, "request frame exceeds 1 MiB"),
            );
            write_response(&mut writer, &response).await?;
            return Ok(());
        }

        let request = match serde_json::from_str::<Request>(line.trim_end()) {
            Ok(request) => request,
            Err(error) => {
                let response = Response::err(
                    0,
                    ApiError::new(ErrorCode::BadRequest, format!("invalid request: {error}")),
                );
                write_response(&mut writer, &response).await?;
                return Ok(());
            }
        };

        let response = handler(request).await;
        write_response(&mut writer, &response).await?;
    }
}

async fn write_response<W>(writer: &mut W, response: &Response) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut encoded = serde_json::to_string(response).map_err(std::io::Error::other)?;
    encoded.push('\n');
    writer.write_all(encoded.as_bytes()).await?;
    writer.flush().await
}

//! Server side of the daemon protocol.
//!
//! One connection speaks either request/reply or, after a `subscribe` request,
//! a stream of sequenced events. The server owns framing only; policy lives in
//! the daemon.

use std::future::Future;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::UnixStream,
    sync::broadcast::{self, error::RecvError},
};

use crate::{
    event::{Event, EventFrame, EventKind},
    protocol::{ApiError, ErrorCode, MAX_FRAME_BYTES, Request, Response},
};

/// A live event stream attached to a connection.
pub struct Subscription {
    pub receiver: broadcast::Receiver<EventFrame>,
    pub initial: Vec<EventFrame>,
    /// Empty means "all kinds".
    pub kinds: Vec<EventKind>,
}

/// What a handler wants to happen on this connection.
pub struct Handled {
    pub response: Response,
    pub subscription: Option<Subscription>,
}

impl Handled {
    #[must_use]
    pub const fn reply(response: Response) -> Self {
        Self {
            response,
            subscription: None,
        }
    }

    #[must_use]
    pub const fn stream(response: Response, subscription: Subscription) -> Self {
        Self {
            response,
            subscription: Some(subscription),
        }
    }
}

/// Serves requests on one accepted connection until the peer disconnects.
///
/// # Errors
///
/// Returns an I/O error when the socket itself fails.
pub async fn serve_connection<F, Fut>(stream: UnixStream, handler: F) -> std::io::Result<()>
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Handled>,
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

        let handled = handler(request).await;
        write_response(&mut writer, &handled.response).await?;

        if let Some(mut subscription) = handled.subscription {
            stream_events(&mut writer, &mut reader, &mut subscription).await?;
            return Ok(());
        }
    }
}

async fn stream_events<W, R>(
    writer: &mut W,
    reader: &mut BufReader<R>,
    subscription: &mut Subscription,
) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    let kinds = subscription.kinds.clone();
    let mut last_seq = 0u64;

    for frame in &subscription.initial {
        if matches_kind(frame, &kinds) {
            last_seq = last_seq.max(frame.seq);
            write_event(writer, frame).await?;
        }
    }

    loop {
        tokio::select! {
            received = subscription.receiver.recv() => {
                match received {
                    Ok(frame) => {
                        if matches_kind(&frame, &kinds) {
                            last_seq = frame.seq;
                            write_event(writer, &frame).await?;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        let resync = EventFrame {
                            seq: last_seq,
                            event: Event::Resync,
                        };
                        write_event(writer, &resync).await?;
                    }
                    Err(RecvError::Closed) => return Ok(()),
                }
            }
            line = read_line(reader) => {
                // A subscriber is one-way; an extra frame or EOF ends the stream.
                if line == 0 {
                    return Ok(());
                }
            }
        }
    }
}

async fn read_line<R>(reader: &mut BufReader<R>) -> usize
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap_or(0)
}

fn matches_kind(frame: &EventFrame, kinds: &[EventKind]) -> bool {
    kinds.is_empty() || kinds.contains(&frame.event.kind())
}

async fn write_event<W>(writer: &mut W, frame: &EventFrame) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut encoded = serde_json::to_string(frame).map_err(std::io::Error::other)?;
    encoded.push('\n');
    writer.write_all(encoded.as_bytes()).await?;
    writer.flush().await
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

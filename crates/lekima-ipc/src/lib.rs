#![forbid(unsafe_code)]
//! IPC between the lekima daemon and every other client.
//!
//! One protocol, three audiences:
//!
//! - the TUI, which wants typed replies;
//! - the CLI, which wants stable JSON;
//! - code agents, which want the same stable JSON over a socket that outlives
//!   their invocation.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::{IpcError, call, call_on, is_running};
pub use protocol::{
    ApiError, Command, ErrorCode, FmAction, MAX_FRAME_BYTES, PlayTarget, Reply, Request, Response,
};
pub use server::serve_connection;

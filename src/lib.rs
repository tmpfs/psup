#![deny(missing_docs)]
//! Process supervisor with inter-process communication support using tokio.
//!
//! Currently only supports Unix, later we plan to add support for Windows using named pipes.
//!
//! ## Supervisor
//!
//! Supervisor manages child processes sending socket information over stdin and then switching 
//! to Unix domain sockets for inter-process communication. Daemon processes are restarted if 
//! they die without being shutdown by the supervisor.
//!
//! ```ignore
//! use psup::{Result, SupervisorBuilder};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!    let worker_cmd = "worker-process";
//!    let supervisor = SupervisorBuilder::new(Box::new(|stream| {
//!            let (reader, mut writer) = stream.into_split();
//!            // Handle worker connections here
//!        }))
//!        .path(std::env::temp_dir().join("supervisor.sock"))
//!        .add_daemon(worker_cmd.to_string(), vec![])
//!        .add_daemon(worker_cmd.to_string(), vec![])
//!        .build();
//!    supervisor.run().await?;
//!    // Block the process here and do your work.
//!    Ok(())
//! }
//! ```
//! 
//! ## Worker
//!
//! Worker reads the socket information from stdin and then connects to the Unix socket.
//!
//! ```ignore
//! use psup::{Result, worker};
//! 
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Read supervisor information from stdin
//!     // and set up the IPC channel with the supervisor
//!     let worker = Worker::new(|stream| {
//!         let (reader, mut writer) = stream.into_split();
//!         // Start sending messages to the supervisor
//!     });
//!     worker.run().await?;
//!     // Block the process here and do your work.
//!     Ok(())
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Enumeration of errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // Errors generated by the rpc library.
    /*
    #[error(transparent)]
    Rpc(#[from] json_rpc2::Error),
    */

    /// Input/output errors.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON serialize/deserialize errors.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Error whilst sending bind notifications.
    #[error(transparent)]
    Oneshot(#[from] tokio::sync::oneshot::error::RecvError),

    /// Error generated whilst attempting to read lines from the socket.
    #[error(transparent)]
    Lines(#[from] tokio_util::codec::LinesCodecError),
}

/// Result type returned by the library.
pub type Result<T> = std::result::Result<T, Error>;

mod worker;
mod supervisor;

pub use worker::Worker;
pub use supervisor::{Supervisor, SupervisorBuilder};

#[cfg(feature = "json_rpc2")]
mod json_rpc;
#[cfg(feature = "json_rpc2")]
pub use json_rpc::*;

/// Message sent over stdin when launching a worker.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerInfo {
    /// Socket path.
    pub path: PathBuf,
    /// Worker identifier.
    pub id: String,
}

/// Message sent to the supervisor when a worker
/// is spawned and has connected to the IPC socket.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Connected {
    /// Worker identifier.
    pub id: String,
}

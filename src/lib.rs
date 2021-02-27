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
//! use psup::{Result, Task, SupervisorBuilder};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!    let worker_cmd = "worker-process";
//!    let supervisor = SupervisorBuilder::new(Box::new(|stream| {
//!            let (reader, mut writer) = stream.into_split();
//!            // Handle worker connections here
//!        }))
//!        .path(std::env::temp_dir().join("supervisor.sock"))
//!        .add_daemon(Task::new(worker_cmd))
//!        .add_daemon(Task::new(worker_cmd))
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
    /// Input/output errors.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Error whilst sending bind notifications.
    #[error(transparent)]
    Oneshot(#[from] tokio::sync::oneshot::error::RecvError),

    /// Generic variant for errors created in user code.
    #[error(transparent)]
    Boxed(#[from] Box<dyn std::error::Error + Send>),
}

impl Error {
    /// Helper function to `Box` an error implementation.
    ///
    /// Ssocket handlers can call `map_err(Error::boxed)?` to propagate
    /// foreign errors.
    pub fn boxed(e: impl std::error::Error + Send + 'static) -> Self {
        let err: Box<dyn std::error::Error + Send> = Box::new(e);
        Error::from(err)
    }
}

/// Result type returned by the library.
pub type Result<T> = std::result::Result<T, Error>;

mod worker;
mod supervisor;

pub use worker::Worker;
pub use supervisor::{Task, Supervisor, SupervisorBuilder};

/// Message sent over stdin when launching a worker.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerInfo {
    /// Socket path.
    pub path: PathBuf,
    /// Worker identifier.
    pub id: String,
}

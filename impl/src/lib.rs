#![deny(missing_docs)]
//! Process supervisor with inter-process communication support using tokio.
//!
//! Currently only supports Unix, later we plan to add support for Windows using named pipes.
//!
//! ## Supervisor
//!
//! Supervisor manages child processes sending socket information using the environment and then switching
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
//!        .add_worker(Task::new(worker_cmd).daemon(true))
//!        .add_worker(Task::new(worker_cmd).daemon(true))
//!        .build();
//!    supervisor.run().await?;
//!    // Block the process here and do your work.
//!    Ok(())
//! }
//! ```
//!
//! ## Worker
//!
//! Worker reads the socket information from the environment and then connects to the Unix socket.
//!
//! ```ignore
//! use psup::{Result, worker};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Read supervisor information from the environment 
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

/// Enumeration of errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Worker is missing the id environment variable.
    #[error("Worker PSUP_WORKER_ID variable is not set")]
    WorkerNoId,

    /// Worker is missing the socket path environment variable.
    #[error("Worker PSUP_SOCKET variable is not set")]
    WorkerNoSocket,

    /// Worker is missing the detached flag environment variable.
    #[error("Worker PSUP_DETACHED variable is not set")]
    WorkerNoDetached,

    /// Input/output errors.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Error whilst sending bind notifications.
    #[error(transparent)]
    Oneshot(#[from] tokio::sync::oneshot::error::RecvError),

    /// Generic variant for errors created in user code.
    #[error(transparent)]
    Boxed(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Helper function to `Box` an error implementation.
    ///
    /// Ssocket handlers can call `map_err(Error::boxed)?` to propagate
    /// foreign errors.
    pub fn boxed(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        let err: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
        Error::from(err)
    }
}

/// Result type returned by the library.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) const WORKER_ID: &str = "PSUP_WORKER_ID";
pub(crate) const SOCKET: &str = "PSUP_SOCKET";
pub(crate) const DETACHED: &str = "PSUP_DETACHED";

mod supervisor;
mod worker;

pub use supervisor::{Supervisor, SupervisorBuilder, Task};
pub use worker::Worker;

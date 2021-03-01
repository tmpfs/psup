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
//! ```no_run
//! use psup_impl::{Error, Result, Task, SupervisorBuilder};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!    let worker_cmd = "worker-process";
//!    let supervisor = SupervisorBuilder::new()
//!        .server(|stream, tx| {
//!             let (reader, mut writer) = stream.into_split();
//!             tokio::task::spawn(async move {
//!                 // Handle worker connection here
//!                 // Use the `tx` supervisor control channel
//!                 // to spawn and shutdown workers
//!                 Ok::<(), Error>(())
//!             });
//!        })
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
//! ```no_run
//! use psup_impl::{Error, Result, Worker};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Read supervisor information from the environment
//!     // and set up the IPC channel with the supervisor
//!     let worker = Worker::new()
//!         .client(|stream, id| async {
//!             let (reader, mut writer) = stream.into_split();
//!             // Start sending messages to the supervisor
//!             Ok::<(), Error>(())
//!         });
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

/// Result type returned by the library.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) const WORKER_ID: &str = "PSUP_WORKER_ID";
pub(crate) const SOCKET: &str = "PSUP_SOCKET";

mod supervisor;
mod worker;

pub use supervisor::{Message, Supervisor, SupervisorBuilder, Task};
pub use worker::Worker;

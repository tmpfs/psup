//! Worker is a process performing a long-running task.
use futures::Future;
use tokio::net::UnixStream;

use super::{Error, Result, SOCKET, WORKER_ID};

/// Worker process handler.
pub struct Worker<H, F>
where
    H: Fn(UnixStream, String) -> F,
    F: Future<Output = Result<()>>,
{
    handler: H,
}

impl<H, F> Worker<H, F>
where
    H: Fn(UnixStream, String) -> F,
    F: Future<Output = Result<()>>,
{
    /// Create a new worker.
    pub fn new(handler: H) -> Self {
        Self { handler }
    }

    /// Start this working running.
    pub async fn run(&self) -> Result<()> {
        // Read worker information from the environment.
        let id = std::env::var(WORKER_ID)
            .or_else(|_| Err(Error::WorkerNoId))?
            .to_string();

        // If we were given a socket path make the connection.
        if let Some(path) = std::env::var(SOCKET).ok() {
            // Connect to the supervisor socket
            let stream = UnixStream::connect(&path).await?;
            (self.handler)(stream, id).await?;
        }

        Ok(())
    }
}

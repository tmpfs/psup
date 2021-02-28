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
    handler: Option<H>,
}

impl<H, F> Worker<H, F>
where
    H: Fn(UnixStream, String) -> F,
    F: Future<Output = Result<()>>,
{
    /// Create a new worker.
    pub fn new() -> Self {
        Self { handler: None }
    }

    /// Set a client connection handler.
    ///
    /// The handler function receives the socket stream and opaque 
    /// worker identifier and can communicate with the supervisor using 
    /// the socket stream.
    pub fn client(mut self, handler: H) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Start this worker running.
    pub async fn run(&self) -> Result<()> {
        if let Some(ref handler) = self.handler {
            // Read worker information from the environment.
            let id = std::env::var(WORKER_ID)
                .or_else(|_| Err(Error::WorkerNoId))?
                .to_string();

            // If we were given a socket path make the connection.
            if let Some(path) = std::env::var(SOCKET).ok() {
                // Connect to the supervisor socket
                let stream = UnixStream::connect(&path).await?;
                (handler)(stream, id).await?;
            }
        }
        Ok(())
    }
}

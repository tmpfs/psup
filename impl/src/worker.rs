//! Worker is a process performing a long-running task.
use super::{Error, Result, SOCKET, WORKER_ID};
use futures::Future;
use tokio::net::UnixStream;

/// Worker process handler.
pub struct Worker<H, F>
where
    H: Fn(UnixStream, String) -> F,
    F: Future<Output = Result<()>>,
{
    handler: Option<H>,
    relaxed: bool,
}

impl<H, F> Worker<H, F>
where
    H: Fn(UnixStream, String) -> F,
    F: Future<Output = Result<()>>,
{
    /// Create a new worker.
    pub fn new() -> Self {
        Self { handler: None, relaxed: false }
    }

    /// Set the relaxed flag.
    ///
    /// When a worker is relaxed it will start with or without a supervisor.
    ///
    /// The default is `false` so workers expect to be run in the context 
    /// of a supervisor and it is an error if no worker id is available in 
    /// the environment.
    ///
    /// When this flag is enabled and the required environment variables
    /// do not exist the worker does not attempt to connect to a supervisor.
    ///
    /// Use this mode of operation when a worker process can be run standalone 
    /// or as a worker for a supervisor process.
    pub fn relaxed(mut self, flag: bool) -> Self {
        self.relaxed = flag; 
        self
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
            if let Some(id) = std::env::var(WORKER_ID).ok() {
                // If we were given a socket path make the connection.
                if let Some(path) = std::env::var(SOCKET).ok() {
                    // Connect to the supervisor socket
                    let stream = UnixStream::connect(&path).await?;
                    (handler)(stream, id.to_string()).await?;
                }
            } else {
                if !self.relaxed {
                    return Err(Error::WorkerNoId);
                } 
            }
        }
        Ok(())
    }
}

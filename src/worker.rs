//! Worker is a process performing a long-running task.
use futures::Future;
use std::path::PathBuf;
use tokio::net::UnixStream;

use super::{Error, Result, WorkerInfo, SOCKET, WORKER_ID};

/// Worker process handler.
pub struct Worker<H, F>
where
    H: Fn(UnixStream, WorkerInfo) -> F,
    F: Future<Output = Result<()>>,
{
    handler: H,
}

impl<H, F> Worker<H, F>
where
    H: Fn(UnixStream, WorkerInfo) -> F,
    F: Future<Output = Result<()>>,
{
    /// Create a new worker.
    pub fn new(handler: H) -> Self {
        Self { handler }
    }

    /// Start this working running.
    pub async fn run(&self) -> Result<()> {
        let mut info = self.read()?;
        if let Some(connect) = info.take() {
            self.connect(connect).await?;
        }
        Ok(())
    }

    /// Read worker information from the environment.
    fn read(&self) -> Result<Option<WorkerInfo>> {
        let id = std::env::var(WORKER_ID)
            .or_else(|_| Err(Error::WorkerNoId))?
            .to_string();
        let path = PathBuf::from(
            std::env::var(SOCKET).or_else(|_| Err(Error::WorkerNoSocket))?,
        );
        Ok(Some(WorkerInfo { id, path }))
    }

    /// Connect to the supervisor socket.
    async fn connect(&self, info: WorkerInfo) -> Result<()> {
        let stream = UnixStream::connect(&info.path).await?;
        (self.handler)(stream, info).await?;
        Ok(())
    }
}

//! Worker is a process performing a long-running task.
use std::io::{self, BufRead};
use futures::Future;
use tokio::net::UnixStream;
use super::{WorkerInfo, Result};

//pub(crate) const SHUTDOWN: &str = "shutdown";

/// Worker process handler.
pub struct Worker<H, F> 
    where
        H: Fn(UnixStream, WorkerInfo) -> F,
        F: Future<Output = Result<()>> {

    handler: H,
}

impl<H, F> Worker<H, F>
    where
        H: Fn(UnixStream, WorkerInfo) -> F,
        F: Future<Output = Result<()>> {

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

    /// Read worker information from stdin.
    fn read(&self) -> Result<Option<WorkerInfo>> {
        let stdin = io::stdin();
        let mut info = None;
        for line in stdin.lock().lines() {
            if let Some(line) = line.ok() {
                info = Some(serde_json::from_str::<WorkerInfo>(&line).unwrap());
                break;
            }
        }
        Ok(info)
    }

    /// Connect to the supervisor socket.
    async fn connect(&self, info: WorkerInfo) -> Result<()> {
        let stream = UnixStream::connect(&info.path).await?;
        (self.handler)(stream, info).await?;
        Ok(())
    }
}


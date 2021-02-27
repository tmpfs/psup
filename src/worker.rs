//! Worker is a process performing a long-running task.
use std::path::PathBuf;
use std::io::{self, BufRead};
use futures::Future;
use tokio::net::UnixStream;
use super::{WorkerInfo, Result};

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
        let mut id = None;
        let mut path = None;
        for line in stdin.lock().lines() {
            if let Some(line) = line.ok() {
                if id.is_none() {
                    id = Some(line);
                } else {
                    path = Some(line);
                }
            }

            if id.is_some() && path.is_some() {
                break;
            }
        }
        Ok(Some(WorkerInfo {id: id.unwrap(), path: PathBuf::from(path.unwrap())}))
    }

    /// Connect to the supervisor socket.
    async fn connect(&self, info: WorkerInfo) -> Result<()> {
        let stream = UnixStream::connect(&info.path).await?;
        (self.handler)(stream, info).await?;
        Ok(())
    }
}


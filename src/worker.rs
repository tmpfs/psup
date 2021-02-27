//! Worker is a process performing a long-running task.
use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};
use std::io::{self, BufRead};
use std::sync::Mutex;

use futures::StreamExt;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};

use async_trait::async_trait;
use log::{debug, error, info};
use once_cell::sync::OnceCell;

use super::{Connected, WorkerInfo, Message, Result, CONNECTED};

pub(crate) const SHUTDOWN: &str = "shutdown";

fn worker_state() -> &'static Mutex<WorkerState> {
    static INSTANCE: OnceCell<Mutex<WorkerState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(WorkerState { id: 0 }))
}

/// Worker process handler.
pub struct Worker;

impl Worker {
    /// Create a new worker.
    pub fn new() -> Self {
        Self {}
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
                info = Some(serde_json::from_str::<Request>(&line).unwrap());
                break;
            }
        }
        Ok(if let Some(mut info) = info.take() {
            Some(info.deserialize::<WorkerInfo>().unwrap())
        } else {
            None
        })
    }

    /// Connect to the supervisor socket.
    async fn connect(&self, info: WorkerInfo) -> Result<()> {
        let stream = UnixStream::connect(info.socket_path).await?;

        let (reader, mut writer) = stream.into_split();

        // Send a notification to the supervisor so that it knows
        // this worker is ready
        let params = serde_json::to_value(Connected { id: info.id })?;
        let req =
            Message::Request(Request::new_notification(CONNECTED, Some(params)));
        let msg = format!("{}\n", serde_json::to_string(&req)?);
        writer.write_all(msg.as_bytes()).await?;
        //writer.write_all(b"{\n").await?;

        let mut lines = FramedRead::new(reader, LinesCodec::new());
        let service: Box<dyn Service<Data = Mutex<WorkerState>>> =
            Box::new(WorkerService {});
        let server = Server::new(vec![&service]);
        while let Some(line) = lines.next().await {
            let line = line?;
            println!("Got worker line {:?}", line);
            match serde_json::from_str::<Message>(&line)? {
                Message::Request(mut req) => {
                    debug!("{:?}", req);
                    let res = server.serve(&mut req, worker_state()).await;
                    debug!("{:?}", res);
                    if let Some(response) = res {
                        let msg = Message::Response(response);
                        writer
                            .write_all(
                                format!("{}\n", serde_json::to_string(&msg)?)
                                    .as_bytes(),
                            )
                            .await?;
                    }
                }
                Message::Response(reply) => {
                    // Currently not handling RPC replies so just log them
                    if let Some(err) = reply.error() {
                        error!("{:?}", err);
                    } else {
                        debug!("{:?}", reply);
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct WorkerState {
    id: usize,
}

struct WorkerService;

#[async_trait]
impl Service for WorkerService {
    type Data = Mutex<WorkerState>;
    async fn handle(
        &self,
        req: &mut Request,
        ctx: &Self::Data,
    ) -> json_rpc2::Result<Option<Response>> {
        let mut response = None;
        if req.matches(CONNECTED) {
            let state = ctx.lock().unwrap();
            info!("Child service connected {}", state.id);
            response = Some(req.into());
        } else if req.matches(SHUTDOWN) {
            info!("Child going down now.");
            response = Some(req.into());
        }
        Ok(response)
    }
}


use futures::stream::StreamExt;
use psup_impl::{
    Error, Message as ControlMessage, Result, SupervisorBuilder, Task,
};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{FramedRead, LinesCodec};

use log::{debug, error, info};

use json_rpc2::{Request, Response};

/// Encodes whether an IPC message is a request or
/// a response so that we can do bi-directional
/// communication over the same socket.
#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    /// RPC request message.
    #[serde(rename = "request")]
    Request(Request),
    /// RPC response message.
    #[serde(rename = "response")]
    Response(Response),
}

/// Message sent to the supervisor when a worker
/// is spawned and has connected to the IPC socket.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Connected {
    /// Worker identifier.
    pub id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").ok().is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let worker_cmd = "cargo";
    let args = vec!["run", "--example", "worker"];
    let supervisor = SupervisorBuilder::new()
        .server(|stream, tx| {
            let (reader, _writer) = stream.into_split();
            tokio::task::spawn(async move {
                let mut lines = FramedRead::new(reader, LinesCodec::new());
                while let Some(line) = lines.next().await {
                    let line = line.map_err(Error::boxed)?;
                    match serde_json::from_str::<Message>(&line)
                        .map_err(Error::boxed)?
                    {
                        Message::Request(mut req) => {
                            info!("{:?}", req);

                            // Demonstrate shutting down a worker process
                            // over the supervisor control channel.
                            let info: Connected = req.deserialize().unwrap();
                            info!("Send shutdown signal with id {:?}", info.id);
                            let _ = tx
                                .send(ControlMessage::Shutdown { id: info.id })
                                .await;
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
                Ok::<(), Error>(())
            });
        })
        .path(std::env::temp_dir().join("supervisor.sock"))
        .add_worker(Task::new(worker_cmd).args(args.clone()).daemon(true))
        .build();
    supervisor.run().await?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(60))
    }
}

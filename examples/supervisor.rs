use psup::{Error, Result, SupervisorBuilder, Message};
use tokio::io::AsyncWriteExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

use log::{info, error, debug};

use async_trait::async_trait;
use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};

/// Message sent to the supervisor when a worker
/// is spawned and has connected to the IPC socket.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Connected {
    /// Worker identifier.
    pub id: String,
}

struct SupervisorService;

#[async_trait]
impl Service for SupervisorService {
    type Data = ();
    async fn handle(
        &self,
        req: &mut Request,
        _ctx: &Self::Data,
    ) -> json_rpc2::Result<Option<Response>> {
        let mut response = None;
        if req.matches("connected") {
            let info: Connected = req.deserialize()?;
            info!("Worker connected {:?}", info);
            // Send ACK to the client in case it asked for a reply
            response = Some(req.into());
        }
        Ok(response)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").ok().is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let worker_cmd = "cargo";
    let args = vec!["run", "--example", "worker", "--all-features"];
    let supervisor = SupervisorBuilder::new(Box::new(|stream| {
            let (reader, mut writer) = stream.into_split();
            tokio::task::spawn(async move {
                let service: Box<
                    dyn Service<Data = ()>,
                > = Box::new(SupervisorService {});
                let server = Server::new(vec![&service]);

                let mut lines = FramedRead::new(reader, LinesCodec::new());
                while let Some(line) = lines.next().await {
                    let line = line?;
                    //log::info!("Supervisor got line {}", line);
                    match serde_json::from_str::<Message>(&line)? {
                        Message::Request(mut req) => {
                            debug!("{:?}", req);
                            let res = server
                                .serve(&mut req, &())
                                .await;
                            debug!("{:?}", res);
                            if let Some(response) = res {
                                let msg = Message::Response(response);
                                writer
                                    .write_all(
                                        format!(
                                            "{}\n",
                                            serde_json::to_string(&msg)?
                                        )
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
                Ok::<(), Error>(())
            });
        }))
        .path(std::env::temp_dir().join("supervisor.sock"))
        .add_daemon(worker_cmd.to_string(), args.clone())
        .add_daemon(worker_cmd.to_string(), args.clone())
        .build();
    supervisor.run().await?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(60))
    }
}

use futures::stream::StreamExt;
use json_rpc2::{futures::Server, Request, Response};
use log::{debug, error, info};
use psup_impl::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio_util::codec::{FramedRead, LinesCodec};

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
pub struct Connected {
    /// Worker identifier.
    pub id: String,
}

/// Read and write line-delimited JSON from a stream executing
/// via a JSON RPC server.
pub async fn serve<R, W, S>(
    reader: ReadHalf<R>,
    mut writer: WriteHalf<W>,
    server: Server<'_, S>,
    state: &S,
) -> Result<()>
where
    R: AsyncRead,
    W: AsyncWrite,
    S: Send + Sync,
{
    let mut lines = FramedRead::new(reader, LinesCodec::new());
    while let Some(line) = lines.next().await {
        let line = line.map_err(Error::boxed)?;
        match serde_json::from_str::<Message>(&line).map_err(Error::boxed)? {
            Message::Request(mut req) => {
                info!("{:?}", req);
                let res = server.serve(&mut req, state).await;
                debug!("{:?}", res);
                if let Some(response) = res {
                    let msg = Message::Response(response);
                    writer
                        .write_all(
                            format!(
                                "{}\n",
                                serde_json::to_string(&msg)
                                    .map_err(Error::boxed)?
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
    Ok(())
}

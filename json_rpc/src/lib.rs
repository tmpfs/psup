//! Adapter for the process supervisor (psup) to serve JSON-RPC over a split socket.
#![deny(missing_docs)]
use futures::stream::StreamExt;
use json_rpc2::{futures::Server, Request, Response};
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
///
/// Request and response functions can be given for logging.
pub async fn serve<S, R, W, I, O, A>(
    server: Server<'_, S>,
    reader: ReadHalf<R>,
    mut writer: WriteHalf<W>,
    state: &S,
    request: I,
    response: O,
    answer: A,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    R: AsyncRead,
    W: AsyncWrite,
    S: Send + Sync,
    I: Fn(&Request),
    O: Fn(&Response),
    A: Fn(Response),
{
    let mut lines = FramedRead::new(reader, LinesCodec::new());
    while let Some(line) = lines.next().await {
        let line = line.map_err(Box::new)?;
        match serde_json::from_str::<Message>(&line).map_err(Box::new)? {
            Message::Request(mut req) => {
                (request)(&req);
                let res = server.serve(&mut req, state).await;
                if let Some(res) = res {
                    (response)(&res);
                    let msg = Message::Response(res);
                    writer
                        .write(
                            serde_json::to_vec(&msg)
                                .map_err(Box::new)?
                                .as_slice(),
                        )
                        .await?;
                    writer.write(b"\n").await?;
                    writer.flush().await?;
                }
            }
            Message::Response(reply) => {
                (answer)(reply);
                //reply.foo();
                // Currently not handling RPC replies so just log them
                /*
                if let Some(err) = reply.error() {
                    error!("{:?}", err);
                } else {
                    debug!("{:?}", reply);
                }
                */
            }
        }
    }
    Ok(())
}

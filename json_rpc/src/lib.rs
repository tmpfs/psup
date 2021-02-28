//! Adapter for the process supervisor (psup) to serve JSON-RPC over a split socket.
#![deny(missing_docs)]
use futures::{Future, stream::StreamExt};
use json_rpc2::{futures::Server, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio_util::codec::{FramedRead, LinesCodec};

/// Generic error type.
type Error = Box<dyn std::error::Error + Send + Sync>;

/// Result type.
type Result<T> = std::result::Result<T, Error>;

/// Encodes whether a packet is a request or
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

/// Prepare a JSON RPC method call wrapped as a [Message](crate::Message).
pub fn call(method: &str, params: Option<Value>) -> Message {
    Message::Request(Request::new_reply(method, params))
}

/// Prepare a JSON RPC notification wrapped as a [Message](crate::Message).
pub fn notify(method: &str, params: Option<Value>) -> Message {
    Message::Request(Request::new_notification(method, params))
}

/// Write a message to the writer as a JSON encoded line.
pub async fn write<W>(
    writer: &mut W,
    msg: &Message,
) -> Result<()> 
    where W: AsyncWrite + Unpin {
    writer
        .write(
            serde_json::to_vec(msg)
                .map_err(Box::new)?
                .as_slice(),
        )
        .await?;
    writer.write(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

/// Read and write line-delimited JSON from a stream executing
/// via a JSON RPC server.
///
/// Request and response functions can be given for logging.
pub async fn serve<S, R, W, I, O, A>(
    server: Server<'_, S>,
    state: &S,
    reader: ReadHalf<R>,
    mut writer: WriteHalf<W>,
    request: I,
    response: O,
    answer: A,
) -> Result<()>
where
    R: AsyncRead,
    W: AsyncWrite,
    S: Send + Sync,
    I: Fn(&Request),
    O: Fn(&Response),
    A: Fn(Response, &mut WriteHalf<W>),
    //F: Future<Output = Result<()>>,
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
                    write(&mut writer, &msg).await?;
                }
            }
            Message::Response(reply) => {
                //(answer)(reply, &mut writer).await?;
                (answer)(reply, &mut writer);
            }
        }
    }
    Ok(())
}

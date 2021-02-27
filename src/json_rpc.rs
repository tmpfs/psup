use serde::{Deserialize, Serialize};
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


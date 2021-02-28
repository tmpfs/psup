use std::sync::Mutex;

use psup_impl::{Error, Result, Worker};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};

use psup_json_rpc::{self, Message};

use once_cell::sync::OnceCell;

use async_trait::async_trait;
use log::{info};

fn worker_state() -> &'static Mutex<WorkerState> {
    static INSTANCE: OnceCell<Mutex<WorkerState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(WorkerState { id: 0 }))
}

/// Message sent to the supervisor when a worker
/// is spawned and has connected to the IPC socket.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Connected {
    /// Worker identifier.
    pub id: String,
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
        if req.matches("conected") {
            let state = ctx.lock().unwrap();
            info!("Child service connected {}", state.id);
            response = Some(req.into());

            /*
            } else if req.matches(SHUTDOWN) {
                info!("Child going down now.");
                response = Some(req.into());
            */
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

    let worker = Worker::new().client(|stream, id| async {
        let (reader, mut writer) = tokio::io::split(stream);
        // Send a notification to the supervisor so that it knows
        // this worker is ready
        let params =
            serde_json::to_value(Connected { id }).map_err(Error::boxed)?;
        let req = Message::Request(Request::new_notification(
            "connected",
            Some(params),
        ));
        //let req =
            //Message::Request(Request::new_reply("connected", Some(params)));
        let msg =
            format!("{}\n", serde_json::to_string(&req).map_err(Error::boxed)?);
        writer.write_all(msg.as_bytes()).await?;

        //let mut lines = FramedRead::new(reader, LinesCodec::new());
        let service: Box<dyn Service<Data = Mutex<WorkerState>>> =
            Box::new(WorkerService {});
        let server = Server::new(vec![&service]);
        psup_json_rpc::serve::<_, _, Mutex<WorkerState>>(
            reader,
            writer,
            server,
            worker_state(),
        )
        .await?;
        Ok::<(), Error>(())
    });
    worker.run().await?;

    // Simulate blocking this process
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60))
    }
}

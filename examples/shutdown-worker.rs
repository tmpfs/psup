use std::sync::Mutex;

use once_cell::sync::OnceCell;
use async_trait::async_trait;
use log::info;

use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};

use psup_impl::{Error, Result, Worker};
use psup_json_rpc::{serve, call, notify, write, Identity};

fn worker_state(id: Option<String>) -> &'static Mutex<WorkerState> {
    static INSTANCE: OnceCell<Mutex<WorkerState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(WorkerState { id: id.unwrap_or(Default::default()) }))
}

#[derive(Debug)]
struct WorkerState {
    id: String,
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

        worker_state(Some(id.to_string()));

        let params =
            serde_json::to_value(Identity { id }).map_err(Error::boxed)?;
        // Use `call()` so we get a reply from the server.
        let req = call("connected", Some(params));
        write(&mut writer, &req).await?;

        //let mut lines = FramedRead::new(reader, LinesCodec::new());
        let service: Box<dyn Service<Data = Mutex<WorkerState>>> =
            Box::new(WorkerService {});
        let server = Server::new(vec![&service]);
        serve::<Mutex<WorkerState>, _, _, _, _, _>(
            server,
            worker_state(None),
            reader,
            writer,
            |req| info!("{:?}", req),
            |res| info!("{:?}", res),
            |reply| {
                info!("{:?}", reply);
                // Receive the answer to the `connected` message
                // and send a `shutdown` notification
                let state = worker_state(None).lock().unwrap();
                let id = state.id.clone();
                let params = serde_json::to_value(Identity { id }).map_err(Box::new)?;
                let req = notify("shutdown", Some(params));
                Ok(Some(req))
            },
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

use std::sync::Mutex;

use once_cell::sync::OnceCell;
use async_trait::async_trait;
use log::info;

use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};

use psup_impl::{Error, Result, Worker};
use psup_json_rpc::{serve, notify, write, Connected};

fn worker_state() -> &'static Mutex<WorkerState> {
    static INSTANCE: OnceCell<Mutex<WorkerState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(WorkerState { id: 0 }))
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
        _req: &mut Request,
        _ctx: &Self::Data,
    ) -> json_rpc2::Result<Option<Response>> {
        Ok(None)
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
        // this worker is ready, if we wanted a reply we can build 
        // the message using `psup_json_rpc::call()`.
        let params =
            serde_json::to_value(Connected { id }).map_err(Error::boxed)?;
        let req = notify("connected", Some(params));
        write(&mut writer, &req).await?;

        //let mut lines = FramedRead::new(reader, LinesCodec::new());
        let service: Box<dyn Service<Data = Mutex<WorkerState>>> =
            Box::new(WorkerService {});
        let server = Server::new(vec![&service]);
        serve::<Mutex<WorkerState>, _, _, _, _, _>(
            server,
            worker_state(),
            reader,
            writer,
            |req| info!("{:?}", req),
            |res| info!("{:?}", res),
            |reply| {
                info!("{:?}", reply);
                Ok(None)
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

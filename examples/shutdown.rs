// 1) Supervisor creates a socket server and spawns a worker
// 2) Worker sends a `connected` RPC messsage which expects a reply
// 3) Server sends an ACK reply to the `connected` message
// 4) Worker receives the answer and sends a `shutdown` message
// 5) Shutdown server notifies the supervisor via the control channel
// 6) Supervisor kills the worker process
// 7) Supervisor process will block, no more workers!
use tokio::sync::mpsc;
use async_trait::async_trait;
use log::info;

use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};

use psup_impl::{
    Error, Result, SupervisorBuilder, Task, Message,
};
use psup_json_rpc::{serve, Connected};

struct ShutdownService;

#[async_trait]
impl Service for ShutdownService {
    type Data = mpsc::Sender<Message>;
    async fn handle(
        &self,
        req: &mut Request,
        ctx: &Self::Data,
    ) -> json_rpc2::Result<Option<Response>> {
        let mut response = None;
        if req.matches("connected") {
            let info: Connected = req.deserialize()?;
            info!("{:?}", info);
            // Send ACK to the client in case it asked for a reply
            response = Some(req.into());
        } else if req.matches("shutdown") {
            let info: Connected = req.deserialize()?;
            info!("Terminating worker {:?}", info);
            let _ = ctx
                .send(Message::Shutdown { id: info.id })
                .await;
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
    let args = vec!["run", "--example", "shutdown-worker"];
    let supervisor = SupervisorBuilder::new()
        .server(|stream, tx| {
            let (reader, writer) = tokio::io::split(stream);
            tokio::task::spawn(async move {

                //tx.foo();

                let service: Box<dyn Service<Data = mpsc::Sender<Message>>> =
                    Box::new(ShutdownService {});
                let server = Server::new(vec![&service]);
                serve::<mpsc::Sender<Message>, _, _, _, _, _>(
                    server,
                    &tx,
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
        })
        .path(std::env::temp_dir().join("supervisor.sock"))
        .add_worker(Task::new(worker_cmd).args(args.clone()).daemon(true))
        .build();
    supervisor.run().await?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(60))
    }
}

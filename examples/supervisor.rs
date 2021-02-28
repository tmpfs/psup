use log::info;
use async_trait::async_trait;

use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};

use psup_impl::{Error, Result, SupervisorBuilder, Task};
use psup_json_rpc::{serve, Connected};

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
            info!("{:?}", info);
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
    let args = vec!["run", "--example", "worker"];
    let supervisor = SupervisorBuilder::new()
        .server(|stream, _tx| {
            let (reader, writer) = tokio::io::split(stream);
            tokio::task::spawn(async move {
                let service: Box<dyn Service<Data = ()>> =
                    Box::new(SupervisorService {});
                let server = Server::new(vec![&service]);
                serve::<(), _, _, _, _, _>(
                    server,
                    &(),
                    reader,
                    writer,
                    |req| info!("{:?}", req),
                    |res| info!("{:?}", res),
                    |reply, _| {
                        info!("{:?}", reply);
                        //Ok(())
                    },
                )
                .await?;
                Ok::<(), Error>(())
            });
        })
        .path(std::env::temp_dir().join("supervisor.sock"))
        .add_worker(Task::new(worker_cmd).args(args.clone()).daemon(true))
        .add_worker(Task::new(worker_cmd).args(args.clone()).daemon(true))
        .build();
    supervisor.run().await?;

    // Implicit worker creation after starting the IPC server.
    //supervisor.spawn(Task::new("cat").daemon(true));

    loop {
        std::thread::sleep(std::time::Duration::from_secs(60))
    }
}

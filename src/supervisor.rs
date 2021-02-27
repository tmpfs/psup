//! Supervisor manages a collection of worker processes.
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;
use tokio::sync::oneshot::{self, Sender};
use tokio_util::codec::{FramedRead, LinesCodec};

use futures::StreamExt;

use async_trait::async_trait;
use json_rpc2::{
    futures::{Server, Service},
    Request, Response,
};
use log::{debug, error, info, warn};
use once_cell::sync::OnceCell;
use rand::Rng;

use super::{Connected, Error, WorkerInfo, Message, Result, CONNECTED};

fn supervisor_state() -> &'static Mutex<SupervisorState> {
    static INSTANCE: OnceCell<Mutex<SupervisorState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(SupervisorState { workers: vec![] }))
}

/// Build a supervisor.
#[derive(Debug)]
pub struct SupervisorBuilder {
    socket: PathBuf,
    commands: Vec<(String, Vec<String>, bool)>,
}

impl SupervisorBuilder {
    /// Create a new supervisor builder.
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            commands: Vec::new(),
        }
    }

    /// Add a worker process marked as a daemon.
    ///
    /// Daemon worker processes are restarted if they exit without being 
    /// explicitly shutdown by the supervisor.
    pub fn add_daemon(mut self, cmd: String, args: Vec<String>) -> Self {
        self.commands.push((cmd, args, true));
        self
    }

    /// Add a worker process.
    pub fn add_worker(mut self, cmd: String, args: Vec<String>) -> Self {
        self.commands.push((cmd, args, false));
        self
    }

    /// Return the supervisor.
    pub fn build(self) -> Supervisor {
        Supervisor {
            socket: self.socket,
            commands: self.commands,
        }
    }
}

/// Supervisor manages long-running worker processes.
#[derive(Debug)]
pub struct Supervisor {
    socket: PathBuf,
    commands: Vec<(String, Vec<String>, bool)>,
}

impl Supervisor {
    /// Start the supervisor running.
    ///
    /// Listens on the socket path and starts any initial workers.
    pub async fn run(&self) -> Result<()> {
        let socket_path = self.socket.clone();
        let (tx, rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            listen(&socket_path, tx)
                .await
                .expect("Supervisor failed to bind to socket");
        });

        let _ = rx.await?;
        info!("Supervisor is listening {}", self.socket.display());

        for (cmd, args, daemon) in self.commands.iter() {
            spawn_worker(cmd.to_string(), args.clone(), daemon.clone(), self.socket.clone());
        }

        Ok(())
    }
}

struct SupervisorState {
    workers: Vec<Worker>,
}

impl SupervisorState {
    fn find(&mut self, id: &str) -> Option<&mut Worker> {
        self.workers.iter_mut().find(|w| w.id == id)
    }

    fn remove(&mut self, pid: u32) -> Option<Worker> {
        let res = self.workers.iter().enumerate().find_map(|(i, w)| {
            if w.pid == pid {
                Some(i)
            } else {
                None
            }
        });
        if let Some(position) = res {
            Some(self.workers.swap_remove(position))
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct Worker {
    cmd: String,
    args: Vec<String>,
    id: String,
    socket_path: PathBuf,
    pid: u32,
    daemon: bool,
    /// If we are shutting down this worker explicitly
    /// this flag will be set to prevent the worker from
    /// being re-spawned.
    reap: bool,
    // Flag set when a child sends the connected message
    connected: bool,
}

impl PartialEq for Worker {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.pid == other.pid
    }
}

impl Eq for Worker {}

struct SupervisorService;

#[async_trait]
impl Service for SupervisorService {
    type Data = Mutex<SupervisorState>;
    async fn handle(
        &self,
        req: &mut Request,
        ctx: &Self::Data,
    ) -> json_rpc2::Result<Option<Response>> {
        let mut response = None;
        if req.matches(CONNECTED) {
            let info: Connected = req.deserialize()?;
            info!("Worker connected {:?}", info);
            let mut state = ctx.lock().unwrap();
            let worker = state.find(&info.id);
            if let Some(worker) = worker {
                worker.connected = true;
                response = Some(req.into());
            } else {
                let err =
                    json_rpc2::Error::boxed(Error::WorkerNotFound(info.id));
                response = Some((req, err).into())
            }
        }
        Ok(response)
    }
}

/// Attempt to restart a worker that died.
fn restart(worker: Worker) {
    info!("Restarting worker {}", worker.id);
    // TODO: retry on fail with backoff and retry limit
    spawn_worker(worker.cmd, worker.args, worker.daemon, worker.socket_path)
}

fn spawn_worker(cmd: String, args: Vec<String>, daemon: bool, socket_path: PathBuf) {
    // Generate a unique id for each worker
    let mut rng = rand::thread_rng();
    let mut hasher = DefaultHasher::new();
    hasher.write_usize(rng.gen());
    let id = format!("{:x}", hasher.finish());

    let worker_socket = socket_path.clone();
    let worker_cmd = cmd.clone();
    let worker_args = args.clone();
    let worker_id = id.clone();

    thread::spawn(move || {
        let mut child = Command::new(worker_cmd)
            .args(worker_args)
            .stdin(Stdio::piped())
            .spawn()?;

        let child_stdin = child.stdin.as_mut().unwrap();
        let connect_params = WorkerInfo {
            socket_path: worker_socket,
            id: worker_id,
        };
        let req = Request::new_notification(
            "connect",
            serde_json::to_value(connect_params).ok(),
        );
        child_stdin.write_all(
            format!("{}\n", serde_json::to_string(&req).unwrap()).as_bytes(),
        )?;

        drop(child_stdin);

        let pid = child.id();
        {
            let worker = Worker {
                cmd,
                args,
                id,
                daemon,
                socket_path,
                pid,
                reap: false,
                connected: false,
            };
            let mut state = supervisor_state().lock().unwrap();
            state.workers.push(worker);
        }

        let _ = child.wait_with_output()?;

        warn!("Worker process died: {}", pid);

        let mut state = supervisor_state().lock().unwrap();
        let worker = state.remove(pid);
        drop(state);
        if let Some(worker) = worker {
            info!("Removed child worker (id: {}, pid {})", worker.id, pid);
            if !worker.reap && worker.daemon {
                restart(worker);
            }
        } else {
            error!("Failed to remove stale worker for pid {}", pid);
        }

        Ok::<(), io::Error>(())
    });
}

async fn listen<P: AsRef<Path>>(socket: P, tx: Sender<()>) -> Result<()> {
    let path = socket.as_ref();

    // If the socket file exists we must remove to prevent `EADDRINUSE`
    if path.exists() {
        std::fs::remove_file(path)?;
    }

    let listener = UnixListener::bind(socket).unwrap();
    tx.send(()).unwrap();

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let (reader, mut writer) = stream.into_split();
                tokio::task::spawn(async move {
                    let service: Box<
                        dyn Service<Data = Mutex<SupervisorState>>,
                    > = Box::new(SupervisorService {});
                    let server = Server::new(vec![&service]);
                    let mut lines = FramedRead::new(reader, LinesCodec::new());
                    while let Some(line) = lines.next().await {
                        let line = line?;
                        match serde_json::from_str::<Message>(&line)? {
                            Message::Request(mut req) => {
                                debug!("{:?}", req);
                                let res = server
                                    .serve(&mut req, supervisor_state())
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
            }
            Err(e) => {
                warn!("Supervisor failed to accept worker socket {}", e);
            }
        }
    }
}

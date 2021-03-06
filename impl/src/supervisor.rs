//! Supervisor manages a collection of worker processes.
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::Hasher,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio::{
    net::{UnixListener, UnixStream},
    process::Command,
    sync::oneshot::{self, Sender},
    sync::{mpsc, Mutex},
    time,
};

use log::{error, info, warn};
use once_cell::sync::OnceCell;
use rand::Rng;

use super::{Result, SOCKET, WORKER_ID};

type IpcHandler = Box<dyn Fn(UnixStream, mpsc::Sender<Message>) + Send + Sync>;

/// Get the supervisor state.
fn supervisor_state() -> &'static Mutex<SupervisorState> {
    static INSTANCE: OnceCell<Mutex<SupervisorState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(SupervisorState { workers: vec![] }))
}

/// Control messages sent by the server handler to the
/// supervisor.
pub enum Message {
    /// Shutdown a worker process using it's opaque identifier.
    ///
    /// If the worker is a daemon it will *not be restarted*.
    Shutdown {
        /// Opaque identifier for the worker.
        id: String,
    },

    /// Spawn a new worker process.
    Spawn {
        /// Task definition for the new process.
        task: Task,
    },
}

/// Defines a worker process command.
#[derive(Debug, Clone)]
pub struct Task {
    cmd: String,
    args: Vec<String>,
    envs: HashMap<String, String>,
    daemon: bool,
    detached: bool,
    limit: usize,
    factor: usize,
}

impl Task {
    /// Create a new task.
    pub fn new(cmd: &str) -> Self {
        Self {
            cmd: cmd.to_string(),
            args: Vec::new(),
            envs: HashMap::new(),
            daemon: false,
            detached: false,
            limit: 5,
            factor: 0,
        }
    }

    /// Set command arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args = args
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect::<Vec<_>>();
        self.args = args;
        self
    }

    /// Set command environment variables.
    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let envs = vars
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
            .collect::<HashMap<_, _>>();
        self.envs = envs;
        self
    }

    /// Set the daemon flag for the worker command.
    ///
    /// Daemon processes are restarted if they die without being explicitly
    /// shutdown by the supervisor.
    pub fn daemon(mut self, flag: bool) -> Self {
        self.daemon = flag;
        self
    }

    /// Set the detached flag for the worker command.
    ///
    /// If a worker is detached it will not connect to the IPC socket.
    pub fn detached(mut self, flag: bool) -> Self {
        self.detached = flag;
        self
    }

    /// Set the retry limit when restarting dead workers.
    ///
    /// Only applies to tasks that have the `daemon` flag set;
    /// non-daemon tasks are not restarted. If this value is
    /// set to zero then it overrides the `daemon` flag and no
    /// attempts to restart the process are made.
    ///
    /// The default value is `5`.
    pub fn retry_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set the retry factor in milliseconds.
    ///
    /// The default value is `0` which means retry attempts
    /// are performed immediately.
    pub fn retry_factor(mut self, factor: usize) -> Self {
        self.factor = factor;
        self
    }

    /// Get a retry state for this task.
    fn retry(&self) -> Retry {
        Retry {
            limit: self.limit,
            factor: self.factor,
            attempts: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Retry {
    /// The limit on the number of times to attempt
    /// to restart a process.
    limit: usize,
    /// The retry delay factor in milliseconds.
    factor: usize,
    /// The current number of attempts.
    attempts: usize,
}

/// Build a supervisor.
pub struct SupervisorBuilder {
    socket: PathBuf,
    commands: Vec<Task>,
    ipc_handler: Option<IpcHandler>,
    shutdown: Option<oneshot::Receiver<()>>,
}

impl SupervisorBuilder {
    /// Create a new supervisor builder.
    pub fn new() -> Self {
        let socket = std::env::temp_dir().join("psup.sock");
        Self {
            socket,
            commands: Vec::new(),
            ipc_handler: None,
            shutdown: None,
        }
    }

    /// Set the IPC server handler.
    pub fn server<F: 'static>(mut self, handler: F) -> Self
    where
        F: Fn(UnixStream, mpsc::Sender<Message>) + Send + Sync,
    {
        self.ipc_handler = Some(Box::new(handler));
        self
    }

    /// Set the socket path.
    pub fn path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.socket = path.as_ref().to_path_buf();
        self
    }

    /// Add a worker process.
    pub fn add_worker(mut self, task: Task) -> Self {
        self.commands.push(task);
        self
    }

    /// Register a shutdown handler with the supervisor.
    ///
    /// When a message is received on the shutdown receiver all
    /// managed processes are killed.
    pub fn shutdown(mut self, rx: oneshot::Receiver<()>) -> Self {
        self.shutdown = Some(rx);
        self
    }

    /// Return the supervisor.
    pub fn build(self) -> Supervisor {
        Supervisor {
            socket: self.socket,
            commands: self.commands,
            ipc_handler: self.ipc_handler.map(Arc::new),
            shutdown: self.shutdown,
        }
    }
}

/// Supervisor manages long-running worker processes.
pub struct Supervisor {
    socket: PathBuf,
    commands: Vec<Task>,
    ipc_handler: Option<Arc<IpcHandler>>,
    shutdown: Option<oneshot::Receiver<()>>,
}

impl Supervisor {
    /// Start the supervisor running.
    ///
    /// Listens on the socket path and starts any initial workers.
    pub async fn run(&mut self) -> Result<()> {
        // Set up the server listener and control channel.
        if let Some(ref ipc_handler) = self.ipc_handler {
            let socket = self.socket.clone();
            let control_socket = self.socket.clone();

            let (control_tx, mut control_rx) = mpsc::channel::<Message>(1024);
            let (tx, rx) = oneshot::channel::<()>();
            let handler = Arc::clone(ipc_handler);

            // Handle global shutdown signal, kills all the workers
            if let Some(shutdown) = self.shutdown.take() {
                tokio::spawn(async move {
                    let _ = shutdown.await;
                    let mut state = supervisor_state().lock().await;
                    let workers = state.workers.drain(..);
                    for worker in workers {
                        let tx = worker.shutdown.clone();
                        let _ = tx.send(worker).await;
                    }
                });
            }

            tokio::spawn(async move {
                while let Some(msg) = control_rx.recv().await {
                    match msg {
                        Message::Shutdown { id } => {
                            let mut state = supervisor_state().lock().await;
                            let mut worker = state.remove(&id);
                            drop(state);
                            if let Some(worker) = worker.take() {
                                let tx = worker.shutdown.clone();
                                let _ = tx.send(worker).await;
                            } else {
                                warn!("Could not find worker to shutdown with id: {}", id);
                            }
                        }
                        Message::Spawn { task } => {
                            // FIXME: return the id to the caller?
                            let id = id();
                            let retry = task.retry();
                            spawn_worker(
                                id,
                                task,
                                control_socket.clone(),
                                retry,
                            );
                        }
                    }
                }
            });

            tokio::spawn(async move {
                listen(&socket, tx, handler, control_tx)
                    .await
                    .expect("Supervisor failed to bind to socket");
            });

            let _ = rx.await?;
            info!("Supervisor is listening {}", self.socket.display());
        }

        // Spawn initial worker processes.
        for task in self.commands.iter() {
            self.spawn(task.clone());
        }

        Ok(())
    }

    /// Spawn a worker task.
    pub fn spawn(&self, task: Task) -> String {
        let id = id();
        let retry = task.retry();
        spawn_worker(id.clone(), task, self.socket.clone(), retry);
        id
    }

    /*
    /// Get the workers mapped from opaque identifier to process PID.
    pub fn workers() -> HashMap<String, u32> {
        let state = supervisor_state().lock().unwrap();
        state.workers.iter()
            .map(|w| (w.id.clone(), w.pid))
            .collect::<HashMap<_, _>>()
    }
    */
}

/// State of the supervisor worker processes.
struct SupervisorState {
    workers: Vec<WorkerState>,
}

impl SupervisorState {
    fn remove(&mut self, id: &str) -> Option<WorkerState> {
        let res = self.workers.iter().enumerate().find_map(|(i, w)| {
            if &w.id == id {
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
struct WorkerState {
    task: Task,
    id: String,
    socket: PathBuf,
    pid: Option<u32>,
    /// If we are shutting down this worker explicitly
    /// this flag will be set to prevent the worker from
    /// being re-spawned.
    reap: bool,
    shutdown: mpsc::Sender<WorkerState>,
}

impl PartialEq for WorkerState {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.pid == other.pid
    }
}

impl Eq for WorkerState {}

/// Attempt to restart a worker that died.
async fn restart(worker: WorkerState, mut retry: Retry) {
    info!("Restarting worker {}", worker.id);
    retry.attempts = retry.attempts + 1;

    if retry.attempts >= retry.limit {
        error!(
            "Failed to restart worker {}, exceeded retry limit {}",
            worker.id, retry.limit
        );
    } else {
        if retry.factor > 0 {
            let ms = retry.attempts * retry.factor;
            info!("Delay restart {}ms", ms);
            time::sleep(Duration::from_millis(ms as u64)).await;
        }
        spawn_worker(worker.id, worker.task, worker.socket, retry)
    }
}

/// Generate a random opaque identifier.
pub fn id() -> String {
    let mut rng = rand::thread_rng();
    let mut hasher = DefaultHasher::new();
    hasher.write_usize(rng.gen());
    format!("{:x}", hasher.finish())
}

fn spawn_worker(id: String, task: Task, socket: PathBuf, retry: Retry) {
    tokio::task::spawn(async move {
        // Setup built in environment variables
        let mut envs = task.envs.clone();
        envs.insert(WORKER_ID.to_string(), id.clone());
        if !task.detached {
            envs.insert(
                SOCKET.to_string(),
                socket.to_string_lossy().into_owned(),
            );
        }

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<WorkerState>(1);

        info!("Spawn worker {} {}", &task.cmd, task.args.join(" "));

        let mut child = Command::new(task.cmd.clone())
            .args(task.args.clone())
            .envs(envs)
            .spawn()?;

        let pid = child.id();

        if let Some(ref id) = pid {
            info!("Worker pid {}", id);
        }

        {
            let worker = WorkerState {
                id: id.clone(),
                task,
                socket,
                pid,
                reap: false,
                shutdown: shutdown_tx,
            };
            let mut state = supervisor_state().lock().await;
            state.workers.push(worker);
        }

        let mut reaping = false;

        loop {
            tokio::select!(
                res = child.wait() => {
                    match res {
                        Ok(status) => {
                            let pid = pid.unwrap_or(0);
                            if !reaping {
                                if let Some(code) = status.code() {
                                    warn!("Worker process died: {} (code: {})", pid, code);
                                } else {
                                    warn!("Worker process died: {} ({})", pid, status);
                                }
                            }
                            let mut state = supervisor_state().lock().await;
                            let worker = state.remove(&id);
                            drop(state);
                            if let Some(worker) = worker {
                                info!("Removed child worker (id: {}, pid {})", worker.id, pid);
                                if !worker.reap && worker.task.daemon {
                                    restart(worker, retry).await;
                                }
                            } else {
                                if !reaping {
                                    error!("Failed to remove stale worker for pid {}", pid);
                                }
                            }
                            break;
                        }
                        Err(e) => return Err(e),
                    }
                }
                mut worker = shutdown_rx.recv() => {
                    if let Some(mut worker) = worker.take() {
                        reaping = true;
                        info!("Shutdown worker {}", worker.id);
                        worker.reap = true;
                        child.kill().await?;
                    }
                }
            )
        }

        Ok::<(), io::Error>(())
    });
}

async fn listen<P: AsRef<Path>>(
    socket: P,
    tx: Sender<()>,
    handler: Arc<IpcHandler>,
    control_tx: mpsc::Sender<Message>,
) -> Result<()> {
    let path = socket.as_ref();

    // If the socket file exists we must remove to prevent `EADDRINUSE`
    if path.exists() {
        std::fs::remove_file(path)?;
    }

    let listener = UnixListener::bind(socket).unwrap();
    tx.send(()).unwrap();

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => (handler)(stream, control_tx.clone()),
            Err(e) => {
                warn!("Supervisor failed to accept worker socket {}", e);
            }
        }
    }
}

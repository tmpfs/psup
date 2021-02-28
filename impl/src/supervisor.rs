//! Supervisor manages a collection of worker processes.
use std::{
    io, thread,
    hash::Hasher,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    sync::Mutex,
    collections::{hash_map::DefaultHasher, HashMap},
};

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot::{self, Sender};

use log::{error, info, warn};
use once_cell::sync::OnceCell;
use rand::Rng;

use super::{Result, SOCKET, WORKER_ID};

type IpcHandler = Box<dyn Fn(UnixStream) + Send + Sync>;

/// Get the supervisor state.
fn supervisor_state() -> &'static Mutex<SupervisorState> {
    static INSTANCE: OnceCell<Mutex<SupervisorState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(SupervisorState { workers: vec![] }))
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
        }
    }

    /// Set command arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
        where
            I: IntoIterator<Item = S>,
            S: AsRef<str> {
        let args = args.into_iter()
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

    /// Get a retry state for this task.
    fn retry(&self) -> Retry {
        Retry { limit: self.limit, attempts: 0 }
    }
}

#[derive(Clone, Copy)]
struct Retry {
    /// The limit on the number of times to attempt 
    /// to restart a process.
    limit: usize,
    /// The current number of attempts.
    attempts: usize,
}

/// Build a supervisor.
pub struct SupervisorBuilder {
    socket: PathBuf,
    commands: Vec<Task>,
    ipc_handler: Option<IpcHandler>,
}

impl SupervisorBuilder {
    /// Create a new supervisor builder.
    pub fn new() -> Self {
        let socket = std::env::temp_dir().join("psup.sock");
        Self {
            socket,
            commands: Vec::new(),
            ipc_handler: None,
        }
    }

    /// Set the IPC server handler.
    pub fn server<F: 'static>(mut self, handler: F) -> Self
        where F: Fn(UnixStream) + Send + Sync {
        self.ipc_handler = Some(Box::new(handler));
        self
    }

    /// Set the socket path.
    pub fn path(mut self, path: PathBuf) -> Self {
        self.socket = path;
        self
    }

    /// Add a worker process.
    pub fn add_worker(mut self, task: Task) -> Self {
        self.commands.push(task);
        self
    }

    /// Return the supervisor.
    pub fn build(self) -> Supervisor {
        Supervisor {
            socket: self.socket,
            commands: self.commands,
            ipc_handler: self.ipc_handler.map(Arc::new),
        }
    }
}

/// Supervisor manages long-running worker processes.
pub struct Supervisor {
    socket: PathBuf,
    commands: Vec<Task>,
    ipc_handler: Option<Arc<IpcHandler>>,
}

impl Supervisor {
    /// Start the supervisor running.
    ///
    /// Listens on the socket path and starts any initial workers.
    pub async fn run(&self) -> Result<()> {
        let socket = self.socket.clone();

        if let Some(ref ipc_handler) = self.ipc_handler {
            let (tx, rx) = oneshot::channel::<()>();
            let ipc = Arc::clone(ipc_handler);
            tokio::spawn(async move {
                listen(&socket, tx, ipc)
                    .await
                    .expect("Supervisor failed to bind to socket");
            });
            let _ = rx.await?;
            info!("Supervisor is listening {}", self.socket.display());
        }

        for task in self.commands.iter() {
            self.spawn(task.clone());
        }

        Ok(())
    }

    /// Spawn a worker task.
    pub fn spawn(&self, task: Task) {
        let retry = task.retry();
        spawn_worker(task, self.socket.clone(), retry);
    }
}

/// State of the supervisor worker processes.
struct SupervisorState {
    workers: Vec<WorkerState>,
}

impl SupervisorState {
    fn remove(&mut self, pid: u32) -> Option<WorkerState> {
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
struct WorkerState {
    task: Task,
    id: String,
    socket: PathBuf,
    pid: u32,
    /// If we are shutting down this worker explicitly
    /// this flag will be set to prevent the worker from
    /// being re-spawned.
    reap: bool,
}

impl PartialEq for WorkerState {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.pid == other.pid
    }
}

impl Eq for WorkerState {}

/// Attempt to restart a worker that died.
fn restart(worker: WorkerState, mut retry: Retry) {
    info!("Restarting worker {}", worker.id);
    retry.attempts = retry.attempts + 1; 

    if retry.attempts >= retry.limit {
        error!("Failed to restart worker {}, exceeded retry limit {}", worker.id, retry.limit);
    } else {
        // TODO: retry on fail with backoff and retry limit
        spawn_worker(worker.task, worker.socket, retry)
    }
}

fn spawn_worker(task: Task, socket: PathBuf, retry: Retry) {
    // Generate a unique id for each worker
    let mut rng = rand::thread_rng();
    let mut hasher = DefaultHasher::new();
    hasher.write_usize(rng.gen());
    let id = format!("{:x}", hasher.finish());

    thread::spawn(move || {
        // Setup built in environment variables
        let mut envs = task.envs.clone();
        envs.insert(WORKER_ID.to_string(), id.clone());
        if !task.detached {
            envs.insert(
                SOCKET.to_string(),
                socket.to_string_lossy().into_owned(),
            );
        }

        info!("Spawn worker {} {}", &task.cmd, task.args.join(" "));

        let child = Command::new(task.cmd.clone())
            .args(task.args.clone())
            .envs(envs)
            .spawn()?;

        let pid = child.id();
        info!("Worker pid {}", &pid);

        {
            let worker = WorkerState {
                task,
                id,
                socket,
                pid,
                reap: false,
            };
            let mut state = supervisor_state().lock().unwrap();
            state.workers.push(worker);
        }

        let result = child.wait_with_output()?;
        if let Some(code) = result.status.code() {
            warn!("Worker process died: {} (code: {})", pid, code);
        } else {
            warn!("Worker process died: {} ({})", pid, result.status);
        }

        let mut state = supervisor_state().lock().unwrap();
        let worker = state.remove(pid);
        drop(state);
        if let Some(worker) = worker {
            info!("Removed child worker (id: {}, pid {})", worker.id, pid);
            if !worker.reap && worker.task.daemon {
                restart(worker, retry);
            }
        } else {
            error!("Failed to remove stale worker for pid {}", pid);
        }

        Ok::<(), io::Error>(())
    });
}

async fn listen<P: AsRef<Path>>(
    socket: P,
    tx: Sender<()>,
    handler: Arc<IpcHandler>,
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
            Ok((stream, _addr)) => (handler)(stream),
            Err(e) => {
                warn!("Supervisor failed to accept worker socket {}", e);
            }
        }
    }
}

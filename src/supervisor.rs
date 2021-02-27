//! Supervisor manages a collection of worker processes.
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot::{self, Sender};

use log::{error, info, warn};
use once_cell::sync::OnceCell;
use rand::Rng;

use super::{WorkerInfo, Result};

/// Get the supervisor state.
fn supervisor_state() -> &'static Mutex<SupervisorState> {
    static INSTANCE: OnceCell<Mutex<SupervisorState>> = OnceCell::new();
    INSTANCE.get_or_init(|| Mutex::new(SupervisorState { workers: vec![] }))
}

/// Build a supervisor.
pub struct SupervisorBuilder {
    socket: PathBuf,
    commands: Vec<(String, Vec<String>, bool)>,
    ipc_handler: Box<dyn Fn(UnixStream) + Send + Sync>,
}

impl SupervisorBuilder {
    /// Create a new supervisor builder.
    pub fn new(ipc_handler: Box<dyn Fn(UnixStream) + Send + Sync>) -> Self {
        let socket = std::env::temp_dir().join("psup.sock");
        Self {
            socket,
            commands: Vec::new(),
            ipc_handler,
        }
    }

    /// Set the socket path.
    pub fn path(mut self, path: PathBuf) -> Self {
        self.socket = path;
        self
    }

    /// Add a worker process marked as a daemon.
    ///
    /// Daemon worker processes are restarted if they exit without being 
    /// explicitly shutdown by the supervisor.
    pub fn add_daemon(mut self, cmd: String, args: Vec<&str>) -> Self {
        let owned = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.commands.push((cmd, owned, true));
        self
    }

    /// Add a worker process.
    pub fn add_worker(mut self, cmd: String, args: Vec<&str>) -> Self {
        let owned = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.commands.push((cmd, owned, false));
        self
    }

    /// Return the supervisor.
    pub fn build(self) -> Supervisor {
        Supervisor {
            socket: self.socket,
            commands: self.commands,
            ipc_handler: Arc::new(self.ipc_handler),
        }
    }
}

/// Supervisor manages long-running worker processes.
pub struct Supervisor {
    socket: PathBuf,
    commands: Vec<(String, Vec<String>, bool)>,
    ipc_handler: Arc<Box<dyn Fn(UnixStream) + Send + Sync>>,
}

impl Supervisor {
    /// Start the supervisor running.
    ///
    /// Listens on the socket path and starts any initial workers.
    pub async fn run(&self) -> Result<()> {
        let socket_path = self.socket.clone();
        let (tx, rx) = oneshot::channel::<()>();

        let ipc = Arc::clone(&self.ipc_handler);

        tokio::spawn(async move {
            listen(&socket_path, tx, ipc)
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

impl PartialEq for WorkerState {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.pid == other.pid
    }
}

impl Eq for WorkerState {}

/// Attempt to restart a worker that died.
fn restart(worker: WorkerState) {
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

    thread::spawn(move || {
        let mut child = Command::new(cmd.clone())
            .args(args.clone())
            .stdin(Stdio::piped())
            .spawn()?;

        let child_stdin = child.stdin.as_mut().unwrap();
        let info = WorkerInfo {
            path: socket_path.clone(),
            id: id.clone(),
        }
        ;
        child_stdin.write_all(
            format!("{}\n", serde_json::to_string(&info).unwrap()).as_bytes(),
        )?;

        drop(child_stdin);

        let pid = child.id();
        {
            let worker = WorkerState {
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

async fn listen<P: AsRef<Path>>(
    socket: P,
    tx: Sender<()>,
    handler: Arc<Box<dyn Fn(UnixStream) + Send + Sync>>) -> Result<()> {
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

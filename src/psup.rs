#![deny(missing_docs)]
//! Binary for the psup(1) process supervisor; for the library use the
//! [psup-impl](https://docs.rs/psup-impl) crate.
use std::{
    collections::HashMap,
    path::PathBuf
};
use clap::{Arg, App};
use anyhow::{anyhow, Result};
use serde::{Serialize, Deserialize};
use psup_impl::{SupervisorBuilder, Task};
use log::info;

/// Settings deserialized from the configuration file.
#[derive(Debug, Serialize, Deserialize)]
struct Settings {
    /// Path for the local socket.
    socket: PathBuf,
    /// List of worker tasks.
    task: Vec<RunTask>,
}

/// Worker task information.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RunTask {
    /// Command for the process.
    command: String,
    /// Arguments for the process.
    args: Option<Vec<String>>,
    /// Environment variables for the process.
    envs: Option<HashMap<String, String>>,
    /// Daemonize the process, will be restarted on exit.
    daemon: Option<bool>,
    /// Limit for restarting daemon processes.
    retry_limit: Option<usize>,
}

impl Into<Task> for RunTask {
    fn into(self) -> Task {
        Task::new(&self.command)
            .args(self.args.unwrap_or(Vec::new()))
            .envs(self.envs.unwrap_or(HashMap::new()))
            .daemon(self.daemon.unwrap_or(false))
            .retry_limit(self.retry_limit.unwrap_or(5))
    }
}

/// Executable entry point.
#[doc(hidden)]
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").ok().is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let matches = App::new("psup")
        .version("1.0")
        .about("Process supervisor")
        .long_about("Reads the TOML configuration file and spawns supervised child processes.")
        .arg(Arg::with_name("config")
           .help("Configuration file")
           .required(true))
           //.index(0))
        .get_matches();

    let config = matches.value_of("config")
        .ok_or_else(|| anyhow!("Configuration file is required!"))?;
    let config = std::fs::read_to_string(config)
        .map_err(|e| anyhow!("Failed to read configuration {} ({})", config, e.to_string()))?;
    let settings: Settings = toml::from_str(&config)?;

    info!("Run {} worker(s)", settings.task.len());

    // Use an empty callback as there is no IPC
    let mut builder = SupervisorBuilder::new()
        .path(settings.socket);
    for runner in settings.task.into_iter() {
        let task: Task = runner.into();
        builder = builder.add_worker(task);
    }

    builder.build().run().await?;

    // Parent lives until explicitly killed
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60))
    }
}

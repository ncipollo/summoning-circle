use std::path::PathBuf;
use std::thread::{self, JoinHandle};

use anyhow::Result;
use tokio::runtime::Runtime;
use tokio::sync::watch;

use crate::feature::config;
use crate::feature::store::Store;
use crate::feature::supervisor::policy::Policy;
use crate::feature::supervisor::{Supervisor, signals};

/// Where the engine reads its config and keeps its state.
#[derive(Clone)]
pub struct Paths {
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub log_dir: PathBuf,
}

/// Loads the config and supervises its processes until `shutdown` fires.
pub async fn run(paths: &Paths, shutdown: watch::Receiver<bool>) -> Result<()> {
    let config = config::load(&paths.config_path)?;
    let store = Store::open(&paths.db_path).await?;

    let supervisor = Supervisor::new(
        &config,
        store,
        paths.log_dir.clone(),
        paths.config_path.clone(),
        Policy::default(),
    );
    supervisor.run(shutdown).await
}

/// The engine running on its own thread and runtime, for callers (like `tray`) whose main
/// thread is taken over by something else.
///
/// The process exits as soon as the engine finishes, whether from SIGINT/SIGTERM, a fatal
/// error, or `stop`: the engine is the reason the process exists.
pub struct Background {
    shutdown: watch::Sender<bool>,
    thread: JoinHandle<()>,
}

impl Background {
    /// Validates the config up front so a bad one fails the caller instead of the thread.
    pub fn spawn(paths: Paths) -> Result<Self> {
        config::load(&paths.config_path)?;
        let runtime = Runtime::new()?;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let signal_tx = shutdown.clone();

        let thread = thread::Builder::new()
            .name("engine".to_string())
            .spawn(move || exit_after(&runtime, &paths, signal_tx, shutdown_rx))?;
        Ok(Self { shutdown, thread })
    }

    /// Asks the engine to shut down and waits for it. The engine thread exits the process when
    /// it finishes, so this does not normally return.
    pub fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.thread.join();
    }
}

fn exit_after(
    runtime: &Runtime,
    paths: &Paths,
    signal_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
) {
    runtime.spawn(signals::listen(signal_tx));
    let code = match runtime.block_on(run(paths, shutdown_rx)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

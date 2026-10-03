use std::path::{Path, PathBuf};

use anyhow::Result;
use tokio::runtime::Builder;

use crate::feature::ps;
use crate::feature::store::{ProcessRecord, Store};

/// Reads the tracked processes at a fixed path, for the tray's timer-driven poll loop.
///
/// Each `tick` opens (and drops) its own `Store` inside a throwaway single-threaded Tokio
/// runtime, rather than caching a `Store` across ticks: a `Store`'s connection pool is tied to
/// the runtime it was opened under, and the surrounding `tao` event loop only ever drives one
/// runtime at a time, each dropped before the next tick's is built.
pub struct Poller {
    db_path: PathBuf,
}

impl Poller {
    pub fn new(db_path: &Path) -> Self {
        Self {
            db_path: db_path.to_path_buf(),
        }
    }

    /// Returns the resolved process list, or an empty list while the store's database file
    /// doesn't exist yet (mirrors `ps`'s own check, so a `tray` launched before `run`/`install`
    /// doesn't silently create an empty database).
    pub fn tick(&self) -> Result<Vec<ProcessRecord>> {
        if !self.db_path.exists() {
            return Ok(Vec::new());
        }

        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(list(&self.db_path))
    }
}

async fn list(db_path: &Path) -> Result<Vec<ProcessRecord>> {
    let store = Store::open(db_path).await?;
    Ok(ps::resolve(store.list().await?).await)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::Poller;
    use crate::feature::store::{ProcessRecord, Store};

    #[test]
    fn tick_returns_empty_before_the_database_exists() {
        let dir = TempDir::new().expect("temp dir should create");
        let poller = Poller::new(&dir.path().join("circle.db"));

        let records = poller.tick().expect("tick should succeed");

        assert!(records.is_empty());
        assert!(!dir.path().join("circle.db").exists());
    }

    /// Plain (non-`#[tokio::test]`) on purpose: `tick` builds its own runtime internally, which
    /// panics ("cannot start a runtime from within a runtime") if called from inside one. The
    /// setup below uses its own throwaway runtime, dropped before `tick` runs, to mirror that.
    #[test]
    fn tick_returns_resolved_records_once_the_database_exists() {
        let dir = TempDir::new().expect("temp dir should create");
        let db_path = dir.path().join("circle.db");

        let setup = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("setup runtime should build");
        setup.block_on(async {
            let store = Store::open(&db_path).await.expect("store should open");
            store
                .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
                .await
                .expect("upsert should succeed");
        });
        drop(setup);

        let records = Poller::new(&db_path).tick().expect("tick should succeed");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "api");
    }
}

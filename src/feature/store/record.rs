use std::str::FromStr;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// A process's status as tracked in the store, plus the display-only `Stale`
/// status `ps` reports for a `Running` record whose pid is no longer alive.
/// `Stale` is intentionally absent from `FromStr`: it is never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    Starting,
    Running,
    Exited,
    Stopped,
    Stale,
}

impl ProcessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessStatus::Starting => "starting",
            ProcessStatus::Running => "running",
            ProcessStatus::Exited => "exited",
            ProcessStatus::Stopped => "stopped",
            ProcessStatus::Stale => "stale",
        }
    }
}

impl FromStr for ProcessStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "starting" => Ok(ProcessStatus::Starting),
            "running" => Ok(ProcessStatus::Running),
            "exited" => Ok(ProcessStatus::Exited),
            "stopped" => Ok(ProcessStatus::Stopped),
            other => bail!("unknown process status '{other}'"),
        }
    }
}

/// The current supervisor's identity, as persisted in the `supervisor` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct SupervisorRecord {
    pub pid: u32,
    pub start_time: Option<i64>,
}

/// A tracked process's pid and start time, used to identify orphans left
/// behind by a previous supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct TrackedPid {
    pub pid: u32,
    pub start_time: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
pub struct ProcessRecord {
    pub name: String,
    pub kind: String,
    pub command: String,
    pub pid: Option<u32>,
    pub status: ProcessStatus,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl ProcessRecord {
    /// A freshly reconciled record: not yet running, no history.
    pub fn starting(
        name: impl Into<String>,
        kind: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            command: command.into(),
            pid: None,
            status: ProcessStatus::Starting,
            restart_count: 0,
            last_exit_code: None,
            started_at: None,
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessStatus;

    #[test]
    fn round_trips_through_str() {
        for status in [
            ProcessStatus::Starting,
            ProcessStatus::Running,
            ProcessStatus::Exited,
            ProcessStatus::Stopped,
        ] {
            let parsed: ProcessStatus = status.as_str().parse().expect("known status should parse");
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn stale_is_display_only_and_not_parseable() {
        assert_eq!(ProcessStatus::Stale.as_str(), "stale");
        assert!("stale".parse::<ProcessStatus>().is_err());
    }

    #[test]
    fn rejects_unknown_status() {
        let error = "paused"
            .parse::<ProcessStatus>()
            .expect_err("unknown status should error");

        assert!(error.to_string().contains("paused"));
    }
}

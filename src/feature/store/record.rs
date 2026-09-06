use std::str::FromStr;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum ProcessStatus {
    Starting,
    Running,
    Exited,
    Stopped,
}

impl ProcessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessStatus::Starting => "starting",
            ProcessStatus::Running => "running",
            ProcessStatus::Exited => "exited",
            ProcessStatus::Stopped => "stopped",
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

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
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
    fn rejects_unknown_status() {
        let error = "paused"
            .parse::<ProcessStatus>()
            .expect_err("unknown status should error");

        assert!(error.to_string().contains("paused"));
    }
}

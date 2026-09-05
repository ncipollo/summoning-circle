use std::str::FromStr;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let status: String = row.get("status")?;
        let pid: Option<i64> = row.get("pid")?;
        let restart_count: i64 = row.get("restart_count")?;
        let started_at: Option<String> = row.get("started_at")?;
        let updated_at: String = row.get("updated_at")?;

        Ok(Self {
            name: row.get("name")?,
            kind: row.get("kind")?,
            command: row.get("command")?,
            pid: pid.map(|pid| pid as u32),
            status: status
                .parse()
                .map_err(|error: anyhow::Error| rusqlite_error(error))?,
            restart_count: restart_count as u32,
            last_exit_code: row.get("last_exit_code")?,
            started_at: started_at
                .map(|value| parse_timestamp(&value))
                .transpose()
                .map_err(rusqlite_error)?,
            updated_at: parse_timestamp(&updated_at).map_err(rusqlite_error)?,
        })
    }
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn rusqlite_error(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(error.into())
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

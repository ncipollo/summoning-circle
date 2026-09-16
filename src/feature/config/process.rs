use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProcessEntry {
    pub name: String,
    #[serde(flatten)]
    pub kind: ProcessKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessKind {
    Shell {
        command: String,
        cwd: Option<PathBuf>,
        env: Option<BTreeMap<String, String>>,
    },
    /// A process with its own lifecycle commands, monitored and stopped through them rather
    /// than by holding a child handle and signaling by pid. `status` is expected to exit 0
    /// while the daemon is alive and non-zero once it's dead.
    Daemon {
        start: String,
        stop: String,
        status: String,
    },
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::ProcessKind;

    #[derive(Debug, Deserialize)]
    struct Wrapper {
        #[serde(flatten)]
        kind: ProcessKind,
    }

    fn parse(toml: &str) -> ProcessKind {
        toml::from_str::<Wrapper>(toml)
            .expect("daemon entry should parse")
            .kind
    }

    #[test]
    fn parses_a_daemon_entry() {
        let kind = parse(
            r#"
            type = "daemon"
            start = "pg_ctl start"
            stop = "pg_ctl stop"
            status = "pg_ctl status"
        "#,
        );

        assert!(matches!(
            kind,
            ProcessKind::Daemon { start, stop, status }
                if start == "pg_ctl start" && stop == "pg_ctl stop" && status == "pg_ctl status"
        ));
    }

    #[test]
    fn rejects_a_daemon_entry_missing_stop() {
        let error = toml::from_str::<Wrapper>(
            r#"
            type = "daemon"
            start = "pg_ctl start"
            status = "pg_ctl status"
        "#,
        )
        .expect_err("missing stop command should be rejected");

        assert!(error.to_string().contains("stop"));
    }
}

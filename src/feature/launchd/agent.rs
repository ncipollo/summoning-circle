use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub const LABEL: &str = "com.ncipollo.summoning-circle";

/// The launchd user-agent plist for summoning-circle. Field names use
/// `rename_all = "PascalCase"` so they serialize as the exact keys launchd expects
/// (`Label`, `ProgramArguments`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LaunchAgent {
    pub label: String,
    pub program_arguments: Vec<String>,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub standard_out_path: String,
    pub standard_error_path: String,
}

/// Builds the launch agent plist that runs `<exe> run [--config <config_override>]`,
/// logging to `agent.out.log` / `agent.err.log` under `log_dir`.
pub fn build(exe: &Path, config_override: Option<&Path>, log_dir: &Path) -> LaunchAgent {
    let mut program_arguments = vec![exe.display().to_string(), "run".to_string()];
    if let Some(config) = config_override {
        program_arguments.push("--config".to_string());
        program_arguments.push(config.display().to_string());
    }

    LaunchAgent {
        label: LABEL.to_string(),
        program_arguments,
        run_at_load: true,
        keep_alive: true,
        standard_out_path: log_dir.join("agent.out.log").display().to_string(),
        standard_error_path: log_dir.join("agent.err.log").display().to_string(),
    }
}

/// Writes `agent` as an XML plist to `path`.
pub fn write(agent: &LaunchAgent, path: &Path) -> Result<()> {
    plist::to_file_xml(path, agent)
        .with_context(|| format!("could not write launch agent plist to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::{LABEL, build, write};

    #[test]
    fn label_matches_the_constant() {
        let agent = build(
            Path::new("/usr/local/bin/summoning-circle"),
            None,
            Path::new("/logs"),
        );

        assert_eq!(agent.label, LABEL);
    }

    #[test]
    fn program_arguments_omit_config_flag_by_default() {
        let agent = build(
            Path::new("/usr/local/bin/summoning-circle"),
            None,
            Path::new("/logs"),
        );

        assert_eq!(
            agent.program_arguments,
            vec![
                "/usr/local/bin/summoning-circle".to_string(),
                "run".to_string()
            ]
        );
    }

    #[test]
    fn program_arguments_include_config_flag_when_overridden() {
        let agent = build(
            Path::new("/usr/local/bin/summoning-circle"),
            Some(Path::new("/tmp/config.toml")),
            Path::new("/logs"),
        );

        assert_eq!(
            agent.program_arguments,
            vec![
                "/usr/local/bin/summoning-circle".to_string(),
                "run".to_string(),
                "--config".to_string(),
                "/tmp/config.toml".to_string(),
            ]
        );
    }

    #[test]
    fn runs_at_load_and_keeps_alive() {
        let agent = build(
            Path::new("/usr/local/bin/summoning-circle"),
            None,
            Path::new("/logs"),
        );

        assert!(agent.run_at_load);
        assert!(agent.keep_alive);
    }

    #[test]
    fn logs_to_the_configured_log_dir() {
        let agent = build(
            Path::new("/usr/local/bin/summoning-circle"),
            None,
            Path::new("/logs"),
        );

        assert_eq!(agent.standard_out_path, "/logs/agent.out.log");
        assert_eq!(agent.standard_error_path, "/logs/agent.err.log");
    }

    #[test]
    fn write_round_trips_through_plist_xml() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("agent.plist");
        let agent = build(
            Path::new("/usr/local/bin/summoning-circle"),
            Some(Path::new("/tmp/config.toml")),
            Path::new("/logs"),
        );

        write(&agent, &path).expect("write should succeed");
        let read_back: super::LaunchAgent =
            plist::from_file(&path).expect("plist should read back");

        assert_eq!(read_back, agent);
    }
}

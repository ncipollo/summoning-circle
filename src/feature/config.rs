pub mod process;

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;

use process::ProcessEntry;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Config {
    pub processes: Vec<ProcessEntry>,
}

/// Loads and validates the config file at `path`.
pub fn load(path: &Path) -> Result<Config> {
    let contents = read(path)?;
    parse(&contents)
}

/// Parses and validates config file contents.
pub fn parse(contents: &str) -> Result<Config> {
    let raw: RawConfig = toml::from_str(contents).context("failed to parse config file")?;
    let processes = raw
        .process
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_entry(index, value))
        .collect::<Result<Vec<_>>>()?;
    validate(&processes)?;
    Ok(Config { processes })
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default, rename = "process")]
    process: Vec<toml::Value>,
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => anyhow!("config file not found at {}", path.display()),
        _ => anyhow::Error::new(error)
            .context(format!("could not read config file at {}", path.display())),
    })
}

fn parse_entry(index: usize, value: toml::Value) -> Result<ProcessEntry> {
    let description = describe_entry(index, &value);
    ProcessEntry::deserialize(value)
        .with_context(|| format!("invalid definition for {description}"))
}

fn describe_entry(index: usize, value: &toml::Value) -> String {
    value
        .get("name")
        .and_then(|name| name.as_str())
        .map(|name| format!("process '{name}'"))
        .unwrap_or_else(|| format!("process at index {index}"))
}

fn validate(processes: &[ProcessEntry]) -> Result<()> {
    let mut seen = HashSet::new();
    for entry in processes {
        if entry.name.trim().is_empty() {
            bail!("process names must not be empty");
        }
        if !seen.insert(entry.name.as_str()) {
            bail!("duplicate process name '{}'", entry.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::{load, parse};

    const SAMPLE: &str = r#"
        [[process]]
        name = "api"
        type = "shell"
        command = "cargo run --release"
        cwd = "/Users/me/src/api"
        env = { RUST_LOG = "info" }

        [[process]]
        name = "tunnel"
        type = "shell"
        command = "ssh -N -L 5432:localhost:5432 db-host"
    "#;

    #[test]
    fn parses_sample_config() {
        let config = parse(SAMPLE).expect("sample config should parse");

        assert_eq!(config.processes.len(), 2);

        let api = &config.processes[0];
        assert_eq!(api.name, "api");
        let super::process::ProcessKind::Shell { command, cwd, env } = &api.kind;
        assert_eq!(command, "cargo run --release");
        assert_eq!(
            cwd.as_deref(),
            Some(std::path::Path::new("/Users/me/src/api"))
        );
        assert_eq!(
            env.as_ref().unwrap().get("RUST_LOG"),
            Some(&"info".to_string())
        );

        let tunnel = &config.processes[1];
        assert_eq!(tunnel.name, "tunnel");
        let super::process::ProcessKind::Shell { cwd, env, .. } = &tunnel.kind;
        assert_eq!(*cwd, None);
        assert_eq!(*env, None);
    }

    #[test]
    fn empty_document_has_no_processes() {
        let config = parse("").expect("empty document should parse");

        assert!(config.processes.is_empty());
    }

    #[test]
    fn rejects_duplicate_names() {
        let toml = r#"
            [[process]]
            name = "api"
            type = "shell"
            command = "one"

            [[process]]
            name = "api"
            type = "shell"
            command = "two"
        "#;

        let error = parse(toml).expect_err("duplicate names should be rejected");

        assert!(error.to_string().contains("api"));
    }

    #[test]
    fn rejects_empty_name() {
        let toml = r#"
            [[process]]
            name = ""
            type = "shell"
            command = "one"
        "#;

        let error = parse(toml).expect_err("empty name should be rejected");

        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn rejects_unknown_type_naming_the_entry() {
        let toml = r#"
            [[process]]
            name = "api"
            type = "http"
            command = "one"
        "#;

        let error = parse(toml).expect_err("unknown type should be rejected");

        assert!(error.to_string().contains("api"));
    }

    #[test]
    fn load_reports_missing_file_path() {
        let path = env::temp_dir().join("summoning-circle-missing-config.toml");

        let error = load(&path).expect_err("missing config file should error");

        assert!(error.to_string().contains(&path.display().to_string()));
    }
}

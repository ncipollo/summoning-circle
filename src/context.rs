use std::path::PathBuf;

use anyhow::{Context as _, Result};

/// Resolved paths a command needs to run, independent of which subcommand was invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
}

impl Context {
    /// Resolves the context, using `config_override` when given and falling back to the
    /// default location under the user's home directory otherwise.
    pub fn resolve(config_override: Option<PathBuf>) -> Result<Self> {
        let home = home_dir()?;
        let data_dir = home.join(".summoning-circle");
        let config_path = config_override.unwrap_or_else(|| data_dir.join("config.toml"));

        Ok(Self {
            config_path,
            data_dir,
        })
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .context("could not determine home directory: HOME is not set")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_default_config_path_under_home_when_no_override() {
        let context = Context::resolve(None).unwrap();

        assert_eq!(context.data_dir.file_name().unwrap(), ".summoning-circle");
        assert_eq!(context.config_path, context.data_dir.join("config.toml"));
    }

    #[test]
    fn uses_override_config_path_when_given() {
        let override_path = PathBuf::from("/tmp/x.toml");

        let context = Context::resolve(Some(override_path.clone())).unwrap();

        assert_eq!(context.config_path, override_path);
    }
}

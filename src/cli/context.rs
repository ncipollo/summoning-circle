use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

const DATA_DIR_NAME: &str = ".summoning-circle";
const CONFIG_FILE_NAME: &str = "config.toml";

/// Paths resolved once at startup and shared by every subcommand.
pub struct Context {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
}

impl Context {
    pub fn new(config_override: Option<PathBuf>) -> Result<Self> {
        let home =
            dirs::home_dir().context("could not resolve the current user's home directory")?;
        Ok(Self::from_home(&home, config_override))
    }

    fn from_home(home: &Path, config_override: Option<PathBuf>) -> Self {
        let data_dir = home.join(DATA_DIR_NAME);
        let config_path = config_override.unwrap_or_else(|| data_dir.join(CONFIG_FILE_NAME));

        Self {
            config_path,
            data_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::Context;

    #[test]
    fn explicit_override_wins() {
        let context =
            Context::from_home(Path::new("/home/user"), Some(PathBuf::from("/tmp/x.toml")));

        assert_eq!(context.config_path, PathBuf::from("/tmp/x.toml"));
        assert_eq!(
            context.data_dir,
            PathBuf::from("/home/user/.summoning-circle")
        );
    }

    #[test]
    fn defaults_under_home_directory() {
        let context = Context::from_home(Path::new("/home/user"), None);

        assert_eq!(
            context.config_path,
            PathBuf::from("/home/user/.summoning-circle/config.toml")
        );
        assert_eq!(
            context.data_dir,
            PathBuf::from("/home/user/.summoning-circle")
        );
    }
}

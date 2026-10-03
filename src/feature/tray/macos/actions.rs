use anyhow::{Context, Result};
use tracing::warn;

/// A per-process action the tray menu can trigger, encoded into (and decoded from) a menu item
/// id so the macOS event loop doesn't need to maintain its own id-to-action table across the
/// menu rebuilds every poll tick produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Restart(String),
    Pause(String),
    Resume(String),
}

const RESTART_PREFIX: &str = "restart:";
const PAUSE_PREFIX: &str = "pause:";
const RESUME_PREFIX: &str = "resume:";

impl Action {
    pub fn id(&self) -> String {
        match self {
            Action::Restart(name) => format!("{RESTART_PREFIX}{name}"),
            Action::Pause(name) => format!("{PAUSE_PREFIX}{name}"),
            Action::Resume(name) => format!("{RESUME_PREFIX}{name}"),
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        if let Some(name) = id.strip_prefix(RESTART_PREFIX) {
            Some(Action::Restart(name.to_string()))
        } else if let Some(name) = id.strip_prefix(PAUSE_PREFIX) {
            Some(Action::Pause(name.to_string()))
        } else {
            id.strip_prefix(RESUME_PREFIX)
                .map(|name| Action::Resume(name.to_string()))
        }
    }

    fn subcommand_args(&self) -> [&str; 2] {
        match self {
            Action::Restart(name) => ["restart", name],
            Action::Pause(name) => ["pause", name],
            Action::Resume(name) => ["resume", name],
        }
    }
}

/// Shells out to this same binary's `restart`/`pause`/`resume` subcommand, reusing the CLI's
/// own control logic as the single source of truth rather than duplicating it in-process. Logs
/// rather than propagating a failure, since this is invoked from a GUI event handler with
/// nowhere to print to.
pub fn dispatch(action: &Action) -> Result<()> {
    let exe = std::env::current_exe().context("could not resolve the current executable path")?;
    let args = action.subcommand_args();

    let status = std::process::Command::new(&exe)
        .args(args)
        .status()
        .with_context(|| format!("could not run '{} {}'", exe.display(), args.join(" ")))?;

    if !status.success() {
        warn!(?action, ?status, "tray action exited with a failure status");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Action;

    #[test]
    fn id_round_trips_through_parse() {
        for action in [
            Action::Restart("api".to_string()),
            Action::Pause("api".to_string()),
            Action::Resume("api".to_string()),
        ] {
            let parsed = Action::parse(&action.id()).expect("id should parse back");
            assert_eq!(parsed, action);
        }
    }

    #[test]
    fn parse_rejects_an_unknown_id() {
        assert_eq!(Action::parse("exit"), None);
        assert_eq!(Action::parse("restart"), None);
    }

    #[test]
    fn subcommand_args_include_the_process_name() {
        assert_eq!(
            Action::Restart("api".to_string()).subcommand_args(),
            ["restart", "api"]
        );
        assert_eq!(
            Action::Pause("api".to_string()).subcommand_args(),
            ["pause", "api"]
        );
        assert_eq!(
            Action::Resume("api".to_string()).subcommand_args(),
            ["resume", "api"]
        );
    }
}

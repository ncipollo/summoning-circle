//! The `config` topic: `config.toml` process declarations.

const CONFIG_EXAMPLE: &str = "\
[[process]]
name = \"api\"
type = \"shell\"
command = \"cargo run --release\"
cwd = \"/Users/me/src/api\"          # optional, defaults to the user's home directory
env = { RUST_LOG = \"info\" }         # optional

[[process]]
name = \"tunnel\"
type = \"shell\"
command = \"ssh -N -L 5432:localhost:5432 db-host\"
";

pub fn render() -> String {
    format!(
        "CONFIG FILE\n\
         summoning-circle reads a TOML config file listing the processes it should\n\
         manage. By default it looks for ~/.summoning-circle/config.toml; pass\n\
         --config <PATH> to use a different file instead.\n\n\
         Each process is declared as a [[process]] entry, tagged by `type`. The\n\
         only type today is `shell`, which launches a command via the shell and\n\
         keeps it alive:\n\n{CONFIG_EXAMPLE}\n\
         FIELDS\n\
         \x20 name     required  unique identifier for the process\n\
         \x20 type     required  process kind; only \"shell\" is supported today\n\
         \x20 command  required  shell command used to launch the process\n\
         \x20 cwd      optional  working directory for the command\n\
         \x20 env      optional  extra environment variables for the command\n\n\
         LIVE RELOAD\n\
         \x20 While `run` is active, editing this file adds, removes, or restarts the\n\
         \x20 affected processes automatically. Invalid edits are logged and ignored\n\
         \x20 until fixed.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const README: &str = include_str!("../../../README.md");

    #[test]
    fn config_example_matches_readme() {
        assert!(
            README.contains(CONFIG_EXAMPLE.trim_end()),
            "config example has drifted from README.md"
        );
    }

    #[test]
    fn config_example_parses() {
        crate::feature::config::parse(CONFIG_EXAMPLE).expect("config example should parse");
    }

    #[test]
    fn page_documents_shell_fields() {
        let page = render();
        for field in ["name", "type", "command", "cwd", "env"] {
            assert!(page.contains(field), "missing field: {field}");
        }
    }

    #[test]
    fn page_documents_default_location() {
        let page = render();
        assert!(page.contains("~/.summoning-circle/config.toml"));
        assert!(page.contains("--config"));
    }

    #[test]
    fn page_documents_live_reload() {
        let page = render();
        assert!(page.to_lowercase().contains("live reload"));
    }
}

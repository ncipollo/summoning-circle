//! The `usage` topic: subcommands and the global `--config` flag.

pub fn render() -> String {
    String::from(
        "USAGE\n\
         summoning-circle [OPTIONS] <COMMAND>\n\n\
         COMMANDS\n\
         \x20 install   Install summoning-circle as a user launch agent\n\
         \x20 uninstall Remove the summoning-circle user launch agent\n\
         \x20 run       Launch configured processes and keep them alive (foreground)\n\
         \x20 ps        List processes tracked by summoning-circle (--json for machine-readable output)\n\n\
         OPTIONS\n\
         \x20 -c, --config <PATH>   Path to the process config file\n\
         \x20                       (default: ~/.summoning-circle/config.toml)\n\n\
         See also: --info config\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_documents_all_subcommands() {
        let page = render();
        for command in ["install", "uninstall", "run", "ps"] {
            assert!(page.contains(command), "missing subcommand: {command}");
        }
    }

    #[test]
    fn page_documents_config_flag() {
        let page = render();
        assert!(page.contains("--config"));
        assert!(page.contains("~/.summoning-circle/config.toml"));
    }

    #[test]
    fn page_documents_ps_json_flag() {
        let page = render();
        assert!(page.contains("--json"));
    }
}

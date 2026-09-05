use std::path::Path;

use anyhow::Result;
use summoning_circle::feature::config;
use summoning_circle::feature::config::Config;
use summoning_circle::feature::config::process::{ProcessEntry, ProcessKind};

use crate::cli::context::Context;

pub fn run(context: &Context) -> Result<()> {
    let config = config::load(&context.config_path)?;

    print!("{}", render(&config, &context.config_path));

    Ok(())
}

fn render(config: &Config, config_path: &Path) -> String {
    if config.processes.is_empty() {
        return format!("no processes configured in {}\n", config_path.display());
    }

    config
        .processes
        .iter()
        .map(render_entry)
        .collect::<Vec<_>>()
        .join("")
}

fn render_entry(entry: &ProcessEntry) -> String {
    match &entry.kind {
        ProcessKind::Shell { command, .. } => format!("{}\tshell\t{}\n", entry.name, command),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use summoning_circle::feature::config;

    use super::render;

    #[test]
    fn renders_configured_processes() {
        let config = config::parse(
            r#"
            [[process]]
            name = "api"
            type = "shell"
            command = "cargo run --release"
        "#,
        )
        .expect("sample config should parse");

        let output = render(&config, Path::new("/tmp/config.toml"));

        assert_eq!(output, "api\tshell\tcargo run --release\n");
    }

    #[test]
    fn renders_placeholder_when_empty() {
        let config = config::parse("").expect("empty config should parse");

        let output = render(&config, Path::new("/tmp/config.toml"));

        assert_eq!(output, "no processes configured in /tmp/config.toml\n");
    }
}

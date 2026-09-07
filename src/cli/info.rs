use anyhow::{Result, bail};
use summoning_circle::feature::info;

pub fn run(topic: Option<&str>) -> Result<()> {
    match topic {
        None => {
            println!("{}", info::directory());
            Ok(())
        }
        Some(name) => match info::topic(name) {
            Some(page) => {
                println!("{page}");
                Ok(())
            }
            None => bail!(unknown_topic_error(name)),
        },
    }
}

fn unknown_topic_error(name: &str) -> String {
    format!(
        "unknown topic \"{name}\": available topics: {}",
        info::names().join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_topic_error_names_available_topics() {
        let error = unknown_topic_error("bogus");
        assert!(error.contains("unknown topic \"bogus\""));
        for name in info::names() {
            assert!(error.contains(name), "missing topic name: {name}");
        }
    }
}

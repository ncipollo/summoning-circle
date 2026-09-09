use std::collections::HashMap;

use crate::feature::config::process::ProcessEntry;

/// The difference between the process entries a supervisor is currently running and a freshly
/// reloaded config.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Diff {
    pub added: Vec<ProcessEntry>,
    pub removed: Vec<String>,
    pub changed: Vec<ProcessEntry>,
}

/// Diffs `new` against `old` by process name. An entry present in both with an unchanged
/// definition (`ProcessEntry`/`ProcessKind` already derive `PartialEq`) appears in neither list.
pub fn diff(old: &[ProcessEntry], new: &[ProcessEntry]) -> Diff {
    let old_by_name: HashMap<&str, &ProcessEntry> = old
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    let new_by_name: HashMap<&str, &ProcessEntry> = new
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();

    let added = new
        .iter()
        .filter(|entry| !old_by_name.contains_key(entry.name.as_str()))
        .cloned()
        .collect();
    let removed = old
        .iter()
        .filter(|entry| !new_by_name.contains_key(entry.name.as_str()))
        .map(|entry| entry.name.clone())
        .collect();
    let changed = new
        .iter()
        .filter(|entry| {
            old_by_name
                .get(entry.name.as_str())
                .is_some_and(|old_entry| *old_entry != *entry)
        })
        .cloned()
        .collect();

    Diff {
        added,
        removed,
        changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::config::process::ProcessKind;

    fn shell(name: &str, command: &str) -> ProcessEntry {
        ProcessEntry {
            name: name.to_string(),
            kind: ProcessKind::Shell {
                command: command.to_string(),
                cwd: None,
                env: None,
            },
        }
    }

    #[test]
    fn empty_to_empty_is_an_empty_diff() {
        assert_eq!(diff(&[], &[]), Diff::default());
    }

    #[test]
    fn a_new_entry_is_added() {
        let new = vec![shell("api", "cargo run")];

        let result = diff(&[], &new);

        assert_eq!(result.added, new);
        assert!(result.removed.is_empty());
        assert!(result.changed.is_empty());
    }

    #[test]
    fn a_dropped_entry_is_removed() {
        let old = vec![shell("api", "cargo run")];

        let result = diff(&old, &[]);

        assert!(result.added.is_empty());
        assert_eq!(result.removed, vec!["api".to_string()]);
        assert!(result.changed.is_empty());
    }

    #[test]
    fn a_different_command_for_the_same_name_is_changed() {
        let old = vec![shell("api", "cargo run")];
        let new = vec![shell("api", "cargo run --release")];

        let result = diff(&old, &new);

        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.changed, new);
    }

    #[test]
    fn an_identical_entry_appears_in_no_list() {
        let old = vec![shell("api", "cargo run")];
        let new = old.clone();

        let result = diff(&old, &new);

        assert_eq!(result, Diff::default());
    }

    #[test]
    fn a_mix_of_add_remove_and_change_in_one_call() {
        let old = vec![shell("api", "cargo run"), shell("tunnel", "ssh -N")];
        let new = vec![
            shell("api", "cargo run --release"),
            shell("worker", "run-worker"),
        ];

        let result = diff(&old, &new);

        assert_eq!(result.added, vec![shell("worker", "run-worker")]);
        assert_eq!(result.removed, vec!["tunnel".to_string()]);
        assert_eq!(result.changed, vec![shell("api", "cargo run --release")]);
    }
}

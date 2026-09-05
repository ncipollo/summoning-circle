use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProcessEntry {
    pub name: String,
    #[serde(flatten)]
    pub kind: ProcessKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessKind {
    Shell {
        command: String,
        cwd: Option<PathBuf>,
        env: Option<BTreeMap<String, String>>,
    },
}

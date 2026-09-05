use std::path::{Path, PathBuf};

use super::Context;

#[test]
fn explicit_override_wins() {
    let context = Context::from_home(Path::new("/home/user"), Some(PathBuf::from("/tmp/x.toml")));

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

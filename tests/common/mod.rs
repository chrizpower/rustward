use rustward::config::{Config, load};
use std::fs;

pub const SERVICE: &str = "version = 2\n[storage]\nstate_database = '/synthetic/state.db'\nlog_directory = '/synthetic/logs'\n";

pub fn validate(source: &str) -> Result<Config, String> {
    validate_set(&[("synthetic-report", source)])
}

pub fn validate_set(tasks: &[(&str, &str)]) -> Result<Config, String> {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    fs::create_dir(root.path().join("tasks")).unwrap();
    fs::create_dir(root.path().join("schedules")).unwrap();
    fs::write(root.path().join("rustward.toml"), SERVICE).unwrap();
    for (id, source) in tasks {
        let path = root.path().join("tasks").join(format!("{id}.toml"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    load(root.path().join("rustward.toml"))
}

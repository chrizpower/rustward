use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub version: u32,
    #[serde(default)]
    pub definitions: Definitions,
    pub storage: Storage,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub retention: Retention,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Definitions {
    pub tasks: PathBuf,
    pub schedules: PathBuf,
}

impl Default for Definitions {
    fn default() -> Self {
        Self {
            tasks: "tasks".into(),
            schedules: "schedules".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    pub state_database: PathBuf,
    pub log_directory: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub active_runs: u32,
    pub queued_runs: u32,
    pub log_bytes_per_run: u64,
    pub retained_log_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            active_runs: 1,
            queued_runs: 100,
            log_bytes_per_run: 10 * 1024 * 1024,
            retained_log_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Retention {
    pub days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self { days: 30 }
    }
}

impl Service {
    pub(super) fn parse(source: &str, base: &Path) -> Result<Self, String> {
        let mut service: Self = toml::from_str(source).map_err(|e| e.to_string())?;
        if service.version != 2 {
            return Err(
                "version: only version 2 is supported; split legacy task lists into files".into(),
            );
        }
        for path in [
            &mut service.definitions.tasks,
            &mut service.definitions.schedules,
            &mut service.storage.state_database,
            &mut service.storage.log_directory,
        ] {
            if path.as_os_str().is_empty() || path.as_os_str().as_encoded_bytes().contains(&0) {
                return Err("service paths must be nonempty and contain no NUL".into());
            }
            if path.is_relative() {
                *path = base.join(&path);
            }
        }
        Ok(service)
    }
}

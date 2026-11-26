use std::{
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Eq)]
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
    #[serde(default)]
    pub run_resources: RunResources,
    #[serde(default)]
    pub telemetry: Telemetry,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RunResources {
    pub memory_max_bytes: Option<NonZeroU64>,
    pub tasks_max: Option<NonZeroU32>,
    pub cpu_quota_percent: Option<NonZeroU32>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    pub state_database: PathBuf,
    pub log_directory: PathBuf,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Retention {
    pub days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self { days: 30 }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    pub metrics: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_resource_limits_are_optional_positive_integers() {
        let source = "version=2\n[storage]\nstate_database='state'\nlog_directory='logs'\n";
        let parse = |extra: &str| Service::parse(&format!("{source}{extra}"), Path::new("/"));
        assert_eq!(parse("").unwrap().run_resources, RunResources::default());
        let limits = parse(
            "[run_resources]\nmemory_max_bytes=67108864\ntasks_max=32\ncpu_quota_percent=150",
        )
        .unwrap()
        .run_resources;
        assert_eq!(limits.memory_max_bytes.unwrap().get(), 67_108_864);
        assert_eq!(limits.tasks_max.unwrap().get(), 32);
        assert_eq!(limits.cpu_quota_percent.unwrap().get(), 150);
        for invalid in [
            "memory_max_bytes=0",
            "tasks_max=-1",
            "cpu_quota_percent=0",
            "unknown=1",
        ] {
            assert!(parse(&format!("[run_resources]\n{invalid}")).is_err());
        }
    }
}

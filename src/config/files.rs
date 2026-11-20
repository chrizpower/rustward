use std::{collections::BTreeMap, fs::File, io::Read, os::fd::AsFd, path::Path};

use rustix::fs::{CWD, Dir, Mode, OFlags};
#[cfg(target_os = "linux")]
use rustix::fs::{ResolveFlags, openat2};

use super::{Config, Schedule, Service, TaskDefinition, valid_id, validate_tasks};

const FILE_BYTES: u64 = 1024 * 1024;
const SET_BYTES: usize = 16 * 1024 * 1024;
const ENTRIES: usize = 10_000;
const DEPTH: usize = 32;

type Sources = BTreeMap<String, String>;

#[derive(Default, PartialEq)]
struct Candidate {
    service: String,
    tasks: Sources,
    schedules: Sources,
    folders: Vec<String>,
    entries: usize,
    bytes: usize,
}

/// Load a complete version 2 configuration. Keep the files unchanged during
/// loading. Two matching reads detect observed edits, but are not a filesystem snapshot.
pub fn load(path: impl AsRef<Path>) -> Result<Config, String> {
    load_checked(path.as_ref(), || {})
}

fn load_checked(path: &Path, between_reads: impl FnOnce()) -> Result<Config, String> {
    if path.file_name().is_none_or(|name| name != "rustward.toml") {
        return Err("service file must be named rustward.toml".into());
    }
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let parent = absolute.parent().unwrap();
    let directory = open(CWD, parent)?;
    let first = collect(&directory, parent)?;
    between_reads();
    if first != collect(&directory, parent)? {
        return Err("configuration changed during loading; keep files unchanged and retry".into());
    }
    let service = Service::parse(&first.service, absolute.parent().unwrap())?;
    let tasks = first
        .tasks
        .into_iter()
        .map(|(id, source)| {
            let mut task: TaskDefinition =
                toml::from_str(&source).map_err(|e| format!("task {id}: {e}"))?;
            task.id = id;
            Ok(task)
        })
        .collect::<Result<Vec<_>, String>>()?;
    validate_tasks(&tasks)?;
    let mut config = Config {
        tasks,
        schedules: Vec::new(),
        service,
    };
    for (id, source) in first.schedules {
        let mut schedule: Schedule =
            toml::from_str(&source).map_err(|e| format!("schedule {id}: {e}"))?;
        schedule.id = id;
        schedule.validate_cron()?;
        config
            .arguments(&schedule.task, &schedule.inputs)
            .map_err(|e| format!("schedule {}: {e}", schedule.id))?;
        config.schedules.push(schedule);
    }
    Ok(config)
}

fn open(parent: impl AsFd, path: &Path) -> Result<File, String> {
    // Reject symlinks in every path component during the open.
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK;
    #[cfg(target_os = "linux")]
    let file = openat2(
        parent,
        path,
        flags,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    );
    #[cfg(target_os = "macos")]
    let file = rustix::fs::openat(parent, path, flags | OFlags::NOFOLLOW_ANY, Mode::empty());
    file.map(File::from)
        .map_err(|e| format!("cannot open {} without symlinks: {e}", path.display()))
}

fn read(file: File, label: &str, total: &mut usize) -> Result<String, String> {
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{label}: expected a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{label}: {e}"))?;
    *total += bytes.len();
    if bytes.len() as u64 > FILE_BYTES || *total > SET_BYTES {
        return Err(format!("{label}: configuration size limit exceeded"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{label}: expected UTF-8"))
}

fn collect(directory: &File, parent: &Path) -> Result<Candidate, String> {
    let mut candidate = Candidate::default();
    candidate.service = read(
        open(directory, Path::new("rustward.toml"))?,
        "rustward.toml",
        &mut candidate.bytes,
    )?;
    let service = Service::parse(&candidate.service, parent)?;
    for (scope, root) in [
        ("tasks", &service.definitions.tasks),
        ("schedules", &service.definitions.schedules),
    ] {
        let root = match root.strip_prefix(parent) {
            Ok(relative)
                if relative.components().all(|part| {
                    matches!(
                        part,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                }) =>
            {
                open(
                    directory,
                    if relative.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        relative
                    },
                )?
            }
            _ => open(CWD, root)?,
        };
        walk(root, "", scope, 0, &mut candidate)?;
    }
    candidate.folders.sort();
    Ok(candidate)
}

fn walk(
    directory: File,
    prefix: &str,
    scope: &str,
    depth: usize,
    candidate: &mut Candidate,
) -> Result<(), String> {
    if !directory.metadata().map_err(|e| e.to_string())?.is_dir() {
        return Err(format!("{scope}/{prefix}: expected a directory"));
    }
    let entries = Dir::read_from(&directory).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .to_str()
            .map_err(|_| "definition names must be ASCII")?;
        if name == "." || name == ".." {
            continue;
        }
        candidate.entries += 1;
        if candidate.entries > ENTRIES {
            return Err("configuration entry limit exceeded".into());
        }
        let relative = format!("{prefix}{name}");
        let file = open(&directory, Path::new(name))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            if !valid_id(name) {
                return Err(format!("{scope}/{relative}: invalid folder component"));
            }
            if depth >= DEPTH {
                return Err("configuration folder depth limit exceeded".into());
            }
            candidate.folders.push(format!("{scope}/{relative}"));
            walk(file, &format!("{relative}/"), scope, depth + 1, candidate)?;
        } else {
            let stem = name
                .strip_suffix(".toml")
                .filter(|stem| valid_id(stem))
                .ok_or_else(|| format!("{scope}/{relative}: invalid definition filename"))?;
            let source = read(file, &format!("{scope}/{relative}"), &mut candidate.bytes)?;
            let id = format!("{prefix}{stem}");
            let sources = if scope == "tasks" {
                &mut candidate.tasks
            } else {
                &mut candidate.schedules
            };
            if sources.insert(id, source).is_some() {
                return Err("duplicate definition ID".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_observed_edits_and_keeps_previous_config() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        std::fs::create_dir(root.path().join("tasks")).unwrap();
        std::fs::create_dir(root.path().join("schedules")).unwrap();
        let path = root.path().join("rustward.toml");
        std::fs::write(&path, include_str!("../../examples/rustward.toml")).unwrap();
        let previous = load(&path).unwrap();
        let error = load_checked(&path, || {
            std::fs::write(root.path().join("tasks/new.toml"), "invalid").unwrap();
        })
        .unwrap_err();
        assert!(error.contains("changed during loading"));
        assert_eq!(previous.task_count(), 0);
    }

    #[test]
    fn keeps_one_configuration_directory_during_publication() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let active = root.path().join("active");
        let replacement = root.path().join("replacement");
        for generation in [&active, &replacement] {
            std::fs::create_dir_all(generation.join("tasks")).unwrap();
            std::fs::create_dir(generation.join("schedules")).unwrap();
            std::fs::write(
                generation.join("rustward.toml"),
                include_str!("../../examples/rustward.toml"),
            )
            .unwrap();
        }
        std::fs::write(
            replacement.join("tasks/new.toml"),
            include_str!("../../examples/tasks/synthetic-report.toml"),
        )
        .unwrap();
        let path = active.join("rustward.toml");
        let opened = load_checked(&path, || {
            std::fs::rename(&active, root.path().join("previous")).unwrap();
            std::fs::rename(&replacement, &active).unwrap();
        })
        .unwrap();
        assert_eq!(opened.task_count(), 0);
        assert_eq!(load(&path).unwrap().task_count(), 1);
    }
}

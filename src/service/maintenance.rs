mod files;

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use rustix::fs::{FlockOperation, OFlags, flock};

use super::{check_database, state_error};
use crate::{
    admission::{
        Store,
        maintenance::{Decision, snapshot},
    },
    config,
    runtime::WorkRoot,
    supervision::RunId,
};

const DATABASE: &str = "state.db";
const LOGS: &str = "logs";
const MANIFEST: &str = "manifest.json";
const USAGE: &str = "Usage:
  rustward state backup <rustward.toml> <new-backup-directory>
  rustward state restore <backup-directory> <new-state-directory>
  rustward state reviews <state-directory>
  rustward state decide <state-directory> <run-id> <keep|cancel|interrupt> <note>
  rustward state release <state-directory>";

pub fn command(args: impl Iterator<Item = OsString>) -> Result<(), String> {
    use crate::telemetry::{Event, Telemetry};
    let args: Vec<_> = args.collect();
    let result = execute(&args);
    let verb = match args.first().and_then(|action| action.to_str()) {
        Some("backup") => Some("state_backup"),
        Some("restore") => Some("state_restore"),
        Some("decide") => Some("restore_decision"),
        Some("release") => Some("restore_release"),
        _ => None,
    };
    if let Some(verb) = verb
        && let Ok(telemetry) = Telemetry::stderr()
    {
        let event = Event::new(verb, "maintenance");
        telemetry.log(if result.is_ok() {
            event
        } else {
            event.error().reason("operation_failed")
        });
        telemetry.flush();
    }
    result
}

fn execute(args: &[OsString]) -> Result<(), String> {
    let usage = || USAGE.to_owned();
    match args {
        [action, config, target] if action == "backup" => {
            backup(Path::new(config), &absolute(target)?)
        }
        [action, source, target] if action == "restore" => {
            restore(&absolute(source)?, &absolute(target)?)
        }
        [action, dir] if action == "reviews" => {
            let (_lock, mut store) = open_existing(&absolute(dir)?.join(DATABASE))?;
            let reviews = store.restore_reviews().map_err(state_error)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&reviews).map_err(message)?
            );
            Ok(())
        }
        [action, dir, run, decision, note] if action == "decide" => {
            let run = RunId::new(run.to_str().ok_or_else(usage)?).map_err(message)?;
            let decision = match decision.to_str() {
                Some("keep") => Decision::Keep,
                Some("cancel") => Decision::Cancel,
                Some("interrupt") => Decision::Interrupt,
                _ => return Err(usage()),
            };
            let (_lock, mut store) = open_existing(&absolute(dir)?.join(DATABASE))?;
            store
                .decide_restore(&run, decision, note.to_str().ok_or_else(usage)?)
                .map_err(state_error)
        }
        [action, dir] if action == "release" => {
            let (_lock, mut store) = open_existing(&absolute(dir)?.join(DATABASE))?;
            store.release_restore().map_err(state_error)
        }
        _ => Err(usage()),
    }
}

fn message(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

fn absolute(path: &OsString) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(message)
}

fn lock_database(path: &Path) -> Result<File, String> {
    check_database(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(message)?;
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|error| format!("State database lock: {error}"))?;
    Ok(file)
}

fn open_existing(path: &Path) -> Result<(File, Store), String> {
    let file = lock_database(path)?;
    let store = Store::open(path).map_err(state_error)?;
    Ok((file, store))
}

fn backup(config: &Path, target: &Path) -> Result<(), String> {
    let config = config::load(config).map_err(message)?;
    let storage = &config.service().storage;
    let _lock = lock_database(&storage.state_database)?;
    let logs = WorkRoot::open(&storage.log_directory).map_err(message)?;
    let stage = files::stage(target)?;
    let database = stage.path().join(DATABASE);
    snapshot(&storage.state_database, &database).map_err(state_error)?;
    files::set_private_file(&database)?;
    let mut copy = Store::open(&database).map_err(state_error)?;
    if copy
        .restore_phase()
        .map_err(state_error)?
        .as_deref()
        .is_some_and(|phase| phase != "released")
    {
        return Err("Finish restore review before backing up this state".to_owned());
    }
    copy.seal_backup().map_err(state_error)?;
    copy.check_integrity().map_err(state_error)?;
    let protected = copy.protected_output().map_err(state_error)?;
    drop(copy);

    let mut manifest = files::Manifest::new();
    manifest.insert(DATABASE.to_owned(), files::file(&database, None)?);
    files::logs(&logs, &stage.path().join(LOGS), &mut manifest)?;
    for (id, expected) in protected {
        if output_bytes(&manifest, &id) != expected as u64 {
            return Err(format!("Protected output is incomplete for run {id}"));
        }
    }
    files::write_manifest(&stage.path().join(MANIFEST), &manifest)?;
    files::publish(stage, target)
}

fn restore(source: &Path, target: &Path) -> Result<(), String> {
    WorkRoot::open(source).map_err(message)?;
    let expected = files::read_manifest(&source.join(MANIFEST))?;
    let logs = WorkRoot::open(source.join(LOGS)).map_err(message)?;
    let stage = files::stage(target)?;
    let database = stage.path().join(DATABASE);
    let mut actual = files::Manifest::new();
    actual.insert(
        DATABASE.to_owned(),
        files::file(&source.join(DATABASE), Some(&database))?,
    );
    files::logs(&logs, &stage.path().join(LOGS), &mut actual)?;
    if actual != expected {
        return Err("Backup contents differ from the manifest".into());
    }
    let mut store = Store::open(&database).map_err(state_error)?;
    store.check_integrity().map_err(state_error)?;
    store
        .begin_restore(|id| output_bytes(&actual, id))
        .map_err(state_error)?;
    drop(store);
    fs::File::open(database)
        .and_then(|file| file.sync_all())
        .map_err(message)?;
    files::publish(stage, target)
}

fn output_bytes(manifest: &files::Manifest, id: &str) -> u64 {
    ["stdout.log", "stderr.log"]
        .iter()
        .filter_map(|stream| manifest.get(&format!("logs/{id}/{stream}")))
        .map(|entry| entry.bytes)
        .sum()
}

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use rustix::fs::{CWD, OFlags, RenameFlags, renameat_with};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::message;
use crate::{runtime::WorkRoot, supervision::RunId};

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Entry {
    digest: String,
    pub(super) bytes: u64,
}

pub(super) type Manifest = BTreeMap<String, Entry>;

pub(super) fn stage(target: &Path) -> Result<TempDir, String> {
    let parent = target
        .parent()
        .ok_or("Destination needs a parent directory")?;
    WorkRoot::open(parent).map_err(message)?;
    tempfile::Builder::new()
        .prefix(".rustward-state-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(parent)
        .map_err(message)
}

pub(super) fn publish(stage: TempDir, target: &Path) -> Result<(), String> {
    sync_directory(stage.path())?;
    renameat_with(CWD, stage.path(), CWD, target, RenameFlags::NOREPLACE).map_err(message)?;
    let _ = stage.keep();
    sync_directory(target.parent().expect("stage parent"))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(message)
}

pub(super) fn set_private_file(path: &Path) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(message)
}

fn private_file(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(message)?;
    let meta = file.metadata().map_err(message)?;
    if !meta.is_file()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.nlink() != 1
        || meta.mode() & 0o7777 != 0o600
    {
        return Err("Invalid backup or log file".into());
    }
    Ok(file)
}

fn create_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(message)
}

pub(super) fn file(source: &Path, target: Option<&Path>) -> Result<Entry, String> {
    let mut input = private_file(source)?;
    let mut output = target.map(create_file).transpose()?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut bytes = [0; 32768];
    loop {
        let count = input.read(&mut bytes).map_err(message)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        let chunk = &bytes[..count];
        hash.update(chunk);
        if let Some(file) = &mut output {
            file.write_all(chunk).map_err(message)?;
        }
    }
    if let Some(file) = output {
        file.sync_all().map_err(message)?;
    } else {
        input.sync_all().map_err(message)?;
    }
    Ok(Entry {
        digest: hex::encode(hash.finalize()),
        bytes: size,
    })
}

pub(super) fn logs(
    source: &WorkRoot,
    target: &Path,
    manifest: &mut Manifest,
) -> Result<(), String> {
    DirBuilder::new()
        .mode(0o700)
        .create(target)
        .map_err(message)?;
    source.checked_directory().map_err(message)?;
    for entry in fs::read_dir(source.path()).map_err(message)? {
        let entry = entry.map_err(message)?;
        let id = RunId::new(
            entry
                .file_name()
                .to_str()
                .ok_or("Invalid log directory name")?,
        )
        .map_err(message)?;
        WorkRoot::open(entry.path()).map_err(message)?;
        let output = target.join(id.as_str());
        DirBuilder::new()
            .mode(0o700)
            .create(&output)
            .map_err(message)?;
        for stream in fs::read_dir(entry.path()).map_err(message)? {
            let stream = stream.map_err(message)?;
            let name = stream.file_name();
            let name = name.to_str().ok_or("Invalid log name")?;
            if !matches!(name, "stdout.log" | "stderr.log") {
                return Err("Unexpected entry in log directory".into());
            }
            let hash = file(&stream.path(), Some(&output.join(name)))?;
            manifest.insert(format!("logs/{}/{name}", id.as_str()), hash);
        }
        sync_directory(&output)?;
    }
    sync_directory(target)
}

pub(super) fn read_manifest(path: &Path) -> Result<Manifest, String> {
    serde_json::from_reader(private_file(path)?).map_err(message)
}

pub(super) fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let mut file = create_file(path)?;
    serde_json::to_writer(&mut file, manifest).map_err(message)?;
    file.sync_all().map_err(message)
}

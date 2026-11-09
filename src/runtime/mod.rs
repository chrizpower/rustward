//! Verification of externally prepared runtimes. This does not prepare code or
//! prove dependency completeness; see docs/temporary/runtime.md.
use std::{
    fs::File,
    path::{Component, Path, PathBuf},
};

use crate::supervision::Launch;

mod files;
mod manifest;
mod work;
use manifest::Manifest;
pub use work::{WorkDirectory, WorkRoot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Missing,
    Invalid,
    Changed,
    Unsafe,
    Command,
    Limit,
    WorkDirectory,
    WorkStorage,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub manifest_bytes: u64,
    pub entries: usize,
    pub content_bytes: u64,
    pub depth: usize,
    pub path_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            manifest_bytes: 16 * 1024 * 1024,
            entries: 100_000,
            content_bytes: 4 * 1024 * 1024 * 1024,
            depth: 64,
            path_bytes: 4096,
        }
    }
}
impl Limits {
    fn valid(self) -> bool {
        self.manifest_bytes > 0
            && self.manifest_bytes < usize::MAX as u64
            && self.entries > 0
            && self.depth > 0
            && self.path_bytes > 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Protection {
    pub preparation_uid: u32,
    pub execution_uid: u32,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    path: PathBuf,
    protection: Protection,
    limits: Limits,
}

pub struct VerifiedRuntime {
    id: String,
    path: PathBuf,
    root: File,
    manifest: Manifest,
}

pub fn manifest_id(source: &str) -> Result<String, Error> {
    Ok(Manifest::parse(source, Limits::default())?.id())
}

pub(super) fn normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
}

impl Catalog {
    pub fn open(
        path: impl AsRef<Path>,
        protection: Protection,
        limits: Limits,
    ) -> Result<Self, Error> {
        if !limits.valid() {
            return Err(Error::Limit);
        }
        if protection.execution_uid == 0 || protection.execution_uid == protection.preparation_uid {
            return Err(Error::Unsafe);
        }
        let path = path.as_ref();
        if !normal_absolute(path) {
            return Err(Error::Unsafe);
        }
        files::protected_root(path, protection)?;
        Ok(Self {
            path: path.to_owned(),
            protection,
            limits,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn execution_uid(&self) -> u32 {
        self.protection.execution_uid
    }

    pub fn verify(&self, id: &str) -> Result<VerifiedRuntime, Error> {
        if !manifest::valid_id(id) {
            return Err(Error::Invalid);
        }
        let catalog = files::protected_root(&self.path, self.protection)?;
        let root = files::open(&catalog, Path::new(id))?;
        files::protect(&root, self.protection, true, false)?;
        let source = files::manifest(&root, self.protection, self.limits)?;
        let manifest = Manifest::parse(&source, self.limits)?;
        if manifest.id() != id {
            return Err(Error::Changed);
        }
        files::verify_tree(&root, &manifest, self.protection, self.limits)?;
        let current_catalog = files::protected_root(&self.path, self.protection)?;
        let current = files::open(&current_catalog, Path::new(id))?;
        if !files::same_file(&root, &current)? {
            return Err(Error::Changed);
        }
        files::protect(&current, self.protection, true, false)?;
        let final_manifest = files::manifest(&current, self.protection, self.limits)?;
        if Manifest::parse(&final_manifest, self.limits)?.id() != id {
            return Err(Error::Changed);
        }
        Ok(VerifiedRuntime {
            id: id.to_owned(),
            path: self.path.join(id),
            root,
            manifest,
        })
    }
}

impl VerifiedRuntime {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn check_command(&self, launch: &Launch) -> Result<(), Error> {
        let executable = Path::new(&launch.executable);
        let directory = Path::new(&launch.working_directory);
        if !normal_absolute(executable) || !normal_absolute(directory) {
            return Err(Error::Command);
        }
        let program = executable
            .strip_prefix(&self.path)
            .ok()
            .and_then(Path::to_str)
            .ok_or(Error::Command)?;
        let cwd = directory
            .strip_prefix(&self.path)
            .ok()
            .and_then(Path::to_str)
            .ok_or(Error::Command)?;
        if !self
            .manifest
            .files
            .get(program)
            .is_some_and(|file| file.executable)
            || (!cwd.is_empty() && !self.manifest.directories.iter().any(|path| path == cwd))
        {
            return Err(Error::Command);
        }
        let file = files::open(&self.root, Path::new(program)).map_err(|_| Error::Command)?;
        if !file.metadata().map_err(|_| Error::Command)?.is_file() {
            return Err(Error::Command);
        }
        Ok(())
    }
}

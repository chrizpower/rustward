use rustix::fs::{Mode, mkdirat};
use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{Error, files, normal_absolute};
use crate::supervision::RunId;

#[derive(Clone)]
pub struct WorkRoot {
    root: Arc<File>,
    path: PathBuf,
    owner: u32,
}
pub struct WorkDirectory {
    root: WorkRoot,
    file: File,
    path: PathBuf,
}

impl WorkRoot {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if !normal_absolute(path) {
            return Err(Error::WorkDirectory);
        }
        let file = files::open(rustix::fs::CWD, path).map_err(|_| Error::WorkDirectory)?;
        let owner = rustix::process::geteuid().as_raw();
        private(&file, owner)?;
        ancestors(path, owner)?;
        Ok(Self {
            root: Arc::new(file),
            path: path.to_owned(),
            owner,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn owner_uid(&self) -> u32 {
        self.owner
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        let file = files::open(rustix::fs::CWD, &self.path).map_err(|_| Error::WorkDirectory)?;
        if !files::same_file(&self.root, &file).map_err(|_| Error::WorkDirectory)? {
            return Err(Error::WorkDirectory);
        }
        private(&file, self.owner)?;
        ancestors(&self.path, self.owner)
    }
    pub fn create(&self, run_id: &RunId) -> Result<WorkDirectory, Error> {
        self.create_checked(run_id, || {})
    }

    fn create_checked(
        &self,
        run_id: &RunId,
        before_create: impl FnOnce(),
    ) -> Result<WorkDirectory, Error> {
        self.check()?;
        before_create();
        mkdirat(&self.root, run_id.as_str(), Mode::from_bits_truncate(0o700)).map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                Error::WorkDirectory
            } else {
                Error::WorkStorage
            }
        })?;
        let file =
            files::open(&self.root, Path::new(run_id.as_str())).map_err(|_| Error::WorkStorage)?;
        private(&file, self.owner).map_err(|_| Error::WorkStorage)?;
        file.sync_all()
            .and_then(|_| self.root.sync_all())
            .map_err(|_| Error::WorkStorage)?;
        Ok(WorkDirectory {
            root: self.clone(),
            file,
            path: self.path.join(run_id.as_str()),
        })
    }
}
impl WorkDirectory {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn owner_uid(&self) -> u32 {
        self.root.owner
    }
    pub fn check(&self) -> Result<(), Error> {
        self.root.check()?;
        let file = files::open(rustix::fs::CWD, &self.path).map_err(|_| Error::WorkDirectory)?;
        if !files::same_file(&self.file, &file).map_err(|_| Error::WorkDirectory)? {
            return Err(Error::WorkDirectory);
        }
        private(&file, self.root.owner)
    }
}
fn private(file: &File, owner: u32) -> Result<(), Error> {
    let meta = file.metadata().map_err(|_| Error::WorkDirectory)?;
    if !meta.is_dir() || meta.uid() != owner || meta.mode() & 0o7777 != 0o700 {
        return Err(Error::WorkDirectory);
    }
    Ok(())
}

fn ancestors(path: &Path, owner: u32) -> Result<(), Error> {
    for parent in path.ancestors().skip(1) {
        let file = files::open(rustix::fs::CWD, parent).map_err(|_| Error::WorkDirectory)?;
        let meta = file.metadata().map_err(|_| Error::WorkDirectory)?;
        if !meta.is_dir()
            || (meta.uid() != 0 && meta.uid() != owner)
            || (meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0)
        {
            return Err(Error::WorkDirectory);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn observed_directory_storage_failure_cannot_return_a_work_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let work = WorkRoot::open(root.path()).unwrap();
        let id = RunId::new(&"a".repeat(64)).unwrap();
        let result = work.create_checked(&id, || std::fs::remove_dir(root.path()).unwrap());
        assert!(matches!(result, Err(Error::WorkStorage)));
    }
}

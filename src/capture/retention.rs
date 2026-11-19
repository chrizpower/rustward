use std::{fs::File, io, os::unix::fs::MetadataExt};

use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat, unlinkat},
    io::Errno,
};

use crate::supervision::RunId;

pub(crate) fn remove_logs(root: &File, id: &RunId) -> io::Result<()> {
    let owner = rustix::process::geteuid().as_raw();
    let directory = match openat2(
        root,
        id.as_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
    ) {
        Ok(directory) => File::from(directory),
        Err(Errno::NOENT) => return root.sync_all(),
        Err(error) => return Err(error.into()),
    };
    let meta = directory.metadata()?;
    if meta.uid() != owner || meta.mode() & 0o7777 != 0o700 {
        return Err(Errno::PERM.into());
    }
    for name in ["stdout.log", "stderr.log"] {
        match statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(meta)
                if FileType::from_raw_mode(meta.st_mode) == FileType::RegularFile
                    && meta.st_uid == owner
                    && meta.st_mode & 0o7777 == 0o600
                    && meta.st_nlink == 1 =>
            {
                unlinkat(&directory, name, AtFlags::empty())?;
            }
            Ok(_) => return Err(Errno::PERM.into()),
            Err(Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
        }
    }
    directory.sync_all()?;
    unlinkat(root, id.as_str(), AtFlags::REMOVEDIR)?;
    root.sync_all()
}

use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::Path,
};

use rustix::fs::{CWD, Dir, Mode, OFlags, ResolveFlags, openat2};
use sha2::{Digest, Sha256};

use super::{Error, Limits, Protection, manifest::Manifest};

pub(super) fn open(parent: impl AsFd, path: &Path) -> Result<File, Error> {
    openat2(
        parent,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )
    .map(File::from)
    .map_err(|error| {
        if error == rustix::io::Errno::NOENT {
            Error::Missing
        } else {
            Error::Unsafe
        }
    })
}

pub(super) fn same_file(first: &File, second: &File) -> Result<bool, Error> {
    let a = first.metadata().map_err(|_| Error::Unsafe)?;
    let b = second.metadata().map_err(|_| Error::Unsafe)?;
    Ok(a.dev() == b.dev() && a.ino() == b.ino())
}

pub(super) fn protect(
    file: &File,
    policy: Protection,
    directory: bool,
    executable: bool,
) -> Result<(), Error> {
    let meta = file.metadata().map_err(|_| Error::Unsafe)?;
    let mode = meta.mode();
    let required = if directory || executable {
        0o005
    } else {
        0o004
    };
    if (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
        || meta.uid() != policy.preparation_uid
        || meta.uid() == policy.execution_uid
        || mode & 0o7022 != 0
        || mode & required != required
        || (!directory && !executable && mode & 0o111 != 0)
    {
        return Err(Error::Unsafe);
    }
    let mut empty: [u8; 0] = [];
    match rustix::fs::fgetxattr(file, "security.capability", &mut empty[..]) {
        Err(rustix::io::Errno::NODATA | rustix::io::Errno::NOTSUP) => {}
        _ => return Err(Error::Unsafe),
    }
    Ok(())
}

pub(super) fn protected_root(path: &Path, policy: Protection) -> Result<File, Error> {
    let root = open(CWD, path)?;
    protect(&root, policy, true, false)?;
    for parent in path.ancestors().skip(1) {
        let file = open(CWD, parent)?;
        let meta = file.metadata().map_err(|_| Error::Unsafe)?;
        if !meta.is_dir()
            || (meta.uid() != 0 && meta.uid() != policy.preparation_uid)
            || meta.uid() == policy.execution_uid
            || meta.mode() & 0o022 != 0
            || meta.mode() & 0o011 != 0o011
        {
            return Err(Error::Unsafe);
        }
    }
    Ok(root)
}

pub(super) fn manifest(root: &File, policy: Protection, limits: Limits) -> Result<String, Error> {
    let file = open(root, Path::new("runtime.toml"))?;
    protect(&file, policy, false, false)?;
    let mut source = Vec::new();
    file.take(limits.manifest_bytes + 1)
        .read_to_end(&mut source)
        .map_err(|_| Error::Invalid)?;
    if source.len() as u64 > limits.manifest_bytes {
        return Err(Error::Limit);
    }
    String::from_utf8(source).map_err(|_| Error::Invalid)
}

pub(super) fn verify_tree(
    root: &File,
    manifest: &Manifest,
    policy: Protection,
    limits: Limits,
) -> Result<(), Error> {
    let before = root.metadata().map_err(|_| Error::Unsafe)?;
    let mut walk = Walk {
        manifest,
        policy,
        limits,
        entries: 0,
        bytes: 0,
        seen: BTreeSet::new(),
        buffer: vec![0; 64 * 1024],
    };
    walk.directory(root, "", 0)?;
    let after = root.metadata().map_err(|_| Error::Unsafe)?;
    if !unchanged(&before, &after) {
        return Err(Error::Changed);
    }
    if walk.seen.len() != manifest.files.len() + manifest.directories.len() {
        return Err(Error::Changed);
    }
    Ok(())
}

struct Walk<'a> {
    manifest: &'a Manifest,
    policy: Protection,
    limits: Limits,
    entries: usize,
    bytes: u64,
    seen: BTreeSet<String>,
    buffer: Vec<u8>,
}
impl Walk<'_> {
    fn directory(&mut self, parent: &File, prefix: &str, depth: usize) -> Result<(), Error> {
        for entry in Dir::read_from(parent).map_err(|_| Error::Unsafe)? {
            let entry = entry.map_err(|_| Error::Unsafe)?;
            let name = entry.file_name().to_str().map_err(|_| Error::Invalid)?;
            if name == "." || name == ".." {
                continue;
            }
            self.entries = self.entries.checked_add(1).ok_or(Error::Limit)?;
            if self.entries > self.limits.entries {
                return Err(Error::Limit);
            }
            let relative = format!("{prefix}{name}");
            if relative.len() > self.limits.path_bytes {
                return Err(Error::Limit);
            }
            let file = open(parent, Path::new(name))?;
            if relative == "runtime.toml" {
                protect(&file, self.policy, false, false)?;
                continue;
            }
            let before = file.metadata().map_err(|_| Error::Unsafe)?;
            if before.is_dir() {
                if depth >= self.limits.depth
                    || self.manifest.directories.binary_search(&relative).is_err()
                {
                    return Err(Error::Changed);
                }
                protect(&file, self.policy, true, false)?;
                self.seen.insert(relative.clone());
                self.directory(&file, &format!("{relative}/"), depth + 1)?;
            } else {
                let expected = self.manifest.files.get(&relative).ok_or(Error::Changed)?;
                protect(&file, self.policy, false, expected.executable)?;
                if before.len() > self.limits.content_bytes - self.bytes {
                    return Err(Error::Limit);
                }
                let mut hash = Sha256::new();
                let mut reader = &file;
                loop {
                    let count = match reader.read(&mut self.buffer) {
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => return Err(Error::Changed),
                    };
                    if count == 0 {
                        break;
                    }
                    self.bytes = self.bytes.checked_add(count as u64).ok_or(Error::Limit)?;
                    if self.bytes > self.limits.content_bytes {
                        return Err(Error::Limit);
                    }
                    hash.update(&self.buffer[..count]);
                }
                if hex::encode(hash.finalize()) != expected.sha256 {
                    return Err(Error::Changed);
                }
                self.seen.insert(relative);
            }
            let after = file.metadata().map_err(|_| Error::Unsafe)?;
            if !unchanged(&before, &after) {
                return Err(Error::Changed);
            }
        }
        Ok(())
    }
}

fn unchanged(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}

use super::{
    Error, Limits, Protection, files,
    manifest::{FileSpec, valid_id},
    normal_absolute,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    launcher: String,
    files: BTreeMap<String, FileSpec>,
    #[serde(default)]
    links: BTreeMap<String, String>,
}
#[derive(Clone)]
pub struct Catalog {
    path: PathBuf,
    policy: Protection,
    limits: Limits,
}
pub struct VerifiedHost {
    id: String,
    launcher: PathBuf,
}
impl VerifiedHost {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn launcher(&self) -> &Path {
        &self.launcher
    }
}
fn path_valid(path: &str, limits: Limits) -> bool {
    path.len() <= limits.path_bytes
        && !path.contains(['\0', '\\'])
        && normal_absolute(Path::new(path))
        && path.split('/').count() <= limits.depth + 1
}
impl Manifest {
    fn parse(source: &str, limits: Limits) -> Result<Self, Error> {
        if source.len() as u64 > limits.manifest_bytes {
            return Err(Error::Limit);
        }
        let result: Self = toml::from_str(source).map_err(|_| Error::Invalid)?;
        if result.version != 1
            || result.files.is_empty()
            || result
                .files
                .len()
                .checked_add(result.links.len())
                .and_then(|n| n.checked_add(1))
                .is_none_or(|n| n > limits.entries)
        {
            return Err(Error::Invalid);
        }
        if !path_valid(&result.launcher, limits) {
            return Err(Error::Invalid);
        }
        if !result
            .files
            .get(&result.launcher)
            .is_some_and(|spec| spec.executable)
        {
            return Err(Error::Invalid);
        }
        for (path, spec) in &result.files {
            if !path_valid(path, limits)
                || !valid_id(&spec.sha256)
                || result.links.contains_key(path)
            {
                return Err(Error::Invalid);
            }
        }
        for (path, target) in &result.links {
            if !path_valid(path, limits)
                || target.is_empty()
                || target.len() > limits.path_bytes
                || target.contains(['\0', '\\'])
            {
                return Err(Error::Invalid);
            }
            result.resolve(Path::new(path), limits)?;
        }
        result.check_links(limits)?;
        Ok(result)
    }
    fn check_links(&self, limits: Limits) -> Result<(), Error> {
        let files = self
            .files
            .keys()
            .map(|file| self.resolve(Path::new(file), limits))
            .collect::<Result<Vec<_>, _>>()?;
        let links = self
            .links
            .keys()
            .map(|path| {
                self.resolve(Path::new(path), limits)
                    .map(|target| (PathBuf::from(path), target))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut used = BTreeSet::<PathBuf>::new();
        loop {
            let next = links
                .iter()
                .filter(|(path, target)| {
                    !used.contains(path)
                        && (files.iter().any(|file| file.starts_with(target))
                            || used.iter().any(|link| link.starts_with(target)))
                })
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            if next.is_empty() {
                break;
            }
            used.extend(next);
        }
        if used.len() != links.len() {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    fn id(&self) -> String {
        let value = serde_json::to_value(self).expect("host manifest contains JSON values");
        let bytes = serde_json::to_vec(&value).expect("host manifest is serializable");
        hex::encode(
            Sha256::new()
                .chain_update(b"rustward-host-v1")
                .chain_update(bytes)
                .finalize(),
        )
    }
    fn resolve(&self, path: &Path, limits: Limits) -> Result<PathBuf, Error> {
        let mut current = path.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..=32 {
            if !seen.insert(current.clone()) {
                return Err(Error::Invalid);
            }
            let mut prefix = PathBuf::from("/");
            let mut replacement = None;
            for component in current.components().skip(1) {
                prefix.push(component);
                if let Some(target) = prefix.to_str().and_then(|p| self.links.get(p)) {
                    let target = Path::new(target);
                    let base = if target.is_absolute() {
                        target.to_owned()
                    } else {
                        prefix.parent().ok_or(Error::Invalid)?.join(target)
                    };
                    let mut normalized = PathBuf::new();
                    for component in base.components() {
                        match component {
                            Component::RootDir | Component::Normal(_) => normalized.push(component),
                            Component::CurDir => {}
                            Component::ParentDir => {
                                if !normalized.pop() {
                                    return Err(Error::Invalid);
                                }
                            }
                            _ => return Err(Error::Invalid),
                        }
                    }
                    let tail = current.strip_prefix(&prefix).map_err(|_| Error::Invalid)?;
                    if !tail.as_os_str().is_empty() {
                        normalized.push(tail);
                    }
                    if !path_valid(normalized.to_str().ok_or(Error::Invalid)?, limits) {
                        return Err(Error::Invalid);
                    }
                    replacement = Some(normalized);
                    break;
                }
            }
            match replacement {
                Some(path) => current = path,
                None => return Ok(current),
            }
        }
        Err(Error::Limit)
    }
}

pub fn manifest_id(source: &str) -> Result<String, Error> {
    Ok(Manifest::parse(source, Limits::default())?.id())
}
impl Catalog {
    pub fn open(path: impl AsRef<Path>, policy: Protection, limits: Limits) -> Result<Self, Error> {
        if !limits.valid()
            || policy.execution_uid == 0
            || policy.execution_uid == policy.preparation_uid
        {
            return Err(Error::Unsafe);
        }
        let path = path.as_ref();
        if !normal_absolute(path) {
            return Err(Error::Unsafe);
        }
        files::protected_root(path, policy)?;
        Ok(Self {
            path: path.to_owned(),
            policy,
            limits,
        })
    }
    pub fn execution_uid(&self) -> u32 {
        self.policy.execution_uid
    }
    pub fn preparation_uid(&self) -> u32 {
        self.policy.preparation_uid
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn verify(&self, id: &str) -> Result<VerifiedHost, Error> {
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let root = files::protected_root(&self.path, self.policy)?;
        let file = files::open(&root, Path::new(&format!("{id}.toml")))?;
        files::protect(&file, self.policy, false, false)?;
        let mut source = String::new();
        file.take(self.limits.manifest_bytes + 1)
            .read_to_string(&mut source)
            .map_err(|_| Error::Invalid)?;
        let manifest = Manifest::parse(&source, self.limits)?;
        if manifest.id() != id {
            return Err(Error::Changed);
        }
        verify_links(&manifest, self.policy, self.limits)?;
        let mut pinned = Vec::new();
        let mut remaining = self.limits.content_bytes;
        let mut buffer = vec![0; 64 * 1024];
        for (path, spec) in &manifest.files {
            let path = manifest.resolve(Path::new(path), self.limits)?;
            let mut file = files::open(rustix::fs::CWD, &path)?;
            protect_host(&file, &path, self.policy, false, spec.executable)?;
            let before = file.metadata().map_err(|_| Error::Unsafe)?;
            if before.len() > remaining {
                return Err(Error::Limit);
            }
            let mut hash = Sha256::new();
            loop {
                let n = match file.read(&mut buffer) {
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(Error::Changed),
                };
                if n == 0 {
                    break;
                }
                remaining = remaining.checked_sub(n as u64).ok_or(Error::Limit)?;
                hash.update(&buffer[..n]);
            }
            let after = file.metadata().map_err(|_| Error::Unsafe)?;
            pinned.push((path.clone(), before.clone()));
            if before.ctime() != after.ctime()
                || before.ctime_nsec() != after.ctime_nsec()
                || hex::encode(hash.finalize()) != spec.sha256
            {
                return Err(Error::Changed);
            }
        }
        verify_links(&manifest, self.policy, self.limits)?;
        for (path, before) in pinned {
            let file = files::open(rustix::fs::CWD, &path)?;
            let after = file.metadata().map_err(|_| Error::Unsafe)?;
            if before.dev() != after.dev()
                || before.ino() != after.ino()
                || before.ctime() != after.ctime()
                || before.ctime_nsec() != after.ctime_nsec()
            {
                return Err(Error::Changed);
            }
        }
        let current_root = files::protected_root(&self.path, self.policy)?;
        if !files::same_file(&root, &current_root)? {
            return Err(Error::Changed);
        }
        let current = files::open(&current_root, Path::new(&format!("{id}.toml")))?;
        files::protect(&current, self.policy, false, false)?;
        let mut final_source = String::new();
        current
            .take(self.limits.manifest_bytes + 1)
            .read_to_string(&mut final_source)
            .map_err(|_| Error::Invalid)?;
        if Manifest::parse(&final_source, self.limits)?.id() != id {
            return Err(Error::Changed);
        }
        Ok(VerifiedHost {
            id: id.to_owned(),
            launcher: manifest.resolve(Path::new(&manifest.launcher), self.limits)?,
        })
    }
}
fn protect_host(
    file: &File,
    path: &Path,
    policy: Protection,
    directory: bool,
    executable: bool,
) -> Result<(), Error> {
    let uid = file.metadata().map_err(|_| Error::Unsafe)?.uid();
    if uid != 0 && uid != policy.preparation_uid {
        return Err(Error::Unsafe);
    }
    files::protect(
        file,
        Protection {
            preparation_uid: uid,
            ..policy
        },
        directory,
        executable,
    )?;
    files::protect_ancestors(path, policy)
}

fn verify_links(manifest: &Manifest, policy: Protection, limits: Limits) -> Result<(), Error> {
    for (path, target) in &manifest.links {
        let link = Path::new(path);
        let parent = manifest.resolve(link.parent().ok_or(Error::Invalid)?, limits)?;
        let parent_file = files::open(rustix::fs::CWD, &parent)?;
        protect_host(&parent_file, &parent, policy, true, false)?;
        let name = link.file_name().ok_or(Error::Invalid)?;
        let stat = rustix::fs::statat(&parent_file, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Error::Unsafe)?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Symlink
            || (stat.st_uid != 0 && stat.st_uid != policy.preparation_uid)
            || stat.st_uid == policy.execution_uid
        {
            return Err(Error::Unsafe);
        }
        let actual =
            rustix::fs::readlinkat(&parent_file, name, Vec::new()).map_err(|_| Error::Unsafe)?;
        if actual.to_bytes() != target.as_bytes() {
            return Err(Error::Changed);
        }
    }

    Ok(())
}

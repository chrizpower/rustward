use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Error, Limits};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    version: u32,
    #[serde(default)]
    pub directories: Vec<String>,
    pub files: BTreeMap<String, FileSpec>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSpec {
    pub sha256: String,
    #[serde(default)]
    pub executable: bool,
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_path(path: &str, limits: Limits) -> bool {
    !path.is_empty()
        && path.len() <= limits.path_bytes
        && !path.contains(['\0', '\\'])
        && path.split('/').count() <= limits.depth
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

impl Manifest {
    pub fn parse(source: &str, limits: Limits) -> Result<Self, Error> {
        if source.len() as u64 > limits.manifest_bytes {
            return Err(Error::Limit);
        }
        let mut manifest: Self = toml::from_str(source).map_err(|_| Error::Invalid)?;
        if manifest.version != 1 || manifest.files.is_empty() {
            return Err(Error::Invalid);
        }
        if manifest
            .files
            .len()
            .checked_add(manifest.directories.len())
            .and_then(|n| n.checked_add(1))
            .is_none_or(|n| n > limits.entries)
        {
            return Err(Error::Limit);
        }
        let mut dirs = BTreeSet::new();
        for path in &manifest.directories {
            if !valid_path(path, limits) || path == "runtime.toml" || !dirs.insert(path.as_str()) {
                return Err(Error::Invalid);
            }
        }
        for (path, spec) in &manifest.files {
            if !valid_path(path, limits)
                || path == "runtime.toml"
                || dirs.contains(path.as_str())
                || !valid_id(&spec.sha256)
            {
                return Err(Error::Invalid);
            }
        }
        for path in manifest.directories.iter().chain(manifest.files.keys()) {
            for (offset, _) in path.match_indices('/') {
                if !dirs.contains(&path[..offset]) {
                    return Err(Error::Invalid);
                }
            }
        }
        manifest.directories.sort();
        Ok(manifest)
    }

    pub fn id(&self) -> String {
        let normalized =
            serde_json::to_value(self).expect("runtime manifest contains only JSON values");
        let bytes =
            serde_json::to_vec(&normalized).expect("normalized runtime manifest is serializable");
        hex::encode(
            Sha256::new()
                .chain_update(b"rustward-runtime-v1")
                .chain_update(bytes)
                .finalize(),
        )
    }
}

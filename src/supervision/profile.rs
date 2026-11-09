use super::Error;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    uid: u32,
    launcher: String,
    runtime: String,
    environment: BTreeMap<String, String>,
}
impl Profile {
    pub fn for_run(uid: u32, runtime: &Path, work: &Path) -> Result<Self, Error> {
        Self::with_launcher(uid, runtime, work, Path::new("/usr/libexec/rustward-exec"))
    }
    pub fn with_launcher(
        uid: u32,
        runtime: &Path,
        work: &Path,
        launcher: &Path,
    ) -> Result<Self, Error> {
        if [runtime, work, launcher]
            .iter()
            .any(|path| path.as_os_str().as_encoded_bytes().len() > 4096)
            || !crate::runtime::normal_absolute(launcher)
            || launcher.to_str().is_none_or(|path| path.contains('\0'))
        {
            return Err(Error::InvalidRequest);
        }
        if uid == 0
            || !crate::runtime::normal_absolute(runtime)
            || !crate::runtime::normal_absolute(work)
            || runtime.as_os_str().as_encoded_bytes().contains(&b':')
        {
            return Err(Error::InvalidRequest);
        }
        let runtime = runtime.to_str().ok_or(Error::InvalidRequest)?;
        let work = work.to_str().ok_or(Error::InvalidRequest)?;
        if runtime.contains('\0') || work.contains('\0') {
            return Err(Error::InvalidRequest);
        }
        let environment = BTreeMap::from([
            ("PATH".into(), format!("{runtime}/bin")),
            ("HOME".into(), work.into()),
            ("TMPDIR".into(), work.into()),
            ("RUSTWARD_WORK_DIRECTORY".into(), work.into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("TZ".into(), "UTC".into()),
        ]);
        Ok(Self {
            uid,
            environment,
            launcher: launcher.to_str().unwrap().into(),
            runtime: runtime.into(),
        })
    }
    pub fn launcher(&self) -> &str {
        &self.launcher
    }
    pub fn runtime_directory(&self) -> &str {
        &self.runtime
    }
    pub fn work_directory(&self) -> &str {
        &self.environment["HOME"]
    }
    pub fn execution_uid(&self) -> u32 {
        self.uid
    }
    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

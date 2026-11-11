use super::{Result, require};
use rustward::{
    controller::{Controller, Roots},
    dispatch::Limits,
    runtime::{self, WorkRoot},
    safety::Safety,
    systemd::{CgroupFs, Manager, Systemd, UserBus},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub type Native = Controller<Systemd<UserBus, CgroupFs>>;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub version: u32,
    pub runtime_id: String,
    pub host_id: String,
    pub preparation_uid: u32,
    pub execution_uid: u32,
    pub source_revision: String,
    pub packages: String,
    pub python_version: String,
    pub ansible_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_b_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_limit: Option<u64>,
}
pub struct Context {
    pub root: PathBuf,
    pub fixture: Fixture,
}
impl Context {
    pub fn load(root: &Path) -> Result<Self> {
        require(root.is_absolute(), "fixture root must be absolute")?;
        let fixture: Fixture = serde_json::from_slice(&fs::read(root.join("fixture.json"))?)?;
        require(fixture.version == 1, "unsupported fixture version")?;
        require(
            fixture
                .catalog_root
                .as_ref()
                .is_none_or(|path| path.is_absolute()),
            "catalog root must be absolute",
        )?;
        require(
            fixture.execution_uid != 0 && fixture.preparation_uid != fixture.execution_uid,
            "invalid fixture ownership policy",
        )?;
        require(
            rustix::process::geteuid().as_raw() == fixture.execution_uid,
            "run acceptance as the fixture execution user",
        )?;
        Ok(Self {
            root: root.to_owned(),
            fixture,
        })
    }
    pub fn database(&self) -> PathBuf {
        self.root.join("state/state.db")
    }
    pub fn work(&self, id: &str) -> PathBuf {
        self.root.join("work").join(id)
    }
    fn catalog_root(&self) -> &Path {
        self.fixture.catalog_root.as_deref().unwrap_or(&self.root)
    }
    pub fn runtime(&self) -> PathBuf {
        self.catalog_root()
            .join("runtimes")
            .join(&self.fixture.runtime_id)
    }
    pub fn evidence(&self, name: &str, value: &impl Serialize) -> Result<()> {
        let path = self.root.join("evidence").join(name);
        let pending = path.with_extension("pending");
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        use std::io::Write;
        output.write_all(&serde_json::to_vec_pretty(value)?)?;
        output.sync_all()?;
        fs::hard_link(&pending, &path)?;
        fs::remove_file(pending)?;
        fs::File::open(path.parent().unwrap())?.sync_all()?;
        Ok(())
    }
    pub fn controller(&self, safety: Safety) -> Result<Native> {
        let policy = runtime::Protection {
            preparation_uid: self.fixture.preparation_uid,
            execution_uid: self.fixture.execution_uid,
        };
        let limits = runtime::Limits::default();
        let roots = Roots {
            runtimes: runtime::Catalog::open(self.catalog_root().join("runtimes"), policy, limits)
                .map_err(|e| format!("runtime catalog: {e:?}"))?,
            hosts: runtime::host::Catalog::open(self.catalog_root().join("hosts"), policy, limits)
                .map_err(|e| format!("host catalog: {e:?}"))?,
            work: WorkRoot::open(self.root.join("work"))
                .map_err(|e| format!("work root: {e:?}"))?,
            logs: WorkRoot::open(self.root.join("logs")).map_err(|e| format!("log root: {e:?}"))?,
        };
        let supervisor = native_supervisor()?;
        Controller::open(
            self.database(),
            roots,
            Limits {
                log_bytes_per_run: self
                    .fixture
                    .capture_limit
                    .unwrap_or(Limits::default().log_bytes_per_run),
                ..Limits::default()
            },
            supervisor,
            safety,
        )
        .map_err(|e| format!("controller: {e:?}").into())
    }
}

pub(super) fn native_supervisor() -> Result<Systemd<UserBus, CgroupFs>> {
    let (bus, _) = manager()?;
    Systemd::new(bus, CgroupFs::default()).map_err(|e| format!("systemd adapter: {e:?}").into())
}

fn validate_manager(uid: u32, expected: u32, version: &str) -> Result<()> {
    require(
        expected != 0 && uid == expected,
        "manager UID does not match the execution user",
    )?;
    require(
        version.split('.').next() == Some("257"),
        "acceptance requires systemd 257",
    )
}
fn manager() -> Result<(UserBus, serde_json::Value)> {
    let mut bus =
        UserBus::connect().map_err(|error| format!("authenticated user bus: {error:?}"))?;
    let uid = bus
        .authenticated_uid()
        .map_err(|error| format!("manager UID: {error:?}"))?;
    let version = bus
        .version()
        .map_err(|error| format!("manager version: {error:?}"))?;
    validate_manager(uid, rustix::process::geteuid().as_raw(), &version)?;
    let result = serde_json::json!({"status":"ready","uid":uid,"version":version});
    Ok((bus, result))
}
pub fn probe_manager() -> Result<serde_json::Value> {
    Ok(manager()?.1)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_requires_the_execution_uid_and_supported_manager() {
        validate_manager(1000, 1000, "257.13-1~deb13u1").unwrap();
        for (uid, expected, version) in [
            (0, 0, "257"),
            (1001, 1000, "257"),
            (1000, 1000, "256"),
            (1000, 1000, "258"),
            (1000, 1000, ""),
        ] {
            assert!(validate_manager(uid, expected, version).is_err());
        }
    }
}

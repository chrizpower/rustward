#![cfg(target_os = "linux")]

use rustward::{supervision::*, systemd::*};
use std::{fs::File, os::fd::AsFd, path::Path, time::Duration};
use zbus::zvariant::Value;
struct ManagerFake {
    uid: u32,
    names: Vec<String>,
    calls: usize,
}
impl Manager for ManagerFake {
    fn version(&mut self) -> Result<String, ManagerError> {
        Ok("257".into())
    }
    fn authenticated_uid(&mut self) -> Result<u32, ManagerError> {
        Ok(self.uid)
    }
    fn environment_names(&mut self) -> Result<Vec<String>, ManagerError> {
        Ok(self.names.clone())
    }
    fn start(&mut self, _: &str, properties: &[(&str, Value<'_>)]) -> Result<(), ManagerError> {
        let get = |name| &properties.iter().find(|(key, _)| *key == name).unwrap().1;
        assert_eq!(get("NoNewPrivileges"), &Value::from(true));
        assert_eq!(get("CapabilityBoundingSet"), &Value::from(0u64));
        assert_eq!(get("AmbientCapabilities"), &Value::from(0u64));
        assert_eq!(
            get("ExecStartEx"),
            &Value::new(vec![(
                "/pinned/launcher",
                vec![
                    "/pinned/launcher",
                    "/runtime=a",
                    "/work dir",
                    "/runtime=a/bin/task=b",
                    "a=b",
                    "two words",
                    "$HOME %i; *",
                    ""
                ],
                vec!["no-env-expand"]
            )])
        );
        let unset = get("UnsetEnvironment").try_clone().unwrap();
        let names: Vec<String> = unset.try_into().unwrap();
        assert!(names.contains(&"SYNTHETIC_INHERITED".into()));
        assert!(names.contains(&"LD_PRELOAD".into()));
        assert!(names.contains(&"GLIBC_TUNABLES".into()));
        self.calls += 1;
        Ok(())
    }
    fn snapshot(&mut self, _: &str) -> Result<Snapshot, ManagerError> {
        unreachable!()
    }
    fn stop(&mut self, _: &str, _: StopMode) -> Result<(), ManagerError> {
        unreachable!()
    }
    fn release(&mut self, _: &str) -> Result<(), ManagerError> {
        unreachable!()
    }
}
struct Groups;
impl Cgroups for Groups {
    fn state(&mut self, _: &str) -> GroupState {
        unreachable!()
    }
}
#[test]
fn controlled_start_pins_uid_drops_privileges_and_preserves_command_arguments() {
    let profile = Profile::with_launcher(
        1000,
        Path::new("/runtime=a"),
        Path::new("/work dir"),
        Path::new("/pinned/launcher"),
    )
    .unwrap();
    let launch = Launch {
        run_id: RunId::new(&"1".repeat(64)).unwrap(),
        executable: "/runtime=a/bin/task=b".into(),
        working_directory: "/runtime=a".into(),
        arguments: vec![
            "a=b".into(),
            "two words".into(),
            "$HOME %i; *".into(),
            "".into(),
        ],
        cleanup_grace: Duration::from_secs(30),
    };
    let file = File::open("/dev/null").unwrap();
    for (uid, names, expected) in [
        (
            1000,
            vec!["SYNTHETIC_INHERITED".into()],
            Ok(StartOutcome::Accepted),
        ),
        (1001, vec![], Err(Error::IdentityMismatch)),
        (1000, vec!["bad=name".into()], Err(Error::InvalidRequest)),
        (1000, vec!["".into()], Err(Error::InvalidRequest)),
        (1000, vec!["a".into(); 4097], Err(Error::InvalidRequest)),
    ] {
        let mut adapter = Systemd::new(
            ManagerFake {
                uid,
                names,
                calls: 0,
            },
            Groups,
        )
        .unwrap();
        assert_eq!(
            adapter.start_controlled(
                &launch,
                &profile,
                Output {
                    stdout: file.as_fd(),
                    stderr: file.as_fd()
                }
            ),
            expected
        );
    }
}

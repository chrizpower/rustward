use rustward::{supervision::*, systemd::*};
use std::{collections::VecDeque, fs::File, os::fd::AsFd, time::Duration};
use zbus::zvariant::Value;

struct FakeManager {
    version: Result<String, ManagerError>,
    replies: VecDeque<Result<Snapshot, ManagerError>>,
    response: Result<(), ManagerError>,
    calls: Vec<String>,
}
impl Manager for FakeManager {
    fn reference(&mut self, _: &str) -> Result<(), ManagerError> {
        self.calls.push("reference".into());
        self.response
    }
    fn version(&mut self) -> Result<String, ManagerError> {
        self.version.clone()
    }
    fn start(&mut self, name: &str, properties: &[(&str, Value<'_>)]) -> Result<(), ManagerError> {
        assert_eq!(name, format!("rustward-run-{}.service", "a".repeat(64)));
        let get = |key| &properties.iter().find(|(k, _)| *k == key).unwrap().1;
        assert_eq!(get("Type"), &Value::from("exec"));
        assert_eq!(get("Restart"), &Value::from("no"));
        assert_eq!(get("KillMode"), &Value::from("control-group"));
        assert_eq!(get("RemainAfterExit"), &Value::from(true));
        assert_eq!(get("AddRef"), &Value::from(true));
        assert_eq!(get("SendSIGKILL"), &Value::from(true));
        assert_eq!(get("TimeoutStopUSec"), &Value::from(30_000_000u64));
        assert_eq!(get("WorkingDirectory"), &Value::from("/synthetic/work"));
        assert_eq!(get("StandardInput"), &Value::from("null"));
        assert_eq!(
            get("ExecStartEx"),
            &Value::new(vec![(
                "/synthetic/program",
                vec!["/synthetic/program", "two words", "$HOME %i; *", ""],
                vec!["no-env-expand"]
            )])
        );
        assert!(matches!(get("StandardOutputFileDescriptor"), Value::Fd(_)));
        assert!(matches!(get("StandardErrorFileDescriptor"), Value::Fd(_)));
        assert!(self.calls.is_empty());
        self.calls.push("start".into());
        self.response
    }
    fn snapshot(&mut self, _: &str) -> Result<Snapshot, ManagerError> {
        self.replies.pop_front().unwrap()
    }
    fn stop(&mut self, _: &str, mode: StopMode) -> Result<(), ManagerError> {
        self.calls.push(format!("stop:{mode:?}"));
        self.response
    }
    fn release(&mut self, _: &str) -> Result<(), ManagerError> {
        self.calls.push("release".into());
        self.response
    }
}
struct Groups(GroupState);
impl Cgroups for Groups {
    fn state(&mut self, _: &str) -> GroupState {
        self.0
    }
}
fn id() -> RunId {
    RunId::new(&"a".repeat(64)).unwrap()
}
fn identity() -> ExecutionIdentity {
    ExecutionIdentity {
        invocation: [1; 16],
        control_group: "/user.slice/run.service".into(),
    }
}
fn snapshot() -> Snapshot {
    Snapshot {
        description: format!("Rustward run {}", id().as_str()),
        transient: true,
        service_type: "exec".into(),
        invocation: vec![1; 16],
        control_group: identity().control_group,
        active_state: "active".into(),
        sub_state: "running".into(),
        job: 0,
        main_pid: 42,
        exit_timestamp: 0,
        exit_code: 0,
        exit_status: 0,
    }
}
fn manager(
    replies: Vec<Result<Snapshot, ManagerError>>,
    response: Result<(), ManagerError>,
) -> FakeManager {
    FakeManager {
        version: Ok("257.13-1~deb13u1".into()),
        replies: replies.into(),
        response,
        calls: vec![],
    }
}
fn adapter(snapshot: Snapshot, group: GroupState) -> Systemd<FakeManager, Groups> {
    Systemd::new(
        manager(vec![Ok(snapshot.clone()), Ok(snapshot)], Ok(())),
        Groups(group),
    )
    .unwrap()
}
fn launch() -> Launch {
    Launch {
        run_id: id(),
        executable: "/synthetic/program".into(),
        arguments: vec!["two words".into(), "$HOME %i; *".into(), "".into()],
        working_directory: "/synthetic/work".into(),
        cleanup_grace: Duration::from_secs(30),
    }
}

#[test]
fn validates_ids_versions_and_requests_before_start() {
    for invalid in [
        "".into(),
        "a".repeat(63),
        "A".repeat(64),
        "g".repeat(64),
        "../".repeat(22),
    ] {
        assert_eq!(RunId::new(&invalid), Err(Error::InvalidRequest));
    }
    for version in ["256", "", "garbled"] {
        let mut mgr = manager(vec![], Ok(()));
        mgr.version = Ok(version.into());
        assert!(matches!(
            Systemd::new(mgr, Groups(GroupState::Empty)),
            Err(Error::IncompatibleManager)
        ));
    }
    let mut mgr = manager(vec![], Ok(()));
    mgr.version = Err(ManagerError::Unavailable);
    assert!(matches!(
        Systemd::new(mgr, Groups(GroupState::Empty)),
        Err(Error::Unavailable)
    ));
    for change in 0..8 {
        let mut request = launch();
        match change {
            0 => request.executable = "relative".into(),
            1 => request.working_directory = "bad\0path".into(),
            2 => request.arguments.push("bad\0argument".into()),
            3 => request.arguments.push("x".repeat(1024 * 1024)),
            4 => request.cleanup_grace = Duration::ZERO,
            5 => request.cleanup_grace = Duration::MAX,
            6 => request.arguments = vec![String::new(); 65537],
            _ => request.cleanup_grace = Duration::from_nanos(1),
        }
        let mut adapter = Systemd::new(manager(vec![], Ok(())), Groups(GroupState::Empty)).unwrap();
        let file = File::open("/dev/null").unwrap();
        assert_eq!(
            adapter.start(
                &request,
                Output {
                    stdout: file.as_fd(),
                    stderr: file.as_fd()
                }
            ),
            Err(Error::InvalidRequest)
        );
    }
}

#[test]
fn start_reports_job_acceptance_and_never_retries_a_lost_reply() {
    for (response, expected) in [
        (Ok(()), StartOutcome::Accepted),
        (Err(ManagerError::Rejected), StartOutcome::Rejected),
        (Err(ManagerError::Conflict), StartOutcome::Conflict),
        (Err(ManagerError::Unavailable), StartOutcome::Uncertain),
    ] {
        let mut adapter =
            Systemd::new(manager(vec![], response), Groups(GroupState::Empty)).unwrap();
        let file = File::open("/dev/null").unwrap();
        assert_eq!(
            adapter.start(
                &launch(),
                Output {
                    stdout: file.as_fd(),
                    stderr: file.as_fd()
                }
            ),
            Ok(expected)
        );
    }
}

#[test]
fn observes_execution_without_treating_pid_absence_as_cleanup() {
    for (mut raw, expected) in [
        (snapshot(), ExecutionState::Running),
        (snapshot(), ExecutionState::Starting),
        (snapshot(), ExecutionState::Exited),
        (snapshot(), ExecutionState::Ended),
    ] {
        match expected {
            ExecutionState::Starting => raw.active_state = "activating".into(),
            ExecutionState::Exited => {
                raw.sub_state = "exited".into();
                raw.exit_timestamp = 1;
                raw.exit_code = 1;
                raw.exit_status = 0;
                raw.main_pid = 0;
            }
            ExecutionState::Ended => {
                raw.active_state = "inactive".into();
                raw.main_pid = 0;
            }
            _ => {}
        }
        let mut adapter = adapter(raw, GroupState::Populated);
        let observed = adapter.observe(&id(), None).unwrap();
        assert!(
            matches!(observed, Observation::Known { state, cleanup: Cleanup::Pending, .. } if state == expected)
        );
    }
    for group in [
        GroupState::Empty,
        GroupState::Absent,
        GroupState::Populated,
        GroupState::Unknown,
    ] {
        let mut raw = snapshot();
        raw.active_state = "failed".into();
        raw.main_pid = 0;
        raw.exit_timestamp = 2;
        raw.exit_code = 2;
        raw.exit_status = 15;
        let mut adapter = adapter(raw, group);
        let cleanup = if matches!(group, GroupState::Empty | GroupState::Absent) {
            Cleanup::Confirmed
        } else {
            Cleanup::Pending
        };
        assert_eq!(
            adapter.observe(&id(), Some(&identity())).unwrap(),
            Observation::Known {
                identity: identity(),
                state: ExecutionState::Ended,
                exit: Some(ProcessExit::Signal(15)),
                cleanup
            }
        );
    }
    let mut raw = snapshot();
    raw.active_state = "inactive".into();
    raw.control_group.clear();
    raw.job = 1;
    assert!(matches!(
        adapter(raw, GroupState::Empty)
            .observe(&id(), Some(&identity()))
            .unwrap(),
        Observation::Known {
            cleanup: Cleanup::Pending,
            ..
        }
    ));
}

#[test]
fn reports_missing_changed_or_inconsistent_evidence_conservatively() {
    for (error, expected) in [
        (ManagerError::Missing, Ok(Observation::Missing)),
        (ManagerError::Unavailable, Err(Error::Unavailable)),
    ] {
        let mut adapter =
            Systemd::new(manager(vec![Err(error)], Ok(())), Groups(GroupState::Empty)).unwrap();
        assert_eq!(adapter.observe(&id(), None), expected);
    }
    for mutation in 0..11 {
        let mut raw = snapshot();
        match mutation {
            0 => raw.description = "other".into(),
            1 => raw.transient = false,
            2 => raw.service_type = "simple".into(),
            3 => raw.invocation = vec![0; 16],
            4 => raw.invocation = vec![],
            5 => raw.control_group = "/../wrong".into(),
            6 => raw.active_state = "reloading".into(),
            7 => raw.main_pid = 0,
            8 => raw.invocation = vec![2; 16],
            9 => raw.control_group = "/other".into(),
            _ => raw.active_state = "deactivating".into(),
        }
        let expected = if matches!(mutation, 0..=2 | 8..=9) {
            Err(Error::IdentityMismatch)
        } else if mutation == 10 {
            Ok(Observation::Known {
                identity: identity(),
                state: ExecutionState::Stopping,
                exit: None,
                cleanup: Cleanup::Pending,
            })
        } else {
            Ok(Observation::Uncertain)
        };
        let prior = if mutation >= 8 {
            Some(identity())
        } else {
            None
        };
        assert_eq!(
            adapter(raw, GroupState::Empty).observe(&id(), prior.as_ref()),
            expected
        );
    }
    let raw = snapshot();
    let mut changed = raw.clone();
    changed.main_pid = 0;
    for after in [Ok(changed), Err(ManagerError::Missing)] {
        let mut adapter = Systemd::new(
            manager(vec![Ok(raw.clone()), after], Ok(())),
            Groups(GroupState::Empty),
        )
        .unwrap();
        assert_eq!(adapter.observe(&id(), None), Ok(Observation::Uncertain));
    }
}

#[test]
fn stop_and_release_require_matching_identity_and_cleanup_evidence() {
    for mode in [StopMode::Graceful, StopMode::Force] {
        for (response, expected) in [
            (Ok(()), StopOutcome::Accepted),
            (Err(ManagerError::Missing), StopOutcome::Missing),
            (Err(ManagerError::Unavailable), StopOutcome::Uncertain),
        ] {
            let raw = snapshot();
            let mut adapter = Systemd::new(
                manager(vec![Ok(raw.clone()), Ok(raw)], response),
                Groups(GroupState::Empty),
            )
            .unwrap();
            assert_eq!(adapter.stop(&id(), &identity(), mode), Ok(expected));
        }
    }
    let mut missing = Systemd::new(
        manager(vec![Err(ManagerError::Missing)], Ok(())),
        Groups(GroupState::Empty),
    )
    .unwrap();
    assert_eq!(
        missing.stop(&id(), &identity(), StopMode::Graceful),
        Ok(StopOutcome::Missing)
    );
    let mut raw = snapshot();
    raw.invocation = vec![0; 16];
    assert_eq!(
        adapter(raw, GroupState::Empty).stop(&id(), &identity(), StopMode::Force),
        Ok(StopOutcome::Uncertain)
    );
    assert_eq!(
        adapter(snapshot(), GroupState::Empty).release(&id(), &identity()),
        Err(Error::CleanupPending)
    );
    for response in [Ok(()), Err(ManagerError::Unavailable)] {
        let mut raw = snapshot();
        raw.active_state = "inactive".into();
        raw.main_pid = 0;
        let mut adapter = Systemd::new(
            manager(vec![Ok(raw.clone()), Ok(raw)], response),
            Groups(GroupState::Empty),
        )
        .unwrap();
        assert_eq!(
            adapter.release(&id(), &identity()),
            response.map_err(|_| Error::Unavailable)
        );
    }
}

#[test]
fn references_recovered_units_without_a_start_and_rejects_changed_identity() {
    let mut native = adapter(snapshot(), GroupState::Populated);
    assert_eq!(native.reference(&id(), Some(&identity())), Ok(()));
    let mut changed = snapshot();
    changed.invocation = vec![2; 16];
    let mut native = Systemd::new(
        manager(vec![Ok(snapshot()), Ok(changed)], Ok(())),
        Groups(GroupState::Populated),
    )
    .unwrap();
    assert_eq!(
        native.reference(&id(), Some(&identity())),
        Err(Error::IdentityMismatch)
    );
    for response in [Err(ManagerError::Missing), Err(ManagerError::Unavailable)] {
        let mut native = Systemd::new(
            manager(vec![response], Ok(())),
            Groups(GroupState::Populated),
        )
        .unwrap();
        assert_eq!(
            native.reference(&id(), Some(&identity())),
            Err(Error::Unavailable)
        );
    }
    let mut native = Systemd::new(
        manager(vec![Ok(snapshot())], Err(ManagerError::Rejected)),
        Groups(GroupState::Populated),
    )
    .unwrap();
    assert_eq!(
        native.reference(&id(), Some(&identity())),
        Err(Error::Unavailable)
    );
}

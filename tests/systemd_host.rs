#![cfg(target_os = "linux")]

use rustward::{
    config::RunResources,
    supervision::*,
    systemd::{CgroupFs, Manager, Systemd, UserBus},
};
use std::{
    fs::{self, File},
    os::fd::AsFd,
    path::Path,
    thread,
    time::{Duration, Instant},
};

struct TestUnits(Vec<String>);
impl Drop for TestUnits {
    fn drop(&mut self) {
        if let Ok(mut bus) = UserBus::connect() {
            for unit in &self.0 {
                let _ = bus.stop(unit, StopMode::Graceful);
            }
        }
    }
}

fn wait_for(
    supervisor: &mut impl Supervisor,
    run_id: &RunId,
    expected: Option<&ExecutionIdentity>,
    accept: impl Fn(&Observation) -> bool,
) -> Observation {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let observed = supervisor.observe(run_id, expected).unwrap();
        if accept(&observed) {
            return observed;
        }
        assert!(
            Instant::now() < deadline,
            "observation deadline: {observed:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires an isolated Debian 13 host with a dedicated systemd user manager and cgroup v2"]
fn confirms_child_cleanup_and_concurrent_unit_isolation() {
    let manager = UserBus::connect().expect("connect to the authenticated user manager");
    let mut supervisor = Systemd::new(manager, CgroupFs::default())
        .expect("verify the systemd baseline")
        .with_run_resources(RunResources {
            memory_max_bytes: 67_108_864_u64.try_into().ok(),
            tasks_max: 32_u32.try_into().ok(),
            cpu_quota_percent: 50_u32.try_into().ok(),
        });
    let directory = tempfile::tempdir().unwrap();
    let output = tempfile::tempfile().unwrap();
    let error = File::options().write(true).open("/dev/null").unwrap();
    let mut cleanup = TestUnits(vec![]);
    let mut ids = vec![];
    for script in ["sleep 120 >/dev/null 2>&1 & printf synthetic", "sleep 120"] {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).unwrap();
        let run_id = RunId::new(&hex::encode(bytes)).unwrap();
        cleanup
            .0
            .push(format!("rustward-run-{}.service", run_id.as_str()));
        let launch = Launch {
            run_id: run_id.clone(),
            executable: "/bin/sh".into(),
            arguments: vec!["-c".into(), script.into()],
            working_directory: directory.path().to_str().unwrap().into(),
            cleanup_grace: Duration::from_millis(100),
        };
        assert_eq!(
            supervisor.start(
                &launch,
                Output {
                    stdout: output.as_fd(),
                    stderr: error.as_fd()
                }
            ),
            Ok(StartOutcome::Accepted)
        );
        ids.push(run_id);
    }
    let exited = wait_for(&mut supervisor, &ids[0], None, |observation| {
        matches!(
            observation,
            Observation::Known {
                state: ExecutionState::Exited,
                exit: Some(ProcessExit::Code(0)),
                cleanup: Cleanup::Pending,
                ..
            }
        )
    });
    let Observation::Known {
        identity: first, ..
    } = exited
    else {
        unreachable!()
    };
    let running = wait_for(&mut supervisor, &ids[1], None, |observation| {
        matches!(
            observation,
            Observation::Known {
                state: ExecutionState::Running,
                ..
            }
        )
    });
    let Observation::Known {
        identity: second, ..
    } = running
    else {
        unreachable!()
    };
    assert_ne!(first.invocation, second.invocation);
    assert_ne!(first.control_group, second.control_group);
    let group = Path::new("/sys/fs/cgroup").join(second.control_group.trim_start_matches('/'));
    let limit = |name| fs::read_to_string(group.join(name)).unwrap();
    assert_eq!(limit("memory.max").trim(), "67108864");
    assert_eq!(limit("pids.max").trim(), "32");
    let cpu: Vec<u64> = limit("cpu.max")
        .split_whitespace()
        .map(|field| field.parse().unwrap())
        .collect();
    assert_eq!(cpu.len(), 2);
    assert_eq!(cpu[0] * 2, cpu[1]);
    assert_eq!(
        supervisor.stop(&ids[0], &first, StopMode::Graceful),
        Ok(StopOutcome::Accepted)
    );
    wait_for(&mut supervisor, &ids[0], Some(&first), |observation| {
        matches!(
            observation,
            Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            }
        )
    });
    assert!(matches!(
        supervisor.observe(&ids[1], Some(&second)).unwrap(),
        Observation::Known {
            state: ExecutionState::Running,
            ..
        }
    ));
    supervisor.release(&ids[0], &first).unwrap();
    assert_eq!(
        supervisor.stop(&ids[1], &second, StopMode::Graceful),
        Ok(StopOutcome::Accepted)
    );
    wait_for(&mut supervisor, &ids[1], Some(&second), |observation| {
        matches!(
            observation,
            Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            }
        )
    });
    supervisor.release(&ids[1], &second).unwrap();
}

#[test]
#[ignore = "requires an isolated Debian 13 host with a dedicated systemd user manager and cgroup v2"]
fn captures_both_streams_through_the_user_manager() {
    use rustward::capture::{Capture, Options};
    let manager = UserBus::connect().expect("connect to the authenticated user manager");
    let mut supervisor =
        Systemd::new(manager, CgroupFs::default()).expect("verify the systemd baseline");
    let root = tempfile::tempdir().unwrap();
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).unwrap();
    let run_id = RunId::new(&hex::encode(bytes)).unwrap();
    let _cleanup = TestUnits(vec![format!("rustward-run-{}.service", run_id.as_str())]);
    let (capture, writers) =
        Capture::create(root.path(), &run_id, Options { retained_bytes: 5 }).unwrap();
    let launch = Launch {
        run_id: run_id.clone(),
        executable: "/bin/sh".into(),
        arguments: vec![
            "-c".into(),
            "printf stdout-data; printf stderr-data >&2; exec sleep 120".into(),
        ],
        working_directory: root.path().to_str().unwrap().into(),
        cleanup_grace: Duration::from_millis(100),
    };
    assert_eq!(
        supervisor.start(&launch, writers.output()),
        Ok(StartOutcome::Accepted)
    );
    drop(writers);
    let observed = wait_for(&mut supervisor, &run_id, None, |observation| {
        matches!(
            observation,
            Observation::Known {
                state: ExecutionState::Running,
                ..
            }
        )
    });
    let Observation::Known { identity, .. } = observed else {
        unreachable!()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let report = capture.report();
        assert_eq!(capture.failure(), None);
        if report.stdout.observed_bytes == 11 && report.stderr.observed_bytes == 11 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "capture observation deadline: {report:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        supervisor.stop(&run_id, &identity, StopMode::Graceful),
        Ok(StopOutcome::Accepted)
    );
    wait_for(&mut supervisor, &run_id, Some(&identity), |observation| {
        matches!(
            observation,
            Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            }
        )
    });
    supervisor.release(&run_id, &identity).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(capture.report().stdout.eof && capture.report().stderr.eof) {
        assert!(Instant::now() < deadline, "capture EOF deadline");
        thread::sleep(Duration::from_millis(20));
    }
    let report = capture.finish().unwrap();
    assert_eq!(report.retained_bytes(), 5);
    assert!(report.truncated());
}

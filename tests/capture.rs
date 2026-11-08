use rustward::{capture::*, supervision::*};
use std::{
    fs,
    io::Write,
    os::{fd::OwnedFd, unix::fs::PermissionsExt},
    thread,
    time::{Duration, Instant},
};

fn id() -> RunId {
    RunId::new(&"a".repeat(64)).unwrap()
}
fn wait_for(capture: &Capture, condition: impl Fn(Report) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition(capture.report()) {
        assert!(Instant::now() < deadline, "capture counter deadline");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn creates_protected_logs_and_does_not_overwrite_existing_evidence() {
    assert_eq!(Options::default().retained_bytes, 10 * 1024 * 1024);
    let root = tempfile::tempdir().unwrap();
    let (capture, output) = Capture::create(root.path(), &id(), Options::default()).unwrap();
    let directory = root.path().join(id().as_str());
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["stdout.log", "stderr.log"] {
        assert_eq!(
            fs::metadata(directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(output);
    let report = capture.finish().unwrap();
    assert_eq!(report.retained_bytes(), 0);
    assert!(!report.truncated());
    assert!(report.stdout.eof && report.stderr.eof);
    fs::write(directory.join("stdout.log"), b"synthetic retained evidence").unwrap();
    assert!(matches!(
        Capture::create(root.path(), &id(), Options::default()),
        Err(SetupError::AlreadyExists)
    ));
    assert_eq!(
        fs::read(directory.join("stdout.log")).unwrap(),
        b"synthetic retained evidence"
    );
    assert!(matches!(
        Capture::create(&root.path().join("missing"), &id(), Options::default()),
        Err(SetupError::Storage)
    ));
}

#[test]
fn shares_one_budget_and_keeps_binary_stream_prefixes() {
    let root = tempfile::tempdir().unwrap();
    let (capture, mut output) =
        Capture::create(root.path(), &id(), Options { retained_bytes: 5 }).unwrap();
    output.stdout.write_all(b"a\0\xff\n123456").unwrap();
    wait_for(&capture, |r| {
        r.stdout.observed_bytes == 10 && r.stdout.retained_bytes == 5
    });
    output.stderr.write_all(b"stderr data").unwrap();
    drop(output);
    let report = capture.finish().unwrap();
    assert_eq!(report.retained_bytes(), 5);
    assert_eq!(report.stdout.observed_bytes, 10);
    assert_eq!(report.stderr.observed_bytes, 11);
    assert!(report.stdout.truncated && report.stderr.truncated);
    assert_eq!(
        fs::read(root.path().join(id().as_str()).join("stdout.log")).unwrap(),
        b"a\0\xff\n1"
    );
    assert!(
        fs::read(root.path().join(id().as_str()).join("stderr.log"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exact_limit_and_empty_output_are_not_truncation() {
    for (limit, data, truncated) in [
        (4, b"data".as_slice(), false),
        (0, b"".as_slice(), false),
        (0, b"data".as_slice(), true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (capture, mut output) = Capture::create(
            root.path(),
            &id(),
            Options {
                retained_bytes: limit,
            },
        )
        .unwrap();
        output.stderr.write_all(data).unwrap();
        drop(output);
        let report = capture.finish().unwrap();
        assert_eq!(report.retained_bytes(), limit.min(data.len() as u64));
        assert_eq!(report.truncated(), truncated);
        assert_eq!(report.stderr.observed_bytes, data.len() as u64);
    }
}

#[test]
fn drains_concurrent_flooding_after_the_shared_limit() {
    let root = tempfile::tempdir().unwrap();
    let (capture, OutputWriters { stdout, stderr }) = Capture::create(
        root.path(),
        &id(),
        Options {
            retained_bytes: 8191,
        },
    )
    .unwrap();
    let writer = |mut stream: std::os::unix::net::UnixStream, byte| {
        thread::spawn(move || {
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            for _ in 0..512 {
                stream.write_all(&[byte; 4096]).unwrap();
            }
        })
    };
    let out = writer(stdout, b'o');
    let err = writer(stderr, b'e');
    out.join().unwrap();
    err.join().unwrap();
    let report = capture.finish().unwrap();
    assert_eq!(report.stdout.observed_bytes, 2 * 1024 * 1024);
    assert_eq!(report.stderr.observed_bytes, 2 * 1024 * 1024);
    assert_eq!(report.retained_bytes(), 8191);
    assert!(report.truncated());
    let directory = root.path().join(id().as_str());
    assert_eq!(
        fs::metadata(directory.join("stdout.log")).unwrap().len()
            + fs::metadata(directory.join("stderr.log")).unwrap().len(),
        8191
    );
    assert!(
        fs::read(directory.join("stdout.log"))
            .unwrap()
            .iter()
            .all(|b| *b == b'o')
    );
    assert!(
        fs::read(directory.join("stderr.log"))
            .unwrap()
            .iter()
            .all(|b| *b == b'e')
    );
}

#[test]
fn output_descriptors_work_with_a_replaceable_supervisor() {
    struct FakeSupervisor;
    impl Supervisor for FakeSupervisor {
        fn start(&mut self, _: &Launch, output: Output<'_>) -> Result<StartOutcome, Error> {
            let out: OwnedFd = output.stdout.try_clone_to_owned().unwrap();
            let err: OwnedFd = output.stderr.try_clone_to_owned().unwrap();
            fs::File::from(out).write_all(b"out").unwrap();
            fs::File::from(err).write_all(b"err").unwrap();
            Ok(StartOutcome::Accepted)
        }
        fn observe(
            &mut self,
            _: &RunId,
            _: Option<&ExecutionIdentity>,
        ) -> Result<Observation, Error> {
            Err(Error::Unavailable)
        }
        fn stop(
            &mut self,
            _: &RunId,
            _: &ExecutionIdentity,
            _: StopMode,
        ) -> Result<StopOutcome, Error> {
            Err(Error::Unavailable)
        }
        fn release(&mut self, _: &RunId, _: &ExecutionIdentity) -> Result<(), Error> {
            Err(Error::Unavailable)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (capture, output) = Capture::create(root.path(), &id(), Options::default()).unwrap();
    let launch = Launch {
        run_id: id(),
        executable: "/synthetic/program".into(),
        arguments: vec![],
        working_directory: "/synthetic/work".into(),
        cleanup_grace: DEFAULT_CLEANUP_GRACE,
    };
    assert_eq!(
        FakeSupervisor.start(&launch, output.output()),
        Ok(StartOutcome::Accepted)
    );
    drop(output);
    let report = capture.finish().unwrap();
    assert_eq!(report.retained_bytes(), 6);
    assert_eq!(
        fs::read(root.path().join(id().as_str()).join("stdout.log")).unwrap(),
        b"out"
    );
    assert_eq!(
        fs::read(root.path().join(id().as_str()).join("stderr.log")).unwrap(),
        b"err"
    );
}

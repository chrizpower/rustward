use super::*;
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
struct SinkState {
    bytes: Vec<u8>,
    flushes: usize,
    syncs: usize,
}
struct Sink {
    state: Arc<Mutex<SinkState>>,
    failure: Option<FailureKind>,
    partial: usize,
    interrupted: bool,
    zero: bool,
    panic: bool,
}
impl Sink {
    fn new(failure: Option<FailureKind>) -> Self {
        Self {
            state: Arc::default(),
            failure,
            partial: 3,
            interrupted: false,
            zero: false,
            panic: false,
        }
    }
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.panic {
            panic!("synthetic worker failure");
        }
        if self.interrupted {
            self.interrupted = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.zero {
            return Ok(0);
        }
        let mut state = self.state.lock().unwrap();
        if self.failure == Some(FailureKind::Write) && !state.bytes.is_empty() {
            return Err(io::Error::other("synthetic storage failure"));
        }
        let count = bytes.len().min(self.partial);
        state.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.state.lock().unwrap().flushes += 1;
        if self.failure == Some(FailureKind::Flush) {
            Err(io::Error::other("synthetic flush failure"))
        } else {
            Ok(())
        }
    }
}
impl LogSink for Sink {
    fn sync(&mut self) -> io::Result<()> {
        self.state.lock().unwrap().syncs += 1;
        if self.failure == Some(FailureKind::Sync) {
            Err(io::Error::other("synthetic sync failure"))
        } else {
            Ok(())
        }
    }
}
fn shared(limit: u64) -> Shared {
    Shared {
        remaining: AtomicU64::new(limit),
        failure: AtomicU8::new(0),
        storage_failed: AtomicBool::new(false),
        counters: Default::default(),
        abandoned: AtomicBool::new(false),
    }
}
fn wait_for_failure(capture: &Capture) -> Failure {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(failure) = capture.failure() {
            return failure;
        }
        assert!(Instant::now() < deadline, "failure notification deadline");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn records_partial_writes_and_retries_interrupted_io() {
    struct Reader {
        steps: VecDeque<io::Result<Vec<u8>>>,
    }
    impl Read for Reader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            assert_eq!(buffer.len(), BUFFER_SIZE);
            let bytes = self.steps.pop_front().unwrap()?;
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
    }
    let reader = Reader {
        steps: [
            Err(io::ErrorKind::Interrupted.into()),
            Ok(b"raw\0\xffbytes".to_vec()),
            Ok(vec![]),
        ]
        .into(),
    };
    let mut sink = Sink::new(None);
    sink.interrupted = true;
    let state = sink.state.clone();
    let shared = shared(100);
    drain(reader, sink, &shared, Stream::Stdout);
    assert_eq!(shared.failure(), None);
    assert_eq!(state.lock().unwrap().bytes, b"raw\0\xffbytes");
    assert_eq!(
        shared.report().stdout,
        StreamReport {
            observed_bytes: 10,
            retained_bytes: 10,
            truncated: false,
            eof: true
        }
    );
}

#[test]
fn storage_failures_are_separate_from_truncation_and_preserve_written_counts() {
    for kind in [FailureKind::Write, FailureKind::Flush, FailureKind::Sync] {
        let shared = shared(100);
        let sink = Sink::new(Some(kind));
        let state = sink.state.clone();
        drain(Cursor::new(b"0123456789"), sink, &shared, Stream::Stderr);
        assert_eq!(
            shared.failure(),
            Some(Failure {
                stream: Stream::Stderr,
                kind
            })
        );
        assert!(shared.failure().unwrap().is_storage());
        let report = shared.report().stderr;
        assert!(report.eof);
        assert!(!report.truncated);
        assert_eq!(report.observed_bytes, 10);
        assert_eq!(
            report.retained_bytes,
            if kind == FailureKind::Write { 3 } else { 10 }
        );
        let state = state.lock().unwrap();
        assert_eq!(state.flushes, 1);
        assert_eq!(state.syncs, 1);
    }
    let shared = shared(100);
    let mut sink = Sink::new(None);
    sink.zero = true;
    drain(Cursor::new(b"data"), sink, &shared, Stream::Stdout);
    assert_eq!(
        shared.failure(),
        Some(Failure {
            stream: Stream::Stdout,
            kind: FailureKind::Write
        })
    );
}

#[test]
fn read_failure_keeps_the_prefix_and_does_not_claim_eof() {
    struct BadReader(bool);
    impl Read for BadReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                return Err(io::Error::other("synthetic read failure"));
            }
            self.0 = true;
            buffer[..4].copy_from_slice(b"data");
            Ok(4)
        }
    }
    let shared = shared(100);
    drain(BadReader(false), Sink::new(None), &shared, Stream::Stdout);
    let failure = shared.failure().unwrap();
    assert_eq!(failure.kind, FailureKind::Read);
    assert!(!failure.is_storage());
    assert_eq!(
        shared.report().stdout,
        StreamReport {
            observed_bytes: 4,
            retained_bytes: 4,
            truncated: false,
            eof: false
        }
    );
}

#[test]
fn failure_is_visible_before_eof_and_both_streams_keep_draining() {
    let (capture, mut output) = Capture::start(
        Sink::new(Some(FailureKind::Write)),
        Sink::new(None),
        Options {
            retained_bytes: 100,
        },
    )
    .unwrap();
    output.stdout.write_all(b"0123456789").unwrap();
    assert_eq!(
        wait_for_failure(&capture),
        Failure {
            stream: Stream::Stdout,
            kind: FailureKind::Write
        }
    );
    assert!(!capture.report().stdout.eof);
    output
        .stdout
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    output
        .stderr
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    output.stdout.write_all(&vec![b'x'; 256 * 1024]).unwrap();
    output.stderr.write_all(&vec![b'y'; 256 * 1024]).unwrap();
    drop(output);
    let error = capture.finish().unwrap_err();
    assert_eq!(error.failure.kind, FailureKind::Write);
    assert_eq!(error.report.stdout.observed_bytes, 256 * 1024 + 10);
    assert_eq!(error.report.stderr.observed_bytes, 256 * 1024);
    assert_eq!(error.report.stdout.retained_bytes, 3);
    assert_eq!(error.report.stderr.retained_bytes, 0);
    assert!(error.report.stdout.eof && error.report.stderr.eof);
    assert!(!error.report.truncated());
}

#[test]
fn worker_panics_are_visible_without_waiting_for_the_other_stream() {
    let mut sink = Sink::new(None);
    sink.panic = true;
    let (capture, mut output) = Capture::start(sink, Sink::new(None), Options::default()).unwrap();
    output.stdout.write_all(b"data").unwrap();
    assert_eq!(
        wait_for_failure(&capture),
        Failure {
            stream: Stream::Stdout,
            kind: FailureKind::Worker
        }
    );
    drop(output);
    let error = capture.finish().unwrap_err();
    assert_eq!(error.failure.kind, FailureKind::Worker);
    assert!(!error.report.stdout.eof);
    assert!(error.report.stderr.eof);
}

#[test]
fn abandonment_wakes_idle_readers_and_joins_workers() {
    let (capture, mut output) =
        Capture::start(Sink::new(None), Sink::new(None), Options::default()).unwrap();
    let shared = capture.shared.clone();
    drop(capture);
    assert_eq!(shared.failure().unwrap().kind, FailureKind::Abandoned);
    assert!(output.stdout.write_all(b"after abandonment").is_err());
    assert!(output.stderr.write_all(b"after abandonment").is_err());
}

#[test]
fn native_file_write_failure_is_reported_as_storage_failure() {
    let full = OpenOptions::new().write(true).open("/dev/full").unwrap();
    let (capture, mut output) = Capture::start(
        full,
        tempfile::tempfile().unwrap(),
        Options { retained_bytes: 16 },
    )
    .unwrap();
    output.stdout.write_all(b"data").unwrap();
    assert_eq!(
        wait_for_failure(&capture),
        Failure {
            stream: Stream::Stdout,
            kind: FailureKind::Write
        }
    );
    drop(output);
    let error = capture.finish().unwrap_err();
    assert!(error.failure.is_storage());
    assert_eq!(error.report.retained_bytes(), 0);
    assert!(!error.report.truncated());
}

#[test]
fn nonblocking_completion_waits_for_sync_while_monitor_remains_available() {
    struct HeldSync {
        entered: std::sync::mpsc::SyncSender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Write for HeldSync {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl LogSink for HeldSync {
        fn sync(&mut self) -> io::Result<()> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            Ok(())
        }
    }
    let (entered, waiting) = std::sync::mpsc::sync_channel(0);
    let (release, proceed) = std::sync::mpsc::sync_channel(0);
    let (mut capture, mut writers) = Capture::start(
        HeldSync {
            entered,
            release: proceed,
        },
        Sink::new(None),
        Options::default(),
    )
    .unwrap();
    let monitor = capture.monitor();
    writers.stdout.write_all(b"known output").unwrap();
    drop(writers);
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let pending = capture.try_finish();
    let report = monitor.report();
    release.send(()).unwrap();
    assert_eq!(pending, None);
    assert!(report.stdout.eof);
    assert_eq!(monitor.failure(), None);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = capture.try_finish() {
            assert_eq!(result.unwrap().retained_bytes(), 12);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    drop(capture);
    assert_eq!(monitor.failure(), None);
}

#[test]
fn cloned_monitor_retains_failure_after_failed_capture_completion() {
    let (capture, mut writers) = Capture::start(
        Sink::new(Some(FailureKind::Write)),
        Sink::new(None),
        Options::default(),
    )
    .unwrap();
    let monitor = capture.monitor();
    writers.stdout.write_all(b"synthetic output").unwrap();
    let first = wait_for_failure(&capture);
    assert_eq!(monitor.failure(), Some(first));
    drop(writers);
    assert_eq!(capture.finish().unwrap_err().failure, first);
    assert_eq!(monitor.failure(), Some(first));
}

#[test]
fn recovery_retains_only_file_evidence_and_rejects_missing_unsafe_or_excess_output() {
    for change in 0..7 {
        let root = tempfile::tempdir().unwrap();
        let id = RunId::new(&"d".repeat(64)).unwrap();
        let (capture, mut writers) =
            Capture::create(root.path(), &id, Options { retained_bytes: 10 }).unwrap();
        writers.stdout.write_all(b"sample").unwrap();
        drop(writers);
        let report = capture.finish().unwrap();
        assert_eq!(report.stdout.retained_bytes, 6);
        let run = root.path().join(id.as_str());
        use std::os::unix::fs::{PermissionsExt, symlink};
        let limit = if change == 1 { 5 } else { 10 };
        match change {
            2 => std::fs::remove_file(run.join("stderr.log")).unwrap(),
            3 => std::fs::set_permissions(
                run.join("stdout.log"),
                std::fs::Permissions::from_mode(0o666),
            )
            .unwrap(),
            4 => {
                std::fs::rename(run.join("stderr.log"), run.join("other")).unwrap();
                symlink("other", run.join("stderr.log")).unwrap();
            }
            5 => std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o777)).unwrap(),
            6 => std::fs::hard_link(run.join("stdout.log"), run.join("alias")).unwrap(),
            _ => {}
        }
        let recovered = recover(root.path(), &id, limit);
        if change == 0 {
            let evidence = Evidence::from_report(recovered.unwrap(), true, true);
            assert!(evidence.valid());
            assert_eq!(evidence.stdout.observed_bytes, None);
            assert_eq!(evidence.retained_bytes(), Some(6));
        } else {
            assert!(recovered.is_err(), "{change}");
        }
    }
}
#[test]
fn evidence_rejects_invalid_counters_and_unknown_live_observations() {
    let mut evidence = Evidence::from_report(Report::default(), true, false);
    assert!(evidence.valid());
    evidence.stdout.retained_bytes = 1;
    assert!(!evidence.valid());
    evidence.stdout.observed_bytes = Some(2);
    assert!(evidence.valid());
    evidence.stdout.truncated = None;
    assert!(!evidence.valid());
    evidence.recovered = true;
    assert!(!evidence.valid());
    evidence.stdout.observed_bytes = None;
    evidence.stderr.observed_bytes = None;
    evidence.stderr.truncated = None;
    assert!(evidence.valid());
    evidence.stdout.retained_bytes = u64::MAX;
    evidence.stderr.retained_bytes = 1;
    assert_eq!(evidence.retained_bytes(), None);
}

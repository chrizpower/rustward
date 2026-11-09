mod evidence;
mod recovery;
use crate::supervision::{Output, RunId};
pub use evidence::{Evidence, StreamEvidence};
pub use recovery::recover;
use std::{
    fs::{DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::AsFd,
        unix::{
            fs::{DirBuilderExt, OpenOptionsExt},
            net::UnixStream,
        },
    },
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};

pub const DEFAULT_LOG_LIMIT: u64 = 10 * 1024 * 1024;
const BUFFER_SIZE: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub retained_bytes: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            retained_bytes: DEFAULT_LOG_LIMIT,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}
impl Stream {
    fn index(self) -> usize {
        match self {
            Self::Stdout => 0,
            Self::Stderr => 1,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Read,
    Write,
    Flush,
    Sync,
    Worker,
    Abandoned,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure {
    pub stream: Stream,
    pub kind: FailureKind,
}
impl Failure {
    pub fn is_storage(self) -> bool {
        matches!(
            self.kind,
            FailureKind::Write | FailureKind::Flush | FailureKind::Sync
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    AlreadyExists,
    Storage,
    Resources,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamReport {
    pub observed_bytes: u64,
    pub retained_bytes: u64,
    pub truncated: bool,
    pub eof: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub stdout: StreamReport,
    pub stderr: StreamReport,
}
impl Report {
    pub fn retained_bytes(self) -> u64 {
        self.stdout
            .retained_bytes
            .saturating_add(self.stderr.retained_bytes)
    }
    pub fn truncated(self) -> bool {
        self.stdout.truncated || self.stderr.truncated
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureError {
    pub failure: Failure,
    pub report: Report,
}

pub struct OutputWriters {
    pub stdout: UnixStream,
    pub stderr: UnixStream,
}
impl OutputWriters {
    pub fn output(&self) -> Output<'_> {
        Output {
            stdout: self.stdout.as_fd(),
            stderr: self.stderr.as_fd(),
        }
    }
}

#[derive(Default)]
struct Counters {
    observed: AtomicU64,
    retained: AtomicU64,
    truncated: AtomicBool,
    eof: AtomicBool,
}
struct Shared {
    remaining: AtomicU64,
    failure: AtomicU8,
    storage_failed: AtomicBool,
    counters: [Counters; 2],
    abandoned: AtomicBool,
}
impl Shared {
    fn fail(&self, stream: Stream, kind: FailureKind) {
        if (Failure { stream, kind }).is_storage() {
            self.storage_failed.store(true, Ordering::Release);
        }
        let kind = match kind {
            FailureKind::Read => 1,
            FailureKind::Write => 2,
            FailureKind::Flush => 3,
            FailureKind::Sync => 4,
            FailureKind::Worker => 5,
            FailureKind::Abandoned => 6,
        };
        let code = stream.index() as u8 * 8 + kind;
        let _ = self
            .failure
            .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
    }
    fn failure(&self) -> Option<Failure> {
        let code = self.failure.load(Ordering::Acquire);
        if code == 0 {
            return None;
        }
        let stream = if code > 8 {
            Stream::Stderr
        } else {
            Stream::Stdout
        };
        let kind = match code % 8 {
            1 => FailureKind::Read,
            2 => FailureKind::Write,
            3 => FailureKind::Flush,
            4 => FailureKind::Sync,
            5 => FailureKind::Worker,
            _ => FailureKind::Abandoned,
        };
        Some(Failure { stream, kind })
    }
    fn claim(&self, count: usize) -> usize {
        let previous = self
            .remaining
            .try_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                Some(remaining.saturating_sub(count as u64))
            })
            .unwrap();
        previous.min(count as u64) as usize
    }
    fn report(&self) -> Report {
        let stream = |index: usize| {
            let counters = &self.counters[index];
            let retained_bytes = counters.retained.load(Ordering::Acquire);
            StreamReport {
                retained_bytes,
                observed_bytes: counters.observed.load(Ordering::Acquire),
                truncated: counters.truncated.load(Ordering::Acquire),
                eof: counters.eof.load(Ordering::Acquire),
            }
        };
        Report {
            stdout: stream(0),
            stderr: stream(1),
        }
    }
}

trait LogSink: Write + Send + 'static {
    fn sync(&mut self) -> io::Result<()>;
}
impl LogSink for File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

fn drain(mut reader: impl Read, mut sink: impl LogSink, shared: &Shared, stream: Stream) {
    let counters = &shared.counters[stream.index()];
    let mut buffer = [0u8; BUFFER_SIZE];
    loop {
        if shared.abandoned.load(Ordering::Acquire) {
            return;
        }
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                shared.fail(stream, FailureKind::Read);
                break;
            }
        };
        if shared.abandoned.load(Ordering::Acquire) {
            return;
        }
        if count == 0 {
            counters.eof.store(true, Ordering::Release);
            break;
        }
        let _ = counters
            .observed
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                Some(old.saturating_add(count as u64))
            });
        if shared.failure().is_some() {
            continue;
        }
        let retained = shared.claim(count);
        if retained < count {
            counters.truncated.store(true, Ordering::Release);
        }
        let mut offset = 0;
        while offset < retained && shared.failure().is_none() {
            match sink.write(&buffer[offset..retained]) {
                Ok(0) => {
                    shared.fail(stream, FailureKind::Write);
                    break;
                }
                Ok(written) => {
                    offset += written;
                    counters
                        .retained
                        .fetch_add(written as u64, Ordering::AcqRel);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    shared.fail(stream, FailureKind::Write);
                    break;
                }
            }
        }
    }
    if sink.flush().is_err() {
        shared.fail(stream, FailureKind::Flush);
    }
    if sink.sync().is_err() {
        shared.fail(stream, FailureKind::Sync);
    }
}

/// Atomic capture state that remains readable while file I/O is blocked.
#[derive(Clone)]
pub struct Monitor {
    shared: Arc<Shared>,
}
impl Monitor {
    pub fn storage_failed(&self) -> bool {
        self.shared.storage_failed.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub(crate) fn inject_failure(&self, kind: FailureKind) {
        self.shared.fail(Stream::Stdout, kind);
    }
    pub fn failure(&self) -> Option<Failure> {
        self.shared.failure()
    }
    pub fn report(&self) -> Report {
        self.shared.report()
    }
}

pub struct Capture {
    shared: Arc<Shared>,
    readers: [UnixStream; 2],
    workers: [Option<JoinHandle<()>>; 2],
}

fn storage(error: io::Error) -> SetupError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        SetupError::AlreadyExists
    } else {
        SetupError::Storage
    }
}

impl Capture {
    /// Create a new per-run log directory under an existing protected root.
    pub fn create(
        root: &Path,
        run_id: &RunId,
        options: Options,
    ) -> Result<(Self, OutputWriters), SetupError> {
        let directory = root.join(run_id.as_str());
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(storage)?;
        let file = |name| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.join(name))
        };
        let stdout = file("stdout.log").map_err(storage)?;
        let stderr = file("stderr.log").map_err(storage)?;
        stdout.sync_all().map_err(storage)?;
        stderr.sync_all().map_err(storage)?;
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
        File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
        Self::start(stdout, stderr, options)
    }

    fn start(
        stdout: impl LogSink,
        stderr: impl LogSink,
        options: Options,
    ) -> Result<(Self, OutputWriters), SetupError> {
        let (out_reader, out_writer) = UnixStream::pair().map_err(|_| SetupError::Resources)?;
        let (err_reader, err_writer) = UnixStream::pair().map_err(|_| SetupError::Resources)?;
        let readers = [
            out_reader.try_clone().map_err(|_| SetupError::Resources)?,
            err_reader.try_clone().map_err(|_| SetupError::Resources)?,
        ];
        let shared = Arc::new(Shared {
            remaining: AtomicU64::new(options.retained_bytes),
            failure: AtomicU8::new(0),
            storage_failed: AtomicBool::new(false),
            counters: Default::default(),
            abandoned: AtomicBool::new(false),
        });
        let mut capture = Self {
            shared,
            readers,
            workers: [None, None],
        };
        capture.spawn(out_reader, stdout, Stream::Stdout)?;
        capture.spawn(err_reader, stderr, Stream::Stderr)?;
        Ok((
            capture,
            OutputWriters {
                stdout: out_writer,
                stderr: err_writer,
            },
        ))
    }

    fn spawn(
        &mut self,
        reader: UnixStream,
        sink: impl LogSink,
        stream: Stream,
    ) -> Result<(), SetupError> {
        let shared = self.shared.clone();
        let worker = thread::Builder::new()
            .name(format!("rustward-capture-{stream:?}"))
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    drain(reader, sink, &shared, stream)
                }))
                .is_err()
                {
                    shared.fail(stream, FailureKind::Worker);
                }
            })
            .map_err(|_| SetupError::Resources)?;
        self.workers[stream.index()] = Some(worker);
        Ok(())
    }

    /// Read the first failure without waiting for EOF or storage operations.
    pub fn failure(&self) -> Option<Failure> {
        self.shared.failure()
    }
    /// Read live counters. EOF does not imply that file synchronization has finished.
    pub fn report(&self) -> Report {
        self.shared.report()
    }

    pub fn monitor(&self) -> Monitor {
        Monitor {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Return only after both workers have ended, without waiting for active I/O.
    pub fn try_finish(&mut self) -> Option<Result<Report, CaptureError>> {
        if self
            .workers
            .iter()
            .flatten()
            .any(|worker| !worker.is_finished())
        {
            return None;
        }
        self.join();
        let report = self.report();
        Some(match self.failure() {
            Some(failure) => Err(CaptureError { failure, report }),
            None => Ok(report),
        })
    }

    /// Wait for both streams to end and both log files to be synchronized.
    /// Close all local output writers before waiting; remote writers also need to close.
    pub fn finish(mut self) -> Result<Report, CaptureError> {
        self.join();
        let report = self.report();
        match self.failure() {
            Some(failure) => Err(CaptureError { failure, report }),
            None => Ok(report),
        }
    }

    fn join(&mut self) {
        for (index, worker) in self.workers.iter_mut().enumerate() {
            if let Some(worker) = worker.take()
                && worker.join().is_err()
            {
                self.shared.fail(
                    if index == 0 {
                        Stream::Stdout
                    } else {
                        Stream::Stderr
                    },
                    FailureKind::Worker,
                );
            }
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if self.workers.iter().any(Option::is_some) {
            self.shared.abandoned.store(true, Ordering::Release);
            self.shared.fail(Stream::Stdout, FailureKind::Abandoned);
            for reader in &self.readers {
                let _ = reader.shutdown(Shutdown::Read);
            }
            self.join();
        }
    }
}

#[cfg(test)]
mod tests;

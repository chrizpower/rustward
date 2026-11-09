use super::{CaptureError, Failure, FailureKind, Report, Stream};
use crate::supervision::RunId;
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat2};
use std::{fs::File, os::unix::fs::MetadataExt, path::Path};

/// Only after confirmed cleanup and unit release. Historical observed counts
/// and truncation are unknown; callers must mark the evidence as recovered.
pub fn recover(root: &Path, id: &RunId, limit: u64) -> Result<Report, CaptureError> {
    let mut report = Report::default();
    let mut failed_stream = Stream::Stdout;
    let run = (|| {
        let directory = File::from(
            openat2(
                CWD,
                root.join(id.as_str()),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::NO_SYMLINKS,
            )
            .map_err(|_| FailureKind::Read)?,
        );
        let owner = rustix::process::geteuid().as_raw();
        let meta = directory.metadata().map_err(|_| FailureKind::Read)?;
        if meta.uid() != owner || meta.mode() & 0o7777 != 0o700 {
            return Err(FailureKind::Read);
        }
        for (name, target, stream) in [
            ("stdout.log", &mut report.stdout, Stream::Stdout),
            ("stderr.log", &mut report.stderr, Stream::Stderr),
        ] {
            failed_stream = stream;
            let file = File::from(
                openat2(
                    &directory,
                    Path::new(name),
                    OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                    ResolveFlags::NO_SYMLINKS,
                )
                .map_err(|_| FailureKind::Read)?,
            );
            let meta = file.metadata().map_err(|_| FailureKind::Read)?;
            if !meta.is_file()
                || meta.uid() != owner
                || meta.mode() & 0o7777 != 0o600
                || meta.nlink() != 1
            {
                return Err(FailureKind::Read);
            }
            target.retained_bytes = meta.len();
            target.eof = true;
            file.sync_all().map_err(|_| FailureKind::Sync)?;
        }
        if report.retained_bytes() > limit {
            return Err(FailureKind::Read);
        }
        directory.sync_all().map_err(|_| FailureKind::Sync)?;
        Ok(())
    })();
    match run {
        Ok(()) => Ok(report),
        Err(kind) => Err(CaptureError {
            failure: Failure {
                stream: failed_stream,
                kind,
            },
            report,
        }),
    }
}

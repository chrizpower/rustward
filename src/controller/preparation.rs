use super::emergency::Registry;
use crate::{
    capture::{self, Capture, OutputWriters},
    dispatch::{LaunchFailure, Permit},
    runtime::{self, Catalog, WorkDirectory, WorkRoot},
    supervision::Profile,
};

pub(super) enum Failure {
    Runtime(LaunchFailure),
    Capture(capture::SetupError),
}
pub(super) struct Prepared {
    pub permit: Permit,
    pub capture: Option<Capture>,
    pub writers: Option<OutputWriters>,
    pub work: Option<WorkDirectory>,
    pub profile: Option<Profile>,
    pub failure: Option<Failure>,
}
pub(super) trait Preparation: Send + Sync + 'static {
    fn prepare(&self, permit: Permit, limit: u64, registry: &Registry) -> Prepared;
    fn remove_logs(&self, id: &crate::supervision::RunId) -> Result<(), crate::admission::Error>;
    fn recover(
        &self,
        id: &crate::supervision::RunId,
        limit: u64,
    ) -> Result<capture::Report, capture::CaptureError>;
}
#[derive(Clone)]
pub struct Roots {
    pub runtimes: Catalog,
    pub hosts: runtime::host::Catalog,
    pub work: WorkRoot,
    pub logs: WorkRoot,
}
impl Roots {
    pub(super) fn binding(&self, uid: u32) -> bool {
        let paths = [
            self.runtimes.path(),
            self.hosts.path(),
            self.work.path(),
            self.logs.path(),
        ];
        uid != 0
            && [
                self.runtimes.execution_uid(),
                self.hosts.execution_uid(),
                self.work.owner_uid(),
                self.logs.owner_uid(),
            ]
            .iter()
            .all(|value| *value == uid)
            && paths.iter().enumerate().all(|(i, path)| {
                paths.iter().enumerate().all(|(j, other)| {
                    i == j || (!path.starts_with(other) && !other.starts_with(path))
                })
            })
    }
    fn verify(
        &self,
        permit: &Permit,
    ) -> Result<(std::path::PathBuf, std::path::PathBuf), runtime::Error> {
        let runtime = self.runtimes.verify(permit.runtime_id())?;
        runtime.check_command(permit.launch())?;
        let host = runtime.host_id().ok_or(runtime::Error::Invalid)?;
        let host = self.hosts.verify(host)?;
        Ok((runtime.path().to_owned(), host.launcher().to_owned()))
    }
}
impl Preparation for Roots {
    fn prepare(&self, permit: Permit, limit: u64, registry: &Registry) -> Prepared {
        let mut result = Prepared {
            permit,
            capture: None,
            writers: None,
            work: None,
            profile: None,
            failure: None,
        };
        let prepared = (|| {
            let (path, launcher) = self.verify(&result.permit)?;
            let work = self.work.create(&result.permit.launch().run_id)?;
            let profile = Profile::with_launcher(
                self.runtimes.execution_uid(),
                &path,
                work.path(),
                &launcher,
            )
            .map_err(|_| runtime::Error::Command)?;
            result.work = Some(work);
            result.profile = Some(profile);
            Ok::<_, runtime::Error>(())
        })();
        if let Err(error) = prepared {
            if error == runtime::Error::WorkStorage {
                registry.safety.fail();
            }
            result.failure = Some(Failure::Runtime(error.into()));
            return result;
        }
        if self.logs.check().is_err() {
            result.failure = Some(Failure::Runtime(LaunchFailure::WorkDirectory));
            return result;
        }
        match Capture::create(
            self.logs.path(),
            &result.permit.launch().run_id,
            capture::Options {
                retained_bytes: limit,
            },
        ) {
            Ok((capture, writers)) => {
                registry.monitor(result.permit.launch().run_id.as_str(), capture.monitor());
                result.capture = Some(capture);
                result.writers = Some(writers);
            }
            Err(error) => {
                if error == capture::SetupError::Storage {
                    registry.safety.fail();
                }
                result.failure = Some(Failure::Capture(error));
                return result;
            }
        }
        let checked = self
            .verify(&result.permit)
            .and_then(|_| result.work.as_ref().unwrap().check())
            .and_then(|_| self.logs.check());
        if let Err(error) = checked {
            if error == runtime::Error::WorkStorage {
                registry.safety.fail();
            }
            result.failure = Some(Failure::Runtime(error.into()));
        }
        result
    }
    fn remove_logs(&self, id: &crate::supervision::RunId) -> Result<(), crate::admission::Error> {
        let root = self
            .logs
            .checked_directory()
            .map_err(|_| crate::admission::Error::Storage)?;
        capture::remove_logs(root, id).map_err(|_| crate::admission::Error::Storage)
    }

    fn recover(
        &self,
        id: &crate::supervision::RunId,
        limit: u64,
    ) -> Result<capture::Report, capture::CaptureError> {
        self.logs.check().map_err(|_| capture::CaptureError {
            failure: capture::Failure {
                stream: capture::Stream::Stdout,
                kind: capture::FailureKind::Read,
            },
            report: capture::Report::default(),
        })?;
        capture::recover(self.logs.path(), id, limit)
    }
}

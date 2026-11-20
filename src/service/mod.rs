//! Local service entry point and control loop.
mod control;

use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use rustix::process::{Signal, geteuid};
use serde_json::{Value, json};

use crate::{
    admission::{self, Admission, Cancellation, Store},
    config::{self, Config},
    controller::{self, Controller, Roots, Start},
    dispatch::{Block, Limits},
    runtime::{self, Catalog, WorkRoot},
    safety::Safety,
    supervision::{RunId, Supervisor},
    systemd::{CgroupFs, Systemd, UserBus},
};
pub use control::call;
use control::{Request, Response};

pub struct Options {
    pub config: PathBuf,
    pub name: String,
    pub runtime_catalog: PathBuf,
    pub host_catalog: PathBuf,
    pub work_directory: PathBuf,
    pub runtime_id: String,
    pub preparation_uid: u32,
}

impl Options {
    pub fn parse(mut args: impl Iterator<Item = OsString>) -> Option<Self> {
        let options = Self {
            config: args.next()?.into(),
            name: args.next()?.into_string().ok()?,
            runtime_catalog: args.next()?.into(),
            host_catalog: args.next()?.into(),
            work_directory: args.next()?.into(),
            runtime_id: args.next()?.into_string().ok()?,
            preparation_uid: args.next()?.to_str()?.parse().ok()?,
        };
        args.next().is_none().then_some(options)
    }
    fn roots(&self, config: &Config, protection: runtime::Protection) -> Result<Roots, String> {
        let limits = runtime::Limits::default();
        let roots = Roots {
            runtimes: Catalog::open(&self.runtime_catalog, protection, limits)
                .map_err(|error| format!("runtime catalog: {error:?}"))?,
            hosts: runtime::host::Catalog::open(&self.host_catalog, protection, limits)
                .map_err(|error| format!("host catalog: {error:?}"))?,
            work: WorkRoot::open(&self.work_directory)
                .map_err(|error| format!("work root: {error:?}"))?,
            logs: WorkRoot::open(&config.service().storage.log_directory)
                .map_err(|error| format!("log root: {error:?}"))?,
        };
        let runtime = roots
            .runtimes
            .verify(&self.runtime_id)
            .map_err(|error| format!("selected runtime: {error:?}"))?;
        let host = runtime
            .host_id()
            .ok_or("selected runtime has no host manifest")?;
        roots
            .hosts
            .verify(host)
            .map_err(|error| format!("selected host: {error:?}"))?;
        Ok(roots)
    }
}

pub fn serve(options: Options) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [Signal::INT, Signal::TERM] {
        let stop = Arc::clone(&stop);
        // The handler only stores an atomic flag.
        unsafe {
            signal_hook_registry::register(signal.as_raw(), move || {
                stop.store(true, Ordering::Release)
            })
        }
        .map_err(|error| format!("signal setup: {error}"))?;
    }
    let listener =
        control::listen(&options.name).map_err(|error| format!("control listener: {error}"))?;
    let protection = runtime::Protection {
        preparation_uid: options.preparation_uid,
        execution_uid: geteuid().as_raw(),
    };
    let service = open(options, protection, |path, roots, limits, safety| {
        let supervisor = Systemd::new(
            UserBus::connect().map_err(|error| format!("user bus: {error:?}"))?,
            CgroupFs::default(),
        )
        .map_err(|error| format!("systemd: {error:?}"))?;
        Controller::open(path, roots, limits, supervisor, safety)
            .map_err(|error| format!("controller: {error:?}"))
    })?;
    run(service, listener, &stop)
}

fn open<S: Supervisor + Clone + Send + 'static>(
    options: Options,
    protection: runtime::Protection,
    open_controller: impl FnOnce(&Path, Roots, Limits, Safety) -> Result<Controller<S>, String>,
) -> Result<Service<S>, String> {
    let config = config::load(&options.config).map_err(|error| error.to_string())?;
    let settings = config.service();
    let path = &settings.storage.state_database;
    check_database(path)?;
    let roots = options.roots(&config, protection)?;
    let limits = Limits {
        active_runs: settings.limits.active_runs,
        log_bytes_per_run: settings.limits.log_bytes_per_run,
        retained_log_bytes: settings.limits.retained_log_bytes,
    };
    let safety = Safety::default();
    let controller = open_controller(path, roots, limits, safety.clone())?;
    let store = Store::open_with_safety(path, safety.clone()).map_err(state_error)?;
    Service::new(config, options.runtime_id, store, controller, safety).map_err(state_error)
}

fn run<S: Supervisor + Clone + Send + 'static>(
    mut service: Service<S>,
    listener: std::os::unix::net::UnixListener,
    stop: &AtomicBool,
) -> Result<(), String> {
    loop {
        if stop.load(Ordering::Acquire) {
            service.shutdown = true;
        }
        if service.step()? {
            return Ok(());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Ok(request) = control::receive(&mut stream) {
                    service.shutdown |= stop.load(Ordering::Acquire);
                    let response = service.request(request);
                    let _ = control::respond(&mut stream, &response);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => control::idle(),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                eprintln!("Control listener: {error}");
                service.shutdown = true;
                service.failed = true;
                control::idle();
            }
        }
    }
}

fn check_database(path: &Path) -> Result<(), String> {
    let directory = path.parent().ok_or("missing state directory")?;
    WorkRoot::open(directory).map_err(|error| format!("state directory: {error:?}"))?;
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.uid() == geteuid().as_raw() && meta.nlink() == 1 => {
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err("invalid state database file".into()),
    }
}

fn state_error(error: admission::Error) -> String {
    format!("{error:?}")
}

struct Service<S: Supervisor + Clone + Send + 'static> {
    config: Config,
    runtime_id: String,
    store: Store,
    controller: Controller<S>,
    safety: Safety,
    paused: bool,
    shutdown: bool,
    failed: bool,
    closed: bool,
    block: Option<Block>,
}

impl<S: Supervisor + Clone + Send + 'static> Service<S> {
    fn new(
        config: Config,
        runtime_id: String,
        mut store: Store,
        mut controller: Controller<S>,
        safety: Safety,
    ) -> Result<Self, admission::Error> {
        controller
            .set_paused(true)
            .map_err(|_| admission::Error::Storage)?;
        store.set_queue_limit(config.service().limits.queued_runs)?;
        store.set_closed(false)?;
        Ok(Self {
            config,
            runtime_id,
            store,
            controller,
            safety,
            paused: true,
            shutdown: false,
            failed: false,
            closed: false,
            block: None,
        })
    }

    fn step(&mut self) -> Result<bool, String> {
        if self.shutdown && !self.closed && !self.safety.failed() {
            match self.store.set_closed(true) {
                Ok(()) => self.closed = true,
                Err(admission::Error::Busy) => {}
                Err(error) => {
                    self.safety.fail();
                    eprintln!("Close admission: {error:?}");
                }
            }
        }
        if let Err(error) = self.controller.tick(Instant::now())
            && error != controller::Error::State(admission::Error::Busy)
        {
            self.safety.fail();
            if !self.failed {
                self.failed = true;
                eprintln!("Controller: {error:?}");
            }
        }
        if self.safety.failed() {
            return if self.controller.local_cleanup_pending() {
                Ok(false)
            } else {
                Err("storage or controller failure".into())
            };
        }
        if self.shutdown {
            if self.closed && self.controller.active_runs() == 0 {
                return if self.failed {
                    Err("control listener failed".into())
                } else {
                    Ok(true)
                };
            }
        } else if !self.paused {
            match self.controller.start_next() {
                Ok(Start::Preparing(_)) => self.block = None,
                Ok(Start::Blocked(block)) => self.block = Some(block),
                Err(controller::Error::State(admission::Error::Busy)) => {}
                Err(error) => {
                    self.safety.fail();
                    eprintln!("Dispatch: {error:?}");
                }
            }
        }
        Ok(false)
    }

    fn request(&mut self, request: Request) -> Response {
        match self.handle(request) {
            Ok(value) => Response::Ok(value),
            Err(error) => Response::Error(state_error(error)),
        }
    }

    fn handle(&mut self, request: Request) -> Result<Value, admission::Error> {
        use admission::Error;
        match request {
            Request::IssueKey => {
                if self.shutdown || self.safety.failed() {
                    return Err(Error::Closed);
                }
                Ok(json!({"key": self.store.issue_key()?}))
            }
            Request::Submit {
                key,
                task_id,
                inputs,
            } => {
                let result = self.store.admit_with_gate(
                    &key,
                    &self.config,
                    &task_id,
                    &inputs,
                    &self.runtime_id,
                    self.shutdown,
                )?;
                let (status, run_id) = match result {
                    Admission::Admitted { run_id } => ("admitted", run_id),
                    Admission::AlreadyAdmitted { run_id } => ("already_admitted", run_id),
                    Admission::HistoryRemoved { run_id } => ("history_removed", run_id),
                };
                Ok(json!({"status": status, "run_id": run_id}))
            }
            Request::Status { run_id } => {
                RunId::new(&run_id).map_err(|_| Error::InvalidInputs)?;
                Ok(json!(self.store.run_state(&run_id)?))
            }
            Request::Cancel { run_id } => {
                RunId::new(&run_id).map_err(|_| Error::InvalidInputs)?;
                let result = self.store.cancel_queued(&run_id)?;
                let status = match result {
                    Cancellation::Cancelled => "Cancelled".into(),
                    Cancellation::AlreadyCancelled => "AlreadyCancelled".into(),
                    Cancellation::NotFound => "NotFound".into(),
                    Cancellation::NotQueued => format!(
                        "{:?}",
                        self.controller
                            .cancel_active(&run_id)
                            .map_err(controller_error)?
                    ),
                };
                Ok(json!({"status": status}))
            }
            Request::Pause | Request::Resume => {
                let paused = matches!(request, Request::Pause);
                if !paused && (self.shutdown || self.safety.failed()) {
                    return Err(Error::Closed);
                }
                self.controller
                    .set_paused(paused)
                    .map_err(controller_error)?;
                self.paused = paused;
                self.block = None;
                Ok(json!(null))
            }
            Request::ServiceStatus => Ok(json!({
                "paused": self.paused, "draining": self.shutdown, "failed": self.safety.failed(),
                "active_runs": self.controller.active_runs(),
                "dispatch_block": self.block.as_ref().map(|block| format!("{block:?}")),
            })),
            Request::Reviews { after } => {
                if let Some(after) = &after {
                    RunId::new(after).map_err(|_| Error::InvalidInputs)?;
                }
                Ok(json!(self.store.pending_reviews(after.as_deref(), 10)?))
            }
            Request::Acknowledge {
                acknowledgement_id,
                review,
                note,
            } => Ok(json!(self.store.acknowledge(
                &acknowledgement_id,
                &review,
                &note
            )?)),
            Request::Shutdown => {
                self.shutdown = true;
                Ok(json!(null))
            }
        }
    }
}

fn controller_error(error: controller::Error) -> admission::Error {
    match error {
        controller::Error::State(error) => error,
        _ => admission::Error::Storage,
    }
}

#[cfg(test)]
mod tests;

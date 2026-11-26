//! Local service entry point and control loop.
mod control;
mod maintenance;
pub use maintenance::command as state_command;
#[cfg(feature = "web-preview")]
mod preview;
mod requests;
mod schedule;
mod web;
#[cfg(feature = "web-preview")]
pub use preview::web_preview;

use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rustix::process::{Signal, geteuid};
use serde_json::{Value, json};

use crate::{
    admission::{self, Store},
    config::{self, Config},
    controller::{self, Controller, Roots, Start},
    dispatch::{Block, Limits},
    runtime::{self, Catalog, WorkRoot},
    safety::Safety,
    supervision::Supervisor,
    systemd::{CgroupFs, Systemd, UserBus},
    telemetry::{Counter, Event, State, Telemetry, rejection},
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
    web: Option<web::Options>,
}

impl Options {
    pub fn parse(mut args: impl Iterator<Item = OsString>) -> Option<Self> {
        let mut options = Self {
            config: args.next()?.into(),
            name: args.next()?.into_string().ok()?,
            runtime_catalog: args.next()?.into(),
            host_catalog: args.next()?.into(),
            work_directory: args.next()?.into(),
            runtime_id: args.next()?.into_string().ok()?,
            preparation_uid: args.next()?.to_str()?.parse().ok()?,
            web: None,
        };
        if let Some(flag) = args.next() {
            if flag != "--web" {
                return None;
            }
            options.web = Some(web::Options {
                socket: args.next()?.into(),
                origin: args.next()?.into_string().ok()?,
                proxy_uid: args.next()?.to_str()?.parse().ok()?,
            });
        }
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

fn install_stop() -> Result<Arc<AtomicBool>, String> {
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
    Ok(stop)
}

pub fn serve(options: Options) -> Result<(), String> {
    let stop = install_stop()?;
    let listener =
        control::listen(&options.name).map_err(|error| format!("control listener: {error}"))?;
    let protection = runtime::Protection {
        preparation_uid: options.preparation_uid,
        execution_uid: geteuid().as_raw(),
    };
    let web_options = options.web.clone();
    let service = open(
        options,
        protection,
        |path, roots, limits, resources, safety| {
            let supervisor = Systemd::new(
                UserBus::connect().map_err(|error| format!("user bus: {error:?}"))?,
                CgroupFs::default(),
            )
            .map_err(|error| format!("systemd: {error:?}"))?
            .with_run_resources(resources);
            Controller::open(path, roots, limits, supervisor, safety)
                .map_err(|error| format!("controller: {error:?}"))
        },
    )?;
    let web = web_options
        .map(|options| {
            let logs = WorkRoot::open(&service.config.service().storage.log_directory)
                .map_err(|error| format!("web log root: {error:?}"))?;
            let metrics = service
                .config
                .service()
                .telemetry
                .metrics
                .then(|| service.safety.telemetry().cloned())
                .flatten();
            web::Web::bind(options, logs, metrics)
        })
        .transpose()?;
    let observations = service.safety.telemetry().cloned();
    let result = run(service, listener, &stop, web.as_ref());
    if let Some(telemetry) = observations {
        telemetry.log(
            Event::new("service_stopped", "service").reason(if result.is_ok() {
                "drained"
            } else {
                "failed"
            }),
        );
        telemetry.flush();
    }
    result
}

fn open<S: Supervisor + Clone + Send + 'static>(
    options: Options,
    protection: runtime::Protection,
    open_controller: impl FnOnce(
        &Path,
        Roots,
        Limits,
        config::RunResources,
        Safety,
    ) -> Result<Controller<S>, String>,
) -> Result<Service<S>, String> {
    let telemetry = Telemetry::stderr().map_err(|error| format!("Service log worker: {error}"))?;
    let config = config::load(&options.config).map_err(|error| {
        telemetry.log(
            Event::new("configuration_rejected", "configuration")
                .reason("invalid")
                .error(),
        );
        telemetry.flush();
        if error == config::SERVICE_FILENAME_ERROR {
            error
        } else {
            "Invalid service configuration".to_owned()
        }
    })?;
    telemetry.log(Event::new("configuration_accepted", "configuration"));
    let settings = config.service();
    if settings.telemetry.metrics && options.web.is_none() {
        return Err("Metrics require a private web proxy".to_owned());
    }
    let path = &settings.storage.state_database;
    check_database(path)?;
    let roots = options.roots(&config, protection)?;
    let limits = Limits {
        active_runs: settings.limits.active_runs,
        log_bytes_per_run: settings.limits.log_bytes_per_run,
        retained_log_bytes: settings.limits.retained_log_bytes,
    };
    let safety = Safety::observed(telemetry.clone());
    let controller = open_controller(path, roots, limits, settings.run_resources, safety.clone())?;
    let store = Store::open_with_safety(path, safety.clone()).map_err(state_error)?;
    let mut service =
        Service::new(config, options.runtime_id, store, controller, safety).map_err(state_error)?;
    service.config_path = Some(options.config);
    telemetry.log(Event::new("service_started", "service"));
    Ok(service)
}

fn run<S: Supervisor + Clone + Send + 'static>(
    mut service: Service<S>,
    listener: std::os::unix::net::UnixListener,
    stop: &AtomicBool,
    web: Option<&web::Web>,
) -> Result<(), String> {
    loop {
        if stop.load(Ordering::Acquire) {
            service.shutdown = true;
        }
        let step = service.step();
        service.publish_metrics();
        if step? {
            return Ok(());
        }
        if let Some(web) = web {
            web.poll(&mut service);
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
            Err(_) => {
                service.event(Event::new("control_listener_failed", "control").error());
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
    config_path: Option<PathBuf>,
    matchers: Vec<(croner::Cron, Vec<u8>)>,
    clock: schedule::Clock,
    next_retention: Instant,
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
        let now = schedule::minute(admission::utc_ms()?);
        let matchers = admission::schedule::matchers(&config);
        store.activate_schedules(&config, &matchers, now)?;
        store.set_closed(false)?;
        if let Some(telemetry) = safety.telemetry() {
            telemetry.seed_queue(store.queue_depth());
        }
        Ok(Self {
            config,
            config_path: None,
            matchers,
            clock: schedule::Clock::new(now),
            next_retention: Instant::now(),
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
                    self.event(
                        Event::new("admission_close_failed", "admission")
                            .error()
                            .reason(rejection(&error)),
                    );
                }
            }
        }
        if let Err(error) = self.controller.tick(Instant::now())
            && error != controller::Error::State(admission::Error::Busy)
        {
            self.safety.fail();
            if !self.failed {
                self.failed = true;
                self.event(Event::new("controller_failed", "execution").error());
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
        } else {
            if let Err(error) = self.expire_history() {
                self.safety.fail();
                self.event(
                    Event::new("retention_failed", "retention")
                        .error()
                        .reason(rejection(&error)),
                );
                return Ok(false);
            }
            if let Err(error) = self.evaluate_schedules()
                && error != admission::Error::Busy
            {
                self.safety.fail();
                self.event(
                    Event::new("schedule_evaluation_failed", "scheduler")
                        .error()
                        .reason(rejection(&error)),
                );
                return Ok(false);
            }
            if self.paused {
                return Ok(false);
            }
            match self.controller.start_next() {
                Ok(Start::Preparing(_)) => self.block = None,
                Ok(Start::Blocked(block)) => self.block = Some(block),
                Err(controller::Error::State(admission::Error::Busy)) => {}
                Err(_) => {
                    self.safety.fail();
                    self.event(Event::new("dispatch_failed", "execution").error());
                }
            }
        }
        Ok(false)
    }

    fn event(&self, event: Event) {
        if let Some(telemetry) = self.safety.telemetry() {
            telemetry.log(event);
        }
    }

    fn publish_metrics(&mut self) {
        let Some(telemetry) = self.safety.telemetry() else {
            return;
        };
        let state = if self.safety.failed() {
            State::Failed
        } else if self.shutdown {
            State::Draining
        } else if self.paused {
            State::Paused
        } else {
            match self.block {
                Some(Block::ReviewRequired) => State::Review,
                Some(Block::ExecutionCapacity) => State::ExecutionCapacity,
                Some(Block::LogCapacity) => State::LogCapacity,
                _ => State::Idle,
            }
        };
        telemetry.state(self.controller.active_runs(), state);
    }

    fn expire_history(&mut self) -> Result<(), admission::Error> {
        let now = Instant::now();
        if now < self.next_retention {
            return Ok(());
        }
        let age_ms = i64::from(self.config.service().retention.days) * 86_400_000;
        let cutoff = admission::utc_ms()? - age_ms;
        let delay = match self
            .store
            .expire(cutoff, |id| self.controller.remove_expired_files(id))
        {
            Ok(0) => 60,
            Ok(count) => {
                if let Some(telemetry) = self.safety.telemetry() {
                    telemetry.add(Counter::Expired, count as u64);
                    telemetry.log(Event::new("history_expired", "retention").count(count as u64));
                }
                1
            }
            Err(admission::Error::Busy) => 1,
            Err(error) => return Err(error),
        };
        self.next_retention = now + Duration::from_secs(delay);
        Ok(())
    }

    fn evaluate_schedules(&mut self) -> Result<(), admission::Error> {
        if self.matchers.is_empty() {
            return Ok(());
        }
        self.evaluate_schedules_at(admission::utc_ms()?, Instant::now())
    }

    fn evaluate_schedules_at(
        &mut self,
        wall: i64,
        monotonic: Instant,
    ) -> Result<(), admission::Error> {
        if let Some((minute, changed)) = self.clock.sample(wall, monotonic) {
            self.store.evaluate_schedules(
                &self.config,
                &self.matchers,
                &self.runtime_id,
                minute,
                changed,
            )?;
            self.clock.evaluated(minute);
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<Value, admission::Error> {
        use admission::Error;
        if self.shutdown || self.safety.failed() {
            return Err(Error::Closed);
        }
        let path = self
            .config_path
            .as_ref()
            .ok_or(Error::InvalidConfiguration)?;
        let candidate = config::load(path).map_err(|_| {
            self.event(
                Event::new("configuration_rejected", "configuration")
                    .reason("invalid")
                    .error(),
            );
            Error::InvalidConfiguration
        })?;
        let old = self.config.service();
        let new = candidate.service();
        if new.storage != old.storage
            || new.limits != old.limits
            || new.run_resources != old.run_resources
            || new.retention != old.retention
            || new.telemetry != old.telemetry
        {
            self.event(
                Event::new("configuration_rejected", "configuration").reason("restart_required"),
            );
            return Err(Error::InvalidConfiguration);
        }
        let matchers = admission::schedule::matchers(&candidate);
        let now = schedule::minute(admission::utc_ms()?);
        self.store.activate_schedules(&candidate, &matchers, now)?;
        let result =
            json!({"tasks":candidate.task_count(),"schedules":candidate.schedules().len()});
        self.config = candidate;
        self.event(Event::new("configuration_accepted", "configuration").reason("reload"));
        self.matchers = matchers;
        self.clock = schedule::Clock::new(now);
        Ok(result)
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

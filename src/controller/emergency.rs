use crate::{
    capture::Monitor,
    safety::Safety,
    supervision::{
        Cleanup, ExecutionIdentity, Observation, RunId, StopMode, StopOutcome, Supervisor,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct Entry {
    pub id: RunId,
    pub identity: Option<ExecutionIdentity>,
    pub cleaned: bool,
    pub called: bool,
    pub grace: Duration,
    pub requested: Option<Instant>,
    pub graceful: bool,
    pub uncertain: bool,
    pub monitor: Option<Monitor>,
}
#[derive(Clone)]
pub(super) struct Registry {
    entries: Arc<Mutex<BTreeMap<String, Entry>>>,
    pub safety: Safety,
}
impl Registry {
    pub fn new(safety: Safety) -> Self {
        Self {
            entries: Arc::default(),
            safety,
        }
    }
    pub fn insert(
        &self,
        id: RunId,
        identity: Option<ExecutionIdentity>,
        cleaned: bool,
        called: bool,
        grace: Duration,
    ) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id.as_str().into(),
                Entry {
                    id,
                    identity,
                    cleaned,
                    called,
                    grace,
                    requested: None,
                    graceful: false,
                    uncertain: false,
                    monitor: None,
                },
            );
    }
    pub fn update(&self, id: &str, apply: impl FnOnce(&mut Entry)) {
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(id)
        {
            apply(entry);
        }
    }
    pub fn monitor(&self, id: &str, monitor: Monitor) {
        self.update(id, |entry| entry.monitor = Some(monitor));
    }
    pub fn snapshots(&self) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }
    pub fn remove(&self, id: &str) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }
    pub fn step(&self, supervisor: &mut impl Supervisor, now: Instant) {
        let entries = self.snapshots();
        if entries
            .iter()
            .any(|entry| entry.monitor.as_ref().is_some_and(Monitor::storage_failed))
        {
            self.safety.fail();
        }
        for mut entry in entries {
            let capture_fault = entry.monitor.as_ref().and_then(Monitor::failure).is_some();
            if self.safety.failed() || capture_fault {
                entry.requested.get_or_insert(now);
                self.update(entry.id.as_str(), |state| {
                    state.requested.get_or_insert(now);
                });
            }
            if !entry.called || entry.cleaned || entry.requested.is_none() {
                continue;
            }
            if entry.identity.is_none() {
                match supervisor.observe(&entry.id, None) {
                    Ok(Observation::Known {
                        identity,
                        cleanup,
                        state,
                        exit,
                    }) if valid_identity(&identity)
                        && !(cleanup == Cleanup::Confirmed
                            && state != crate::supervision::ExecutionState::Ended)
                        && !(exit.is_some()
                            && matches!(
                                state,
                                crate::supervision::ExecutionState::Starting
                                    | crate::supervision::ExecutionState::Running
                            )) =>
                    {
                        entry.identity = Some(identity.clone());
                        entry.cleaned = cleanup == Cleanup::Confirmed;
                        self.update(entry.id.as_str(), |state| {
                            if state.identity.is_none() {
                                state.identity = Some(identity);
                            }
                            state.cleaned |= entry.cleaned;
                        });
                    }
                    _ => {
                        self.update(entry.id.as_str(), |state| state.uncertain = true);
                        continue;
                    }
                }
            }
            if entry.cleaned {
                continue;
            }
            let force = now.saturating_duration_since(entry.requested.unwrap()) >= entry.grace;
            if entry.graceful && !force {
                continue;
            }
            let outcome = supervisor.stop(
                &entry.id,
                entry.identity.as_ref().unwrap(),
                if force {
                    StopMode::Force
                } else {
                    StopMode::Graceful
                },
            );
            self.update(entry.id.as_str(), |state| match outcome {
                Ok(StopOutcome::Accepted) => state.graceful = true,
                _ => state.uncertain = true,
            });
        }
    }
}
pub(super) fn valid_identity(identity: &ExecutionIdentity) -> bool {
    identity.invocation != [0; 16]
        && identity.control_group != "/"
        && identity
            .control_group
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && identity.control_group.starts_with('/')
        && identity.control_group.len() <= 4096
        && !identity.control_group.contains('\0')
}
pub(super) struct Guard {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    registry: Registry,
}
impl Guard {
    pub fn start<S: Supervisor + Send + 'static>(
        mut supervisor: S,
        registry: Registry,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let quitting = stop.clone();
        let shared = registry.clone();
        let worker = thread::Builder::new()
            .name("rustward-emergency".into())
            .spawn(move || {
                loop {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        shared.step(&mut supervisor, Instant::now())
                    }));
                    if result.is_err() {
                        shared.safety.fail();
                        break;
                    }
                    if quitting.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
            registry,
        })
    }
    pub fn failed(&self) -> bool {
        self.worker.as_ref().is_some_and(JoinHandle::is_finished)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.registry.snapshots().is_empty() {
            self.registry.safety.fail();
        }
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

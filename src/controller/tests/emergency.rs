use super::*;
#[test]
fn discovery_and_stop_failures_require_review_without_false_cleanup() {
    for observation in [
        Observation::Missing,
        Observation::Uncertain,
        known(false),
        known(true),
        Observation::Known {
            identity: identity(),
            state: ExecutionState::Running,
            exit: None,
            cleanup: Cleanup::Confirmed,
        },
    ] {
        let id = RunId::new(&"c".repeat(64)).unwrap();
        let safety = Safety::default();
        let registry = Registry::new(safety.clone());
        let mut fake = Fake::default();
        fake.0
            .lock()
            .unwrap()
            .observations
            .insert(id.as_str().into(), observation.clone());
        fake.0.lock().unwrap().stop_response = Some(StopOutcome::Uncertain);
        registry.insert(id.clone(), None, false, true, Duration::from_secs(1));
        safety.fail();
        registry.step(&mut fake, Instant::now());
        let entry = registry.snapshots().pop().unwrap();
        assert_eq!(entry.cleaned, observation == known(true));
        if observation == known(false) {
            assert!(entry.identity.is_some());
            assert!(entry.uncertain);
            assert_eq!(fake.0.lock().unwrap().stops.len(), 1);
        } else {
            assert!(fake.0.lock().unwrap().stops.is_empty());
        }
    }
    for group in ["/", "/../run", "/a//b", "relative", "/a\0b"] {
        let mut id = identity();
        id.control_group = group.into();
        assert!(!super::super::emergency::valid_identity(&id));
    }
    let mut id = identity();
    id.invocation = [0; 16];
    assert!(!super::super::emergency::valid_identity(&id));
}
#[test]
fn emergency_worker_panic_closes_the_controller_without_releasing_capacity() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    let mut c = f.controller(Limits::default());
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    started(&mut c, &f);
    f.fake.0.lock().unwrap().panic_stop = true;
    c.cancel_active(&id).unwrap();
    until(|| c.emergency.failed());
    assert!(c.failed());
    assert_eq!(
        c.tick(Instant::now()),
        Err(crate::controller::Error::Resources)
    );
    assert_eq!(c.active_runs(), 1);
}
#[test]
fn bound_supervisor_delegates_observation_stop_and_release() {
    let registry = Registry::new(Safety::default());
    let profile = Profile::for_run(1000, Path::new("/runtime"), Path::new("/work")).unwrap();
    let mut fake = Fake::default();
    let id = RunId::new(&"c".repeat(64)).unwrap();
    fake.0
        .lock()
        .unwrap()
        .observations
        .insert(id.as_str().into(), known(true));
    let mut bound = Bound {
        inner: &mut fake,
        profile: &profile,
        registry: &registry,
    };
    assert_eq!(bound.observe(&id, Some(&identity())).unwrap(), known(true));
    assert_eq!(
        bound.stop(&id, &identity(), StopMode::Force).unwrap(),
        StopOutcome::Accepted
    );
    bound.release(&id, &identity()).unwrap();
}
struct PanicPreparation;
impl Preparation for PanicPreparation {
    fn prepare(&self, _: dispatch::Permit, _: u64, _: &Registry) -> Prepared {
        panic!("synthetic preparation panic")
    }
    fn recover(&self, _: &RunId, _: u64) -> Result<capture::Report, capture::CaptureError> {
        panic!("synthetic recovery panic")
    }
}
#[test]
fn preparation_panic_closes_execution_and_keeps_the_unfinished_intent() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    let mut c = f.controller(Limits::default());
    c.preparation = Arc::new(PanicPreparation);
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    until(|| c.tick(Instant::now()).is_err());
    assert!(c.failed());
    assert_eq!(
        c.tick(Instant::now()),
        Err(crate::controller::Error::Resources)
    );
    assert_eq!(c.active_runs(), 1);
    assert!(f.fake.0.lock().unwrap().launches.is_empty());
    assert_eq!(
        c.run_state(&id).unwrap().unwrap().phase,
        RunPhase::Launching
    );
}
#[test]
fn recovery_worker_panic_cannot_publish_completion_or_restart() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    {
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
    }
    f.ended(&id);
    f.safety = Safety::default();
    let mut c = f.controller(Limits::default());
    c.preparation = Arc::new(PanicPreparation);
    until(|| c.tick(Instant::now()).is_err());
    assert!(c.failed());
    assert_eq!(c.active_runs(), 1);
    assert_eq!(f.fake.0.lock().unwrap().launches.len(), 1);
}

#[test]
fn native_adapter_can_be_used_on_the_emergency_worker() {
    fn supported<S: Supervisor + Clone + Send + 'static>() {}
    supported::<crate::systemd::Systemd<crate::systemd::UserBus, crate::systemd::CgroupFs>>();
}

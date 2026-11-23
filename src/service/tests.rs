use super::*;
use crate::controller::tests::{Fixture, started};
use std::{collections::BTreeMap, thread, time::Duration};

fn ok(response: Response) -> Value {
    match response {
        Response::Ok(value) => value,
        Response::Error(error) => panic!("{error}"),
    }
}

#[test]
fn service_retries_admission_and_drains_active_work_on_shutdown() {
    let mut fixture = Fixture::new();
    let (send, records) = std::sync::mpsc::channel();
    let telemetry = Telemetry::start(move |bytes| {
        send.send(serde_json::from_slice::<Value>(bytes).unwrap())
            .unwrap();
        Ok(())
    })
    .unwrap();
    fixture.safety = Safety::observed(telemetry.clone());
    let config = config::load("examples/rustward.toml").unwrap();
    let store = Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap();
    let controller = fixture.controller(Limits::default());
    let mut service = Service::new(
        config,
        "runtime-1".into(),
        store,
        controller,
        fixture.safety.clone(),
    )
    .unwrap();
    let mut submit = |key: Option<String>| {
        let key = key.unwrap_or_else(|| {
            ok(service.request(Request::IssueKey))["key"]
                .as_str()
                .unwrap()
                .to_owned()
        });
        let response = ok(service.request(Request::Submit {
            key: key.clone(),
            task_id: "synthetic-report".into(),
            inputs: BTreeMap::from([("name".into(), "sample".into())]),
        }));
        (key, response)
    };
    let (key, first) = submit(None);
    let (_, retry) = submit(Some(key.clone()));
    assert_eq!(retry["status"], "already_admitted");
    assert_eq!(first["run_id"], retry["run_id"]);
    let (_, waiting) = submit(None);
    let id = first["run_id"].as_str().unwrap().to_owned();
    let waiting = waiting["run_id"].as_str().unwrap().to_owned();
    assert!(!service.step().unwrap());
    assert_eq!(service.controller.active_runs(), 0);
    ok(service.request(Request::Resume));
    assert!(!service.step().unwrap());
    started(&mut service.controller, &fixture);
    ok(service.request(Request::Shutdown));
    assert_eq!(
        ok(service.request(Request::Submit {
            key,
            task_id: "synthetic-report".into(),
            inputs: BTreeMap::from([("name".into(), "sample".into())]),
        }))["status"],
        "already_admitted"
    );
    assert!(!service.step().unwrap());
    assert!(
        matches!(service.request(Request::IssueKey), Response::Error(ref error) if error == "Closed")
    );
    fixture.ended(&id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !service.step().unwrap() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        ok(service.request(Request::Status { run_id: id }))["result"],
        "Success"
    );
    assert_eq!(
        ok(service.request(Request::Status { run_id: waiting }))["phase"],
        "Queued"
    );
    service.publish_metrics();
    let scrape = telemetry.scrape();
    assert!(scrape.contains("rustward_queue_depth 1\n"));
    assert!(scrape.contains("rustward_admissions_total{trigger=\"manual\"} 2"));
    assert!(scrape.contains("rustward_completions_total{result=\"success\"} 1"));
    assert!(scrape.contains("rustward_run_duration_seconds_count 1"));
    telemetry.flush();
    let events: Vec<_> = records
        .try_iter()
        .map(|entry| entry["event"].as_str().unwrap().to_owned())
        .collect();
    for expected in [
        "run_admitted",
        "dispatch_committed",
        "launch_reply",
        "run_completed",
    ] {
        assert!(events.iter().any(|event| event == expected), "{expected}");
    }
}

#[test]
fn control_round_trip_uses_one_bounded_json_request() {
    let name = format!("test-{}", std::process::id());
    let listener = control::listen(&name).unwrap();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    assert!(
                        matches!(control::receive(&mut stream).unwrap(), Request::Submit { inputs, .. }
                        if inputs["count"].as_integer() == Some(7) && inputs["enabled"].as_bool() == Some(true))
                    );
                    control::respond(&mut stream, &Response::Ok(json!({"status": "admitted"})))
                        .unwrap();
                    break;
                }
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
    });
    let request = br#"{"command":"submit","key":"synthetic","task_id":"example","inputs":{"count":7,"enabled":true}}"#;
    let mut response = Vec::new();
    assert!(call(&name, request.as_slice(), &mut response).unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&response).unwrap(),
        json!({"ok":{"status":"admitted"}})
    );
    worker.join().unwrap();
    assert!(call(&name, io::repeat(b' '), io::sink()).is_err());
    assert!(control::listen("invalid name").is_err());
}

#[test]
fn startup_checks_private_state_and_prepared_runtime() {
    use crate::controller::tests::native::Native;
    let native = Native::new(true);
    let base = native.roots.work.path().parent().unwrap();
    let config_path = base.join("rustward.toml");
    fs::create_dir(base.join("tasks")).unwrap();
    fs::create_dir(base.join("schedules")).unwrap();
    fs::write(
        &config_path,
        "version=2\n[storage]\nstate_database='work/state.db'\nlog_directory='logs'\n",
    )
    .unwrap();
    let uid = geteuid().as_raw();
    let config = config::load(&config_path).unwrap();
    let mut options = Options {
        config: config_path,
        name: "unused".into(),
        runtime_catalog: base.join("runtimes"),
        host_catalog: base.join("hosts"),
        work_directory: base.join("work"),
        runtime_id: native.id.clone(),
        preparation_uid: uid,
        web: None,
    };
    let protection = runtime::Protection {
        preparation_uid: uid,
        execution_uid: uid + 1,
    };
    assert_eq!(
        options.roots(&config, protection).unwrap().logs.path(),
        base.join("logs")
    );
    options.runtime_id = "wrong".into();
    assert!(
        options
            .roots(&config, protection)
            .err()
            .unwrap()
            .starts_with("selected runtime")
    );
    let path = &config.service().storage.state_database;
    check_database(path).unwrap();
    fs::write(path, []).unwrap();
    check_database(path).unwrap();
    fs::hard_link(path, path.with_extension("duplicate")).unwrap();
    assert!(check_database(path).is_err());
    assert!(check_database(&base.join("unsafe.db")).is_err());
    fs::remove_file(path.with_extension("duplicate")).unwrap();
    options.runtime_id = native.id.clone();
    let fixture = Fixture::new();
    let mut service = open(options, protection, |path, roots, limits, safety| {
        assert_eq!(roots.logs.path(), base.join("logs"));
        assert_eq!(limits.log_bytes_per_run, 10 * 1024 * 1024);
        Ok(fixture.controller_at(path, limits, safety))
    })
    .unwrap();
    assert!(service.paused);
    assert!(ok(service.request(Request::IssueKey))["key"].is_string());
}

#[test]
fn service_loop_runs_control_requests_and_releases_the_endpoint() {
    let fixture = Fixture::new();
    let name = format!("loop-test-{}", std::process::id());
    let listener = control::listen(&name).unwrap();
    let path = fixture.path.clone();
    let safety = fixture.safety.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || {
        let config = config::load("examples/rustward.toml").unwrap();
        let store = Store::open_with_safety(path, safety.clone()).unwrap();
        let controller = fixture.controller(Limits::default());
        let service = Service::new(config, "runtime-1".into(), store, controller, safety).unwrap();
        run(service, listener, &worker_stop, None).unwrap();
    });
    let request = |request: Value| {
        let mut bytes = Vec::new();
        let succeeded = call(&name, request.to_string().as_bytes(), &mut bytes).unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response.get("ok").is_some(), succeeded);
        response
    };
    let state = request(json!({"command":"service_status"}));
    assert_eq!(state["ok"]["paused"], true);
    assert_eq!(
        request(json!({"command":"reviews","after":"a".repeat(64)}))["ok"],
        json!([])
    );
    assert_eq!(
        request(json!({"command":"status","run_id":"invalid"}))["error"],
        "InvalidInputs"
    );
    let key = request(json!({"command":"issue_key"}))["ok"]["key"].clone();
    let submit = request(
        json!({"command":"submit","task_id":"synthetic-report","key":key,"inputs":{"name":"sample"}}),
    );
    let run_id = &submit["ok"]["run_id"];
    let cancel = || request(json!({"command":"cancel","run_id":run_id}));
    assert_eq!(cancel()["ok"]["status"], "Cancelled");
    assert_eq!(cancel()["ok"]["status"], "AlreadyCancelled");
    assert_eq!(
        request(json!({"command":"cancel","run_id":"a".repeat(64)}))["ok"]["status"],
        "NotFound"
    );
    assert_eq!(request(json!({"command":"pause"}))["ok"], Value::Null);
    assert_eq!(request(json!({"command":"resume"}))["ok"], Value::Null);
    let deadline = Instant::now() + Duration::from_secs(2);
    while request(json!({"command":"service_status"}))["ok"]["dispatch_block"] != "Empty" {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    assert!(control::listen(&name).is_ok());
}

#[test]
fn storage_failure_closes_the_service_loop() {
    let fixture = Fixture::new();
    let mut service = Service::new(
        config::load("examples/rustward.toml").unwrap(),
        "runtime-1".into(),
        Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
        fixture.controller(Limits::default()),
        fixture.safety.clone(),
    )
    .unwrap();
    fixture.safety.fail();
    assert!(matches!(
        service.request(Request::Resume),
        Response::Error(_)
    ));
    assert!(service.step().is_err());
}

#[test]
fn restart_requires_review_before_dispatching_queued_work() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let (_, waiting) = fixture.admit();
    let service = |fixture: &Fixture| {
        Service::new(
            config::load("examples/rustward.toml").unwrap(),
            "runtime-1".into(),
            Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
            fixture.controller(Limits::default()),
            fixture.safety.clone(),
        )
        .unwrap()
    };
    {
        let mut daemon = service(&fixture);
        ok(daemon.request(Request::Resume));
        daemon.step().unwrap();
        started(&mut daemon.controller, &fixture);
    }
    fixture.ended(&id);
    let telemetry = Telemetry::start(|_| Ok(())).unwrap();
    fixture.safety = Safety::observed(telemetry.clone());
    let mut daemon = service(&fixture);
    ok(daemon.request(Request::Resume));
    let deadline = Instant::now() + Duration::from_secs(5);
    let review = loop {
        daemon.step().unwrap();
        let reviews = ok(daemon.request(Request::Reviews { after: None }));
        let review = &reviews[0];
        if review["state"]["phase"] == "Completed" {
            break serde_json::from_value(review.clone()).unwrap();
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(daemon.block, Some(Block::ReviewRequired));
    daemon.publish_metrics();
    let scrape = telemetry.scrape();
    assert!(scrape.contains("rustward_dispatch_state{state=\"review\"} 1"));
    assert!(scrape.contains("rustward_queue_depth 1\n"));
    assert!(scrape.contains("rustward_completions_total{result=\"interruption\"} 1"));
    assert!(scrape.contains("rustward_admissions_total{trigger=\"manual\"} 0"));
    assert_eq!(
        ok(daemon.request(Request::Status {
            run_id: waiting.clone()
        }))["phase"],
        "Queued"
    );
    ok(daemon.request(Request::Acknowledge {
        acknowledgement_id: "a".repeat(64),
        review: Box::new(review),
        note: "Synthetic task stopped locally. Resume the fixture.".into(),
    }));
    ok(daemon.request(Request::Cancel { run_id: waiting }));
    assert!(matches!(
        daemon.request(Request::Cancel { run_id: id }),
        Response::Ok(_)
    ));
    assert_eq!(
        ok(daemon.request(Request::Reviews { after: None })),
        json!([])
    );
}

#[test]
fn loss_of_storage_stops_active_work_before_service_exits() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut service = Service::new(
        config::load("examples/rustward.toml").unwrap(),
        "runtime-1".into(),
        Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
        fixture.controller(Limits::default()),
        fixture.safety.clone(),
    )
    .unwrap();
    ok(service.request(Request::Resume));
    service.step().unwrap();
    started(&mut service.controller, &fixture);
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_close BEFORE UPDATE OF closed ON issuer
         BEGIN SELECT RAISE(ABORT, 'synthetic storage fault'); END;",
        )
        .unwrap();
    ok(service.request(Request::Shutdown));
    assert!(!service.step().unwrap());
    assert!(fixture.safety.failed());
    assert!(service.controller.local_cleanup_pending());
    fixture.ended(&id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.step().is_ok() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!service.controller.local_cleanup_pending());
}

#[test]
fn failed_dispatch_write_closes_admission() {
    let mut fixture = Fixture::new();
    fixture.admit();
    let mut service = Service::new(
        config::load("examples/rustward.toml").unwrap(),
        "runtime-1".into(),
        Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
        fixture.controller(Limits::default()),
        fixture.safety.clone(),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_launch BEFORE INSERT ON launches
         BEGIN SELECT RAISE(ABORT, 'synthetic storage fault'); END;",
        )
        .unwrap();
    ok(service.request(Request::Resume));
    assert!(!service.step().unwrap());
    assert!(fixture.safety.failed());
    assert!(matches!(
        service.request(Request::IssueKey),
        Response::Error(_)
    ));
    assert!(matches!(
        service.request(Request::Pause),
        Response::Error(_)
    ));
    assert!(service.step().is_err());
}

#[test]
fn reload_preserves_active_schedules_after_invalid_candidate() {
    let fixture = Fixture::new();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("tasks")).unwrap();
    fs::create_dir(root.path().join("schedules")).unwrap();
    let path = root.path().join("rustward.toml");
    let settings = include_str!("../../examples/rustward.toml");
    fs::write(&path, settings).unwrap();
    fs::copy(
        "examples/tasks/synthetic-report.toml",
        root.path().join("tasks/synthetic-report.toml"),
    )
    .unwrap();
    let mut daemon = Service::new(
        config::load(&path).unwrap(),
        "runtime-1".into(),
        Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
        fixture.controller(Limits::default()),
        fixture.safety.clone(),
    )
    .unwrap();
    daemon.config_path = Some(path.clone());
    let schedule = root.path().join("schedules/latest.toml");
    let valid = "task='synthetic-report'\ncron='* * * * *'\n[inputs]\nname='sample'\n";
    fs::write(&schedule, valid).unwrap();
    assert_eq!(ok(daemon.request(Request::Reload))["schedules"], 1);
    let request_history = || Request::ScheduleHistory {
        schedule_id: "latest".into(),
        after_due_ms: None,
        after_gap_id: None,
    };
    let prior = ok(daemon.request(request_history()));
    fs::write(&schedule, "invalid").unwrap();
    assert!(
        matches!(daemon.request(Request::Reload), Response::Error(message) if message == "InvalidConfiguration")
    );
    assert_eq!(ok(daemon.request(request_history())), prior);
    fs::write(&schedule, valid).unwrap();
    fs::write(&path, format!("{settings}\n[limits]\nqueued_runs=1\n")).unwrap();
    assert!(
        matches!(daemon.request(Request::Reload), Response::Error(message) if message == "InvalidConfiguration")
    );
    fs::write(&path, settings).unwrap();

    let minute = prior["checkpoint_ms"].as_i64().unwrap();
    let now = Instant::now();
    daemon.evaluate_schedules_at(minute + 60_000, now).unwrap();
    let history = ok(daemon.request(request_history()));
    let run_id = history["occurrences"][0]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        ok(daemon.request(Request::Status { run_id }))["phase"],
        "Queued"
    );
    ok(daemon.request(Request::SchedulePause {
        schedule_id: Some("latest".into()),
    }));
    daemon
        .evaluate_schedules_at(minute + 120_000, now + Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        ok(daemon.request(request_history()))["occurrences"][1]["decision"],
        "paused"
    );
    ok(daemon.request(Request::ScheduleResume {
        schedule_id: Some("latest".into()),
    }));
    assert_eq!(ok(daemon.request(request_history()))["paused"], false);
    ok(daemon.request(Request::Resume));
    daemon.step().unwrap();
    started(&mut daemon.controller, &fixture);
    let run_id = history["occurrences"][0]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.ended(&run_id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while daemon.controller.active_runs() > 0 {
        assert!(Instant::now() < deadline);
        daemon.step().unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        ok(daemon.request(Request::Status { run_id }))["result"],
        "Success"
    );
    daemon.evaluate_schedules().unwrap();
    ok(daemon.request(Request::Shutdown));
    assert!(
        matches!(daemon.request(Request::Reload), Response::Error(message) if message == "Closed")
    );
    assert!(
        matches!(daemon.request(Request::SchedulePause { schedule_id: None }), Response::Error(message) if message == "Closed")
    );
}

mod telemetry;

use super::*;

#[test]
fn protected_log_capacity_is_visible_without_dispatching() {
    let mut fixture = Fixture::new();
    fixture.admit();
    let telemetry = Telemetry::start(|_| Ok(())).unwrap();
    fixture.safety = Safety::observed(telemetry.clone());
    let mut daemon = Service::new(
        config::load("examples/rustward.toml").unwrap(),
        "runtime-1".into(),
        Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
        fixture.controller(Limits {
            active_runs: 1,
            log_bytes_per_run: 128,
            retained_log_bytes: 64,
        }),
        fixture.safety.clone(),
    )
    .unwrap();
    ok(daemon.request(Request::Resume));
    daemon.step().unwrap();
    daemon.publish_metrics();
    assert_eq!(daemon.block, Some(Block::LogCapacity));
    let scrape = telemetry.scrape();
    assert!(scrape.contains("rustward_dispatch_state{state=\"log_capacity\"} 1"));
    assert!(scrape.contains("rustward_active_runs 0"));
    assert!(scrape.contains("rustward_queue_depth 1"));
}

#[test]
fn configuration_requires_a_proxy_and_does_not_echo_invalid_values() {
    let root = tempfile::tempdir().unwrap();
    for name in ["tasks", "schedules"] {
        fs::create_dir(root.path().join(name)).unwrap();
    }
    let path = root.path().join("rustward.toml");
    let settings =
        "version=2\n[storage]\nstate_database='state.db'\nlog_directory='logs'\n[telemetry]\n";
    let open = || {
        let options = Options {
            config: path.clone(),
            name: "synthetic".into(),
            runtime_catalog: root.path().join("runtimes"),
            host_catalog: root.path().join("hosts"),
            work_directory: root.path().join("work"),
            runtime_id: "synthetic".into(),
            preparation_uid: geteuid().as_raw(),
            web: None,
        };
        super::super::open::<crate::controller::tests::Fake>(
            options,
            runtime::Protection {
                preparation_uid: 1,
                execution_uid: 2,
            },
            |_, _, _, _, _| panic!("Invalid configuration reached the controller"),
        )
        .err()
        .unwrap()
    };
    fs::write(&path, format!("{settings}metrics = true\n")).unwrap();
    assert_eq!(open(), "Metrics require a private web proxy");
    assert!(!root.path().join("state.db").exists());
    fs::write(&path, format!("{settings}metrics = 'synthetic secret'\n")).unwrap();
    assert_eq!(open(), "Invalid service configuration");
}

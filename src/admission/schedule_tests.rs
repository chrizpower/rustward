use super::*;
use crate::admission::{RunPhase, RunResult};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

const MINUTE: i64 = 60_000;

struct Fixture {
    root: TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for name in ["tasks", "schedules"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        fs::write(
            root.path().join("rustward.toml"),
            "version=2\n[storage]\nstate_database='state.db'\nlog_directory='logs'\n",
        )
        .unwrap();
        fs::write(root.path().join("tasks/task.toml"), "label='Task'\nexecutable='/synthetic/task'\nworking_directory='/synthetic'\narguments=[]\ninputs=[]\n").unwrap();
        let store = Store::open(root.path().join("state.db")).unwrap();
        Self { root, store }
    }

    fn schedule(&self, id: &str, cron: &str) {
        fs::write(
            self.root.path().join(format!("schedules/{id}.toml")),
            format!("task='task'\ncron='{cron}'\n"),
        )
        .unwrap();
    }

    fn config(&self) -> Config {
        crate::config::load(self.root.path().join("rustward.toml")).unwrap()
    }

    fn activate(&mut self, config: &Config, minute: i64) {
        self.store
            .activate_schedules(config, &matchers(config), minute * MINUTE)
            .unwrap();
    }

    fn evaluate(&mut self, config: &Config, minute: i64, changed: bool) -> Result<(), Error> {
        self.store.evaluate_schedules(
            config,
            &matchers(config),
            "runtime",
            minute * MINUTE,
            changed,
        )
    }

    fn history(&mut self, id: &str) -> Value {
        self.store.schedule_history(id, None, None).unwrap()
    }
}

#[test]
fn enforces_the_shared_queue_and_independent_pause_controls() {
    let mut f = Fixture::new();
    f.schedule("one", "* * * * *");
    f.schedule("two", "* * * * *");
    f.schedule("future", "59 * * * *");
    let config = f.config();
    f.store.set_queue_limit(1).unwrap();
    f.activate(&config, 0);
    f.evaluate(&config, 0, false).unwrap();
    assert_eq!(f.history("one")["occurrences"], serde_json::json!([]));
    f.evaluate(&config, 1, false).unwrap();
    let history = f.history("one");
    let run = history["occurrences"][0]["run_id"].as_str().unwrap();
    assert_eq!(
        f.store.run_state(run).unwrap().unwrap().phase,
        RunPhase::Queued
    );
    assert_eq!(f.history("two")["occurrences"][0]["decision"], "queue_full");
    f.evaluate(&config, 2, false).unwrap();
    assert_eq!(
        f.history("one")["occurrences"][1]["decision"],
        "schedule_queued"
    );
    f.store.cancel_queued(run).unwrap();
    f.store.pause_schedule(Some("one"), true).unwrap();
    assert_eq!(
        f.store.pause_schedule(Some("missing"), true),
        Err(Error::UnknownSchedule)
    );
    f.evaluate(&config, 3, false).unwrap();
    assert_eq!(f.history("one")["occurrences"][2]["decision"], "paused");
    assert_eq!(f.history("two")["occurrences"][2]["decision"], "admitted");
    f.store.pause_schedule(None, true).unwrap();
    f.evaluate(&config, 4, false).unwrap();
    assert_eq!(f.history("two")["occurrences"][3]["decision"], "paused");
    let history = f
        .store
        .schedule_history("two", Some(2 * MINUTE), Some(1))
        .unwrap();
    assert_eq!(history["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(history["global_paused"], true);
    f.store.pause_schedule(None, false).unwrap();
    f.store.set_closed(true).unwrap();
    f.evaluate(&config, 5, false).unwrap();
    assert_eq!(
        f.history("two")["occurrences"][4]["decision"],
        "admission_closed"
    );
    assert_eq!(
        f.store.run_state(run).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
}

#[test]
fn restart_clock_changes_and_reload_preserve_schedule_identity() {
    let mut f = Fixture::new();
    f.schedule("old", "* * * * *");
    let config = f.config();
    f.activate(&config, 0);
    f.evaluate(&config, 1, false).unwrap();
    f.store.pause_schedule(Some("old"), true).unwrap();
    f.store = Store::open(f.root.path().join("state.db")).unwrap();
    f.activate(&config, 4);
    let history = f.history("old");
    assert_eq!(history["gaps"][0]["after_ms"], Value::Null);
    assert_eq!(history["gaps"][1]["missed_count"], 3);
    f.evaluate(&config, 2, true).unwrap();
    f.evaluate(&config, 3, false).unwrap();
    f.evaluate(&config, 4, false).unwrap();
    assert_eq!(f.history("old")["occurrences"].as_array().unwrap().len(), 1);
    f.evaluate(&config, 5, false).unwrap();
    assert_eq!(f.history("old")["occurrences"][1]["decision"], "paused");
    f.evaluate(&config, 10, true).unwrap();
    f.evaluate(&config, 13, false).unwrap();
    assert_eq!(f.history("old")["gaps"][4]["missed_count"], 3);
    f.schedule("old", "10-59 * * * *");
    let edited = f.config();
    f.activate(&edited, 13);
    f.activate(&edited, 12);
    f.evaluate(&edited, 14, false).unwrap();
    let history = f.history("old");
    assert_ne!(
        history["occurrences"][0]["revision"],
        history["occurrences"][2]["revision"]
    );
    fs::rename(
        f.root.path().join("schedules/old.toml"),
        f.root.path().join("schedules/new.toml"),
    )
    .unwrap();
    let renamed = f.config();
    f.activate(&renamed, 14);
    assert_eq!(f.history("old")["active"], false);
    assert_eq!(f.history("new")["paused"], false);
    f.evaluate(&renamed, 15, false).unwrap();
    assert_eq!(f.history("new")["occurrences"][0]["decision"], "admitted");
    f.activate(&renamed, 1515);
    assert_eq!(f.history("new")["gaps"][1]["missed_count"], Value::Null);
    assert!(matches!(
        f.store.schedule_history("missing", None, None),
        Err(Error::UnknownSchedule)
    ));
}

#[test]
fn failed_occurrence_write_rolls_back_all_runs_and_checkpoints() {
    let mut f = Fixture::new();
    f.schedule("first", "* * * * *");
    f.schedule("second", "* * * * *");
    let config = f.config();
    f.activate(&config, 0);
    let path = f.root.path().join("state.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_occurrence BEFORE INSERT ON occurrences WHEN NEW.schedule_id='second' BEGIN SELECT RAISE(ABORT,'synthetic fault'); END;").unwrap();
    assert_eq!(f.evaluate(&config, 1, false), Err(Error::Storage));
    for table in ["runs", "occurrences"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT max(checkpoint_ms) FROM schedule_state", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_occurrence").unwrap();
    f.store = Store::open(&path).unwrap();
    f.activate(&config, 1);
    f.evaluate(&config, 2, false).unwrap();
    assert_eq!(f.history("first")["occurrences"][0]["due_ms"], 2 * MINUTE);
    assert_eq!(f.history("second")["occurrences"][0]["due_ms"], 2 * MINUTE);
}

#[test]
fn active_scheduled_run_can_have_one_queued_successor() {
    let mut f = Fixture::new();
    f.schedule("minute", "* * * * *");
    let config = f.config();
    f.activate(&config, 0);
    f.evaluate(&config, 1, false).unwrap();
    let mut dispatcher = crate::dispatch::Session::open(
        f.root.path().join("state.db"),
        crate::dispatch::Limits::default(),
    )
    .unwrap();
    dispatcher.set_paused(false).unwrap();
    let crate::dispatch::Claim::Ready(permit) = dispatcher.claim_next().unwrap() else {
        panic!("scheduled run was not eligible");
    };
    f.evaluate(&config, 2, false).unwrap();
    f.evaluate(&config, 3, false).unwrap();
    let history = f.history("minute");
    assert_eq!(
        history["occurrences"][0]["run_id"],
        permit.launch().run_id.as_str()
    );
    assert_eq!(history["occurrences"][1]["decision"], "admitted");
    assert_eq!(history["occurrences"][2]["decision"], "schedule_queued");
}

#[test]
fn scheduled_inputs_use_the_manual_admission_size_limit() {
    let mut f = Fixture::new();
    fs::write(f.root.path().join("tasks/task.toml"),
        "label='Task'\nexecutable='/synthetic/task'\nworking_directory='/synthetic'\narguments=[]\ninputs=[{id='value',type='string'}]\n",
    ).unwrap();
    fs::write(
        f.root.path().join("schedules/large.toml"),
        format!(
            "task='task'\ncron='* * * * *'\n[inputs]\nvalue='{}'\n",
            "x".repeat(65_536)
        ),
    )
    .unwrap();
    let config = f.config();
    f.activate(&config, 0);
    f.evaluate(&config, 1, false).unwrap();
    let history = f.history("large");
    assert_eq!(history["occurrences"][0]["decision"], "too_large");
    assert_eq!(history["occurrences"][0]["run_id"], Value::Null);
}

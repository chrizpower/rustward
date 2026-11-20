use std::fs;

use rusqlite::Connection;
use rustward::{admission::Store, config::load};

const TASK: &str = "label = 'Sample'\nexecutable = '/synthetic/program'\nworking_directory = '/synthetic'\narguments = []\ninputs = []\n";
const SERVICE: &str = include_str!("../examples/rustward.toml");

#[test]
fn complete_revisions_ignore_discovery_order_and_defaults_but_include_schedules() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let database = root.path().join("state.db");
    let mut store = Store::open(&database).unwrap();
    let db = Connection::open(&database).unwrap();
    let mut record = |folder: &str,
                      reverse: bool,
                      cron: &str,
                      explicit_defaults: bool,
                      schedule_id: &str| {
        let path = root.path().join(folder);
        fs::create_dir_all(path.join("tasks")).unwrap();
        fs::create_dir_all(path.join("schedules")).unwrap();
        let service = if explicit_defaults {
            format!(
                "{SERVICE}\n[definitions]\ntasks = 'tasks'\nschedules = 'schedules'\n[limits]\nactive_runs = 1\nqueued_runs = 100\nlog_bytes_per_run = 10485760\nretained_log_bytes = 1073741824\n[retention]\ndays = 30\n"
            )
        } else {
            SERVICE.into()
        };
        fs::write(path.join("rustward.toml"), service).unwrap();
        for id in if reverse { ["b", "a"] } else { ["a", "b"] } {
            fs::write(path.join(format!("tasks/{id}.toml")), TASK).unwrap();
        }
        let schedule = if explicit_defaults {
            format!(
                "# Same definition with changed key order\ninputs = {{}}\ncron = '{cron}'\ntask = 'a'\n"
            )
        } else {
            format!("task = 'a'\ncron = '{cron}'\n")
        };
        fs::write(path.join(format!("schedules/{schedule_id}.toml")), schedule).unwrap();
        let config = load(path.join("rustward.toml")).unwrap();
        let key = store.issue_key().unwrap();
        store
            .admit(&key, &config, "a", &Default::default(), "runtime-1")
            .unwrap();
        db.query_row(
            "SELECT task_revision, config_revision FROM runs ORDER BY sequence DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
        .unwrap()
    };
    let initial = record("first", false, "0 2 * * *", false, "daily");
    let equivalent = record("second", true, " 0  2 * * * ", true, "daily");
    assert_eq!(initial, equivalent);
    let changed_time = record("third", false, "0 3 * * *", false, "daily");
    assert_eq!(initial.0, changed_time.0);
    assert_ne!(initial.1, changed_time.1);
    let moved_schedule = record("fourth", false, "0 2 * * *", false, "nightly");
    assert_eq!(initial.0, moved_schedule.0);
    assert_ne!(initial.1, moved_schedule.1);
}

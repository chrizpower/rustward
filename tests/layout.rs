use std::{
    collections::BTreeMap,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::Path,
};

use rusqlite::Connection;
use rustward::{
    admission::{Admission, Error, Store},
    config::{Config, load},
};
use tempfile::TempDir;

const SERVICE: &str = include_str!("../examples/rustward.toml");
const TASK: &str = include_str!("../examples/tasks/synthetic-report.toml");

fn write(root: &Path, path: &str, source: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}
fn setup() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("tasks")).unwrap();
    fs::create_dir(root.path().join("schedules")).unwrap();
    write(root.path(), "rustward.toml", SERVICE);
    root
}
fn read(root: &Path) -> Result<Config, String> {
    load(root.join("rustward.toml"))
}
fn schedule(task: &str, cron: &str) -> String {
    format!("task = '{task}'\ncron = '{cron}'\n[inputs]\nname = 'sample'\n")
}

#[test]
fn nested_equal_basenames_and_separate_scopes() {
    let root = setup();
    for id in ["backup/database", "maintenance/database", "a1-b/report"] {
        write(root.path(), &format!("tasks/{id}.toml"), TASK);
        write(
            root.path(),
            &format!("schedules/{id}.toml"),
            schedule(id, "0 2 * * *"),
        );
    }
    let config = read(root.path()).unwrap();
    assert_eq!(
        config.task_ids().collect::<Vec<_>>(),
        ["a1-b/report", "backup/database", "maintenance/database"]
    );
    assert_eq!(config.schedules().len(), 3);
    assert_eq!(config.schedules()[1].id(), "backup/database");
    assert_eq!(config.schedules()[1].task(), "backup/database");
    assert_eq!(config.schedules()[1].cron(), "0 2 * * *");
    assert_eq!(
        config.schedules()[1].inputs()["name"].as_str(),
        Some("sample")
    );
    assert_eq!(
        config
            .arguments(
                "backup/database",
                &BTreeMap::from([("name".into(), "a b".into())])
            )
            .unwrap(),
        ["--name", "a b", "2", "false"]
    );
}

#[test]
fn rejects_invalid_files_folders_and_input_names() {
    for path in [
        "Upper.toml",
        "with_under.toml",
        "1first.toml",
        "a.TOML",
        ".toml",
        "a.txt",
        "a\\b.toml",
        ".hidden/a.toml",
        "Upper/a.toml",
        "a.b/c.toml",
        "a.toml/b.toml",
        "a/README",
        "a/.hidden",
    ] {
        let root = setup();
        write(root.path(), &format!("tasks/{path}"), TASK);
        assert!(read(root.path()).is_err(), "{path}");
    }
    let root = setup();
    fs::create_dir(root.path().join("tasks/Invalid")).unwrap();
    assert!(read(root.path()).is_err());
    fs::remove_dir(root.path().join("tasks/Invalid")).unwrap();
    fs::write(
        root.path()
            .join("tasks")
            .join(std::ffi::OsString::from_vec(vec![255])),
        TASK,
    )
    .unwrap();
    assert!(read(root.path()).is_err());
    for source in [
        TASK.replace("id = \"name\"", "id = 'group/name'"),
        format!("id = 'duplicate'\n{TASK}"),
        format!("version = 2\n{TASK}"),
    ] {
        let root = setup();
        write(root.path(), "tasks/report.toml", source);
        assert!(read(root.path()).is_err());
    }
}

#[test]
fn rejects_symlinks_at_every_loaded_path_level() {
    for scope in ["tasks", "schedules"] {
        for target in ["file", "folder", "missing"] {
            let root = setup();
            write(root.path(), "file", TASK);
            fs::create_dir(root.path().join("folder")).unwrap();
            symlink(
                root.path().join(target),
                root.path().join(scope).join("link.toml"),
            )
            .unwrap();
            assert!(read(root.path()).is_err(), "{scope}/{target}");
        }
        let root = setup();
        fs::rename(root.path().join(scope), root.path().join("real")).unwrap();
        symlink("real", root.path().join(scope)).unwrap();
        assert!(read(root.path()).is_err());
    }
    let root = setup();
    fs::rename(
        root.path().join("rustward.toml"),
        root.path().join("real.toml"),
    )
    .unwrap();
    symlink("real.toml", root.path().join("rustward.toml")).unwrap();
    assert!(read(root.path()).is_err());
    let outer = tempfile::tempdir().unwrap();
    let root = setup();
    symlink(root.path(), outer.path().join("alias")).unwrap();
    assert!(read(&outer.path().join("alias")).is_err());
    write(
        root.path(),
        "rustward.toml",
        format!(
            "{SERVICE}\n[definitions]\ntasks = '{}'\n",
            outer.path().join("alias/tasks").display()
        ),
    );
    assert!(read(root.path()).is_err());
}

#[test]
fn rejects_special_files_and_invalid_utf8() {
    let root = setup();
    let _socket =
        std::os::unix::net::UnixListener::bind(root.path().join("tasks/socket.toml")).unwrap();
    assert!(read(root.path()).is_err());
    fs::remove_file(root.path().join("tasks/socket.toml")).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        root.path().join("tasks/pipe.toml"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    assert!(read(root.path()).is_err());
    fs::remove_file(root.path().join("tasks/pipe.toml")).unwrap();
    write(root.path(), "tasks/report.toml", [255]);
    assert!(read(root.path()).is_err());
}

#[test]
fn validates_references_and_all_fixed_inputs_before_returning_a_set() {
    let root = setup();
    write(root.path(), "tasks/report.toml", TASK);
    let previous = read(root.path()).unwrap();
    for source in [
        schedule("missing", "0 2 * * *"),
        schedule("../report", "0 2 * * *"),
        "task = 'report'\ncron = '0 2 * * *'".into(),
        schedule("report", "0 2 * * *").replace("name = 'sample'", "name = 1"),
        format!("{}count = 4", schedule("report", "0 2 * * *")),
        format!("{}other = true", schedule("report", "0 2 * * *")),
        format!("{}count = [1]", schedule("report", "0 2 * * *")),
        format!("{}count = 1.0", schedule("report", "0 2 * * *")),
        format!("id = 'duplicate'\n{}", schedule("report", "0 2 * * *")),
        format!("version = 2\n{}", schedule("report", "0 2 * * *")),
    ] {
        write(root.path(), "schedules/bad.toml", source);
        assert!(read(root.path()).is_err());
        assert_eq!(previous.task_count(), 1);
        assert!(previous.schedules().is_empty());
    }
    write(
        root.path(),
        "schedules/bad.toml",
        schedule("report", "0 2 * * *"),
    );
    write(root.path(), "tasks/broken.toml", "label = 'incomplete'");
    assert!(read(root.path()).is_err());
    fs::remove_file(root.path().join("tasks/broken.toml")).unwrap();
    assert!(read(root.path()).is_ok());
    write(
        root.path(),
        "tasks/report.toml",
        TASK.replace("required = true", "default = 'sample'"),
    );
    write(
        root.path(),
        "schedules/bad.toml",
        "task = 'report'\ncron = '0 2 * * *'",
    );
    assert!(read(root.path()).is_ok());
}

#[test]
fn validates_numeric_cron_syntax_without_executing_schedules() {
    let root = setup();
    write(root.path(), "tasks/report.toml", TASK);
    for cron in [
        "* * * * *",
        "0,15,30,45 0-23/2 1 * 1",
        "*/5 2 1-31/2 1-12 0-6",
        "0 2 1 * 1",
        "0/5 2 * * *",
        "  0   2 * * *  ",
    ] {
        write(
            root.path(),
            "schedules/report.toml",
            schedule("report", cron),
        );
        assert!(read(root.path()).is_ok(), "{cron}");
    }
    for cron in [
        "",
        "0 2 * *",
        "0 0 2 * * *",
        "@daily",
        "0 2 * JAN *",
        "0 2 * * 7",
        "60 2 * * *",
        "0 24 * * *",
        "0 2 0 * *",
        "0 2 * 13 *",
        "*/0 2 * * *",
        "*/-1 2 * * *",
        "1-0 2 * * *",
        "1,,2 2 * * *",
        "+1 2 * * *",
        "1/ 2 * * *",
        "1/2/3 2 * * *",
        "1-2-3 2 * * *",
        "99999999999999999999 2 * * *",
    ] {
        write(
            root.path(),
            "schedules/report.toml",
            schedule("report", cron),
        );
        assert!(read(root.path()).is_err(), "{cron}");
    }
}

#[test]
fn service_defaults_paths_and_strict_schema() {
    let root = setup();
    let config = read(root.path()).unwrap();
    let service = config.service();
    assert_eq!(service.definitions.tasks, root.path().join("tasks"));
    assert_eq!(service.definitions.schedules, root.path().join("schedules"));
    assert_eq!(service.limits.active_runs, 1);
    assert_eq!(service.limits.queued_runs, 100);
    assert_eq!(service.limits.log_bytes_per_run, 10485760);
    assert_eq!(service.limits.retained_log_bytes, 1073741824);
    assert_eq!(service.retention.days, 30);
    write(
        root.path(),
        "rustward.toml",
        SERVICE
            .replace("/synthetic/state/rustward.db", "state.db")
            .replace("/synthetic/logs", "logs"),
    );
    let config = read(root.path()).unwrap();
    assert_eq!(
        config.service().storage.state_database,
        root.path().join("state.db")
    );
    assert_eq!(
        config.service().storage.log_directory,
        root.path().join("logs")
    );
    for source in [
        SERVICE.replace("version = 2", "version = 1"),
        SERVICE.replace("version = 2", ""),
        SERVICE.replace("state_database", "unknown"),
        SERVICE.replace("log_directory", "unknown"),
        "version = 1\ntasks = []".into(),
        format!("unknown = true\n{SERVICE}"),
        format!("{SERVICE}\n[definitions]\nunknown = 'tasks'"),
        format!("{SERVICE}\n[definitions]\ntasks = ''"),
        format!("{SERVICE}\n[definitions]\ntasks = \"a\\u0000b\""),
        format!("{SERVICE}\n[limits]\nunknown = 1"),
        format!("{SERVICE}\n[limits]\nactive_runs = -1"),
        format!("{SERVICE}\n[retention]\nunknown = 1"),
        format!("{SERVICE}\n[retention]\ndays = 1.5"),
        format!("{SERVICE}\n[definitions]\ntasks = 'missing'"),
        format!("{SERVICE}\n[definitions]\ntasks = 'rustward.toml'"),
    ] {
        write(root.path(), "rustward.toml", source);
        assert!(read(root.path()).is_err());
    }
    assert!(load(root.path().join("other.toml")).is_err());
    fs::remove_file(root.path().join("rustward.toml")).unwrap();
    assert!(read(root.path()).is_err());
}

#[test]
fn enforces_file_set_entry_and_depth_bounds() {
    let root = setup();
    write(root.path(), "tasks/large.toml", vec![b' '; 1024 * 1024 + 1]);
    assert!(read(root.path()).unwrap_err().contains("size limit"));
    fs::remove_file(root.path().join("tasks/large.toml")).unwrap();
    for i in 0..16 {
        write(
            root.path(),
            &format!("tasks/file-{i}.toml"),
            vec![b' '; 1024 * 1024],
        );
    }
    assert!(read(root.path()).unwrap_err().contains("size limit"));
    let root = setup();
    for i in 0..10001 {
        fs::create_dir(root.path().join(format!("tasks/folder-{i}"))).unwrap();
    }
    assert!(read(root.path()).unwrap_err().contains("entry limit"));
    let root = setup();
    let prefix = "folder/".repeat(32);
    write(root.path(), &format!("tasks/{prefix}report.toml"), TASK);
    assert!(read(root.path()).is_ok());
    fs::create_dir(root.path().join(format!("tasks/{prefix}extra"))).unwrap();
    assert!(read(root.path()).unwrap_err().contains("depth limit"));
}

fn admit(store: &mut Store, key: &str, config: &Config, task: &str) -> Result<Admission, Error> {
    store.admit(
        key,
        config,
        task,
        &BTreeMap::from([("name".into(), "sample".into())]),
        "runtime-1",
    )
}
fn revisions(db: &Connection) -> Vec<(Vec<u8>, Vec<u8>)> {
    db.prepare("SELECT task_revision, config_revision FROM runs ORDER BY sequence")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn relocation_moves_removal_and_admission_keep_the_correct_identities() {
    let root = setup();
    write(root.path(), "tasks/backup/report.toml", TASK);
    write(
        root.path(),
        "schedules/daily.toml",
        schedule("backup/report", "0 2 * * *"),
    );
    let database = root.path().join("state.db");
    let mut store = Store::open(&database).unwrap();
    let db = Connection::open(&database).unwrap();
    let key = store.issue_key().unwrap();
    let Admission::Admitted { run_id } = admit(
        &mut store,
        &key,
        &read(root.path()).unwrap(),
        "backup/report",
    )
    .unwrap() else {
        panic!()
    };
    let original = revisions(&db)[0].clone();
    let original_row: (String, Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT task_id, task_snapshot, fingerprint FROM runs JOIN admissions USING (run_id)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    fs::rename(root.path().join("tasks"), root.path().join("moved-tasks")).unwrap();
    fs::rename(
        root.path().join("schedules"),
        root.path().join("moved-schedules"),
    )
    .unwrap();
    write(
        root.path(),
        "rustward.toml",
        format!(
            "{SERVICE}\n[definitions]\ntasks = '{}'\nschedules = 'moved-schedules'\n[limits]\nqueued_runs = 25\n",
            root.path().join("moved-tasks").display()
        ),
    );
    let key2 = store.issue_key().unwrap();
    admit(
        &mut store,
        &key2,
        &read(root.path()).unwrap(),
        "backup/report",
    )
    .unwrap();
    assert_eq!(revisions(&db)[1], original);
    fs::rename(
        root.path().join("moved-tasks/backup"),
        root.path().join("moved-tasks/archive"),
    )
    .unwrap();
    assert!(read(root.path()).is_err());
    write(
        root.path(),
        "moved-schedules/daily.toml",
        schedule("archive/report", "0 2 * * *"),
    );
    let moved = read(root.path()).unwrap();
    assert_eq!(
        admit(&mut store, &key, &moved, "backup/report"),
        Ok(Admission::AlreadyAdmitted {
            run_id: run_id.clone()
        })
    );
    assert_eq!(
        admit(&mut store, &key, &moved, "archive/report"),
        Err(Error::PayloadConflict)
    );
    let key3 = store.issue_key().unwrap();
    admit(&mut store, &key3, &moved, "archive/report").unwrap();
    assert_ne!(revisions(&db)[2].0, original.0);
    assert_ne!(revisions(&db)[2].1, original.1);
    fs::rename(
        root.path().join("moved-tasks/archive/report.toml"),
        root.path().join("moved-tasks/archive/renamed.toml"),
    )
    .unwrap();
    fs::rename(
        root.path().join("moved-schedules/daily.toml"),
        root.path().join("moved-schedules/nightly.toml"),
    )
    .unwrap();
    write(
        root.path(),
        "moved-schedules/nightly.toml",
        schedule("archive/renamed", "0 2 * * *"),
    );
    let moved = read(root.path()).unwrap();
    assert_eq!(moved.schedules()[0].id(), "nightly");
    assert_eq!(moved.task_ids().collect::<Vec<_>>(), ["archive/renamed"]);
    fs::remove_dir_all(root.path().join("moved-tasks/archive")).unwrap();
    fs::remove_file(root.path().join("moved-schedules/nightly.toml")).unwrap();
    let empty = read(root.path()).unwrap();
    drop(store);
    let mut store = Store::open(&database).unwrap();
    assert_eq!(
        admit(&mut store, &key, &empty, "backup/report"),
        Ok(Admission::AlreadyAdmitted { run_id })
    );
    let retained: (String, Vec<u8>, Vec<u8>) = db.query_row("SELECT task_id, task_snapshot, fingerprint FROM runs JOIN admissions USING (run_id) ORDER BY sequence LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(retained, original_row);
    assert_eq!(revisions(&db)[0], original);
}

#![cfg(target_os = "linux")]

//! Use only a separately prepared synthetic fixture. No program is executed.
use rustward::{
    runtime::{Catalog, Limits, Protection, WorkRoot},
    supervision::{Launch, RunId},
};
use std::{fs::OpenOptions, path::PathBuf};

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

#[test]
#[ignore = "requires an isolated host with a separately owned, protected synthetic runtime and private work root"]
fn prepared_runtime_host_permissions() {
    let execution_uid = rustix::process::geteuid().as_raw();
    let protection = Protection {
        preparation_uid: required("RUSTWARD_PREPARATION_UID").parse().unwrap(),
        execution_uid,
    };
    let catalog = Catalog::open(
        PathBuf::from(required("RUSTWARD_RUNTIME_CATALOG")),
        protection,
        Limits::default(),
    )
    .unwrap();
    let verified = catalog.verify(&required("RUSTWARD_RUNTIME_ID")).unwrap();
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).unwrap();
    let run_id = RunId::new(&hex::encode(bytes)).unwrap();
    let launch = Launch {
        run_id: run_id.clone(),
        executable: required("RUSTWARD_RUNTIME_PROGRAM"),
        working_directory: required("RUSTWARD_RUNTIME_CWD"),
        arguments: vec![],
        cleanup_grace: std::time::Duration::from_secs(30),
    };
    verified.check_command(&launch).unwrap();
    assert!(
        OpenOptions::new()
            .write(true)
            .open(&launch.executable)
            .is_err()
    );
    assert!(
        OpenOptions::new()
            .write(true)
            .open(verified.path().join("runtime.toml"))
            .is_err()
    );
    let work = WorkRoot::open(PathBuf::from(required("RUSTWARD_RUNTIME_WORK_ROOT"))).unwrap();
    assert_eq!(work.owner_uid(), execution_uid);
    let directory = work.create(&run_id).unwrap();
    let file = directory.path().join("synthetic-evidence");
    std::fs::write(&file, b"writable run data").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"writable run data");
    directory.check().unwrap();
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(directory.path()).unwrap();
}

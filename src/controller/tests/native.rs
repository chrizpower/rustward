use super::*;
use crate::runtime::{self, Catalog, WorkRoot, host};
use sha2::{Digest, Sha256};
fn mode(path: &Path, bits: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
}
pub(crate) struct Native {
    _outer: tempfile::TempDir,
    pub(crate) roots: Roots,
    path: PathBuf,
    pub(crate) id: String,
    source: String,
    db: PathBuf,
}
impl Native {
    pub(crate) fn new(host_ref: bool) -> Self {
        let base = std::env::var_os("RUSTWARD_TEST_FIXTURE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let outer = tempfile::Builder::new()
            .prefix(".runtime-fixture-controller-")
            .tempdir_in(base)
            .unwrap();
        mode(outer.path(), 0o755);
        let uid = rustix::process::geteuid().as_raw();
        let policy = runtime::Protection {
            preparation_uid: uid,
            execution_uid: uid + 1,
        };
        for (path, bits) in [
            ("runtimes", 0o755),
            ("hosts", 0o755),
            ("work", 0o700),
            ("logs", 0o700),
        ] {
            fs::create_dir(outer.path().join(path)).unwrap();
            mode(&outer.path().join(path), bits);
        }
        let host_source = format!(
            "version=1\nlauncher='/usr/bin/env'\n[files.'/usr/bin/env']\nsha256='{}'\nexecutable=true\n",
            hex::encode(Sha256::digest(fs::read("/usr/bin/env").unwrap()))
        );
        let host_id = host::manifest_id(&host_source).unwrap();
        let host_path = outer.path().join("hosts").join(format!("{host_id}.toml"));
        fs::write(&host_path, host_source).unwrap();
        mode(&host_path, 0o644);
        let mut source = format!(
            "version=1\ndirectories=['bin']\n[files.'bin/task']\nsha256='{}'\nexecutable=true\n",
            hex::encode(Sha256::digest(b"synthetic"))
        );
        if host_ref {
            source = format!("host='{host_id}'\n{source}");
        }
        let id = runtime::manifest_id(&source).unwrap();
        let path = outer.path().join("runtimes").join(&id);
        fs::create_dir(&path).unwrap();
        mode(&path, 0o755);
        fs::create_dir(path.join("bin")).unwrap();
        mode(&path.join("bin"), 0o755);
        fs::write(path.join("bin/task"), b"synthetic").unwrap();
        mode(&path.join("bin/task"), 0o755);
        fs::write(path.join("runtime.toml"), &source).unwrap();
        mode(&path.join("runtime.toml"), 0o644);
        let roots = Roots {
            runtimes: Catalog::open(
                outer.path().join("runtimes"),
                policy,
                runtime::Limits::default(),
            )
            .unwrap(),
            hosts: host::Catalog::open(
                outer.path().join("hosts"),
                policy,
                runtime::Limits::default(),
            )
            .unwrap(),
            work: WorkRoot::open(outer.path().join("work")).unwrap(),
            logs: WorkRoot::open(outer.path().join("logs")).unwrap(),
        };
        let db = outer.path().join("state.db");
        Self {
            _outer: outer,
            roots,
            path,
            id,
            source,
            db,
        }
    }
    fn permit(&self) -> (Session, dispatch::Permit) {
        let config_root = self._outer.path().join("config");
        fs::create_dir(&config_root).unwrap();
        fs::create_dir(config_root.join("tasks")).unwrap();
        fs::create_dir(config_root.join("schedules")).unwrap();
        fs::write(
            config_root.join("rustward.toml"),
            "version=2\n[storage]\nstate_database='state'\nlog_directory='logs'\n",
        )
        .unwrap();
        fs::write(
            config_root.join("tasks/task.toml"),
            format!(
                "label='Synthetic'\narguments=[]\ninputs=[]\nexecutable='{}'\nworking_directory='{}'\n",
                self.path.join("bin/task").display(),
                self.path.display()
            ),
        )
        .unwrap();
        let config = crate::config::load(config_root.join("rustward.toml")).unwrap();
        let mut store = Store::open(&self.db).unwrap();
        let key = store.issue_key().unwrap();
        store
            .admit(&key, &config, "task", &BTreeMap::new(), &self.id)
            .unwrap();
        let mut session = Session::open(&self.db, Limits::default()).unwrap();
        session.set_paused(false).unwrap();
        let dispatch::Claim::Ready(permit) = session.claim_next().unwrap() else {
            panic!()
        };
        (session, permit)
    }
}
#[test]
fn native_preparation_verifies_host_creates_private_roots_and_preserves_command() {
    let n = Native::new(true);
    let (_session, permit) = n.permit();
    let registry = Registry::new(Safety::default());
    registry.insert(
        permit.launch().run_id.clone(),
        None,
        false,
        false,
        Duration::from_secs(30),
    );
    let mut prepared = n.roots.prepare(permit, 31, &registry);
    assert!(prepared.failure.is_none());
    assert_eq!(
        prepared.permit.launch().working_directory,
        n.path.to_str().unwrap()
    );
    assert_eq!(
        prepared.profile.as_ref().unwrap().environment()["HOME"],
        prepared.work.as_ref().unwrap().path().to_str().unwrap()
    );
    assert_eq!(
        n.roots.runtimes.verify(&n.id).unwrap().host_id(),
        Some(
            host::manifest_id(
                &fs::read_to_string(
                    n.roots
                        .hosts
                        .path()
                        .read_dir()
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .path()
                )
                .unwrap()
            )
            .unwrap()
            .as_str()
        )
    );
    drop(prepared.writers.take());
    let report = prepared.capture.take().unwrap().finish().unwrap();
    assert_eq!(report.retained_bytes(), 0);
    assert!(
        n.roots
            .recover(&prepared.permit.launch().run_id, 31)
            .is_ok()
    );
    assert!(!n.roots.binding(0));
    assert!(!n.roots.binding(n.roots.runtimes.execution_uid()));
    assert!(matches!(
        Controller::open(
            &n.db,
            n.roots.clone(),
            Limits::default(),
            Fake::default(),
            Safety::default()
        ),
        Err(crate::controller::Error::Context)
    ));
}
#[test]
fn native_preparation_refuses_missing_host_changed_runtime_and_capture_collision() {
    for change in 0..4 {
        let n = Native::new(change != 0);
        let (_session, permit) = n.permit();
        let registry = Registry::new(Safety::default());
        match change {
            1 => fs::write(n.path.join("bin/task"), b"changed").unwrap(),
            2 => {
                fs::create_dir(n.roots.logs.path().join(permit.launch().run_id.as_str())).unwrap();
            }
            3 => fs::write(
                n.path.join("runtime.toml"),
                n.source.replace("executable=true", "executable=false"),
            )
            .unwrap(),
            _ => {}
        }
        let prepared = n.roots.prepare(permit, 31, &registry);
        assert!(prepared.failure.is_some());
        assert!(prepared.capture.is_none());
    }
}

#[test]
fn replaced_private_log_root_is_rejected_in_setup_and_recovery() {
    let n = Native::new(true);
    let (_session, permit) = n.permit();
    let id = permit.launch().run_id.clone();
    let registry = Registry::new(Safety::default());
    fs::rename(n.roots.logs.path(), n._outer.path().join("old-logs")).unwrap();
    fs::create_dir(n.roots.logs.path()).unwrap();
    mode(n.roots.logs.path(), 0o700);
    let prepared = n.roots.prepare(permit, 31, &registry);
    assert!(matches!(
        prepared.failure,
        Some(Failure::Runtime(dispatch::LaunchFailure::WorkDirectory))
    ));
    assert!(prepared.capture.is_none());
    assert!(n.roots.recover(&id, 31).is_err());
}

#[test]
fn removal_checks_the_pinned_root_and_retries_absent_output() {
    let fixture = Native::new(true);
    let id = RunId::new(&"a".repeat(64)).unwrap();
    let (capture, writers) = Capture::create(
        fixture.roots.logs.path(),
        &id,
        capture::Options { retained_bytes: 16 },
    )
    .unwrap();
    drop(writers);
    drop(capture);
    fixture.roots.remove_logs(&id).unwrap();
    fixture.roots.remove_logs(&id).unwrap();
    let logs = fixture.roots.logs.path();
    fs::rename(logs, logs.with_extension("old")).unwrap();
    fs::create_dir(logs).unwrap();
    mode(logs, 0o700);
    assert_eq!(
        fixture.roots.remove_logs(&id),
        Err(admission::Error::Storage)
    );
}

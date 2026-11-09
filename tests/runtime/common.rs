#![allow(dead_code)]
use rustward::{
    runtime::{Catalog, Limits, Protection, WorkRoot, manifest_id},
    supervision::{Launch, RunId},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub struct RuntimeFixture {
    pub outer: tempfile::TempDir,
    pub catalog_path: PathBuf,
    pub work_path: PathBuf,
    pub policy: Protection,
    pub id: String,
    pub source: String,
}
impl RuntimeFixture {
    pub fn new() -> Self {
        // The real metadata describes a preparation user. A different numeric
        // execution UID tests the policy without launching or changing users.
        let outer = tempfile::Builder::new()
            .prefix(".runtime-fixture-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        mode(outer.path(), 0o755);
        let preparation_uid = fs::metadata(outer.path()).unwrap().uid();
        let policy = Protection {
            preparation_uid,
            execution_uid: preparation_uid + 1,
        };
        let catalog_path = outer.path().join("runtimes");
        fs::create_dir(&catalog_path).unwrap();
        mode(&catalog_path, 0o755);
        let work_path = outer.path().join("work");
        fs::create_dir(&work_path).unwrap();
        mode(&work_path, 0o700);
        let mut fixture = Self {
            outer,
            catalog_path,
            work_path,
            policy,
            id: String::new(),
            source: String::new(),
        };
        let (id, source) = fixture.add(b"#!/synthetic/interpreter\nfixed code A\n");
        fixture.id = id;
        fixture.source = source;
        fixture
    }
    pub fn add(&self, content: &[u8]) -> (String, String) {
        let source = format!(
            "version=1\ndirectories=['bin','data','empty']\n[files.'bin/task']\nsha256='{}'\nexecutable=true\n[files.'data/value']\nsha256='{}'\n",
            hex::encode(Sha256::digest(content)),
            hex::encode(Sha256::digest(b"fixed data"))
        );
        let id = manifest_id(&source).unwrap();
        let path = self.catalog_path.join(&id);
        fs::create_dir(&path).unwrap();
        mode(&path, 0o755);
        for dir in ["bin", "data", "empty"] {
            fs::create_dir(path.join(dir)).unwrap();
            mode(&path.join(dir), 0o755);
        }
        fs::write(path.join("bin/task"), content).unwrap();
        mode(&path.join("bin/task"), 0o755);
        fs::write(path.join("data/value"), b"fixed data").unwrap();
        mode(&path.join("data/value"), 0o644);
        fs::write(path.join("runtime.toml"), &source).unwrap();
        mode(&path.join("runtime.toml"), 0o644);
        (id, source)
    }
    pub fn path(&self) -> PathBuf {
        self.catalog_path.join(&self.id)
    }
    pub fn catalog(&self) -> Catalog {
        Catalog::open(&self.catalog_path, self.policy, Limits::default()).unwrap()
    }
    pub fn work(&self) -> WorkRoot {
        WorkRoot::open(&self.work_path).unwrap()
    }
    pub fn launch(&self) -> Launch {
        Launch {
            run_id: RunId::new(&"1".repeat(64)).unwrap(),
            executable: self.path().join("bin/task").to_str().unwrap().into(),
            working_directory: self.path().to_str().unwrap().into(),
            arguments: vec!["space and 'quotes'".into(), "2".into(), "false".into()],
            cleanup_grace: std::time::Duration::from_secs(30),
        }
    }
}
pub fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

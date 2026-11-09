use super::*;
use sha2::{Digest, Sha256};

#[test]
fn manifest_normalization_has_a_stable_versioned_encoding() {
    let hash = hex::encode(Sha256::digest(b"x"));
    let a = format!("version=1\n[files.x]\nsha256='{hash}'\n");
    let b = format!(
        "# comment\nversion = 1\ndirectories=[]\n[files.x]\nexecutable=false\nsha256=\"{hash}\"\n"
    );
    let normalized = serde_json::json!({"version":1,"directories":[],"files":{"x":{"sha256":hash,"executable":false}}});
    let expected = hex::encode(
        Sha256::new()
            .chain_update(b"rustward-runtime-v1")
            .chain_update(serde_json::to_vec(&normalized).unwrap())
            .finalize(),
    );
    assert_eq!(manifest_id(&a).unwrap(), expected);
    assert_eq!(manifest_id(&b).unwrap(), expected);
    let fixture = RuntimeFixture::new();
    assert_eq!(
        manifest_id(
            &fixture
                .source
                .replace("['bin','data','empty']", "['empty','bin','data']")
        )
        .unwrap(),
        fixture.id
    );
}

#[test]
fn rejects_invalid_manifest_schema_paths_hashes_and_folder_relationships() {
    let hash = "a".repeat(64);
    for source in [
        "version=2\nfiles={}".into(),
        "version=1\nfiles={}".into(),
        format!("version=1\nunknown=true\n[files.x]\nsha256='{hash}'"),
        format!("version=1\n[files.x]\nsha256='{hash}'\nunknown=true"),
        format!("version=1\ndirectories=['bin','bin']\n[files.x]\nsha256='{hash}'"),
        format!("version=1\ndirectories=['bin']\n[files.bin]\nsha256='{hash}'"),
        format!("version=1\n[files.'bin/task']\nsha256='{hash}'"),
        format!("version=1\ndirectories=['bin/data']\n[files.x]\nsha256='{hash}'"),
        "version=1\n[files.x]\nsha256='invalid'".into(),
    ] {
        assert!(manifest_id(&source).is_err(), "{source}");
    }
    for path in [
        "",
        "/x",
        "x/",
        "x//y",
        ".",
        "..",
        "x/../y",
        "x/./y",
        "runtime.toml",
        r"x\y",
    ] {
        let source = format!("version=1\n[files.'{path}']\nsha256='{hash}'");
        assert!(manifest_id(&source).is_err(), "{path}");
    }
}

#[test]
fn enforces_manifest_entry_content_depth_and_path_limits() {
    let fixture = RuntimeFixture::new();
    for limits in [
        Limits {
            manifest_bytes: 0,
            ..Limits::default()
        },
        Limits {
            entries: 0,
            ..Limits::default()
        },
        Limits {
            depth: 0,
            ..Limits::default()
        },
        Limits {
            path_bytes: 0,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            Catalog::open(&fixture.catalog_path, fixture.policy, limits),
            Err(Error::Limit)
        ));
    }
    for limits in [
        Limits {
            manifest_bytes: 1,
            ..Limits::default()
        },
        Limits {
            entries: 1,
            ..Limits::default()
        },
        Limits {
            content_bytes: 0,
            ..Limits::default()
        },
        Limits {
            depth: 1,
            ..Limits::default()
        },
        Limits {
            path_bytes: 1,
            ..Limits::default()
        },
    ] {
        let catalog = Catalog::open(&fixture.catalog_path, fixture.policy, limits).unwrap();
        assert!(catalog.verify(&fixture.id).is_err());
    }
}

#[test]
fn rejects_special_files_and_invalid_utf8_names_without_blocking() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = RuntimeFixture::new();
    let path = fixture.path().join(std::ffi::OsString::from_vec(vec![255]));
    fs::write(path, b"x").unwrap();
    assert!(matches!(
        fixture.catalog().verify(&fixture.id),
        Err(Error::Invalid)
    ));
    let fixture = RuntimeFixture::new();
    let fifo = fixture.path().join("data/value");
    fs::remove_file(&fifo).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert!(fixture.catalog().verify(&fixture.id).is_err());
}

#[test]
fn verifies_the_maximum_default_depth_with_one_bounded_hash_buffer() {
    let fixture = RuntimeFixture::new();
    let directories: Vec<String> = (1..64).map(|depth| vec!["x"; depth].join("/")).collect();
    let file = format!("{}/leaf", directories.last().unwrap());
    let hash = hex::encode(Sha256::digest(b"x"));
    let source = format!(
        "version=1\ndirectories={}\n[files.'{file}']\nsha256='{hash}'\n",
        serde_json::to_string(&directories).unwrap()
    );
    let id = manifest_id(&source).unwrap();
    let path = fixture.catalog_path.join(&id);
    fs::create_dir(&path).unwrap();
    mode(&path, 0o755);
    for directory in directories {
        let path = path.join(directory);
        fs::create_dir(&path).unwrap();
        mode(&path, 0o755);
    }
    fs::write(path.join(file), b"x").unwrap();
    fs::write(path.join("runtime.toml"), source).unwrap();
    assert_eq!(fixture.catalog().verify(&id).unwrap().id(), id);
}

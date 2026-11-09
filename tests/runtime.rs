#[path = "runtime/common.rs"]
mod common;
#[path = "runtime/host.rs"]
mod host;
#[path = "runtime/validation.rs"]
mod validation;
#[path = "runtime/work.rs"]
mod work;

use common::*;
use rustward::runtime::{Catalog, Error, Limits, Protection, manifest_id};
use std::{fs, os::unix::fs::symlink};

#[test]
fn verifies_complete_tree_and_retains_exact_command_paths_and_arguments() {
    let fixture = RuntimeFixture::new();
    let verified = fixture.catalog().verify(&fixture.id).unwrap();
    assert_eq!(verified.id(), fixture.id);
    assert_eq!(verified.path(), fixture.path());
    let launch = fixture.launch();
    verified.check_command(&launch).unwrap();
    assert_eq!(launch.arguments, ["space and 'quotes'", "2", "false"]);
    let mut nested = fixture.launch();
    nested.working_directory = fixture.path().join("data").to_str().unwrap().into();
    verified.check_command(&nested).unwrap();
    let mut empty = fixture.launch();
    empty.working_directory = fixture.path().join("empty").to_str().unwrap().into();
    verified.check_command(&empty).unwrap();
}

#[test]
fn changed_missing_unlisted_or_mismatched_runtime_never_verifies() {
    for operation in ["changed", "missing", "extra", "extra-directory", "manifest"] {
        let fixture = RuntimeFixture::new();
        let catalog = fixture.catalog();
        match operation {
            "changed" => fs::write(fixture.path().join("data/value"), b"changed data").unwrap(),
            "missing" => fs::remove_file(fixture.path().join("data/value")).unwrap(),
            "extra" => fs::write(fixture.path().join("extra"), b"unexpected").unwrap(),
            "extra-directory" => fs::create_dir(fixture.path().join("extra")).unwrap(),
            _ => fs::write(
                fixture.path().join("runtime.toml"),
                fixture
                    .source
                    .replace("executable=true", "executable=false"),
            )
            .unwrap(),
        }
        assert!(catalog.verify(&fixture.id).is_err(), "{operation}");
    }
    let fixture = RuntimeFixture::new();
    assert!(matches!(
        fixture.catalog().verify(&"0".repeat(64)),
        Err(Error::Missing)
    ));
    for id in ["runtime-old", "../outside", "", &"A".repeat(64)] {
        assert!(matches!(fixture.catalog().verify(id), Err(Error::Invalid)));
    }
}

#[test]
fn rejects_symlinks_in_catalog_runtime_manifest_files_and_directories() {
    for operation in ["catalog", "runtime", "manifest", "file", "directory"] {
        let fixture = RuntimeFixture::new();
        let catalog = fixture.catalog();
        let path = match operation {
            "catalog" => fixture.catalog_path.clone(),
            "runtime" => fixture.path(),
            "manifest" => fixture.path().join("runtime.toml"),
            "file" => fixture.path().join("data/value"),
            _ => fixture.path().join("data"),
        };
        let moved = path.with_extension("moved");
        fs::rename(&path, &moved).unwrap();
        symlink(&moved, &path).unwrap();
        if operation == "catalog" {
            assert!(
                Catalog::open(&fixture.catalog_path, fixture.policy, Limits::default()).is_err()
            );
        }
        assert!(catalog.verify(&fixture.id).is_err(), "{operation}");
    }
}

#[test]
fn rejects_unsafe_ownership_modes_and_command_escapes() {
    let fixture = RuntimeFixture::new();
    for policy in [
        Protection {
            preparation_uid: fixture.policy.preparation_uid,
            execution_uid: fixture.policy.preparation_uid,
        },
        Protection {
            preparation_uid: fixture.policy.preparation_uid,
            execution_uid: 0,
        },
        Protection {
            preparation_uid: fixture.policy.preparation_uid + 5,
            execution_uid: fixture.policy.execution_uid,
        },
    ] {
        assert!(matches!(
            Catalog::open(&fixture.catalog_path, policy, Limits::default()),
            Err(Error::Unsafe)
        ));
    }
    for (path, bits) in [
        ("bin/task", 0o777),
        ("bin/task", 0o4755),
        ("bin/task", 0o644),
        ("data/value", 0o755),
        ("bin", 0o775),
        ("runtime.toml", 0o755),
        ("data/value", 0o600),
    ] {
        let fixture = RuntimeFixture::new();
        mode(&fixture.path().join(path), bits);
        assert!(
            matches!(fixture.catalog().verify(&fixture.id), Err(Error::Unsafe)),
            "{path} {bits:o}"
        );
    }
    let verified = fixture.catalog().verify(&fixture.id).unwrap();
    for path in [
        fixture.catalog_path.join("outside"),
        fixture.path().join("bin/../bin/task"),
        fixture.path().join("data/value"),
        fixture.path().join("absent"),
    ] {
        let mut launch = fixture.launch();
        launch.executable = path.to_str().unwrap().into();
        assert_eq!(verified.check_command(&launch), Err(Error::Command));
    }
    let mut launch = fixture.launch();
    launch.working_directory = fixture.work_path.to_str().unwrap().into();
    assert_eq!(verified.check_command(&launch), Err(Error::Command));
}

#[test]
fn relocation_never_remaps_an_admitted_absolute_command() {
    let fixture = RuntimeFixture::new();
    let original = fixture.launch();
    let catalog = fixture.catalog();
    let moved = fixture.outer.path().join("moved-catalog");
    fs::rename(&fixture.catalog_path, &moved).unwrap();
    assert!(matches!(catalog.verify(&fixture.id), Err(Error::Missing)));
    let moved_catalog = Catalog::open(&moved, fixture.policy, Limits::default()).unwrap();
    let verified = moved_catalog.verify(&fixture.id).unwrap();
    assert_eq!(verified.id(), fixture.id);
    assert_eq!(verified.check_command(&original), Err(Error::Command));
}

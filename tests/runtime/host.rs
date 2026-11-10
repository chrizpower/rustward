use super::*;
use rustward::runtime::host;
use sha2::{Digest, Sha256};

fn source(fixture: &RuntimeFixture) -> String {
    format!(
        "version=1\nlauncher='/usr/bin/env'\n[files.'/usr/bin/env']\nsha256='{}'\nexecutable=true\n[files.'{}']\nsha256='{}'\n[links]\n'{}'='value'\n",
        hex::encode(Sha256::digest(fs::read("/usr/bin/env").unwrap())),
        fixture.path().join("data/value").display(),
        hex::encode(Sha256::digest(b"fixed data")),
        fixture.path().join("data/alias").display()
    )
}
fn setup(fixture: &RuntimeFixture, source: &str) -> (host::Catalog, String) {
    let path = fixture.outer.path().join("hosts");
    fs::create_dir(&path).unwrap();
    mode(&path, 0o755);
    let id = host::manifest_id(source).unwrap();
    fs::write(path.join(format!("{id}.toml")), source).unwrap();
    mode(&path.join(format!("{id}.toml")), 0o644);
    (
        host::Catalog::open(path, fixture.policy, Limits::default()).unwrap(),
        id,
    )
}
#[test]
fn verifies_pinned_host_files_and_explicit_links_without_changing_old_runtime_ids() {
    let f = RuntimeFixture::new();
    symlink("value", f.path().join("data/alias")).unwrap();
    let source = source(&f);
    let (catalog, id) = setup(&f, &source);
    let host = catalog.verify(&id).unwrap();
    assert_eq!(host.id(), id);
    assert_eq!(host.launcher(), std::path::Path::new("/usr/bin/env"));
    assert_eq!(
        host::manifest_id(&source.replace("launcher='/usr/bin/env'", "launcher = '/usr/bin/env'"))
            .unwrap(),
        id
    );
    assert_eq!(f.catalog().execution_uid(), f.policy.execution_uid);
    assert_eq!(catalog.preparation_uid(), f.policy.preparation_uid);
    assert_eq!(manifest_id(&f.source).unwrap(), f.id);
    let with_host = format!("host='{id}'\n{}", f.source);
    assert_ne!(manifest_id(&with_host).unwrap(), f.id);
    let invalid = format!("host='old'\n{}", f.source);
    assert_eq!(manifest_id(&invalid), Err(Error::Invalid));
}
#[test]
fn rejects_changed_missing_unsafe_or_unpinned_host_content() {
    for change in 0..7 {
        let f = RuntimeFixture::new();
        symlink("value", f.path().join("data/alias")).unwrap();
        let source = source(&f);
        let (catalog, id) = setup(&f, &source);
        match change {
            0 => fs::write(f.path().join("data/value"), b"changed").unwrap(),
            1 => fs::remove_file(f.path().join("data/value")).unwrap(),
            2 => mode(&f.path().join("data/value"), 0o666),
            3 => {
                fs::remove_file(f.path().join("data/alias")).unwrap();
                symlink("other", f.path().join("data/alias")).unwrap();
            }
            4 => fs::write(
                catalog.path().join(format!("{id}.toml")),
                source.replace("executable=true", "executable=false"),
            )
            .unwrap(),
            5 => {
                fs::remove_file(f.path().join("data/alias")).unwrap();
                fs::write(f.path().join("data/alias"), b"value").unwrap();
            }
            _ => mode(catalog.path(), 0o777),
        }
        assert!(catalog.verify(&id).is_err(), "{change}");
    }
}
#[test]
fn validates_host_schema_paths_links_and_limits() {
    let f = RuntimeFixture::new();
    let good = source(&f);
    for bad in [
        "".into(),
        good.replace("version=1", "version=2"),
        good.replace("launcher='/usr/bin/env'", "launcher='/bin/env'"),
        good.replace("/usr/bin/env", "relative"),
        good.replace("value'", "../alias'"),
        good.replace("value'", "../../../../../../../../outside'"),
        good.replace("[links]", "unknown=1\n[links]"),
        good.replace("/usr/bin/env", "/usr//bin/env"),
    ] {
        assert!(host::manifest_id(&bad).is_err(), "{bad}");
    }
    symlink("value", f.path().join("data/alias")).unwrap();
    let (catalog, id) = setup(&f, &good);
    assert!(matches!(catalog.verify("bad"), Err(Error::Invalid)));
    assert!(matches!(
        catalog.verify(&"0".repeat(64)),
        Err(Error::Missing)
    ));
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
            content_bytes: 1,
            ..Limits::default()
        },
        Limits {
            path_bytes: 4,
            ..Limits::default()
        },
    ] {
        let limited = host::Catalog::open(catalog.path(), f.policy, limits).unwrap();
        assert!(limited.verify(&id).is_err());
    }
    for policy in [
        Protection {
            execution_uid: 0,
            ..f.policy
        },
        Protection {
            execution_uid: f.policy.preparation_uid,
            ..f.policy
        },
    ] {
        assert!(host::Catalog::open(catalog.path(), policy, Limits::default()).is_err());
    }
    assert!(host::Catalog::open("relative", f.policy, Limits::default()).is_err());
}

#[test]
fn verifies_directory_aliases_that_reach_pinned_files_through_a_child_link() {
    let fixture = RuntimeFixture::new();
    let data = fixture.path().join("data");
    fs::create_dir(data.join("folder")).unwrap();
    mode(&data.join("folder"), 0o755);
    symlink("../value", data.join("folder/link")).unwrap();
    symlink("folder", data.join("alias-dir")).unwrap();
    let source = format!(
        "version=1\nlauncher='/usr/bin/env'\n[files.'/usr/bin/env']\nsha256='{}'\nexecutable=true\n[files.'{}']\nsha256='{}'\n[links]\n'{}'='folder'\n'{}'='../value'\n",
        hex::encode(Sha256::digest(fs::read("/usr/bin/env").unwrap())),
        data.join("value").display(),
        hex::encode(Sha256::digest(b"fixed data")),
        data.join("alias-dir").display(),
        data.join("folder/link").display()
    );
    let (catalog, id) = setup(&fixture, &source);
    catalog.verify(&id).unwrap();
    assert!(host::manifest_id(&format!("{source}'/unused-alias'='/empty'\n")).is_err());
}

use super::*;
use rustward::{runtime::WorkRoot, supervision::RunId};
use std::os::unix::fs::MetadataExt;

#[test]
fn creates_separate_private_work_directories_and_never_reuses_or_deletes_them() {
    let fixture = RuntimeFixture::new();
    let root = fixture.work();
    let a = root.create(&RunId::new(&"a".repeat(64)).unwrap()).unwrap();
    let b = root.create(&RunId::new(&"b".repeat(64)).unwrap()).unwrap();
    assert_ne!(a.path(), b.path());
    assert_eq!(root.owner_uid(), a.owner_uid());
    assert_eq!(fs::metadata(a.path()).unwrap().mode() & 0o777, 0o700);
    fs::write(a.path().join("retained"), b"data").unwrap();
    a.check().unwrap();
    b.check().unwrap();
    assert!(matches!(
        root.create(&RunId::new(&"a".repeat(64)).unwrap()),
        Err(Error::WorkDirectory)
    ));
    let path = a.path().to_owned();
    drop(a);
    assert_eq!(fs::read(path.join("retained")).unwrap(), b"data");
}

#[test]
fn rejects_unsafe_work_roots_symlinks_and_replaced_directories() {
    let fixture = RuntimeFixture::new();
    mode(&fixture.work_path, 0o755);
    assert!(matches!(
        WorkRoot::open(&fixture.work_path),
        Err(Error::WorkDirectory)
    ));
    mode(&fixture.work_path, 0o700);
    let root = fixture.work();
    let id = RunId::new(&"c".repeat(64)).unwrap();
    symlink(fixture.path(), fixture.work_path.join(id.as_str())).unwrap();
    assert!(matches!(root.create(&id), Err(Error::WorkDirectory)));
    let dir = root.create(&RunId::new(&"d".repeat(64)).unwrap()).unwrap();
    fs::rename(dir.path(), fixture.work_path.join("old")).unwrap();
    fs::create_dir(dir.path()).unwrap();
    mode(dir.path(), 0o700);
    assert_eq!(dir.check(), Err(Error::WorkDirectory));
    fs::rename(&fixture.work_path, fixture.outer.path().join("old-root")).unwrap();
    fs::create_dir(&fixture.work_path).unwrap();
    mode(&fixture.work_path, 0o700);
    assert!(matches!(
        root.create(&RunId::new(&"e".repeat(64)).unwrap()),
        Err(Error::WorkDirectory)
    ));
    let link = fixture.outer.path().join("work-link");
    symlink(&fixture.work_path, &link).unwrap();
    assert!(matches!(WorkRoot::open(link), Err(Error::WorkDirectory)));
    assert!(matches!(
        WorkRoot::open("relative"),
        Err(Error::WorkDirectory)
    ));
}

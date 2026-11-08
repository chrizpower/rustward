use super::*;

#[test]
fn checks_bounded_cgroup_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let mut groups = CgroupFs {
        root: directory.path().to_owned(),
    };
    assert_eq!(CgroupFs::default().root, PathBuf::from("/sys/fs/cgroup"));
    assert_eq!(groups.state("/unit"), GroupState::Unknown);
    std::fs::write(directory.path().join("cgroup.controllers"), "cpu memory").unwrap();
    for path in [
        "",
        "/",
        "relative",
        "/../unit",
        "/./unit",
        "/unit/",
        "/a//b",
        "/bad\0name",
    ] {
        assert_eq!(groups.state(path), GroupState::Unknown);
    }
    assert_eq!(groups.state("/unit"), GroupState::Absent);
    std::fs::create_dir(directory.path().join("unit")).unwrap();
    let events = directory.path().join("unit/cgroup.events");
    for (text, expected) in [
        ("populated 0\nfrozen 0\n", GroupState::Empty),
        ("populated 1\n", GroupState::Populated),
        ("frozen 0\n", GroupState::Unknown),
        ("populated 2\n", GroupState::Unknown),
        ("populated 0 extra\n", GroupState::Unknown),
        ("populated 0\npopulated 1\n", GroupState::Unknown),
    ] {
        std::fs::write(&events, text).unwrap();
        assert_eq!(groups.state("/unit"), expected);
    }
    std::fs::write(&events, vec![b'x'; 4097]).unwrap();
    assert_eq!(groups.state("/unit"), GroupState::Unknown);
    std::fs::write(&events, [255]).unwrap();
    assert_eq!(groups.state("/unit"), GroupState::Unknown);
    std::fs::remove_file(&events).unwrap();
    std::fs::create_dir(events).unwrap();
    assert_eq!(groups.state("/unit"), GroupState::Unknown);
}

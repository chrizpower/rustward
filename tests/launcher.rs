use std::{fs, process::Command};
#[test]
fn launcher_replaces_its_process_with_exact_arguments_and_clean_environment() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().to_str().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustward-exec"))
        .args(["/synthetic/runtime", work, "/usr/bin/env"])
        .env_clear()
        .env("SYNTHETIC_INHERITED", "private")
        .env("PATH", "/bad")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let lines = text.lines().collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "PATH=/synthetic/runtime/bin".into(),
        format!("HOME={work}"),
        format!("TMPDIR={work}"),
        format!("RUSTWARD_WORK_DIRECTORY={work}"),
        "LANG=C.UTF-8".into(),
        "LC_ALL=C.UTF-8".into(),
        "TZ=UTC".into(),
    ];
    assert_eq!(lines, expected.iter().map(String::as_str).collect());
    let executable = root.path().join("task=value");
    fs::copy("/usr/bin/printf", &executable).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustward-exec"))
        .args([
            "/synthetic/runtime",
            work,
            executable.to_str().unwrap(),
            "%s\n",
            "a=b",
            "two words",
            "$HOME;*",
            "",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a=b\ntwo words\n$HOME;*\n\n");
    let output = Command::new(env!("CARGO_BIN_EXE_rustward-exec"))
        .args(["/synthetic/runtime", work, "/synthetic/missing"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.stderr, b"Rustward could not start the command.\n");
}

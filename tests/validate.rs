use std::process::Command;

#[test]
fn validates_without_running_the_program() {
    // The example program and working directory do not exist.
    let output = Command::new(env!("CARGO_BIN_EXE_rustward"))
        .args(["validate", "examples/tasks.toml"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("1 tasks")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn reports_invalid_configuration_and_file_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_rustward"))
        .args(["validate", "Cargo.toml"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Invalid configuration")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rustward"))
        .args(["validate", "examples/missing.toml"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Cannot read configuration")
    );
}

#[test]
fn rejects_unsupported_cli_arguments() {
    for arguments in [
        vec![],
        vec!["run", "examples/tasks.toml"],
        vec!["validate"],
        vec!["validate", "examples/tasks.toml", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rustward"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr).unwrap().contains("Usage:"));
    }
}

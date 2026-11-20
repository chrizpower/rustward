use std::process::Command;

#[test]
fn validates_without_running_the_program() {
    // The example program and working directory do not exist.
    let output = Command::new(env!("CARGO_BIN_EXE_rustward"))
        .args(["validate", "examples/rustward.toml"])
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
            .contains("Invalid configuration")
    );
}

#[test]
fn rejects_unsupported_cli_arguments() {
    for arguments in [
        vec![],
        vec!["run", "examples/rustward.toml"],
        vec!["validate"],
        vec!["validate", "examples/rustward.toml", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rustward"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr).unwrap().contains("Usage:"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn service_reports_startup_and_control_errors() {
    let binary = env!("CARGO_BIN_EXE_rustward");
    let output = Command::new(binary)
        .args([
            "serve",
            "examples/missing.toml",
            "validation",
            "/runtimes",
            "/hosts",
            "/work",
            "missing",
            "0",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("service file must be named rustward.toml")
    );
    let output = Command::new(binary)
        .args(["ctl", "validation"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Control request"));
}

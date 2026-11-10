use super::*;
use rustward::{
    admission::{RunPhase, RunResult, RunState},
    capture::{Evidence, Report},
    dispatch::Record,
    supervision::{ExecutionIdentity, ProcessExit},
};
fn record() -> Record {
    Record {
        start_state: "accepted".into(),
        identity: Some(ExecutionIdentity {
            invocation: [1; 16],
            control_group: "/synthetic/unit".into(),
        }),
        exit: Some(ProcessExit::Code(0)),
        cleanup_confirmed: true,
        review_required: false,
        log_bytes: 0,
        stop_reason: None,
        output_failed: false,
        launch_failure: None,
        capture_setup_failure: None,
        output_evidence: Some(Evidence::from_report(Report::default(), true, false)),
    }
}
#[test]
fn acceptance_requires_completed_clean_and_synchronized_success() {
    let state = RunState {
        phase: RunPhase::Completed,
        result: Some(RunResult::Success),
    };
    proof::complete_success(&state, &record()).unwrap();
    for change in 0..8 {
        let mut changed = record();
        match change {
            0 => changed.cleanup_confirmed = false,
            1 => changed.review_required = true,
            2 => changed.output_failed = true,
            3 => changed.identity = None,
            4 => changed.exit = Some(ProcessExit::Code(1)),
            5 => changed.output_evidence = None,
            6 => changed.output_evidence.as_mut().unwrap().synchronized = false,
            _ => changed.output_evidence.as_mut().unwrap().recovered = true,
        }
        assert!(proof::complete_success(&state, &changed).is_err());
    }
    assert!(
        proof::complete_success(
            &RunState {
                phase: RunPhase::CleaningUp,
                result: Some(RunResult::Success)
            },
            &record()
        )
        .is_err()
    );
}
#[test]
fn duplicate_start_markers_cannot_pass_and_live_processes_are_detected() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("starts.jsonl"), "{}\n{}\n").unwrap();
    assert!(proof::starts(root.path()).is_err());
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let time = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    assert!(
        proof::process_ended(&serde_json::json!({"pid":std::process::id(),"start_time":time}))
            .is_err()
    );
    assert!(
        proof::process_ended(
            &serde_json::json!({"pid":std::process::id(),"start_time":"different"})
        )
        .is_ok()
    );
    assert!(proof::process_ended(&serde_json::json!({"pid":0,"start_time":"0"})).is_err());
}
#[test]
fn manifest_command_reuses_the_production_hash_encoding() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        format!("version=1\n[files.'task']\nsha256='{}'\n", "a".repeat(64)),
    )
    .unwrap();
    command(&[
        "id".into(),
        "runtime".into(),
        file.path().to_str().unwrap().into(),
    ])
    .unwrap();
    assert!(
        command(&[
            "id".into(),
            "unknown".into(),
            file.path().to_str().unwrap().into()
        ])
        .is_err()
    );
    assert!(command(&[]).is_err());
}

#[test]
fn prepared_tasks_use_the_accepted_layout_and_argument_generation() {
    let root = tempfile::tempdir().unwrap();
    let code = "import sys;from pathlib import Path;sys.path.insert(0,sys.argv[1]);from prepare import configuration;configuration(Path(sys.argv[2]),Path('/synthetic/runtime'))";
    let status = std::process::Command::new("python3")
        .args(["-I", "-B", "-c", code])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("acceptance"))
        .arg(root.path())
        .status()
        .unwrap();
    assert!(status.success());
    let config = rustward::config::load(root.path().join("configuration/rustward.toml")).unwrap();
    assert_eq!(config.task_count(), 3);
    assert_eq!(
        config
            .arguments("local-playbook", &execution::values())
            .unwrap(),
        ["-I", "-B", "playbook.py", proof::TEXT, "7", "true"]
    );
    assert_eq!(
        config.arguments("ordinary", &execution::values()).unwrap(),
        [
            "-I",
            "-B",
            "ordinary.py",
            "complete",
            proof::TEXT,
            "7",
            "true"
        ]
    );
}

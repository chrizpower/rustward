use super::{Result, context::Context, execution, proof, require};
use rustward::{
    admission::{RunPhase, RunResult, Store},
    safety::Safety,
};
use serde_json::json;
use std::{fs, path::Path, time::Duration};

pub fn run(root: &Path, kind: &str) -> Result<()> {
    let (case, task, marker, code, useful) = match kind {
        "task" => (
            "A04-task-failure",
            "remote-failure",
            "A04_TASK_FAILURE",
            2,
            "A04_EXPECTED_REMOTE_FAILURE",
        ),
        "unreachable" => (
            "A04-unreachable",
            "remote-unreachable",
            "A04_UNREACHABLE",
            4,
            "UNREACHABLE!",
        ),
        _ => return Err("expected task or unreachable failure case".into()),
    };
    let context = Context::load(root)?;
    let safety = Safety::default();
    let mut store = Store::open_with_safety(context.database(), safety.clone())
        .map_err(|e| format!("store: {e:?}"))?;
    let mut controller = context.controller(safety)?;
    let id = execution::admit(&context, &mut store, task)?;
    context.evidence(&format!("{case}-admission.json"), &json!({"case":case,"run_id":id,"expected_result":"Failure","expected_exit":code,"inputs":proof::inputs()}))?;
    let mut observed = serde_json::Value::Null;
    let outcome = (|| {
        controller
            .set_paused(false)
            .map_err(|e| format!("resume: {e:?}"))?;
        execution::start(&mut controller, &id)?;
        execution::drive(&mut controller, &id, Duration::from_secs(120))?;
        let state = controller
            .run_state(&id)
            .map_err(|e| format!("state: {e:?}"))?
            .ok_or("missing state")?;
        let record = controller
            .record(&id)
            .map_err(|e| format!("record: {e:?}"))?
            .ok_or("missing record")?;
        observed = json!({"state":state,"record":record});
        proof::complete_failure(&state, &record, code)?;
        let parent = proof::workload(&context, &id, marker, &record)?;
        let stdout = fs::read_to_string(context.root.join("logs").join(&id).join("stdout.log"))?;
        require(
            stdout.contains(useful),
            "missing expected failure diagnostic",
        )?;
        if kind == "unreachable" {
            require(
                stdout.contains("Failed to connect to the host via ssh"),
                "missing SSH connection failure",
            )?;
        }
        require(
            controller.active_runs() == 0 && !controller.failed(),
            "failed run did not release healthy capacity after cleanup",
        )?;
        drop(controller);
        drop(store);
        let mut reopened = Store::open(context.database()).map_err(|e| format!("reopen: {e:?}"))?;
        let retained = reopened
            .run_state(&id)
            .map_err(|e| format!("retained state: {e:?}"))?
            .ok_or("failure disappeared")?;
        require(
            retained.phase == RunPhase::Completed && retained.result == Some(RunResult::Failure),
            "failure was not durable",
        )?;
        context.evidence(&format!("{case}.json"), &json!({"case":case,"status":"passed","run_id":id,"expected_result":"Failure","expected_exit":code,"runtime_id":context.fixture.runtime_id,"host_id":context.fixture.host_id,"state":state,"record":record,"inputs":proof::inputs(),"start":parent}))
    })();
    if let Err(error) = outcome {
        let report = json!({"case":case,"status":"failed","run_id":id,"reason":error.to_string(),"observed":observed});
        eprintln!("Acceptance decision: {report}");
        context.evidence(&format!("{case}.json"), &report)?;
        return Err(error);
    }
    println!("{case} passed with the expected Failure result.");
    Ok(())
}

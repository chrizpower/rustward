use super::{
    Result,
    context::{Context, Native},
    execution, proof, require,
};
use rustward::{
    admission::{Cancellation, RunPhase, RunResult, Store},
    controller::Start,
    dispatch::{Block, StopDecision},
    safety::Safety,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn queued(context: &Context, store: &mut Store, controller: &mut Native) -> Result<String> {
    let id = execution::admit(context, store, "remote-queued")?;
    context.evidence(
        "A05-queued-admission.json",
        &json!({"case":"A05-queued","run_id":id}),
    )?;
    require(
        store
            .cancel_queued(&id)
            .map_err(|e| format!("queued cancel: {e:?}"))?
            == Cancellation::Cancelled,
        "queued cancellation was not recorded",
    )?;
    require(
        store
            .cancel_queued(&id)
            .map_err(|e| format!("repeat cancel: {e:?}"))?
            == Cancellation::AlreadyCancelled,
        "queued cancellation was not idempotent",
    )?;
    controller
        .set_paused(false)
        .map_err(|e| format!("resume: {e:?}"))?;
    require(
        matches!(
            controller
                .start_next()
                .map_err(|e| format!("queue: {e:?}"))?,
            Start::Blocked(Block::Empty)
        ),
        "cancelled queued run reached dispatch",
    )?;
    let state = controller
        .run_state(&id)
        .map_err(|e| format!("queued state: {e:?}"))?
        .ok_or("queued run disappeared")?;
    require(
        state.phase == RunPhase::Completed && state.result == Some(RunResult::Cancellation),
        "queued cancellation result differs",
    )?;
    require(
        controller
            .record(&id)
            .map_err(|e| format!("queued record: {e:?}"))?
            .is_none()
            && !context.work(&id).exists()
            && !context.root.join("logs").join(&id).exists()
            && controller.active_runs() == 0,
        "queued cancellation started or prepared work",
    )?;
    context.evidence(
        "A05-queued.json",
        &json!({"case":"A05-queued","status":"passed","run_id":id,"state":state,"start_count":0}),
    )?;
    Ok(id)
}

pub(super) fn remote_probe(context: &Context, kind: &str) -> Result<Value> {
    let output = Command::new(context.runtime().join("bin/python3"))
        .args(["-I", "-B"])
        .arg(context.runtime().join("remote_state.py"))
        .arg(kind)
        .output()?;
    require(output.status.success(), "remote marker probe failed")?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub(super) fn wait_remote(
    context: &Context,
    controller: &mut Native,
    id: &str,
    kind: &str,
    dispatched: Instant,
) -> Result<Value> {
    loop {
        controller
            .tick(Instant::now())
            .map_err(|e| format!("tick: {e:?}"))?;
        let state = controller
            .run_state(id)
            .map_err(|e| format!("state: {e:?}"))?
            .ok_or("run disappeared")?;
        require(
            state.result.is_none(),
            "run ended before remote startup was confirmed",
        )?;
        require(
            controller.active_runs() == 1 && dispatched.elapsed() < Duration::from_secs(45),
            "remote startup deadline or capacity check failed",
        )?;
        if state.phase == RunPhase::Running {
            let value = remote_probe(context, kind)?;
            if value["state"] == "alive" {
                proof::remote_started(&value, kind, id)?;
                require(
                    dispatched.elapsed() < Duration::from_secs(45),
                    "remote proof arrived too late",
                )?;
                return Ok(value);
            }
            require(
                matches!(value["state"].as_str(), Some("absent" | "probe-failed")),
                "remote work ended or returned invalid startup evidence",
            )?;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

pub fn run(root: &Path, kind: &str) -> Result<()> {
    let (case, task, marker, result) = match kind {
        "cancel" => (
            "A05-cancel",
            "remote-cancel",
            "A05_CANCEL",
            RunResult::Cancellation,
        ),
        "timeout" => (
            "A05-timeout",
            "remote-timeout",
            "A05_TIMEOUT",
            RunResult::Timeout,
        ),
        _ => return Err("expected cancel or timeout".into()),
    };
    let context = Context::load(root)?;
    let safety = Safety::default();
    let mut store = Store::open_with_safety(context.database(), safety.clone())
        .map_err(|e| format!("store: {e:?}"))?;
    let mut controller = context.controller(safety)?;
    let queued_cancel = if kind == "cancel" {
        Some(queued(&context, &mut store, &mut controller)?)
    } else {
        None
    };
    let id = execution::admit(&context, &mut store, task)?;
    context.evidence(
        &format!("{case}-admission.json"),
        &json!({"case":case,"run_id":id,"expected_result":result}),
    )?;
    let mut observed = Value::Null;
    let outcome = (|| {
        controller
            .set_paused(false)
            .map_err(|e| format!("resume: {e:?}"))?;
        let dispatched = Instant::now();
        execution::start(&mut controller, &id)?;
        let remote = wait_remote(&context, &mut controller, &id, kind, dispatched)?;
        context.evidence(&format!("{case}-remote-start.json"), &remote)?;
        if kind == "cancel" {
            require(
                controller
                    .cancel_active(&id)
                    .map_err(|e| format!("cancel: {e:?}"))?
                    == StopDecision::Recorded,
                "active cancellation was not recorded",
            )?;
            require(
                controller.active_runs() == 1,
                "cancellation released capacity before cleanup",
            )?;
        }
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
        proof::complete_stopped(&state, &record, result)?;
        let parent = proof::workload(&context, &id, marker, &record)?;
        require(
            controller.active_runs() == 0 && !controller.failed(),
            "local stop did not release healthy capacity",
        )?;
        if kind == "timeout" {
            require(
                dispatched.elapsed() >= Duration::from_secs(60),
                "timeout occurred before the configured duration",
            )?;
        }
        let waiting = execution::admit(&context, &mut store, "remote-queued")?;
        require(
            matches!(
                controller
                    .start_next()
                    .map_err(|e| format!("review gate: {e:?}"))?,
                Start::Blocked(Block::ReviewRequired)
            ),
            "external uncertainty did not block dispatch",
        )?;
        let elapsed_ms = dispatched.elapsed().as_millis();
        drop(controller);
        drop(store);
        let mut reopened = context.controller(Safety::default())?;
        reopened
            .set_paused(false)
            .map_err(|e| format!("resume reopened: {e:?}"))?;
        require(
            reopened
                .run_state(&id)
                .map_err(|e| format!("retained state: {e:?}"))?
                == Some(state.clone())
                && reopened
                    .record(&id)
                    .map_err(|e| format!("retained record: {e:?}"))?
                    == Some(record.clone()),
            "stopping history changed after reopen",
        )?;
        require(
            matches!(
                reopened
                    .start_next()
                    .map_err(|e| format!("retained review: {e:?}"))?,
                Start::Blocked(Block::ReviewRequired)
            ),
            "review block did not survive reopen",
        )?;
        require(
            reopened
                .run_state(&waiting)
                .map_err(|e| format!("waiting state: {e:?}"))?
                .is_some_and(|state| state.phase == RunPhase::Queued)
                && !context.work(&waiting).exists(),
            "blocked queued work was lost or started",
        )?;
        if let Some(cancelled) = &queued_cancel {
            require(
                reopened
                    .run_state(cancelled)
                    .map_err(|e| format!("retained queued cancellation: {e:?}"))?
                    .is_some_and(|state| {
                        state.phase == RunPhase::Completed
                            && state.result == Some(RunResult::Cancellation)
                    }),
                "queued cancellation was not durable",
            )?;
        }
        context.evidence(&format!("{case}.json"), &json!({"case":case,"status":"passed","run_id":id,"state":state,"record":record,"remote_before":remote,"start":parent,"inputs":proof::inputs(),"queued_run_id":waiting,"queued_cancelled_id":queued_cancel,"elapsed_ms":elapsed_ms,"runtime_id":context.fixture.runtime_id,"host_id":context.fixture.host_id}))
    })();
    if let Err(error) = outcome {
        let report = json!({"case":case,"status":"failed","run_id":id,"reason":error.to_string(),"observed":observed});
        eprintln!("Acceptance decision: {report}");
        context.evidence(&format!("{case}.json"), &report)?;
        return Err(error);
    }
    println!("{case} passed; external review remains required.");
    Ok(())
}

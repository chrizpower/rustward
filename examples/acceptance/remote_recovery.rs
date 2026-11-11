use super::{Result, context::Context, execution, proof, recovery::ChildGuard, require, stopping};
use rustward::{
    admission::{RunPhase, Store},
    controller::Start,
    dispatch::Block,
    safety::Safety,
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn child(root: &Path, id: &str, case: &str, kind: &str) -> Result<()> {
    let context = Context::load(root)?;
    let mut c = context.controller(Safety::default())?;
    c.set_paused(false).map_err(|e| format!("resume: {e:?}"))?;
    let dispatched = Instant::now();
    execution::start(&mut c, id)?;
    let remote = stopping::wait_remote(&context, &mut c, id, kind, dispatched)?;
    let record = c
        .record(id)
        .map_err(|e| format!("record: {e:?}"))?
        .ok_or("missing record")?;
    context.evidence(
        &format!("{case}-ready.json"),
        &json!({"run_id":id,"record":record,"remote":remote}),
    )?;
    loop {
        c.tick(Instant::now())
            .map_err(|e| format!("child tick: {e:?}"))?;
        thread::sleep(Duration::from_millis(20));
    }
}

pub fn run(root: &Path, case: &str, task: &str, kind: &str) -> Result<()> {
    let context = Context::load(root)?;
    let mut store = Store::open(context.database()).map_err(|e| format!("store: {e:?}"))?;
    let id = execution::admit(&context, &mut store, task)?;
    let queued = execution::admit(&context, &mut store, "remote-queued")?;
    context.evidence(
        &format!("{case}-admission.json"),
        &json!({"case":case,"run_id":id,"queued_run_id":queued}),
    )?;
    drop(store);
    let mut child = ChildGuard(
        Command::new(std::env::current_exe()?)
            .args(["remote-child"])
            .arg(root)
            .arg(&id)
            .arg(case)
            .arg(kind)
            .stdin(Stdio::null())
            .stdout(fs::File::create(
                context
                    .root
                    .join("evidence")
                    .join(format!("{case}-child.stdout")),
            )?)
            .stderr(fs::File::create(
                context
                    .root
                    .join("evidence")
                    .join(format!("{case}-child.stderr")),
            )?)
            .spawn()?,
    );
    let ready = context
        .root
        .join("evidence")
        .join(format!("{case}-ready.json"));
    let deadline = Instant::now() + Duration::from_secs(100);
    while !ready.exists() {
        require(
            child.0.try_wait()?.is_none(),
            "remote controller exited before startup proof",
        )?;
        require(
            Instant::now() < deadline,
            "remote controller startup deadline",
        )?;
        thread::sleep(Duration::from_millis(20));
    }
    let before = proof::json_file(&ready)?;
    let starts = fs::read(context.work(&id).join("starts.jsonl"))?;
    child.0.kill()?;
    use std::os::unix::process::ExitStatusExt;
    require(
        child.0.wait()?.signal() == Some(9),
        "controller was not killed with SIGKILL",
    )?;
    let mut c = context.controller(Safety::default())?;
    require(
        matches!(
            c.start_next().map_err(|e| format!("review gate: {e:?}"))?,
            Start::Blocked(Block::ReviewRequired)
        ),
        "recovery did not block dispatch",
    )?;
    execution::drive(&mut c, &id, Duration::from_secs(90))?;
    let state = c
        .run_state(&id)
        .map_err(|e| format!("state: {e:?}"))?
        .ok_or("missing state")?;
    let record = c
        .record(&id)
        .map_err(|e| format!("record: {e:?}"))?
        .ok_or("missing record")?;
    proof::recovered(&state, &record, &before)?;
    require(
        fs::read(context.work(&id).join("starts.jsonl"))? == starts,
        "remote wrapper started again",
    )?;
    proof::process_ended(&proof::starts(&context.work(&id))?)?;
    require(
        c.run_state(&queued)
            .map_err(|e| format!("queued: {e:?}"))?
            .is_some_and(|s| s.phase == RunPhase::Queued)
            && !context.work(&queued).exists(),
        "queued work was lost or started",
    )?;
    require(
        c.active_runs() == 0,
        "local recovery retained capacity after completion",
    )?;
    require(
        matches!(
            c.start_next()
                .map_err(|e| format!("review after cleanup: {e:?}"))?,
            Start::Blocked(Block::ReviewRequired)
        ),
        "review vanished after cleanup",
    )?;
    let mut acknowledgement = None;
    let mut remote_after = serde_json::Value::Null;
    if case == "A07" {
        remote_after = stopping::remote_probe(&context, kind)?;
        proof::remote_started(&remote_after, kind, &id)?;
        require(
            remote_after["pid"] == before["remote"]["pid"]
                && remote_after["start_time"] == before["remote"]["start_time"],
            "async process identity changed",
        )?;
        let mut admin =
            Store::open(context.database()).map_err(|e| format!("review store: {e:?}"))?;
        let reviewed = admin
            .review(&id)
            .map_err(|e| format!("review: {e:?}"))?
            .ok_or("missing review")?;
        let key = rustward::review::new_acknowledgement_id()
            .map_err(|e| format!("acknowledgement ID: {e:?}"))?;
        let note = "Fixture review: the matching remote asynchronous process is still alive. Local cleanup is complete. Accept the remaining external risk for this disposable test target; do not restart the interrupted run.";
        let ack = admin
            .acknowledge(&key, &reviewed, note)
            .map_err(|e| format!("acknowledge: {e:?}"))?;
        require(
            admin
                .acknowledge(&key, &reviewed, note)
                .map_err(|e| format!("matching acknowledgement: {e:?}"))?
                == ack,
            "acknowledgement retry changed",
        )?;
        let after = c
            .run_state(&id)
            .map_err(|e| format!("acknowledged state: {e:?}"))?
            .ok_or("lost acknowledged state")?;
        require(
            after == state
                && !c
                    .record(&id)
                    .map_err(|e| format!("acknowledged record: {e:?}"))?
                    .ok_or("lost record")?
                    .review_required,
            "acknowledgement changed the result or failed to clear review",
        )?;
        require(
            c.run_state(&queued)
                .map_err(|e| format!("retained queue: {e:?}"))?
                .is_some_and(|s| s.phase == RunPhase::Queued)
                && !context.work(&queued).exists(),
            "acknowledgement started queued work",
        )?;
        acknowledgement = Some(ack);
    }
    context.evidence(&format!("{case}.json"),&json!({"case":case,"status":"passed","run_id":id,"queued_run_id":queued,"before":before,"state":state,"record":record,"start_count":1,"remote_after":remote_after,"acknowledgement":acknowledgement}))?;
    Ok(())
}

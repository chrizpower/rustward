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
    context.evidence(&format!("{case}.json"),&json!({"case":case,"status":"passed","run_id":id,"queued_run_id":queued,"before":before,"state":state,"record":record,"start_count":1}))?;
    Ok(())
}

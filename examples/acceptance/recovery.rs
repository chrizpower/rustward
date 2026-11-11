use super::{Result, context::Context, execution, proof, require};
use rustward::{
    admission::{RunPhase, RunResult, Store},
    controller::Start,
    dispatch::Block,
    safety::Safety,
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub(super) struct ChildGuard(pub(super) Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn child(root: &Path, id: &str) -> Result<()> {
    let context = Context::load(root)?;
    let mut controller = context.controller(Safety::default())?;
    controller
        .set_paused(false)
        .map_err(|e| format!("child resume: {e:?}"))?;
    execution::start(&mut controller, id)?;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        controller
            .tick(Instant::now())
            .map_err(|e| format!("child tick: {e:?}"))?;
        let state = controller
            .run_state(id)
            .map_err(|e| format!("child state: {e:?}"))?
            .ok_or("child run disappeared")?;
        if state.phase == RunPhase::Running && context.work(id).join("ready.json").exists() {
            let record = controller
                .record(id)
                .map_err(|e| format!("child record: {e:?}"))?
                .ok_or("child record disappeared")?;
            require(record.identity.is_some(), "child launch lacks identity")?;
            context.evidence("recovery-ready.json", &json!({"run_id":id,"record":record}))?;
            break;
        }
        require(
            Instant::now() < deadline,
            "recovery synchronization deadline",
        )?;
        thread::sleep(Duration::from_millis(20));
    }
    loop {
        controller
            .tick(Instant::now())
            .map_err(|e| format!("child polling: {e:?}"))?;
        thread::sleep(Duration::from_millis(20));
    }
}
pub fn run(context: &Context) -> Result<()> {
    let mut store =
        Store::open(context.database()).map_err(|e| format!("recovery store: {e:?}"))?;
    let id = execution::admit(context, &mut store, "recovery")?;
    let queued = execution::admit(context, &mut store, "ordinary")?;
    drop(store);
    let stdout = fs::File::create(context.root.join("evidence/recovery-child.stdout"))?;
    let stderr = fs::File::create(context.root.join("evidence/recovery-child.stderr"))?;
    let mut child = ChildGuard(
        Command::new(std::env::current_exe()?)
            .arg("recovery-child")
            .arg(&context.root)
            .arg(&id)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(100);
    let ready = context.root.join("evidence/recovery-ready.json");
    while !ready.exists() {
        require(
            child.0.try_wait()?.is_none(),
            "recovery child exited before synchronization",
        )?;
        require(
            Instant::now() < deadline,
            "recovery child did not reach synchronization",
        )?;
        thread::sleep(Duration::from_millis(20));
    }
    let before = proof::json_file(&ready)?;
    let starts_before = fs::read(context.work(&id).join("starts.jsonl"))?;
    child.0.kill()?;
    let status = child.0.wait()?;
    use std::os::unix::process::ExitStatusExt;
    require(
        status.signal() == Some(9),
        "recovery did not use abrupt process termination",
    )?;
    let mut controller = context.controller(Safety::default())?;
    require(
        matches!(
            controller
                .start_next()
                .map_err(|e| format!("recovery gate: {e:?}"))?,
            Start::Blocked(Block::ReviewRequired)
        ),
        "recovery did not block new dispatch",
    )?;
    execution::drive(&mut controller, &id, Duration::from_secs(60))?;
    let state = controller
        .run_state(&id)
        .map_err(|e| format!("recovered state: {e:?}"))?
        .ok_or("recovered state missing")?;
    let record = controller
        .record(&id)
        .map_err(|e| format!("recovered record: {e:?}"))?
        .ok_or("recovered record missing")?;
    require(
        state.phase == RunPhase::Completed && state.result == Some(RunResult::Interruption),
        "recovery changed the required result",
    )?;
    require(
        record.cleanup_confirmed && record.review_required && record.output_failed,
        "recovery lacks cleanup or uncertainty evidence",
    )?;
    require(
        serde_json::to_value(&record.identity)? == before["record"]["identity"],
        "recovery changed admitted execution identity",
    )?;
    let evidence = record
        .output_evidence
        .as_ref()
        .ok_or("missing recovered output evidence")?;
    require(
        evidence.recovered
            && evidence.stdout.observed_bytes.is_none()
            && evidence.stderr.observed_bytes.is_none(),
        "recovery inferred historical output counts",
    )?;
    require(
        fs::read(context.work(&id).join("starts.jsonl"))? == starts_before,
        "recovery started the workload again",
    )?;
    proof::process_ended(&proof::starts(&context.work(&id))?)?;
    proof::process_ended(&proof::json_file(&context.work(&id).join("child.json"))?)?;
    require(
        controller
            .run_state(&queued)
            .map_err(|e| format!("queued state: {e:?}"))?
            .is_some_and(|state| state.phase == RunPhase::Queued),
        "recovery lost or started queued work",
    )?;
    require(
        !context.work(&queued).exists(),
        "recovery prepared queued work automatically",
    )?;
    context.evidence("local-recovery.json",&json!({"status":"passed","run_id":id,"queued_run_id":queued,"before":before,"state":state,"record":record,"start_count":1}))?;
    Ok(())
}

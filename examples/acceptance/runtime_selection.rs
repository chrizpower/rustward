use super::{Result, context::Context, execution, proof, require, stopping};
use rusqlite::{Connection, OpenFlags};
use rustward::{
    admission::{Admission, RunPhase, RunResult, Store},
    config,
    controller::Start,
    dispatch::Block,
    safety::Safety,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn identity(context: &Context, id: &str) -> Result<Value> {
    let connection =
        Connection::open_with_flags(context.database(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let row:(String,Vec<u8>,String,Vec<u8>,Vec<u8>,Vec<u8>)=connection.query_row(
        "SELECT runtime_id,task_snapshot,arguments,fingerprint,task_revision,config_revision FROM runs JOIN admissions USING(run_id) WHERE run_id=?1",
        [id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    Ok(json!(row))
}
pub fn run(root: &Path) -> Result<()> {
    let context = Context::load(root)?;
    let b = context
        .fixture
        .runtime_b_id
        .as_ref()
        .ok_or("missing runtime B")?;
    let cfg_a = config::load(root.join("configuration/rustward.toml"))?;
    let cfg_b = config::load(root.join("configuration-b/rustward.toml"))?;
    let safety = Safety::default();
    let mut store = Store::open_with_safety(context.database(), safety.clone())
        .map_err(|e| format!("store: {e:?}"))?;
    let mut c = context.controller(safety)?;
    let (key, a) = execution::admit_request(
        &mut store,
        &cfg_a,
        "runtime-selection",
        &context.fixture.runtime_id,
    )?;
    let before = identity(&context, &a)?;
    require(
        store
            .admit(&key, &cfg_b, "runtime-selection", &execution::values(), b)
            .map_err(|e| format!("retry after selection: {e:?}"))?
            == Admission::AlreadyAdmitted { run_id: a.clone() },
        "selection changed a matching retry",
    )?;
    c.set_paused(false).map_err(|e| format!("resume: {e:?}"))?;
    let dispatched = Instant::now();
    execution::start(&mut c, &a)?;
    let remote = stopping::wait_remote(&context, &mut c, &a, "runtime-a", dispatched)?;
    require(
        proof::json_file(&context.work(&a).join("runtime-version.json"))?["version"] == "A",
        "queued run changed runtime",
    )?;
    let (_, next) = execution::admit_request(&mut store, &cfg_b, "runtime-selection", b)?;
    require(
        matches!(
            c.start_next().map_err(|e| format!("capacity: {e:?}"))?,
            Start::Blocked(Block::ExecutionCapacity)
        ),
        "active runtime A lost its slot",
    )?;
    require(
        identity(&context, &a)? == before,
        "active admission identity changed",
    )?;
    let released = Command::new(context.runtime().join("bin/python3"))
        .args(["-I", "-B"])
        .arg(context.runtime().join("remote_release.py"))
        .arg("runtime-a")
        .arg(&a)
        .output()?;
    require(released.status.success(), "runtime A release failed")?;
    execution::drive(&mut c, &a, Duration::from_secs(90))?;
    let a_state = c
        .run_state(&a)
        .map_err(|e| format!("A state: {e:?}"))?
        .ok_or("missing A")?;
    let a_record = c
        .record(&a)
        .map_err(|e| format!("A record: {e:?}"))?
        .ok_or("missing A record")?;
    proof::complete_success(&a_state, &a_record)?;
    proof::workload(&context, &a, "A08", &a_record)?;
    execution::start(&mut c, &next)?;
    execution::drive(&mut c, &next, Duration::from_secs(90))?;
    let state = c
        .run_state(&next)
        .map_err(|e| format!("B state: {e:?}"))?
        .ok_or("missing B")?;
    let record = c
        .record(&next)
        .map_err(|e| format!("B record: {e:?}"))?
        .ok_or("missing B record")?;
    proof::complete_success(&state, &record)?;
    proof::workload(&context, &next, "A08", &record)?;
    require(
        proof::json_file(&context.work(&next).join("runtime-version.json"))?["version"] == "B",
        "new run did not use runtime B",
    )?;
    require(
        identity(&context, &a)? == before,
        "completion rewrote runtime A history",
    )?;
    context.evidence("A08-selection.json",&json!({"status":"passed","run_id":next,"a_run_id":a,"runtime_a":context.fixture.runtime_id,"runtime_b":b,"before":before,"after":identity(&context,&a)?,"remote_before":remote,"state":state,"record":record}))
}
pub fn queue_missing(root: &Path) -> Result<()> {
    let context = Context::load(root)?;
    let mut store = Store::open(context.database()).map_err(|e| format!("store: {e:?}"))?;
    let cfg = config::load(root.join("configuration/rustward.toml"))?;
    let (_, id) = execution::admit_request(
        &mut store,
        &cfg,
        "runtime-selection",
        &context.fixture.runtime_id,
    )?;
    context.evidence(
        "A08-missing-admission.json",
        &json!({"run_id":id,"before":identity(&context,&id)?}),
    )
}
pub fn missing(root: &Path) -> Result<()> {
    let context = Context::load(root)?;
    let admitted = proof::json_file(&root.join("evidence/A08-missing-admission.json"))?;
    let id = admitted["run_id"].as_str().ok_or("missing run ID")?;
    let mut c = context.controller(Safety::default())?;
    c.set_paused(false).map_err(|e| format!("resume: {e:?}"))?;
    execution::start(&mut c, id)?;
    execution::drive(&mut c, id, Duration::from_secs(90))?;
    let state = c
        .run_state(id)
        .map_err(|e| format!("state: {e:?}"))?
        .ok_or("missing state")?;
    let record = c
        .record(id)
        .map_err(|e| format!("record: {e:?}"))?
        .ok_or("missing record")?;
    require(
        state.phase == RunPhase::Completed
            && state.result == Some(RunResult::Failure)
            && record.launch_failure.is_some()
            && record.identity.is_none()
            && record.cleanup_confirmed
            && !context.work(id).join("starts.jsonl").exists(),
        "missing runtime was substituted or launched",
    )?;
    require(
        identity(&context, id)? == admitted["before"],
        "missing runtime changed admission",
    )?;
    let selected = proof::json_file(&root.join("evidence/A08-selection.json"))?;
    require(
        identity(
            &context,
            selected["a_run_id"].as_str().ok_or("missing A ID")?,
        )? == selected["before"],
        "removal rewrote completed history",
    )?;
    context.evidence(
        "A08-missing.json",
        &json!({"status":"passed","run_id":id,"state":state,"record":record,"admission":admitted}),
    )
}

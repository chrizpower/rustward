use super::{Result, context::Context, execution, proof, require};
use rustward::{
    admission::{Cancellation, Store},
    safety::Safety,
};
use serde_json::json;
use std::{fs, path::Path, time::Duration};
pub fn run(root: &Path) -> Result<()> {
    let context = Context::load(root)?;
    let limit = context
        .fixture
        .capture_limit
        .ok_or("missing capture limit")?;
    require(limit == 65536, "unexpected flood limit")?;
    let safety = Safety::default();
    let mut store = Store::open_with_safety(context.database(), safety.clone())
        .map_err(|e| format!("store: {e:?}"))?;
    let mut c = context.controller(safety)?;
    let id = execution::admit(&context, &mut store, "remote-flood")?;
    context.evidence(
        "A09-admission.json",
        &json!({"case":"A09","run_id":id,"limit":limit}),
    )?;
    c.set_paused(false).map_err(|e| format!("resume: {e:?}"))?;
    execution::start(&mut c, &id)?;
    execution::drive(&mut c, &id, Duration::from_secs(120))?;
    let state = c
        .run_state(&id)
        .map_err(|e| format!("state: {e:?}"))?
        .ok_or("missing state")?;
    let record = c
        .record(&id)
        .map_err(|e| format!("record: {e:?}"))?
        .ok_or("missing record")?;
    proof::truncated_success(&state, &record, limit)?;
    proof::process_ended(&proof::starts(&context.work(&id))?)?;
    require(
        proof::json_file(&context.work(&id).join("inputs.json"))? == proof::inputs(),
        "flood inputs changed",
    )?;
    let stdout = fs::read(root.join("logs").join(&id).join("stdout.log"))?;
    let stderr = fs::read(root.join("logs").join(&id).join("stderr.log"))?;
    require(
        (stdout.len() + stderr.len()) as u64 == limit
            && String::from_utf8_lossy(&stdout).contains("A09_STDOUT"),
        "retained flood prefix differs",
    )?;
    require(
        c.active_runs() == 0 && !c.failed(),
        "flood retained capacity or closed execution",
    )?;
    let next = execution::admit(&context, &mut store, "remote-queued")?;
    require(
        store
            .cancel_queued(&next)
            .map_err(|e| format!("cancel probe: {e:?}"))?
            == Cancellation::Cancelled,
        "truncation closed admission",
    )?;
    context.evidence("A09.json",&json!({"case":"A09","status":"passed","run_id":id,"state":state,"record":record,"limit":limit,"retained_bytes":stdout.len()+stderr.len()}))
}

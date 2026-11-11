use super::{
    Result,
    context::{Context, native_supervisor},
    execution, proof, require, stopping,
};
use rusqlite::Connection;
use rustward::{
    admission::{self, RunPhase, Store},
    config, controller,
    safety::Safety,
    supervision::{Cleanup, Observation, RunId, Supervisor},
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

pub fn run(root: &Path) -> Result<()> {
    let context = Context::load(root)?;
    let safety = Safety::default();
    let mut store = Store::open_with_safety(context.database(), safety.clone())
        .map_err(|e| format!("store: {e:?}"))?;
    let mut c = context.controller(safety)?;
    let id = execution::admit(&context, &mut store, "remote-storage")?;
    let queued = execution::admit(&context, &mut store, "remote-queued")?;
    let fresh = store.issue_key().map_err(|e| format!("fresh key: {e:?}"))?;
    context.evidence(
        "A10-admission.json",
        &json!({"case":"A10","run_id":id,"queued_run_id":queued}),
    )?;
    c.set_paused(false).map_err(|e| format!("resume: {e:?}"))?;
    let started = Instant::now();
    execution::start(&mut c, &id)?;
    let remote = stopping::wait_remote(&context, &mut c, &id, "storage", started)?;
    let before = c
        .record(&id)
        .map_err(|e| format!("before record: {e:?}"))?
        .ok_or("missing before record")?;
    let expected = before.identity.as_ref().ok_or("missing identity")?;
    require(!c.failed(), "storage gate closed before injection")?;
    let mut observer = native_supervisor()?;
    let blocker = Connection::open(context.database())?;
    blocker.execute_batch("BEGIN IMMEDIATE")?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(context.work(&id).join("write-output"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stopped = matches!(
            observer.observe(
                &RunId::new(&id).map_err(|e| format!("id: {e:?}"))?,
                Some(expected)
            ),
            Ok(Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            })
        );
        if c.failed() && stopped {
            break;
        }
        require(
            Instant::now() < deadline,
            "storage failure did not stop local work while SQLite was locked",
        )?;
        thread::sleep(Duration::from_millis(30));
    }
    proof::process_ended(&proof::starts(&context.work(&id))?)?;
    let durable: (String, Option<String>) = blocker.query_row(
        "SELECT phase,result FROM runs WHERE run_id=?1",
        [&id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    require(
        durable.1.is_none(),
        "storage failure published a result while SQLite writes were blocked",
    )?;
    context.evidence("A10-before-unlock.json",&json!({"sqlite_write_lock_held":true,"gate_failed":c.failed(),"local_cleanup_confirmed":true,"durable_phase":durable.0,"durable_result":durable.1,"identity":expected}))?;
    blocker.execute_batch("ROLLBACK")?;
    drop(blocker);
    let cfg = config::load(root.join("configuration/rustward.toml"))?;
    require(
        matches!(
            store.admit(
                &fresh,
                &cfg,
                "remote-queued",
                &execution::values(),
                &context.fixture.runtime_id
            ),
            Err(admission::Error::Closed)
        ),
        "storage failure did not close new admission",
    )?;
    require(
        matches!(
            c.start_next(),
            Err(controller::Error::State(admission::Error::Closed))
        ),
        "storage failure did not close dispatch",
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
    proof::storage_interruption(&state, &record)?;
    require(
        c.failed() && c.active_runs() == 0,
        "storage gate reopened or local capacity was not released",
    )?;
    require(
        c.run_state(&queued)
            .map_err(|e| format!("queued state: {e:?}"))?
            .is_some_and(|s| s.phase == RunPhase::Queued)
            && !context.work(&queued).exists(),
        "storage failure started or lost queued work",
    )?;
    context.evidence("A10.json",&json!({"case":"A10","status":"passed","run_id":id,"queued_run_id":queued,"remote_before":remote,"before":before,"state":state,"record":record,"storage_gate_closed":true,"stopped_while_sqlite_locked":true}))
}

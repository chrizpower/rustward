use super::{
    Result,
    context::{Context, Native},
    proof, recovery, require,
};
use rustward::{
    admission::{Admission, RunPhase, RunResult, Store},
    config,
    controller::Start,
    dispatch::Block,
    safety::Safety,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
pub fn values() -> BTreeMap<String, toml::Value> {
    BTreeMap::from([
        ("text".into(), proof::TEXT.into()),
        ("count".into(), 7.into()),
        ("enabled".into(), true.into()),
    ])
}
pub fn admit(context: &Context, store: &mut Store, task: &str) -> Result<String> {
    let config = config::load(context.root.join("configuration/rustward.toml"))?;
    admit_request(store, &config, task, &context.fixture.runtime_id).map(|(_, id)| id)
}
pub fn admit_request(
    store: &mut Store,
    config: &config::Config,
    task: &str,
    runtime_id: &str,
) -> Result<(String, String)> {
    let key = store.issue_key().map_err(|e| format!("issue key: {e:?}"))?;
    let first = store
        .admit(&key, config, task, &values(), runtime_id)
        .map_err(|e| format!("admit: {e:?}"))?;
    let Admission::Admitted { run_id } = first else {
        return Err("first submission was not admitted".into());
    };
    let again = store
        .admit(&key, config, task, &values(), runtime_id)
        .map_err(|e| format!("matching retry: {e:?}"))?;
    require(
        again
            == Admission::AlreadyAdmitted {
                run_id: run_id.clone(),
            },
        "retry did not retain the run ID",
    )?;
    Ok((key, run_id))
}
pub fn start(controller: &mut Native, id: &str) -> Result<()> {
    require(
        matches!(controller.start_next().map_err(|e|format!("start: {e:?}"))?,Start::Preparing(actual) if actual.as_str()==id),
        "FIFO selected another run or dispatch was blocked",
    )
}
pub fn drive(controller: &mut Native, id: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        controller
            .tick(Instant::now())
            .map_err(|e| format!("tick: {e:?}"))?;
        let state = controller
            .run_state(id)
            .map_err(|e| format!("run state: {e:?}"))?
            .ok_or("run disappeared")?;
        if state.phase == RunPhase::Completed {
            return Ok(());
        }
        require(
            controller.active_runs() == 1,
            "unfinished run released controller capacity",
        )?;
        require(Instant::now() < deadline, "completion deadline exceeded")?;
        thread::sleep(Duration::from_millis(20));
    }
}
pub fn run(root: &Path, remote: bool) -> Result<()> {
    let context = Context::load(root)?;
    require(
        fs::read_to_string("/etc/os-release")?
            .lines()
            .any(|line| line == "VERSION_ID=\"13\"" || line == "VERSION_ID=13"),
        "acceptance requires Debian 13",
    )?;
    let safety = Safety::default();
    let mut store = Store::open_with_safety(context.database(), safety.clone())
        .map_err(|e| format!("store: {e:?}"))?;
    let mut controller = context.controller(safety)?;
    require(
        fs::OpenOptions::new()
            .write(true)
            .open(context.runtime().join("bin/python3"))
            .is_err(),
        "execution user can write the runtime",
    )?;
    context.evidence("baseline.json",&json!({"fixture":context.fixture,"kernel":fs::read_to_string("/proc/version")?,"os_release":fs::read_to_string("/etc/os-release")?,"service_status":fs::read_to_string("/proc/self/status")?}))?;
    controller
        .set_paused(false)
        .map_err(|e| format!("resume: {e:?}"))?;
    let mut completed = Vec::new();
    let cases: &[(&str, &str)] = if remote {
        &[("A03", "remote-playbook")]
    } else {
        &[("A01", "local-playbook"), ("A02", "ordinary")]
    };
    for &(case, task) in cases {
        let id = admit(&context, &mut store, task)?;
        context.evidence(&format!("{case}-admission.json"),&json!({"case":case,"run_id":id,"expected":"Success after cleanup and synchronized capture","inputs":proof::inputs(),"runtime_id":context.fixture.runtime_id}))?;
        let outcome = (|| {
            start(&mut controller, &id)?;
            drive(&mut controller, &id, Duration::from_secs(120))?;
            let state = controller
                .run_state(&id)
                .map_err(|e| format!("state: {e:?}"))?
                .ok_or("missing completed state")?;
            let record = controller
                .record(&id)
                .map_err(|e| format!("record: {e:?}"))?
                .ok_or("missing completed record")?;
            require(
                controller.active_runs() == 0 && !controller.failed(),
                "completion did not release healthy capacity",
            )?;
            proof::case(&context, &id, case, &state, &record)
        })();
        if let Err(error) = outcome {
            let state = controller.run_state(&id).ok().flatten();
            let record = controller.record(&id).ok().flatten();
            let failure = json!({"case":case,"status":"failed","run_id":id,"expected":"Success after cleanup and synchronized capture","reason":error.to_string(),"state":state,"record":record});
            eprintln!("Acceptance decision: {failure}");
            context.evidence(&format!("{case}.json"), &failure)?;
            return Err(error);
        }
        completed.push(id);
    }
    require(
        matches!(
            controller
                .start_next()
                .map_err(|e| format!("empty queue: {e:?}"))?,
            Start::Blocked(Block::Empty)
        ),
        "unexpected queued work",
    )?;
    drop(controller);
    drop(store);
    let mut reopened = Store::open(context.database()).map_err(|e| format!("reopen: {e:?}"))?;
    for id in completed {
        let state = reopened
            .run_state(&id)
            .map_err(|e| format!("persisted state: {e:?}"))?
            .ok_or("completed run was not retained")?;
        require(
            state.phase == RunPhase::Completed && state.result == Some(RunResult::Success),
            "completion was not durable",
        )?;
    }
    drop(reopened);
    if !remote {
        recovery::run(&context)?;
    }
    let names: Vec<_> = cases.iter().map(|(name, _)| *name).collect();
    let result = if remote {
        json!({"status":"passed","cases":names,"scope":"A03","not_run":["A06","A07","A08","A09","A10"]})
    } else {
        json!({"status":"passed","cases":names,"additional":["local-recovery"],"not_run":["A03","A04","A05","A06","A07","A08","A09","A10"]})
    };
    context.evidence("result.json", &result)?;
    println!("Controller acceptance passed: {}", names.join(", "));
    Ok(())
}

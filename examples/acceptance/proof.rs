use super::{Result, context::Context, require};
use rustward::{
    admission::{RunPhase, RunResult, RunState},
    dispatch::Record,
    supervision::ProcessExit,
};
use serde_json::{Value, json};
use std::{fs, path::Path};
pub const TEXT: &str = "He said \"hello\"; it's a test";
pub fn inputs() -> Value {
    json!({"text":TEXT,"count":7,"enabled":true})
}
pub fn json_file(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn starts(work: &Path) -> Result<Value> {
    let text = fs::read_to_string(work.join("starts.jsonl"))?;
    let lines = text.lines().collect::<Vec<_>>();
    require(lines.len() == 1, "workload started more than once")?;
    Ok(serde_json::from_str(lines[0])?)
}
pub fn process_ended(value: &Value) -> Result<()> {
    let pid = value["pid"].as_u64().ok_or("missing process PID")?;
    require(pid > 0 && pid <= u32::MAX.into(), "invalid process PID")?;
    let expected = value["start_time"]
        .as_str()
        .ok_or("missing process start time")?;
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
        Ok(stat) => {
            let tail = stat
                .rsplit_once(')')
                .ok_or("invalid process stat")?
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            require(tail.len() > 19, "short process stat")?;
            require(
                tail[19] != expected || matches!(tail[0], "Z" | "X"),
                "workload process is still alive",
            )
        }
    }
}
pub fn complete_success(state: &RunState, record: &Record) -> Result<()> {
    require(
        state.phase == RunPhase::Completed && state.result == Some(RunResult::Success),
        "run did not complete with Success",
    )?;
    require(
        record.cleanup_confirmed && !record.review_required && !record.output_failed,
        "success has incomplete or uncertain evidence",
    )?;
    require(
        record.identity.is_some() && record.exit == Some(ProcessExit::Code(0)),
        "success lacks execution identity or exit evidence",
    )?;
    let output = record
        .output_evidence
        .as_ref()
        .ok_or("missing output evidence")?;
    require(
        output.synchronized && !output.recovered,
        "success lacks complete synchronized output",
    )?;
    require(
        output.retained_bytes() == Some(record.log_bytes),
        "output accounting differs",
    )?;
    require(
        output.stdout.truncated == Some(false) && output.stderr.truncated == Some(false),
        "unexpected output truncation",
    )
}
pub fn case(
    context: &Context,
    id: &str,
    case: &str,
    state: &RunState,
    record: &Record,
) -> Result<()> {
    complete_success(state, record)?;
    let work = context.work(id);
    let parent = starts(&work)?;
    process_ended(&parent)?;
    require(
        json_file(&work.join("inputs.json"))? == inputs(),
        "wrapper input types or values differ",
    )?;
    if case != "A03" {
        require(
            json_file(&work.join("target.json"))? == inputs(),
            "target content differs",
        )?;
    }
    if case == "A02" {
        process_ended(&json_file(&work.join("child.json"))?)?;
    }
    let stdout = fs::read(context.root.join("logs").join(id).join("stdout.log"))?;
    let stderr = fs::read(context.root.join("logs").join(id).join("stderr.log"))?;
    require(
        stdout.len() as u64
            == record
                .output_evidence
                .as_ref()
                .unwrap()
                .stdout
                .retained_bytes,
        "stdout length differs",
    )?;
    require(
        stderr.len() as u64
            == record
                .output_evidence
                .as_ref()
                .unwrap()
                .stderr
                .retained_bytes,
        "stderr length differs",
    )?;
    require(
        String::from_utf8_lossy(&stdout).contains(&format!("{case}_STDOUT")),
        "missing stdout marker",
    )?;
    require(
        String::from_utf8_lossy(&stderr).contains(&format!("{case}_STDERR")),
        "missing stderr marker",
    )?;
    context.evidence(&format!("{case}.json"),&json!({"case":case,"status":"passed","run_id":id,"runtime_id":context.fixture.runtime_id,"host_id":context.fixture.host_id,"state":state,"record":record,"inputs":inputs(),"start":parent}))
}

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
    complete(state, record, RunResult::Success, 0)
}
pub fn complete_failure(state: &RunState, record: &Record, code: i32) -> Result<()> {
    require(code > 0, "expected failure exit must be positive")?;
    complete(state, record, RunResult::Failure, code)
}
fn complete(state: &RunState, record: &Record, result: RunResult, code: i32) -> Result<()> {
    require(
        state.phase == RunPhase::Completed && state.result == Some(result),
        "run did not complete with the expected result",
    )?;
    require(
        record.cleanup_confirmed && !record.review_required && !record.output_failed,
        "completion has incomplete or uncertain evidence",
    )?;
    require(
        record.identity.is_some() && record.exit == Some(ProcessExit::Code(code)),
        "completion lacks the expected identity or exit evidence",
    )?;
    output(record)
}
pub fn complete_stopped(state: &RunState, record: &Record, result: RunResult) -> Result<()> {
    let reason = match result {
        RunResult::Cancellation => "cancellation",
        RunResult::Timeout => "timeout",
        _ => return Err("expected Cancellation or Timeout".into()),
    };
    require(
        state.phase == RunPhase::Completed && state.result == Some(result),
        "stop result was not retained",
    )?;
    require(
        record.cleanup_confirmed && record.review_required && !record.output_failed,
        "stopped run lacks cleanup or external review evidence",
    )?;
    require(
        record.identity.is_some()
            && record.exit.is_some()
            && record.stop_reason.as_deref() == Some(reason),
        "stopped run lacks identity, exit, or stop reason",
    )?;
    output(record)
}
fn output(record: &Record) -> Result<()> {
    let output = record
        .output_evidence
        .as_ref()
        .ok_or("missing output evidence")?;
    require(
        output.synchronized && !output.recovered,
        "completion lacks synchronized output",
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
    let parent = workload(context, id, case, record)?;
    let work = context.work(id);
    if case != "A03" {
        require(
            json_file(&work.join("target.json"))? == inputs(),
            "target content differs",
        )?;
    }
    if case == "A02" {
        process_ended(&json_file(&work.join("child.json"))?)?;
    }
    context.evidence(&format!("{case}.json"),&json!({"case":case,"status":"passed","run_id":id,"runtime_id":context.fixture.runtime_id,"host_id":context.fixture.host_id,"state":state,"record":record,"inputs":inputs(),"start":parent}))
}

pub fn workload(context: &Context, id: &str, case: &str, record: &Record) -> Result<Value> {
    let work = context.work(id);
    let parent = starts(&work)?;
    process_ended(&parent)?;
    require(
        json_file(&work.join("inputs.json"))? == inputs(),
        "wrapper input types or values differ",
    )?;
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
    Ok(parent)
}

pub fn remote_started(value: &Value, kind: &str, run_id: &str) -> Result<()> {
    require(
        value["kind"] == kind && value["state"] == "alive" && value["run_id"] == run_id,
        "remote work is not confirmed alive",
    )?;
    require(value["inputs"] == inputs(), "remote work inputs differ")?;
    require(
        value["pid"].as_u64().is_some_and(|pid| pid > 0)
            && value["start_time"]
                .as_str()
                .is_some_and(|time| !time.is_empty()),
        "remote process identity is missing",
    )
}

pub fn recovered(state: &RunState, record: &Record, before: &Value) -> Result<()> {
    require(
        state.phase == RunPhase::Completed && state.result == Some(RunResult::Interruption),
        "recovery did not retain Interruption",
    )?;
    require(
        record.cleanup_confirmed && record.review_required && record.output_failed,
        "recovery lacks cleanup or review",
    )?;
    require(
        record.identity.is_some()
            && serde_json::to_value(&record.identity)? == before["record"]["identity"],
        "recovery changed execution identity",
    )?;
    let output = record
        .output_evidence
        .as_ref()
        .ok_or("missing recovered output")?;
    require(
        output.recovered
            && output.stdout.observed_bytes.is_none()
            && output.stderr.observed_bytes.is_none()
            && output.stdout.truncated.is_none()
            && output.stderr.truncated.is_none(),
        "recovery invented historical counts",
    )
}

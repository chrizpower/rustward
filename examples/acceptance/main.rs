//! Opt-in synthetic host acceptance. This is not a service CLI.
mod context;
mod execution;
mod failure;
mod flood;
mod proof;
mod recovery;
mod remote_recovery;
mod runtime_selection;
mod stopping;
#[cfg(test)]
mod tests;

use std::{env, path::Path, process::ExitCode};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn main() -> ExitCode {
    match command(&env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Acceptance failed: {error}");
            ExitCode::FAILURE
        }
    }
}
fn command(args: &[String]) -> Result<()> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["id", kind, path] => {
            let source=std::fs::read_to_string(path)?;
            let id=match *kind {
                "runtime"=>rustward::runtime::manifest_id(&source),
                "host"=>rustward::runtime::host::manifest_id(&source),
                _=>return Err("expected runtime or host manifest".into()),
            }.map_err(|error|format!("manifest: {error:?}"))?;
            println!("{id}"); Ok(())
        }
        ["probe-manager"]=>{println!("{}",context::probe_manager()?);Ok(())},
        ["run", root]=>execution::run(Path::new(root), false),
        ["remote-child",root,id,case,kind]=>remote_recovery::child(Path::new(root),id,case,kind),
        ["run-scenario",root,"A09"]=>flood::run(Path::new(root)),
        ["run-scenario",root,"A08"]=>runtime_selection::run(Path::new(root)),
        ["runtime-queue-missing",root]=>runtime_selection::queue_missing(Path::new(root)),
        ["runtime-missing",root]=>runtime_selection::missing(Path::new(root)),
        ["run-scenario",root,"A07"]=>remote_recovery::run(Path::new(root),"A07","remote-async","async"),
        ["run-scenario",root,"A06"]=>remote_recovery::run(Path::new(root),"A06","remote-recovery","recovery"),
        ["run-stop", root, kind]=>stopping::run(Path::new(root), kind),
        ["run-failure", root, kind]=>failure::run(Path::new(root), kind),
        ["run-ssh", root]=>execution::run(Path::new(root), true),
        ["recovery-child",root,id]=>recovery::child(Path::new(root),id),
        _=>Err("usage: acceptance id <runtime|host> <file>, probe-manager, run <root>, run-ssh <root>, run-failure <root> <task|unreachable>, run-stop <root> <cancel|timeout>, or recovery-child <root> <run-id>".into()),
    }
}

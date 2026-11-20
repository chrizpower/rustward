//! Opt-in synthetic host acceptance. This is not a service CLI.
#[cfg(target_os = "linux")]
mod context;
#[cfg(target_os = "linux")]
mod execution;
#[cfg(target_os = "linux")]
mod failure;
#[cfg(target_os = "linux")]
mod flood;
#[cfg(target_os = "linux")]
mod proof;
#[cfg(target_os = "linux")]
mod recovery;
#[cfg(target_os = "linux")]
mod remote_recovery;
#[cfg(target_os = "linux")]
mod runtime_selection;
#[cfg(target_os = "linux")]
mod stopping;
#[cfg(target_os = "linux")]
mod storage_fault;
#[cfg(all(target_os = "linux", test))]
mod tests;

#[cfg(target_os = "linux")]
use std::{env, path::Path, process::ExitCode};
#[cfg(target_os = "linux")]
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[cfg(target_os = "linux")]
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    match command(&env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Acceptance failed: {error}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(target_os = "linux")]
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
        ["run-scenario",root,"A10"]=>storage_fault::run(Path::new(root)),
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

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("Host acceptance requires Linux and systemd.");
    std::process::ExitCode::FAILURE
}

use std::process::ExitCode;
#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let _ = rustward::launcher::execute(std::env::args_os().skip(1));
    eprintln!("Rustward could not start the command.");
    ExitCode::FAILURE
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("Rustward task execution requires Linux.");
    ExitCode::FAILURE
}

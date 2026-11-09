use std::process::ExitCode;
fn main() -> ExitCode {
    let _ = rustward::launcher::execute(std::env::args_os().skip(1));
    eprintln!("Rustward could not start the command.");
    ExitCode::FAILURE
}

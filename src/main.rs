use rustward::config;

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let command = arguments.next();
    let path = arguments.next();
    if command.as_deref() != Some(std::ffi::OsStr::new("validate"))
        || path.is_none()
        || arguments.next().is_some()
    {
        eprintln!("Usage: rustward validate <rustward.toml>");
        return ExitCode::FAILURE;
    }
    match config::load(path.unwrap()) {
        Ok(config) => {
            println!(
                "Configuration is valid ({} tasks, {} schedules).",
                config.task_count(),
                config.schedules().len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Invalid configuration: {error}");
            ExitCode::FAILURE
        }
    }
}

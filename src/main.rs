use rustward::config;

use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let command = arguments.next();
    let path = arguments.next();
    if command.as_deref() != Some(std::ffi::OsStr::new("validate"))
        || path.is_none()
        || arguments.next().is_some()
    {
        eprintln!("Usage: rustward validate <config.yaml>");
        return ExitCode::FAILURE;
    }
    let source = match fs::read_to_string(path.unwrap()) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("Cannot read configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    match config::validate(&source) {
        Ok(config) => {
            println!("Configuration is valid ({} tasks).", config.task_count());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Invalid configuration: {error}");
            ExitCode::FAILURE
        }
    }
}

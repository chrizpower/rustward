use rustward::{config, service};

use std::{env, io, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let command = args.next();
    let result = match command.as_deref().and_then(|value| value.to_str()) {
        Some("validate") => match (args.next(), args.next()) {
            (Some(path), None) => config::load(path)
                .map(|config| {
                    println!(
                        "Configuration is valid ({} tasks, {} schedules).",
                        config.task_count(),
                        config.schedules().len()
                    );
                    true
                })
                .map_err(|error| format!("Invalid configuration: {error}")),
            _ => Err(usage()),
        },
        Some("serve") => service::Options::parse(args)
            .ok_or_else(usage)
            .and_then(service::serve)
            .map(|()| true),
        Some("ctl") => match (args.next(), args.next()) {
            (Some(name), None) => name.to_str().ok_or_else(usage).and_then(|name| {
                service::call(name, io::stdin().lock(), io::stdout().lock())
                    .map_err(|error| format!("Control request: {error}"))
            }),
            _ => Err(usage()),
        },
        _ => Err(usage()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    "Usage:
  rustward validate <rustward.toml>
  rustward serve <rustward.toml> <name> <runtime-catalog> <host-catalog> <work-directory> <runtime-id> <preparation-uid>
  rustward ctl <name> < request.json".into()
}

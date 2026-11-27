use rustward::config;
#[cfg(target_os = "linux")]
use rustward::service;

#[cfg(target_os = "linux")]
use std::io;
use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let command = args.next();
    let result = match command.as_deref().and_then(|value| value.to_str()) {
        Some("--version") if args.next().is_none() => {
            println!("rustward {}", rustward::VERSION);
            Ok(true)
        }
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
        #[cfg(target_os = "linux")]
        Some("serve") => service::Options::parse(args)
            .ok_or_else(usage)
            .and_then(service::serve)
            .map(|()| true),
        #[cfg(all(target_os = "linux", feature = "web-preview"))]
        Some("web-preview") => service::web_preview(args).map(|()| true),
        #[cfg(target_os = "linux")]
        Some("state") => service::state_command(args).map(|()| true),
        #[cfg(target_os = "linux")]
        Some("ctl") => match (args.next(), args.next()) {
            (Some(name), None) => name.to_str().ok_or_else(usage).and_then(|name| {
                service::call(name, io::stdin().lock(), io::stdout().lock())
                    .map_err(|error| format!("Control request: {error}"))
            }),
            _ => Err(usage()),
        },
        #[cfg(target_os = "macos")]
        Some("serve" | "ctl" | "state" | "web-preview") => Err(
            "This command requires Linux. For the synthetic console, run just web-preview.".into(),
        ),
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

#[cfg(target_os = "linux")]
fn usage() -> String {
    "Usage:
  rustward --version
  rustward validate <rustward.toml>
  rustward serve <rustward.toml> <name> <runtime-catalog> <host-catalog> <work-directory> <runtime-id> <preparation-uid>
    [--web <socket> <https-origin> <proxy-uid> | --web-tcp <ip:port> <public-origin> <proxy-token-file>]
  rustward state <backup|restore|reviews|decide|release|runtime-references> ...
  rustward ctl <name> < request.json".into()
}

#[cfg(not(target_os = "linux"))]
fn usage() -> String {
    "Usage: rustward --version | rustward validate <rustward.toml>".into()
}

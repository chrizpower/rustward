use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).peekable();
    if arguments.len() == 1
        && arguments
            .next_if(|argument| argument == "--version")
            .is_some()
    {
        println!("rustward-exec {}", rustward::VERSION);
        return ExitCode::SUCCESS;
    }

    #[cfg(target_os = "linux")]
    {
        let _ = rustward::launcher::execute(arguments);
        eprintln!("Rustward could not start the command.");
    }
    #[cfg(not(target_os = "linux"))]
    eprintln!("Rustward task execution requires Linux.");
    ExitCode::FAILURE
}

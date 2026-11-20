use std::{ffi::OsString, fs, net::TcpListener, os::unix::fs::PermissionsExt};

use super::*;

pub fn web_preview(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let port = args
        .next()
        .map(|value| value.to_str().unwrap_or("").parse::<u16>())
        .transpose()
        .map_err(|_| "Invalid preview port")?
        .unwrap_or(0);
    if args.next().is_some() {
        return Err("Usage: rustward web-preview [port]".into());
    }
    run_preview(port).map_err(|error| error.to_string())
}

fn run_preview(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let stop = install_stop()?;
    let root = tempfile::tempdir()?;
    let path = root.path();
    for name in ["logs", "tasks", "schedules"] {
        fs::create_dir(path.join(name))?;
    }
    fs::set_permissions(path.join("logs"), fs::Permissions::from_mode(0o700))?;
    fs::write(
        path.join("rustward.toml"),
        "version=2\n[storage]\nstate_database='state.db'\nlog_directory='logs'\n",
    )?;
    for (id, label) in [
        ("ordinary", "Example task"),
        ("hold", "Long task"),
        ("flood", "Truncated task"),
        ("review", "Interrupted task"),
    ] {
        fs::write(
            path.join(format!("tasks/{id}.toml")),
            format!(
                "label='{label}'\nexecutable='/synthetic/task'\nworking_directory='/synthetic'\narguments=[{{literal='{id}'}},{{input='name'}}]\ninputs=[{{id='name',type='string',required=true}},{{id='count',type='integer',default=2}},{{id='enabled',type='boolean',default=true}},{{id='mode',type='string',choices=['fast','slow'],default='fast'}}]\n"
            ),
        )?;
    }
    fs::write(
        path.join("schedules/example.toml"),
        "task='ordinary'\ncron='0 0 * * *'\n[inputs]\nname='scheduled'\n",
    )?;
    let config = config::load(path.join("rustward.toml"))?;
    let logs = WorkRoot::open(path.join("logs")).map_err(|error| format!("{error:?}"))?;
    let db = path.join("state.db");
    let safety = Safety::default();
    let controller = controller::preview::controller(&db, logs.clone(), safety.clone())
        .map_err(|error| format!("{error:?}"))?;
    let store = Store::open_with_safety(&db, safety.clone()).map_err(state_error)?;
    let mut service =
        Service::new(config, "synthetic".into(), store, controller, safety).map_err(state_error)?;
    service
        .handle(Request::SchedulePause { schedule_id: None })
        .map_err(state_error)?;
    service.handle(Request::Resume).map_err(state_error)?;
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let address = listener.local_addr()?;
    let web = web::Web::preview(listener, logs)?;
    let control = control::listen(&format!("preview-{}", std::process::id()))?;
    println!("Rustward preview: http://{address}");
    run(service, control, &stop, Some(&web))?;
    Ok(())
}

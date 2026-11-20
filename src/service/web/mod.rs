#[cfg(any(test, feature = "web-preview"))]
use std::net::TcpListener;
mod http;
#[cfg(test)]
mod tests;
mod view;

use std::{
    collections::BTreeMap,
    os::unix::fs::{FileTypeExt, MetadataExt},
    os::unix::net::UnixListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{
    Service,
    control::{Request, Response},
};
use crate::{
    admission::Error,
    runtime::WorkRoot,
    supervision::{RunId, Supervisor},
};

pub(super) struct Options {
    pub socket: PathBuf,
    pub origin: String,
    pub proxy_uid: u32,
}

pub(super) enum Listener {
    Proxy(UnixListener, u32),
    #[cfg(any(test, feature = "web-preview"))]
    Preview(TcpListener),
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Tasks {
        after: Option<String>,
    },
    Task {
        task_id: String,
    },
    Submit {
        key: String,
        task_id: String,
        inputs: BTreeMap<String, FormValue>,
    },
    Runs {
        before: Option<i64>,
        #[serde(default)]
        queued: bool,
    },
    Schedules {
        after: Option<String>,
    },
    Output {
        run_id: String,
        stdout: u64,
        stderr: u64,
    },
    Control {
        request: Request,
    },
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "lowercase",
    deny_unknown_fields
)]
enum FormValue {
    String(String),
    Integer(String),
    Boolean(bool),
}

impl FormValue {
    fn typed(self) -> Result<toml::Value, Error> {
        Ok(match self {
            Self::String(text) => toml::Value::String(text),
            Self::Integer(text) => {
                toml::Value::Integer(text.parse().map_err(|_| Error::InvalidInputs)?)
            }
            Self::Boolean(value) => toml::Value::Boolean(value),
        })
    }
}

type Call = (Operation, oneshot::Sender<Response>);

pub(super) struct Web {
    calls: mpsc::Receiver<Call>,
    logs: WorkRoot,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    socket: Option<SocketPath>,
}

struct SocketPath {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl SocketPath {
    fn new(path: PathBuf) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

impl Drop for SocketPath {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        }) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Web {
    pub(super) fn bind(options: Options, logs: WorkRoot) -> Result<Self, String> {
        use std::os::unix::fs::PermissionsExt;
        let site = http::Site::new(&options.origin, "https")?;
        if options.proxy_uid == rustix::process::geteuid().as_raw() {
            return Err("The web proxy must use a separate UID".into());
        }
        let parent = options
            .socket
            .parent()
            .filter(|path| path.is_absolute())
            .ok_or("Web socket requires an absolute path")?;
        let metadata = std::fs::symlink_metadata(parent).map_err(|err| err.to_string())?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o022 != 0
        {
            return Err("The web socket directory must be owned by the service and protected from other writers".into());
        }
        let listener = UnixListener::bind(&options.socket).map_err(|err| err.to_string())?;
        let path = SocketPath::new(options.socket).map_err(|err| err.to_string())?;
        std::fs::set_permissions(&path.path, std::fs::Permissions::from_mode(0o666))
            .map_err(|err| err.to_string())?;
        Self::start(
            Listener::Proxy(listener, options.proxy_uid),
            site,
            logs,
            Some(path),
        )
    }

    #[cfg(any(test, feature = "web-preview"))]
    pub(super) fn preview(listener: TcpListener, logs: WorkRoot) -> Result<Self, String> {
        let origin = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().map_err(|err| err.to_string())?.port()
        );
        let site = http::Site::new(&origin, "http")?;
        Self::start(Listener::Preview(listener), site, logs, None)
    }

    fn start(
        listener: Listener,
        site: http::Site,
        logs: WorkRoot,
        socket: Option<SocketPath>,
    ) -> Result<Self, String> {
        let (sender, calls) = mpsc::sync_channel(16);
        let stop = Arc::new(AtomicBool::new(false));
        let thread =
            http::start(listener, site, sender, stop.clone()).map_err(|err| err.to_string())?;
        Ok(Self {
            calls,
            logs,
            stop,
            thread: Some(thread),
            socket,
        })
    }

    pub(super) fn poll<S: Supervisor + Clone + Send + 'static>(&self, service: &mut Service<S>) {
        if let Ok((request, reply)) = self.calls.try_recv()
            && !reply.is_closed()
        {
            let result = match self.handle(service, request) {
                Ok(value) => Response::Ok(value),
                Err(error) => Response::Error(format!("{error:?}")),
            };
            let _ = reply.send(result);
        }
    }

    fn handle<S: Supervisor + Clone + Send + 'static>(
        &self,
        service: &mut Service<S>,
        operation: Operation,
    ) -> Result<Value, Error> {
        match operation {
            Operation::Tasks { after } => Ok(json!(
                service.config.task_cards(after.as_deref().unwrap_or(""))
            )),
            Operation::Task { task_id } => service
                .config
                .task_form(&task_id)
                .ok_or(Error::InvalidInputs),
            Operation::Submit {
                key,
                task_id,
                inputs,
            } => {
                let inputs = inputs
                    .into_iter()
                    .map(|(id, value)| Ok((id, value.typed()?)))
                    .collect::<Result<_, Error>>()?;
                service.handle(Request::Submit {
                    key,
                    task_id,
                    inputs,
                })
            }
            Operation::Runs { before, queued } => view::runs(&mut service.store, before, queued),
            Operation::Schedules { after } => {
                let now = chrono::DateTime::from_timestamp_millis(crate::admission::utc_ms()?)
                    .ok_or(Error::Storage)?;
                let mut page = Vec::new();
                for (schedule, (matcher, _)) in service
                    .config
                    .schedules()
                    .iter()
                    .zip(&service.matchers)
                    .filter(|(s, _)| s.id() > after.as_deref().unwrap_or(""))
                    .take(20)
                {
                    let (paused, checkpoint, last) =
                        view::schedule(&mut service.store, schedule.id())?;
                    let checkpoint = chrono::DateTime::from_timestamp_millis(checkpoint)
                        .ok_or(Error::Storage)?;
                    let next = matcher
                        .find_next_occurrence(&now.max(checkpoint), false)
                        .ok()
                        .map(|date| date.timestamp_millis());
                    page.push(
                        json!({"id":schedule.id(),"task_id":schedule.task(),"cron":schedule.cron(),
                        "paused":paused,"next_ms":next,"last_ms":last}),
                    );
                }
                Ok(json!(page))
            }
            Operation::Output {
                run_id,
                stdout,
                stderr,
            } => {
                let id = RunId::new(&run_id).map_err(|_| Error::InvalidInputs)?;
                let state = service
                    .store
                    .run_state(&run_id)?
                    .ok_or(Error::InvalidInputs)?;
                let record = service
                    .controller
                    .record(&run_id)
                    .map_err(super::controller_error)?;
                let streams = view::output(&self.logs, &id, stdout, stderr)
                    .map_err(|_| Error::OutputUnavailable)?;
                Ok(json!({"state":state,"record":record,"streams":streams}))
            }
            Operation::Control {
                request: Request::Shutdown | Request::Reload,
            } => Err(Error::InvalidInputs),
            Operation::Control { request } => service.handle(request),
        }
    }
}

impl Drop for Web {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.socket.take();
    }
}

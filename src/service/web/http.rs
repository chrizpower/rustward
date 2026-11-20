use std::{
    convert::Infallible,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response as HttpResponse, StatusCode, Uri,
    body::{Bytes, Incoming},
    header::{CACHE_CONTROL, CONTENT_TYPE, HOST, ORIGIN},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Semaphore, oneshot},
    time::timeout,
};

use super::{Call, Listener, Operation, Response};

const REQUEST_LIMIT: usize = 128 * 1024;
const RESPONSE_LIMIT: usize = 512 * 1024;
const TIME_LIMIT: Duration = Duration::from_secs(3);

pub(super) struct Site {
    origin: String,
    host: String,
}

impl Site {
    pub(super) fn new(origin: &str, scheme: &str) -> Result<Self, String> {
        let uri: Uri = origin.parse().map_err(|_| "Invalid web origin")?;
        let host = uri
            .authority()
            .ok_or("Web origin needs an authority")?
            .as_str();
        if origin != format!("{scheme}://{host}") || host.contains('@') {
            return Err("Web origin must contain only the scheme and authority".into());
        }
        Ok(Self {
            origin: origin.into(),
            host: host.into(),
        })
    }
}

fn response(
    status: StatusCode,
    content_type: &str,
    body: impl Into<Bytes>,
) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder().status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CACHE_CONTROL, "no-store")
        .header("content-security-policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'")
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "no-referrer")
        .body(Full::new(body.into())).expect("static response headers")
}

fn error(status: StatusCode, message: &'static str) -> HttpResponse<Full<Bytes>> {
    response(status, "text/plain; charset=utf-8", message)
}

async fn route(
    request: Request<Incoming>,
    site: &Site,
    sender: &mpsc::SyncSender<Call>,
) -> HttpResponse<Full<Bytes>> {
    let headers = request.headers();
    if headers.get(HOST).and_then(|value| value.to_str().ok()) != Some(site.host.as_str()) {
        return error(StatusCode::FORBIDDEN, "Wrong host");
    }
    match (request.method(), request.uri().path()) {
        (&Method::GET, "/") => response(
            StatusCode::OK,
            "text/html; charset=utf-8",
            include_str!("assets/index.html"),
        ),
        (&Method::GET, "/console.js") => response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            include_str!("assets/console.js"),
        ),
        (&Method::GET, "/console.css") => response(
            StatusCode::OK,
            "text/css; charset=utf-8",
            include_str!("assets/console.css"),
        ),
        (&Method::POST, "/api") => {
            if headers.get(ORIGIN).and_then(|value| value.to_str().ok())
                != Some(site.origin.as_str())
                || headers
                    .get("x-rustward-csrf")
                    .is_none_or(|value| value != "1")
                || headers
                    .get(CONTENT_TYPE)
                    .is_none_or(|value| value != "application/json")
            {
                return error(StatusCode::FORBIDDEN, "Request origin or headers rejected");
            }
            let Ok(Ok(body)) = timeout(
                TIME_LIMIT,
                Limited::new(request.into_body(), REQUEST_LIMIT).collect(),
            )
            .await
            else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Request body exceeds the size or time limit",
                );
            };
            let Ok(operation) = serde_json::from_slice::<Operation>(&body.to_bytes()) else {
                return error(StatusCode::BAD_REQUEST, "Invalid request");
            };
            let (send, receive) = oneshot::channel();
            if sender.try_send((operation, send)).is_err() {
                return error(StatusCode::SERVICE_UNAVAILABLE, "Service is busy");
            }
            let Ok(Ok(reply)) = timeout(TIME_LIMIT, receive).await else {
                return error(StatusCode::SERVICE_UNAVAILABLE, "Service did not reply");
            };
            let body = serde_json::to_vec(&reply).expect("JSON response");
            if body.len() > RESPONSE_LIMIT {
                return error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Response exceeds the size limit",
                );
            }
            let status = if matches!(reply, Response::Error(_)) {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::OK
            };
            response(status, "application/json", body)
        }
        _ => error(StatusCode::NOT_FOUND, "Page not found"),
    }
}

fn serve(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
    site: Arc<Site>,
    sender: mpsc::SyncSender<Call>,
    capacity: Arc<Semaphore>,
) {
    let Ok(permit) = capacity.try_acquire_owned() else {
        return;
    };
    tokio::spawn(async move {
        let _permit = permit;
        let handler = service_fn(move |request| {
            let site = site.clone();
            let sender = sender.clone();
            async move { Ok::<_, Infallible>(route(request, &site, &sender).await) }
        });
        let connection = http1::Builder::new()
            .keep_alive(false)
            .timer(TokioTimer::new())
            .header_read_timeout(TIME_LIMIT)
            .serve_connection(TokioIo::new(stream), handler);
        let _ = timeout(TIME_LIMIT * 3, connection).await;
    });
}

pub(super) fn start(
    listener: Listener,
    site: Site,
    sender: mpsc::SyncSender<Call>,
    stop: Arc<AtomicBool>,
) -> io::Result<std::thread::JoinHandle<()>> {
    match &listener {
        Listener::Proxy(listener, _) => listener.set_nonblocking(true)?,
        #[cfg(any(test, feature = "web-preview"))]
        Listener::Preview(listener) => listener.set_nonblocking(true)?,
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    std::thread::Builder::new()
        .name("rustward-web".into())
        .spawn(move || {
            let site = Arc::new(site);
            let capacity = Arc::new(Semaphore::new(16));
            runtime.block_on(async move {
                while !stop.load(Ordering::Acquire) {
                    match &listener {
                        Listener::Proxy(listener, uid) => {
                            if let Ok((stream, _)) = listener.accept()
                                && rustix::net::sockopt::socket_peercred(&stream)
                                    .is_ok_and(|peer| peer.uid.as_raw() == *uid)
                                && stream.set_nonblocking(true).is_ok()
                                && let Ok(stream) = tokio::net::UnixStream::from_std(stream)
                            {
                                serve(stream, site.clone(), sender.clone(), capacity.clone());
                                tokio::task::yield_now().await;
                                continue;
                            }
                        }
                        #[cfg(any(test, feature = "web-preview"))]
                        Listener::Preview(listener) => {
                            if let Ok((stream, _)) = listener.accept()
                                && stream.set_nonblocking(true).is_ok()
                                && let Ok(stream) = tokio::net::TcpStream::from_std(stream)
                            {
                                serve(stream, site.clone(), sender.clone(), capacity.clone());
                                tokio::task::yield_now().await;
                                continue;
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
        })
}

use super::*;
use crate::{admission::Store, config, controller::tests::Fixture, dispatch::Limits};
use std::{
    io::{Read, Write},
    net::TcpStream,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

fn http(mut stream: impl Read + Write, request: String) -> (u16, String) {
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    (
        headers.split_whitespace().nth(1).unwrap().parse().unwrap(),
        body.into(),
    )
}

fn post(host: &str, origin: &str, body: &str, csrf: bool) -> String {
    let header = if csrf { "X-Rustward-CSRF: 1\r\n" } else { "" };
    format!(
        "POST /api HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\n{header}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn serve_options_select_one_web_transport() {
    let parse = |flags: &[&str]| {
        crate::service::Options::parse(
            [
                "rustward.toml",
                "service",
                "/runtimes",
                "/hosts",
                "/work",
                "runtime",
                "1",
            ]
            .into_iter()
            .chain(flags.iter().copied())
            .map(Into::into),
        )
    };
    let tcp = ["--web-tcp", "[::]:8080", "http://rustward.test", "/token"];
    let unix = ["--web", "/web.sock", "https://rustward.test", "33"];
    assert!(
        matches!(parse(&tcp).unwrap().web, Some(Options::Tcp { address, .. }) if address.port() == 8080)
    );
    assert!(matches!(
        parse(&unix).unwrap().web,
        Some(Options::Unix { .. })
    ));
    assert!(parse(&["--web-tcp", "[::]:0", "http://rustward.test", "/token"]).is_none());
    assert!(parse(&tcp[..3]).is_none());
    assert!(parse(&[tcp, unix].concat()).is_none());
}

#[test]
fn tcp_web_requires_one_proxy_token_on_every_route() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let logs = WorkRoot::open(root.path()).unwrap();
    let token_file = root.path().join("proxy-token");
    let token = "a1".repeat(32);
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = probe.local_addr().unwrap();
    let options = |origin: &str| Options::Tcp {
        address,
        origin: origin.into(),
        token_file: token_file.clone(),
    };
    let origin = "https://rustward.test:8443";
    assert!(Web::bind(options(origin), logs.clone(), None).is_err());
    std::fs::write(&token_file, format!("{token}\n")).unwrap();
    let mode = |bits| {
        std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(bits)).unwrap()
    };
    mode(0o644);
    assert!(Web::bind(options(origin), logs.clone(), None).is_err());
    mode(0o600);
    std::fs::write(&token_file, token.to_uppercase()).unwrap();
    assert!(Web::bind(options(origin), logs.clone(), None).is_err());
    std::fs::write(&token_file, format!("{token}\n")).unwrap();
    assert!(Web::bind(options("https://rustward.test/path"), logs.clone(), None).is_err());
    assert!(Web::bind(options(origin), logs.clone(), None).is_err());
    drop(probe);
    let _web = Web::bind(
        options(origin),
        logs,
        Some(crate::telemetry::Telemetry::stderr().unwrap()),
    )
    .unwrap();
    let host = "rustward.test:8443";
    let header = format!("X-Rustward-Proxy-Token: {token}\r\n");
    let wrong = format!("X-Rustward-Proxy-Token: {}\r\n", "b2".repeat(32));
    let get = |path: &str, host: &str, headers: &str| {
        http(
            TcpStream::connect(address).unwrap(),
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{headers}\r\n"),
        )
        .0
    };
    for path in [
        "/",
        "/console.js",
        "/console.css",
        "/metrics",
        "/healthz",
        "/readyz",
        "/unknown",
    ] {
        assert_eq!(get(path, host, "X-Rustward-Monitor: 1\r\n"), 403);
    }
    assert_eq!(get("/", host, &wrong), 403);
    assert_eq!(get("/", host, &header.repeat(2)), 403);
    assert_eq!(get("/", "wrong.test", &header), 403);
    assert_eq!(get("/", host, &header), 200);
    assert_eq!(get("/metrics", host, &header), 403);
    assert_eq!(
        get(
            "/metrics",
            host,
            &format!("{header}X-Rustward-Monitor: 1\r\n")
        ),
        200
    );
    assert_eq!(
        http(
            TcpStream::connect(address).unwrap(),
            post(host, origin, "{}", true)
        )
        .0,
        403
    );
    let submit = |origin: &str, csrf| {
        let request =
            post(host, origin, "{}", csrf).replacen("\r\n\r\n", &format!("\r\n{header}\r\n"), 1);
        http(TcpStream::connect(address).unwrap(), request).0
    };
    assert_eq!(submit("http://rustward.test:8443", true), 403);
    assert_eq!(submit(origin, false), 403);
    assert_eq!(submit(origin, true), 400);
}

#[test]
fn serves_assets_and_bounded_operations_with_origin_checks() {
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let host = address.to_string();
    let origin = format!("http://{host}");
    let logs = WorkRoot::open(fixture.log_path()).unwrap();
    let web = Web::preview(listener, logs, None).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || {
        let mut service = Service::new(
            config::load("examples/rustward.toml").unwrap(),
            "runtime-1".into(),
            Store::open_with_safety(&fixture.path, fixture.safety.clone()).unwrap(),
            fixture.controller(Limits::default()),
            fixture.safety.clone(),
        )
        .unwrap();
        while !worker_stop.load(Ordering::Acquire) {
            service.step().unwrap();
            web.poll(&mut service);
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let get = |path: &str, host: &str| {
        http(
            TcpStream::connect(address).unwrap(),
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        )
    };
    for (path, marker) in [
        ("/", "type=\"module\" crossorigin src=\"/console.js\""),
        ("/", "href=\"/console.css\""),
        ("/console.js", "Run details"),
        ("/console.css", ":root"),
    ] {
        let (status, body) = get(path, &host);
        assert_eq!(status, 200);
        assert!(body.contains(marker));
    }
    assert_eq!(get("/", "wrong.example").0, 403);
    assert_eq!(get("/missing", &host).0, 404);
    assert_eq!(get("/metrics", &host).0, 404);
    let request = |origin: &str, body: &str, csrf| {
        http(
            TcpStream::connect(address).unwrap(),
            post(&host, origin, body, csrf),
        )
    };
    assert_eq!(request("https://wrong.example", "{}", true).0, 403);
    assert_eq!(request(&origin, "{}", false).0, 403);
    assert_eq!(request(&origin, "{}", true).0, 400);
    assert_eq!(request(&origin, &" ".repeat(128 * 1024 + 1), true).0, 400);
    let api = |operation: Value| {
        let (status, body) = request(&origin, &operation.to_string(), true);
        (status, serde_json::from_str::<Value>(&body).unwrap())
    };
    let control = |request: Value| api(json!({"operation":"control","request":request}));
    assert_eq!(control(json!({"command":"reload"})).0, 422);
    assert_eq!(api(json!({"operation":"task","task_id":"missing"})).0, 422);
    assert_eq!(
        api(json!({"operation":"tasks"})).1["ok"][0]["id"],
        "synthetic-report"
    );
    assert_eq!(
        api(json!({"operation":"tasks","after":"synthetic-report"})).1["ok"],
        json!([])
    );
    assert!(
        api(json!({"operation":"task","task_id":"synthetic-report"})).1["ok"]["inputs"].is_array()
    );
    assert!(api(json!({"operation":"schedules"})).1["ok"][0]["next_ms"].is_number());
    control(json!({"command":"schedule_pause"}));
    assert_eq!(
        api(json!({"operation":"schedules"})).1["ok"][0]["paused"],
        true
    );
    let key = control(json!({"command":"issue_key"})).1["ok"]["key"].clone();
    let submitted = control(
        json!({"command":"submit","key":key,"task_id":"synthetic-report","inputs":{"name":"sample"}}),
    );
    let key = control(json!({"command":"issue_key"})).1["ok"]["key"].clone();
    let form = |value: &str| {
        api(
            json!({"operation":"submit","key":key,"task_id":"synthetic-report",
        "inputs":{"name":{"type":"string","value":"sample"},"count":{"type":"integer","value":value},
                  "enabled":{"type":"boolean","value":true}}}),
        )
    };
    assert_eq!(form("9223372036854775808").0, 422);
    assert_eq!(form("3").0, 200);
    let id = submitted.1["ok"]["run_id"].as_str().unwrap();
    assert!(
        api(json!({"operation":"runs","queued":true})).1["ok"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["run_id"] == id && row["runtime_id"] == "runtime-1")
    );
    let output =
        || api(json!({"operation":"output","run_id":id,"stdout":0,"stderr":0})).1["ok"].clone();
    assert_eq!(output()["streams"]["stdout"], Value::Null);
    assert_eq!(
        api(json!({"operation":"output","run_id":"invalid","stdout":0,"stderr":0})).0,
        422
    );
    assert_eq!(
        api(json!({"operation":"output","run_id":"a".repeat(64),"stdout":0,"stderr":0})).0,
        422
    );
    control(json!({"command":"resume"}));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let value = output();
        if value["streams"]["stdout"]["next"]
            .as_u64()
            .is_some_and(|n| n > 0)
        {
            assert_eq!(value["streams"]["stderr"]["next"], 15);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    control(json!({"command":"cancel","run_id":id}));
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
}

#[test]
fn unix_web_socket_rejects_the_wrong_peer_and_removes_its_path() {
    let root = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = root.path().join("proxy.sock");
    let logs = WorkRoot::open(root.path()).unwrap();
    let uid = rustix::process::geteuid().as_raw();
    let options = |origin: &str, proxy_uid| Options::Unix {
        socket: socket.clone(),
        origin: origin.into(),
        proxy_uid,
    };
    assert!(Web::bind(options("http://rustward.test", uid + 1), logs.clone(), None).is_err());
    assert!(Web::bind(options("https://rustward.test", uid), logs.clone(), None).is_err());
    let web = Web::bind(
        options("https://rustward.test", uid + 1),
        logs.clone(),
        None,
    )
    .unwrap();
    assert!(
        Web::bind(
            options("https://rustward.test", uid + 1),
            logs.clone(),
            None
        )
        .is_err()
    );
    let mut stream = UnixStream::connect(&socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut data = Vec::new();
    match stream.read_to_end(&mut data) {
        Ok(_) => assert!(data.is_empty()),
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset),
    }
    drop(web);
    assert!(!socket.exists());
    let listener = UnixListener::bind(&socket).unwrap();
    let mut site = http::Site::new("https://rustward.test", "https").unwrap();
    use crate::telemetry::{State, Telemetry};
    let monitor = Telemetry::start(|_| Ok(())).unwrap();
    site.metrics = Some(monitor.clone());
    let _web = Web::start(
        Listener::Proxy(listener, uid),
        site,
        logs,
        Some(SocketPath::new(socket.clone()).unwrap()),
    )
    .unwrap();
    let stream = UnixStream::connect(&socket).unwrap();
    assert_eq!(
        http(
            stream,
            "GET / HTTP/1.1\r\nHost: rustward.test\r\n\r\n".into()
        )
        .0,
        200
    );
    let probe = |path: &str, headers: &str| {
        http(
            UnixStream::connect(&socket).unwrap(),
            format!("GET {path} HTTP/1.1\r\nHost: rustward.test\r\n{headers}\r\n"),
        )
    };
    let marker = "X-Rustward-Monitor: 1\r\n";
    assert_eq!(probe("/metrics", "").0, 403);
    assert_eq!(probe("/healthz", "").0, 403);
    assert_eq!(probe("/metrics", "X-Rustward-Monitor: 0\r\n").0, 403);
    assert_eq!(probe("/healthz", marker).0, 503);
    monitor.state(0, State::Paused);
    assert_eq!(probe("/healthz", marker).0, 200);
    assert_eq!(probe("/readyz", marker).0, 503);
    monitor.state(0, State::Idle);
    assert_eq!(probe("/readyz", marker).0, 200);
    monitor.state(0, State::Failed);
    assert_eq!(probe("/healthz", marker).0, 503);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let (status, body) = probe("/metrics", marker);
                assert_eq!(status, 200);
                assert!(body.contains("# TYPE rustward_run_duration_seconds histogram"));
            });
        }
    });
}

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
fn serves_assets_and_bounded_operations_with_origin_checks() {
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let host = address.to_string();
    let origin = format!("http://{host}");
    let logs = WorkRoot::open(fixture.log_path()).unwrap();
    let web = Web::preview(listener, logs).unwrap();
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
            .any(|row| row["run_id"] == id)
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
    let options = |origin: &str, proxy_uid| Options {
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
    site.metrics = Some(crate::telemetry::Telemetry::start(|_| Ok(())).unwrap());
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
    let scrape = |headers: &str| {
        http(
            UnixStream::connect(&socket).unwrap(),
            format!("GET /metrics HTTP/1.1\r\nHost: rustward.test\r\n{headers}\r\n"),
        )
    };
    assert_eq!(scrape("").0, 403);
    assert_eq!(scrape("X-Rustward-Monitor: 0\r\n").0, 403);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let (status, body) = scrape("X-Rustward-Monitor: 1\r\n");
                assert_eq!(status, 200);
                assert!(body.contains("# TYPE rustward_run_duration_seconds histogram"));
            });
        }
    });
}

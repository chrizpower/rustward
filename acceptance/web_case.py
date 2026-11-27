"""Check private Nginx access and execution through the web API."""
import base64
import json
import os
import pwd
import secrets
import socket
import ssl
import sqlite3
import subprocess
import tempfile
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from fixtures.common import TEXT
from prepare import configuration, create_root
from service_case import require, wait_for


def run(root, tools, account, fixture, user_command):
    for transport in ("unix", "tcp"):
        run_transport(root, tools, account, fixture, user_command, transport)


def loopback_address():
    with socket.socket() as port:
        port.bind(("127.0.0.1", 0))
        return f"127.0.0.1:{port.getsockname()[1]}"


def run_transport(root, tools, account, fixture, user_command, transport):
    from prometheus_client.parser import text_string_to_metric_families

    case = "web-tcp" if transport == "tcp" else "web"
    web = root / case
    create_root(web, account)
    configuration(web, root / "runtimes" / fixture["runtime_id"])
    with (web / "configuration/rustward.toml").open("a") as settings:
        settings.write("\n[telemetry]\nmetrics = true\n")
    name = f"acceptance-{case}"
    executable = tools / "rustward"
    credentials = "fixture:synthetic-browser-password"
    authorization = "Basic " + base64.b64encode(credentials.encode()).decode()
    monitor_secret = "synthetic-monitor-password"
    monitor_auth = "Basic " + base64.b64encode(f"monitor:{monitor_secret}".encode()).decode()
    with tempfile.TemporaryDirectory(prefix="rustward-nginx-") as scratch:
        directory = Path(scratch)
        directory.chmod(0o755)
        address = loopback_address()
        origin = "https://" + address
        if transport == "tcp":
            backend = loopback_address()
            while backend == address:
                backend = loopback_address()
            token = secrets.token_hex(32)
            token_file = directory / "proxy-token"
            fd = os.open(token_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400)
            with os.fdopen(fd, "w") as output:
                output.write(token + "\n")
            os.chown(token_file, account.pw_uid, account.pw_gid)
            options = ["--web-tcp", backend, origin, token_file]
            upstream = f"http://{backend}"
        else:
            token = ""
            socket_directory = web / "proxy"
            socket_directory.mkdir(mode=0o755)
            os.chown(socket_directory, account.pw_uid, account.pw_gid)
            socket_path = socket_directory / "rustward.sock"
            options = ["--web", socket_path, origin, pwd.getpwnam("www-data").pw_uid]
            upstream = f"http://unix:{socket_path}:"
        args = [executable, "serve", web / "configuration/rustward.toml", name,
                root / "runtimes", root / "hosts", web / "work",
                fixture["runtime_id"], fixture["preparation_uid"], *options]
        cert = directory / "fixture.crt"
        key = directory / "fixture.key"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                        "-out", cert, "-keyout", key], check=True, capture_output=True, timeout=20)
        password = subprocess.check_output(["openssl", "passwd", "-apr1", credentials.split(":", 1)[1]],
                                           text=True, timeout=5).strip()
        (directory / "users").write_text("fixture:" + password + "\n")
        monitor_hash = subprocess.check_output(["openssl", "passwd", "-apr1", monitor_secret],
                                               text=True, timeout=5).strip()
        (directory / "monitors").write_text("monitor:" + monitor_hash + "\n")
        monitoring_routes = "\n".join(f"""
        location = {path} {{
            auth_basic "Rustward monitoring";
            auth_basic_user_file {directory}/monitors;
            proxy_set_header Host $http_host;
            proxy_set_header Authorization "";
            proxy_set_header X-Rustward-Proxy-Token "{token}";
            proxy_set_header X-Rustward-Monitor 1;
            proxy_pass {upstream};
        }}""" for path in ("/metrics", "/healthz", "/readyz"))
        config = directory / "nginx.conf"
        config.touch(mode=0o600, exist_ok=False)
        config.write_text(f"""
user www-data;
worker_processes 1;
pid {directory}/nginx.pid;
error_log stderr warn;
events {{ worker_connections 32; }}
http {{
    access_log off;
    client_body_temp_path {directory}/client;
    proxy_temp_path {directory}/proxy;
    server {{
        listen {address} ssl;
        ssl_certificate {cert};
        ssl_certificate_key {key};
        auth_basic "Rustward acceptance";
        auth_basic_user_file {directory}/users;
        {monitoring_routes}
        location / {{
            proxy_set_header Host $http_host;
            proxy_set_header Authorization "";
            proxy_set_header X-Rustward-Proxy-Token "{token}";
            proxy_set_header X-Rustward-Monitor "";
            proxy_pass {upstream};
        }}
    }}
}}
""")
        tls = ssl.create_default_context(cafile=str(cert))

        def request(path, body=None, authenticate=True, request_origin=origin, csrf=True, monitor=False,
                    supplied_token=None):
            headers = {"Authorization": monitor_auth if monitor else authorization} if authenticate else {}
            if supplied_token is not None:
                headers["X-Rustward-Proxy-Token"] = supplied_token
            data = None
            if body is not None:
                data = json.dumps(body).encode()
                headers.update({"Origin": request_origin, "Content-Type": "application/json"})
                if csrf:
                    headers["X-Rustward-CSRF"] = "1"
            try:
                with urlopen(Request(origin + path, data=data, headers=headers), context=tls, timeout=4) as response:
                    return response.status, response.read()
            except HTTPError as error:
                return error.code, error.read()

        def scrape():
            status, data = request("/metrics", monitor=True)
            require(status == 200, f"metrics route returned {status}")
            return data, {
                (sample.name, tuple(sorted(sample.labels.items()))): sample.value
                for family in text_string_to_metric_families(data.decode())
                for sample in family.samples
            }

        def api(operation, **fields):
            status, data = request("/api", {"operation": operation, **fields})
            require(status == 200, f"web {operation} returned {status}: {data!r}")
            return json.loads(data)["ok"]

        def control(command, **fields):
            return api("control", request={"command": command, **fields})

        def ready():
            try:
                return request("/", authenticate=False)[0] == 401
            except URLError:
                return False

        def direct_tcp_status(path, supplied_token=None):
            headers = {"Host": address, "X-Rustward-Monitor": "1"}
            if supplied_token is not None:
                headers["X-Rustward-Proxy-Token"] = supplied_token
            try:
                with urlopen(Request("http://" + backend + path, headers=headers), timeout=4) as response:
                    return response.status
            except HTTPError as error:
                return error.code
            except URLError:
                return None

        with (root / "evidence" / f"{case}-server.log").open("w") as log:
            daemon = subprocess.Popen(user_command(account, args), stdout=log, stderr=log)
            nginx = None
            try:
                if transport == "tcp":
                    wait_for(lambda: direct_tcp_status("/") == 403, "TCP upstream did not start")
                else:
                    wait_for(socket_path.is_socket, "Unix upstream did not start")
                nginx = subprocess.Popen(["nginx", "-c", config, "-g", "daemon off;"], stdout=log, stderr=log)
                wait_for(ready, "Nginx did not require authentication")
                code, page = request("/")
                require(code == 200 and b"Rustward" in page, "authenticated page was not served")
                for path, marker in (("/console.js", b"Run details"), ("/console.css", b":root")):
                    require(path.encode() in page, f"console page did not reference {path}")
                    status, asset = request(path)
                    require(status == 200 and marker in asset, f"embedded asset {path} was not served")
                require(request("/metrics", authenticate=False)[0] == 401, "metrics allowed anonymous access")
                require(request("/metrics")[0] == 401, "browser credentials reached metrics")
                for path in ("/healthz", "/readyz"):
                    require(request(path)[0] == 401, "browser credentials reached monitoring health")
                require(request("/healthz", monitor=True)[0] == 200, "paused service did not report healthy")
                code, body = request("/readyz", monitor=True)
                require(code == 503 and json.loads(body) == {"ok": False, "state": "paused"},
                        "readiness did not report the dispatch pause")
                require(request("/", monitor=True)[0] == 401, "monitoring credentials reached console")
                _, before = scrape()
                require(before[("rustward_dispatch_state", (("state", "paused"),))] == 1,
                        "scrape did not report the dispatch pause")
                malicious = {"operation": "control", "request": {"command": "resume"}}
                require(request("/api", malicious, request_origin="https://other.invalid")[0] == 403,
                        "cross-origin request reached control")
                require(request("/api", malicious, csrf=False)[0] == 403, "request without CSRF header reached control")
                if transport == "tcp":
                    for path in ("/", "/console.js", "/metrics", "/healthz", "/readyz"):
                        require(direct_tcp_status(path) == 403, f"direct TCP reached {path}")
                    require(direct_tcp_status("/", "forged") == 403, "TCP accepted a forged token")
                    require(request("/", authenticate=False, supplied_token=token)[0] == 401,
                            "the upstream token bypassed Nginx authentication")
                    require(request("/", supplied_token="forged")[0] == 200,
                            "Nginx did not replace the client token")
                else:
                    with socket.socket(socket.AF_UNIX) as direct:
                        direct.settimeout(4)
                        direct.connect(str(socket_path))
                        try:
                            direct.sendall(f"GET / HTTP/1.1\r\nHost: {address}\r\n\r\n".encode())
                            require(not direct.recv(1024), "direct peer bypassed Nginx")
                        except (BrokenPipeError, ConnectionResetError):
                            pass
                tasks = api("tasks")
                require(any(task["id"] == "ordinary" for task in tasks), "fixture task missing")
                key_value = control("issue_key")["key"]
                inputs = {"text": TEXT, "count": 7, "enabled": True}
                first = control("submit", key=key_value, task_id="ordinary", inputs=inputs)
                retry = control("submit", key=key_value, task_id="ordinary", inputs=inputs)
                run_id = first["run_id"]
                require(retry == {"status": "already_admitted", "run_id": run_id}, "web retry created another run")
                require(control("service_status")["paused"], "rejected request changed dispatch state")
                control("resume")
                wait_for(lambda: control("status", run_id=run_id) == {"phase": "Completed", "result": "Success"},
                         "web run did not complete")
                output = api("output", run_id=run_id, stdout=0, stderr=0)
                require(bytes(output["streams"]["stdout"]["bytes"]) == b"A02_STDOUT\n", "web stdout differs")
                require(bytes(output["streams"]["stderr"]["bytes"]) == b"A02_STDERR\n", "web stderr differs")
                wait_for(lambda: request("/readyz", monitor=True)[0] == 200,
                         "resumed service did not report ready")
                data, after = scrape()
                for metric, labels in [
                    ("rustward_admissions_total", (("trigger", "manual"),)),
                    ("rustward_completions_total", (("result", "success"),)),
                    ("rustward_run_duration_seconds_count", ()),
                ]:
                    require(after[(metric, labels)] == 1, f"unexpected {metric} count")
                for secret in (run_id, key_value, TEXT, "A02_STDOUT", monitor_secret):
                    require(secret.encode() not in data, "metrics contain run data")
                daemon.terminate()
                require(daemon.wait(timeout=20) == 0, "web daemon failed to drain")
            finally:
                for process in (nginx, daemon):
                    if process is not None and process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait(timeout=5)

    logs = (root / "evidence" / f"{case}-server.log").read_text()
    records = [json.loads(line) for line in logs.splitlines() if line.startswith('{"timestamp_ms":')]
    run_events = {record["event"] for record in records if record.get("run_id") == run_id}
    require({"run_admitted", "dispatch_committed", "run_completed"} <= run_events,
            "service logs do not trace the synthetic run")
    for secret in (key_value, TEXT, "A02_STDOUT", monitor_secret, *([token] if token else [])):
        require(secret not in logs, "service logs contain run data")

    with sqlite3.connect((web / "state/state.db").as_uri() + "?mode=ro", uri=True) as db:
        require(db.execute("SELECT run_id,phase,result FROM runs").fetchall() == [(run_id, "completed", "success")],
                "web database did not retain one successful run")
    (root / "evidence" / f"{case}.json").write_text(json.dumps({
        "status": "passed", "run_id": run_id, "authentication_required": True,
        "direct_peer_rejected": True, "cross_origin_rejected": True, "retained_log_bytes": 22,
        "metrics_access_verified": True, "service_log_trace_verified": True,
        "monitoring_health_verified": True,
    }) + "\n")

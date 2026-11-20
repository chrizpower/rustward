"""Check private Nginx access and execution through the web API."""
import base64
import json
import os
import pwd
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
    web = root / "web"
    create_root(web, account)
    configuration(web, root / "runtimes" / fixture["runtime_id"])
    socket_directory = web / "proxy"
    socket_directory.mkdir(mode=0o755)
    os.chown(socket_directory, account.pw_uid, account.pw_gid)
    socket_path = socket_directory / "rustward.sock"
    proxy = pwd.getpwnam("www-data")
    name = "acceptance-web"
    executable = tools / "rustward"
    credentials = "fixture:synthetic-browser-password"
    authorization = "Basic " + base64.b64encode(credentials.encode()).decode()
    with socket.socket() as port:
        port.bind(("127.0.0.1", 0))
        address = "127.0.0.1:" + str(port.getsockname()[1])
    origin = "https://" + address
    args = [executable, "serve", web / "configuration/rustward.toml", name,
            root / "runtimes", root / "hosts", web / "work",
            fixture["runtime_id"], fixture["preparation_uid"], "--web", socket_path, origin, proxy.pw_uid]

    with tempfile.TemporaryDirectory(prefix="rustward-nginx-") as scratch:
        directory = Path(scratch)
        directory.chmod(0o755)
        cert = directory / "fixture.crt"
        key = directory / "fixture.key"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                        "-out", cert, "-keyout", key], check=True, capture_output=True, timeout=20)
        password = subprocess.check_output(["openssl", "passwd", "-apr1", credentials.split(":", 1)[1]],
                                           text=True, timeout=5).strip()
        (directory / "users").write_text("fixture:" + password + "\n")
        config = directory / "nginx.conf"
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
        location / {{
            proxy_set_header Host $http_host;
        proxy_set_header Authorization "";
            proxy_pass http://unix:{socket_path}:;
        }}
    }}
}}
""")
        tls = ssl.create_default_context(cafile=str(cert))

        def request(path, body=None, authenticate=True, request_origin=origin, csrf=True):
            headers = {"Authorization": authorization} if authenticate else {}
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

        with (root / "evidence/web-server.log").open("w") as log:
            daemon = subprocess.Popen(user_command(account, args), stdout=log, stderr=log)
            nginx = None
            try:
                wait_for(socket_path.is_socket, "web upstream did not start")
                nginx = subprocess.Popen(["nginx", "-c", config, "-g", "daemon off;"], stdout=log, stderr=log)
                wait_for(ready, "Nginx did not require authentication")
                code, page = request("/")
                require(code == 200 and b"Rustward" in page, "authenticated page was not served")
                malicious = {"operation": "control", "request": {"command": "resume"}}
                require(request("/api", malicious, request_origin="https://other.invalid")[0] == 403,
                        "cross-origin request reached control")
                require(request("/api", malicious, csrf=False)[0] == 403, "request without CSRF header reached control")
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
                wait_for(lambda: control("status", run_id=run_id)["result"] == "Success", "web run did not complete")
                output = api("output", run_id=run_id, stdout=0, stderr=0)
                require(bytes(output["streams"]["stdout"]["bytes"]) == b"A02_STDOUT\n", "web stdout differs")
                require(bytes(output["streams"]["stderr"]["bytes"]) == b"A02_STDERR\n", "web stderr differs")
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

    with sqlite3.connect((web / "state/state.db").as_uri() + "?mode=ro", uri=True) as db:
        require(db.execute("SELECT run_id,phase,result FROM runs").fetchall() == [(run_id, "completed", "success")],
                "web database did not retain one successful run")
    (root / "evidence/web.json").write_text(json.dumps({
        "status": "passed", "run_id": run_id, "authentication_required": True,
        "direct_peer_rejected": True, "cross_origin_rejected": True, "retained_log_bytes": 22,
    }) + "\n")

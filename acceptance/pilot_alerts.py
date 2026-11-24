"""Rehearse Prometheus and Alertmanager delivery to a loopback SMTP sink."""
import json
from contextlib import ExitStack, contextmanager
from email import policy
from email.parser import BytesParser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from queue import Empty, Queue
import secrets
import socket
import subprocess
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from service_case import require, wait_for

RECIPIENT = "pilot@rustward.invalid"
MARKER = {"X-Rustward-Monitor": "1"}


def ports(count):
    with ExitStack() as stack:
        sockets = [stack.enter_context(socket.socket()) for _ in range(count)]
        for sock in sockets:
            sock.bind(("127.0.0.1", 0))
        return [sock.getsockname()[1] for sock in sockets]


def request(origin, path, headers=None, data=None):
    try:
        with urlopen(Request(origin + path, data=data, headers=headers or {}), timeout=2) as response:
            return response.status, response.read()
    except HTTPError as error:
        return error.code, error.read()
    except (OSError, URLError):
        return 0, b""


@contextmanager
def process(command, log):
    with log.open("w") as output:
        child = subprocess.Popen(command, stdout=output, stderr=output)
        try:
            yield child
        finally:
            stop(child)


def stop(child):
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=8)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=5)


class Mailbox:
    def __init__(self):
        self.mail = Queue(maxsize=32)

    async def handle_DATA(self, server, session, envelope):
        if envelope.rcpt_tos != [RECIPIENT]:
            return "550 Recipient outside the pilot"
        message = BytesParser(policy=policy.default).parsebytes(envelope.content)
        self.mail.put_nowait(message["Subject"].strip())
        return "250 Message accepted"


@contextmanager
def bridge(target):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path != "/metrics":
                self.send_error(404)
                return
            status, body = request(target, self.path, MARKER)
            self.send_response(status or 502)
            self.send_header("Content-Type", "text/plain; version=0.0.4")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format, *args):
            pass

    import threading
    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield f"127.0.0.1:{server.server_port}"
        finally:
            server.shutdown()
            worker.join(timeout=3)


def write_settings(directory, target, smtp, manager):
    config = {
        "global": {"scrape_interval": "1s", "scrape_timeout": "1s", "evaluation_interval": "1s"},
        "scrape_configs": [{"job_name": "rustward-pilot", "static_configs": [{"targets": [target]}]}],
        "rule_files": [str(Path(__file__).parent / "fixtures/pilot-alerts.yml")],
        "alerting": {"alertmanagers": [{"static_configs": [{"targets": [manager]}]}]},
    }
    (directory / "prometheus.yml").write_text(json.dumps(config))
    config = {
        "global": {"smtp_smarthost": smtp, "smtp_from": "rustward@rustward.invalid", "smtp_require_tls": False},
        "route": {"receiver": "pilot", "group_by": ["alertname", "result"],
                  "group_wait": "0s", "group_interval": "1s", "repeat_interval": "2m"},
        "receivers": [{"name": "pilot", "email_configs": [
            {"to": RECIPIENT, "send_resolved": False,
             "headers": {"Subject": "{{ .CommonLabels.alertname }} {{ .CommonLabels.result }}"}}
        ]}],
    }
    (directory / "alertmanager.yml").write_text(json.dumps(config))


def run(preview, directory):
    from aiosmtpd.controller import Controller

    directory.mkdir(exist_ok=True, parents=True)
    mailbox = Mailbox()
    smtp_port, manager_port, prometheus_port, rustward_port = ports(4)
    smtp = Controller(mailbox, hostname="127.0.0.1", port=smtp_port)
    origin = f"http://127.0.0.1:{rustward_port}"
    observed = set()

    def delivered(expected):
        while True:
            try:
                observed.add(mailbox.mail.get_nowait())
            except Empty:
                return expected in observed

    def probe(path, code, state):
        received, data = request(origin, path, MARKER)
        return received == code and json.loads(data) == {"ok": code == 200, "state": state}

    def api(operation, **values):
        headers = {"Origin": origin, "Content-Type": "application/json", "X-Rustward-CSRF": "1"}
        code, body = request(origin, "/api", headers, json.dumps({"operation": operation, **values}).encode())
        require(code == 200, f"Pilot API {operation} returned {code}")
        return json.loads(body)["ok"]

    def control(command, **values):
        return api("control", request={"command": command, **values})

    def run_task(task, result):
        fields = {"name": {"type": "string", "value": "pilot"}} if task == "review" else {}
        run_id = api("submit", key=control("issue_key")["key"], task_id=task, inputs=fields)["run_id"]
        wait_for(lambda: control("status", run_id=run_id) == {"phase": "Completed", "result": result},
                 f"Pilot run did not complete with {result}")
        wait_for(lambda: delivered("RustwardRunProblem " + result.lower()),
                 f"SMTP did not receive the {result} alert")

    with ExitStack() as stack:
        smtp.start()
        stack.callback(smtp.stop)
        rustward = stack.enter_context(process([str(preview), "web-preview", "--pilot", str(rustward_port)],
                                                directory / "rustward.log"))
        wait_for(lambda: request(origin, "/readyz", MARKER)[0] == 200, "Pilot did not become ready")
        target = stack.enter_context(bridge(origin))
        manager = f"127.0.0.1:{manager_port}"
        write_settings(directory, target, f"127.0.0.1:{smtp_port}", manager)
        stack.enter_context(process(["prometheus-alertmanager", f"--config.file={directory}/alertmanager.yml",
                                     f"--storage.path={directory}/alerts", f"--web.listen-address={manager}",
                                     "--cluster.listen-address="], directory / "alertmanager.log"))
        stack.enter_context(process(["prometheus", f"--config.file={directory}/prometheus.yml",
                                     f"--storage.tsdb.path={directory}/series",
                                     f"--web.listen-address=127.0.0.1:{prometheus_port}"],
                                    directory / "prometheus.log"))
        wait_for(lambda: delivered("RustwardScheduleGap"), "SMTP did not receive the schedule gap alert")
        require(b"rustward_schedule_unknown_gaps_total 1\n" in request(origin, "/metrics", MARKER)[1],
                "Pilot did not report one unknown schedule gap")
        control("pause")
        wait_for(lambda: probe("/readyz", 503, "paused"),
                 "Readiness did not report the deliberate pause")
        require(request(origin, "/healthz", MARKER)[0] == 200, "Pause affected liveness")
        require(request(origin, "/healthz")[0] == 403, "Monitoring route accepted a request without the marker")
        control("resume")
        run_task("failure", "Failure")
        run_task("timeout", "Timeout")
        pending = control("reviews", after=None)
        require(len(pending) == 1 and pending[0]["state"]["result"] == "Timeout",
                "Timed-out run did not require review")
        control("acknowledge", acknowledgement_id=secrets.token_hex(32), review=pending[0],
                note="Synthetic cleanup and external effects checked")
        run_task("review", "Interruption")
        require(probe("/readyz", 503, "review"),
                "Readiness did not report the review block")
        require(request(origin, "/healthz", MARKER)[0] == 200, "Review affected liveness")
        stop(rustward)
        require(rustward.returncode == 0, "Pilot service did not stop cleanly")
        wait_for(lambda: delivered("RustwardUnavailable"), "SMTP did not receive the outage alert")
        require("RustwardUnhealthy" not in observed, "An operator pause or review emitted a health alert")
    report = {"status": "passed", "recipient": RECIPIENT, "delivered": sorted(observed)}
    (directory / "pilot-alerts.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    run(Path(sys.argv[1]), Path(sys.argv[2]))

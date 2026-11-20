"""Check the daemon through its local control interface."""
import json
import signal
import sqlite3
import subprocess
import time

from prepare import configuration, create_root
from fixtures.common import TEXT


def require(condition, detail):
    if not condition:
        raise RuntimeError(detail)


def wait_for(check, detail):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.03)
    raise RuntimeError(detail)


def run(root, tools, account, fixture, user_command):
    service_root = root / "daemon"
    create_root(service_root, account)
    runtime = root / "runtimes" / fixture["runtime_id"]
    configuration(service_root, runtime)
    config = service_root / "configuration/rustward.toml"
    with config.open("a") as output:
        output.write("\n[limits]\nactive_runs=1\nqueued_runs=2\nlog_bytes_per_run=32\nretained_log_bytes=54\n")
    task = service_root / "configuration/tasks"
    (task / "drain.toml").write_text((task / "recovery.toml").read_text().replace(
        'label="recovery"', 'label="drain"').replace('literal="hold"', 'literal="drain"'))
    executable = tools / "rustward"
    name = "acceptance-daemon"
    evidence = root / "evidence"
    args = [executable, "serve", config, name, root / "runtimes",
            root / "hosts", service_root / "work", fixture["runtime_id"], fixture["preparation_uid"]]

    def request(command, startup=False, **fields):
        result = subprocess.run(user_command(account, [executable, "ctl", name]),
                                input=json.dumps({"command": command, **fields}),
                                text=True, capture_output=True, timeout=10)
        if startup and not result.stdout and "Connection refused" in result.stderr:
            return None
        require(result.returncode == 0, f"{command} failed: {result.stderr or result.stdout}")
        return json.loads(result.stdout)["ok"]

    def start(output):
        return subprocess.Popen(user_command(account, args), stdout=output, stderr=output)

    def ready():
        return wait_for(lambda: request("service_status", startup=True), "daemon startup deadline")

    def submit(task_id="ordinary", key=None):
        key = key or request("issue_key")["key"]
        admitted = request("submit", key=key, task_id=task_id,
                           inputs={"text": TEXT, "count": 7, "enabled": True})
        return key, admitted["status"], admitted["run_id"]

    def completed(run_id, result="Success"):
        def check():
            current = request("status", run_id=run_id)
            if current["phase"] == "Completed":
                require(current["result"] == result, "unexpected daemon run result")
                return True
            return False
        wait_for(check, "daemon completion deadline")

    def logs(run_id):
        return service_root / "logs" / run_id

    def work(run_id):
        return service_root / "work" / run_id

    with (evidence / "daemon.log").open("w") as output:
        process = start(output)
        try:
            require(ready()["paused"], "new daemon did not start paused")
            key, _, first = submit()
            require(submit(key=key)[1:] == ("already_admitted", first), "retry changed run identity")
            request("resume")
            completed(first)
            _, _, second = submit()
            completed(second)
            require(logs(first).is_dir(), "logs reclaimed before pressure")
            _, _, third = submit()
            completed(third)
            require(not logs(first).exists() and logs(second).is_dir(), "reclamation order differs")
            require(request("status", run_id=first)["result"] == "Success", "reclamation lost history")

            _, _, interrupted = submit("recovery")
            _, _, queued = submit()
            wait_for(lambda: (work(interrupted) / "ready.json").is_file()
                     and request("status", run_id=interrupted)["phase"] == "Running",
                     "interruptible run did not start")
            process.kill()
            require(process.wait(timeout=10) == -signal.SIGKILL, "daemon did not crash as planned")
            process = start(output)
            require(ready()["paused"], "restarted daemon did not start paused")
            request("resume")
            completed(interrupted, "Interruption")
            wait_for(lambda: request("service_status")["dispatch_block"] == "ReviewRequired",
                     "dispatch did not report review block")
            require(request("status", run_id=queued)["phase"] == "Queued", "queued work started during review")
            require(len((work(interrupted) / "starts.jsonl").read_text().splitlines()) == 1,
                    "interrupted task restarted")
            require((logs(interrupted) / "stdout.log").stat().st_size == 11, "recovery lost protected output")
            reviewed, = request("reviews")
            require(reviewed["run_id"] == interrupted, "wrong run needs review")
            request("acknowledge", acknowledgement_id="a" * 64, review=reviewed,
                    note="Synthetic task stopped locally. No external effects. Resume the fixture.")
            completed(queued)

            _, _, draining = submit("drain")
            wait_for(lambda: (work(draining) / "ready.json").is_file(), "drain task did not start")
            _, _, preserved = submit()
            process.send_signal(signal.SIGTERM)
            wait_for(lambda: request("service_status")["draining"], "daemon did not enter drain")
            (work(draining) / "finish").touch()
            require(process.wait(timeout=30) == 0, "planned shutdown failed")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)

    path = service_root / "state/state.db"
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as database:
        require(database.execute("PRAGMA integrity_check").fetchone() == ("ok",), "daemon database integrity")
        require(not database.execute("PRAGMA foreign_key_check").fetchall(), "daemon database foreign keys")
        for run_id, expected in ((draining, ("completed", "success", 1)), (preserved, ("queued", None, 0))):
            row = database.execute("SELECT phase,result,dispatched FROM runs WHERE run_id=?", (run_id,)).fetchone()
            require(row == expected, "planned shutdown changed queued work or stopped the active task")
        require(database.execute("SELECT log_bytes FROM launches WHERE run_id=?", (first,)).fetchone() == (0,),
                "reclamation did not release accounting")
    require((logs(draining) / "stdout.log").read_bytes() == b"A02_STDOUT\n", "drained output is missing")
    (evidence / "daemon.json").write_text(json.dumps({
        "status": "passed", "reclaimed": first, "interrupted": interrupted,
        "recovered_successor": queued, "drained": draining, "preserved": preserved,
    }) + "\n")

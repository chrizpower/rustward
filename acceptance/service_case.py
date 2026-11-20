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


def wait_for(check, detail, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.03)
    raise RuntimeError(detail)


def control(account, executable, name, user_command, command, startup=False, error=None, **fields):
    result = subprocess.run(user_command(account, [executable, "ctl", name]),
                            input=json.dumps({"command": command, **fields}),
                            text=True, capture_output=True, timeout=10)
    if startup and not result.stdout and "Connection refused" in result.stderr:
        return None
    if error is not None:
        require(result.returncode != 0 and json.loads(result.stdout) == {"error": error},
                f"{command} did not report {error}")
        return None
    require(result.returncode == 0, f"{command} failed: {result.stderr or result.stdout}")
    return json.loads(result.stdout)["ok"]


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

    def request(command, **fields):
        return control(account, executable, name, user_command, command, **fields)

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


def run_schedules(root, tools, account, fixture, user_command):
    scheduled = root / "scheduled"
    create_root(scheduled, account)
    configuration(scheduled, root / "runtimes" / fixture["runtime_id"])
    config = scheduled / "configuration/rustward.toml"
    schedule = scheduled / "configuration/schedules/minute.toml"
    executable = tools / "rustward"
    name = "acceptance-scheduler"
    args = [executable, "serve", config, name, root / "runtimes", root / "hosts",
            scheduled / "work", fixture["runtime_id"], fixture["preparation_uid"]]

    def request(command, **fields):
        return control(account, executable, name, user_command, command, **fields)

    def history():
        return request("schedule_history", schedule_id="minute")

    def ready():
        return wait_for(lambda: request("service_status", startup=True), "scheduler startup deadline")

    with (root / "evidence/scheduler.log").open("w") as output:
        process = subprocess.Popen(user_command(account, args), stdout=output, stderr=output)
        try:
            require(ready()["paused"], "scheduler daemon did not start paused")
            valid = "task='ordinary'\ncron='* * * * *'\n[inputs]\n" + f"text={json.dumps(TEXT)}\ncount=7\nenabled=true\n"
            schedule.write_text(valid)
            require(request("reload")["schedules"] == 1, "schedule activation failed")
            schedule.write_text("invalid")
            request("reload", error="InvalidConfiguration")
            records = wait_for(lambda: history()["occurrences"], "scheduled admission deadline", timeout=125)
            require(len(records) == 1 and records[0]["decision"] == "admitted",
                    "active schedule did not admit exactly once after rejected reload")
            run_id = records[0]["run_id"]
            request("resume")
            wait_for(lambda: request("status", run_id=run_id)["result"] == "Success",
                     "scheduled run did not complete")
            request("schedule_pause", schedule_id="minute")
            request("shutdown")
            require(process.wait(timeout=30) == 0, "scheduler shutdown failed")
            schedule.write_text(valid)
            process = subprocess.Popen(user_command(account, args), stdout=output, stderr=output)
            require(ready()["paused"], "scheduler restart did not start paused")
            restarted = history()
            require(restarted["paused"], "scheduler restart lost pause")
            require([record["run_id"] for record in restarted["occurrences"] if record["run_id"]] == [run_id],
                    "scheduler restart replayed an occurrence")
            request("shutdown")
            require(process.wait(timeout=30) == 0, "restarted scheduler shutdown failed")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)

    work = scheduled / "work" / run_id
    require(len((work / "starts.jsonl").read_text().splitlines()) == 1, "scheduled run started twice")
    path = scheduled / "state/state.db"
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as database:
        require(database.execute("PRAGMA integrity_check").fetchone() == ("ok",), "scheduler database integrity")
        require(database.execute("SELECT phase,result FROM runs").fetchall() == [("completed", "success")],
                "scheduler retained extra or incomplete runs")
        require(database.execute("SELECT due_ms FROM occurrences WHERE run_id=?", (run_id,)).fetchall()
                == [(records[0]["due_ms"],)], "scheduler occurrence and run differ")
    (root / "evidence/scheduler.json").write_text(json.dumps({
        "status": "passed", "schedule_id": "minute", "run_id": run_id,
        "due_ms": records[0]["due_ms"], "invalid_reload_rejected": True,
    }) + "\n")

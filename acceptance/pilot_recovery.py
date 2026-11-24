"""Rehearse a state restore with real systemd runs and one schedule owner."""
from contextlib import ExitStack
import json
import subprocess

from fixtures.common import TEXT
from pilot_alerts import stop
from prepare import configuration, create_root
from service_case import control, require, wait_for


def run(root, tools, account, fixture, user_command):
    original = root / "pilot-original"
    replacement = root / "pilot-restored"
    runtime = root / "runtimes" / fixture["runtime_id"]
    executable = tools / "rustward"
    evidence = root / "evidence"
    for location in (original, replacement):
        create_root(location, account)
        configuration(location, runtime)
        (location / "configuration/schedules/minute.toml").write_text(
            "task='ordinary'\ncron='* * * * *'\n[inputs]\n"
            f"text={json.dumps(TEXT)}\ncount=7\nenabled=true\n"
        )
    backup = original / "state" / "backup"
    restored = replacement / "state" / "restored"
    source_config = original / "configuration/rustward.toml"
    restored_config = replacement / "configuration/rustward.toml"
    restored_config.write_text(
        f"version=2\n[storage]\nstate_database={json.dumps(str(restored / 'state.db'))}\n"
        f"log_directory={json.dumps(str(restored / 'logs'))}\n"
    )
    old_name, new_name = "pilot-original", "pilot-restored"

    def args(config, name, work):
        return user_command(account, [
            executable, "serve", config, name, root / "runtimes", root / "hosts",
            work, fixture["runtime_id"], fixture["preparation_uid"],
        ])

    def state(*arguments, fails_with=None):
        result = subprocess.run(user_command(account, [executable, "state", *arguments]),
                                capture_output=True, text=True, timeout=45)
        if fails_with:
            require(result.returncode != 0 and fails_with in result.stderr,
                    f"State command {arguments[0]} did not reject the operation")
        else:
            require(result.returncode == 0, f"State command {arguments[0]} failed")
        return result.stdout

    def request(name, command, **values):
        return control(account, executable, name, user_command, command, **values)

    def wait_ready(name):
        return wait_for(lambda: request(name, "service_status", startup=True), "Pilot service startup deadline")

    def admit(name, key):
        return request(name, "submit", key=key, task_id="ordinary",
                       inputs={"text": TEXT, "count": 7, "enabled": True})

    def admitted_occurrences(name):
        history = request(name, "schedule_history", schedule_id="minute")
        return [row for row in history["occurrences"] if row["decision"] == "admitted"]

    def completed(name, run_id):
        wait_for(lambda: request(name, "status", run_id=run_id)
                 == {"phase": "Completed", "result": "Success"}, "Pilot run completion deadline")

    with ExitStack() as stack:
        log = stack.enter_context((evidence / "pilot-recovery.log").open("w"))
        before = subprocess.Popen(args(source_config, old_name, original / "work"), stdout=log, stderr=log)
        stack.callback(stop, before)
        require(wait_ready(old_name)["paused"], "Original service did not start paused")
        state("backup", source_config, backup, fails_with="lock")
        contender = subprocess.run(args(source_config, "pilot-contender", original / "work"),
                                   text=True, capture_output=True, timeout=15)
        require(contender.returncode != 0 and "OwnerBusy" in contender.stderr,
                "A second scheduler obtained the original database")
        first = wait_for(lambda: admitted_occurrences(old_name), "Original scheduler did not admit a run", timeout=125)[0]
        request(old_name, "schedule_pause")
        request(old_name, "resume")
        completed(old_name, first["run_id"])
        request(old_name, "pause")
        kept_key = request(old_name, "issue_key")["key"]
        kept = admit(old_name, kept_key)["run_id"]
        cancelled = admit(old_name, request(old_name, "issue_key")["key"])["run_id"]
        stale_key = request(old_name, "issue_key")["key"]
        request(old_name, "shutdown")
        require(before.wait(timeout=30) == 0, "Original service failed to stop")

        state("backup", source_config, backup)
        state("restore", backup, restored)
        state("release", restored, fails_with="InvalidTransition")
        gated = subprocess.run(args(restored_config, new_name, replacement / "work"),
                               text=True, capture_output=True, timeout=15)
        require(gated.returncode != 0 and "Closed" in gated.stderr, "Restored service started before review")
        pending = json.loads(state("reviews", restored))
        require({run["run_id"] for run in pending} == {kept, cancelled}, "Unexpected restored runs")
        state("decide", restored, kept, "keep", "Synthetic work confirmed safe to execute")
        state("decide", restored, cancelled, "cancel", "Synthetic work discarded")
        state("release", restored)
        after = subprocess.Popen(args(restored_config, new_name, replacement / "work"),
                                 stdout=log, stderr=log)
        stack.callback(stop, after)
        require(wait_ready(new_name)["paused"], "Restored dispatch was not paused")
        require(request(new_name, "status", run_id=kept)["phase"] == "Queued", "Kept run executed before resume")
        require(request(new_name, "status", run_id=cancelled)["result"] == "Cancellation",
                "Cancelled run was not recorded")
        require(admit(new_name, kept_key) == {"run_id": kept, "status": "already_admitted"},
                "Known admission did not keep its identity")
        request(new_name, "submit", key=stale_key, task_id="ordinary", inputs={}, error="InvalidKey")
        old_output = restored / "logs" / first["run_id"] / "stdout.log"
        require(old_output.read_bytes() == b"A02_STDOUT\n", "Restored task output differs")
        require(admitted_occurrences(new_name) == [first], "Restore replayed an earlier occurrence")
        request(new_name, "resume")
        completed(new_name, kept)
        request(new_name, "schedule_resume")
        second = wait_for(lambda: next((row for row in admitted_occurrences(new_name)
                                       if row["due_ms"] > first["due_ms"]), None),
                          "Restored scheduler did not admit a new occurrence", timeout=125)
        request(new_name, "schedule_pause")
        completed(new_name, second["run_id"])
        request(new_name, "shutdown")
        require(after.wait(timeout=30) == 0, "Restored service failed to stop")

    require(before.poll() == 0 and after.poll() == 0, "Pilot left a scheduler running")
    for location, run_id in [(original, first["run_id"]), (replacement, second["run_id"])]:
        starts = location / "work" / run_id / "starts.jsonl"
        require(len(starts.read_text().splitlines()) == 1, "A schedule occurrence ran more than once")
    (evidence / "pilot-recovery.json").write_text(json.dumps({
        "status": "passed", "old_due_ms": first["due_ms"], "new_due_ms": second["due_ms"],
        "held_until_review": True, "second_owner_rejected": True, "old_key_rejected": True,
    }) + "\n")

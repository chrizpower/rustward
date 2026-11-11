"""Run A03 with ephemeral credentials on the isolated controller host."""
import json
import os
from pathlib import Path
import pwd
import sys
import time

from fixtures.ssh_policy import CREDENTIALS, ALIAS, USER, options
from inside import checked, invoke, user_command, wait_manager, stop_fixture_units
from prepare import prepare
from ssh_credentials import client_key, export_status

ROOT = Path("/opt/rustward-ssh-acceptance")
TOOLS = Path("/opt/rustward-acceptance-tools")


def pin(value):
    fields = value["host_key"].split()
    if len(fields) != 2 or fields[0] != "ssh-ed25519":
        raise ValueError("invalid fixture host public key")
    (CREDENTIALS / "known_hosts").write_text(f"{ALIAS} {' '.join(fields)}\n")
    (CREDENTIALS / "known_hosts").chmod(0o644)
    (CREDENTIALS / "connection.json").write_text(json.dumps({"address": value["address"], "port": 2222}))
    (CREDENTIALS / "connection.json").chmod(0o644)


def probe(account, runtime, evidence):
    connection = json.loads((CREDENTIALS / "connection.json").read_text())
    # Both probes are read-only. An admitted task is never retried.
    def command(known):
        return user_command(account, [runtime / "bin/ssh", *options(known),
                            "-i", CREDENTIALS / "client_key", "-p", "2222",
                            f"{USER}@{connection['address']}", "true"])
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        result = invoke(command(CREDENTIALS / "known_hosts"), timeout=10, capture=True)
        if result.returncode == 0:
            break
        time.sleep(0.2)
    else:
        raise RuntimeError("pinned SSH target did not become ready")
    wrong = invoke(command(CREDENTIALS / "wrong_known_hosts"), timeout=15, capture=True)
    rejected = wrong.returncode != 0 and "Host key verification failed" in wrong.stderr
    (evidence / "host-key-check.json").write_text(json.dumps({"status": "passed" if rejected else "failed",
        "wrong_key_rejected": rejected, "correct_key_accepted": True,
        "returncode": wrong.returncode}) + "\n")
    if not rejected:
        raise RuntimeError("wrong host key was not rejected")


def check_commands(account, runtime, evidence):
    result = checked(user_command(account, [runtime / "bin/python3", "-I", "-B",
                     runtime / "ssh_commands.py", evidence / "ssh-commands"]),
                     capture_output=True, timeout=30)
    report = json.loads(result.stdout)
    if report.get("status") != "passed":
        raise RuntimeError("Ansible SSH command check did not pass")
    (evidence / "ssh-command-check.json").write_text(json.dumps(report, indent=2) + "\n")


def run(revision):
    account = pwd.getpwnam("rustward")
    fixture = prepare(ROOT, TOOLS, "rustward", revision, remote=True)
    evidence = ROOT / "evidence"
    status = {"status": "failed"}
    try:
        checked(["/usr/bin/systemctl", "start", f"user@{account.pw_uid}.service"])
        checked(user_command(account, ["/usr/bin/systemctl", "--user", "start", "dbus.socket"]))
        wait_manager(account, TOOLS, evidence)
        runtime = ROOT / "runtimes" / fixture["runtime_id"]
        probe(account, runtime, evidence)
        check_commands(account, runtime, evidence)
        checked(user_command(account, [TOOLS / "acceptance", "run-ssh", ROOT]), timeout=240)
        status["status"] = "passed"
    finally:
        (evidence / "ssh-host-runner.json").write_text(json.dumps(status) + "\n")
        try:
            stop_fixture_units(ROOT, account)
        finally:
            checked(["/usr/bin/systemctl", "stop", f"user@{account.pw_uid}.service"])


def main():
    operation = sys.argv[1]
    if os.geteuid() != 0:
        raise PermissionError("SSH fixture setup requires root")
    if operation == "client-key":
        print(json.dumps(client_key()))
    elif operation == "pin":
        pin(json.load(sys.stdin))
    elif operation == "run":
        run(sys.argv[2])
    elif operation == "scan":
        report = export_status([Path("/opt/rustward-acceptance"), ROOT])
        print(json.dumps(report))
        return 0 if report["status"] == "passed" else 1
    else:
        raise ValueError("unknown SSH fixture operation")
    return 0

"""Prepare separate A05 stores and preserve their external review blocks."""
import json
from pathlib import Path
import pwd

from inside import checked, user_command, wait_manager, stop_fixture_units
from prepare import prepare_shared
from ssh_inside import ROOT as SOURCE, TOOLS, probe, check_commands
from stopping_runner import ROOTS


def run(kind, revision):
    if kind not in ROOTS:
        raise ValueError("unknown stopping case")
    root = Path(ROOTS[kind])
    account = pwd.getpwnam("rustward")
    fixture = prepare_shared(root, SOURCE, "rustward", revision)
    evidence = root / "evidence"
    status = {"status": "failed", "case": f"A05-{kind}", "writers_stopped": False}
    try:
        checked(["/usr/bin/systemctl", "start", f"user@{account.pw_uid}.service"], timeout=30)
        checked(user_command(account, ["/usr/bin/systemctl", "--user", "start", "dbus.socket"]), timeout=30)
        wait_manager(account, TOOLS, evidence)
        runtime = SOURCE / "runtimes" / fixture["runtime_id"]
        probe(account, runtime, evidence, kind)
        check_commands(account, runtime, evidence, kind)
        checked(user_command(account, [TOOLS / "acceptance", "run-stop", root, kind]), timeout=210)
        status["status"] = "passed"
    finally:
        try:
            try:
                stop_fixture_units(root, account)
            finally:
                checked(["/usr/bin/systemctl", "stop", f"user@{account.pw_uid}.service"], timeout=30)
            status["writers_stopped"] = True
        except BaseException:
            status["status"] = "failed"
            raise
        finally:
            with (evidence / f"A05-{kind}-host.json").open("x") as output:
                json.dump(status, output)


def confirm_stopped():
    for kind, path in ROOTS.items():
        root = Path(path)
        if not root.exists():
            continue
        report = json.loads((root / "evidence" / f"A05-{kind}-host.json").read_text())
        if report.get("writers_stopped") is not True:
            raise RuntimeError(f"stopping fixture writers are not confirmed stopped: {kind}")


def complete():
    confirm_stopped()
    for kind, path in ROOTS.items():
        names = [f"A05-{kind}", f"A05-{kind}-host", "host-key-check", "ssh-command-check"]
        if kind == "cancel":
            names.append("A05-queued")
        for name in names:
            report = json.loads((Path(path) / "evidence" / f"{name}.json").read_text())
            if report.get("status") != "passed":
                raise RuntimeError(f"stopping case has no pass evidence: {name}")
    with (SOURCE / "evidence/A05.json").open("x") as output:
        json.dump({"status": "passed", "case": "A05", "subcases": ["A05-queued", "A05-cancel", "A05-timeout"]}, output)

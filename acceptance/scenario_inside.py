"""Prepare independent stores and stop all local writers before export."""
import json
from pathlib import Path
import pwd
from inside import checked, user_command, wait_manager, stop_fixture_units
from prepare import prepare_shared
from ssh_inside import ROOT as SOURCE, TOOLS, probe, check_commands
from remaining_runner import CASES


def run(case, revision):
    settings = CASES[case]
    root = Path(settings["root"])
    account = pwd.getpwnam("rustward")
    fixture = prepare_shared(root, SOURCE, "rustward", revision)
    evidence = root / "evidence"
    status = {"status":"failed", "case":case, "writers_stopped":False}
    try:
        checked(["/usr/bin/systemctl", "start", f"user@{account.pw_uid}.service"], timeout=30)
        checked(user_command(account, ["/usr/bin/systemctl", "--user", "start", "dbus.socket"]), timeout=30)
        wait_manager(account, TOOLS, evidence)
        runtime = SOURCE / "runtimes" / fixture["runtime_id"]
        probe(account, runtime, evidence, settings["mode"])
        check_commands(account, runtime, evidence, settings["mode"])
        checked(user_command(account, [TOOLS / "acceptance", "run-scenario", root, case]), timeout=240)
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
            with (evidence / f"{case}-host.json").open("x") as output:
                json.dump(status, output)


def confirm_stopped():
    for case, settings in CASES.items():
        root = Path(settings["root"])
        if not root.exists(): continue
        report = json.loads((root / "evidence" / f"{case}-host.json").read_text())
        if report.get("writers_stopped") is not True:
            raise RuntimeError(f"case writers are not confirmed stopped: {case}")

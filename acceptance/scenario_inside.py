"""Prepare independent stores and stop all local writers before export."""
import json
from pathlib import Path
import pwd
import os
from inside import checked, user_command, wait_manager, stop_fixture_units
from prepare import prepare_shared
from ssh_inside import ROOT as SOURCE, TOOLS, probe, check_commands
from remaining_runner import CASES


def run(case, revision):
    settings = CASES[case]
    root = Path(settings["root"])
    account = pwd.getpwnam("rustward")
    if case == "A08":
        from variant_prepare import prepare
        fixture = prepare(root, SOURCE, TOOLS, revision)
    else:
        fixture = prepare_shared(root, SOURCE, "rustward", revision)
    if case == "A09":
        fixture["capture_limit"] = 65536
        (root / "fixture.json").write_text(json.dumps(fixture, indent=2) + "\n")
    if case == "A10":
        prepare_storage(root, account)
    evidence = root / "evidence"
    status = {"status":"failed", "case":case, "writers_stopped":False}
    try:
        checked(["/usr/bin/systemctl", "start", f"user@{account.pw_uid}.service"], timeout=30)
        checked(user_command(account, ["/usr/bin/systemctl", "--user", "start", "dbus.socket"]), timeout=30)
        wait_manager(account, TOOLS, evidence)
        runtime = (root if case == "A08" else SOURCE) / "runtimes" / fixture["runtime_id"]
        probe(account, runtime, evidence, settings["mode"])
        check_commands(account, runtime, evidence, settings["mode"])
        checked(user_command(account, [TOOLS / "acceptance", "run-scenario", root, case]), timeout=240)
        if case == "A10":
            capacity = os.statvfs(root / "logs")
            status["log_filesystem"] = {"free_bytes":capacity.f_bavail * capacity.f_frsize, "total_bytes":capacity.f_blocks * capacity.f_frsize}
            if status["log_filesystem"]["free_bytes"] != 0:
                raise RuntimeError("log filesystem was not exhausted")
        if case == "A08":
            checked(user_command(account, [TOOLS / "acceptance", "runtime-queue-missing", root]), timeout=30)
            removed = root / "removed-runtimes"
            removed.mkdir()
            (root / "runtimes" / fixture["runtime_id"]).rename(removed / fixture["runtime_id"])
            checked(user_command(account, [TOOLS / "acceptance", "runtime-missing", root]), timeout=120)
            selection = json.loads((evidence / "A08-selection.json").read_text())
            missing = json.loads((evidence / "A08-missing.json").read_text())
            if selection.get("status") != "passed" or missing.get("status") != "passed":
                raise RuntimeError("runtime selection evidence is incomplete")
            with (evidence / "A08.json").open("x") as output:
                json.dump({"case":"A08","status":"passed","run_id":selection["run_id"],"selection":selection,"missing":missing}, output)
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


def prepare_storage(root, account):
    checked(["/usr/bin/mount", "-t", "tmpfs", "-o",
             f"size=1048576,uid={account.pw_uid},gid={account.pw_gid},mode=0700,nosuid,nodev",
             "tmpfs", root / "logs"], timeout=15)
    with (root / "logs/.ballast").open("xb") as output:
        output.write(b"\0" * 786432)
        output.flush()
        os.fsync(output.fileno())


def confirm_stopped():
    for case, settings in CASES.items():
        root = Path(settings["root"])
        if not root.exists(): continue
        report = json.loads((root / "evidence" / f"{case}-host.json").read_text())
        if report.get("writers_stopped") is not True:
            raise RuntimeError(f"case writers are not confirmed stopped: {case}")

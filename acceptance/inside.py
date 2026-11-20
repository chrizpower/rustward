"""Run prepared acceptance tools on an isolated Debian 13 systemd host."""
import argparse
import json
import os
from pathlib import Path
import pwd
import sqlite3
import signal
import subprocess
import sys
import time

from prepare import prepare
import service_case
import web_case


def invoke(command, timeout=240, capture=False):
    process = subprocess.Popen(command, text=True, start_new_session=True,
                               stdout=subprocess.PIPE if capture else None,
                               stderr=subprocess.PIPE if capture else None)
    try:
        output, error = process.communicate(timeout=timeout)
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.communicate()
        raise
    # A crashed harness can leave controller children outside the user manager.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    return subprocess.CompletedProcess(command, process.returncode, output, error)


def checked(command, capture_output=False, timeout=240):
    result = invoke(command, capture=capture_output, timeout=timeout)
    result.check_returncode()
    return result


def user_command(account, command, extra=None):
    environment = {"PATH": "/usr/bin:/bin", "HOME": account.pw_dir,
                   "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC",
                   "XDG_RUNTIME_DIR": f"/run/user/{account.pw_uid}",
                   "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{account.pw_uid}/bus",
                   "SYNTHETIC_INHERITED": "must-not-reach-the-task"}
    environment.update(extra or {})
    return ["/usr/bin/setpriv", f"--reuid={account.pw_uid}", f"--regid={account.pw_gid}",
            "--clear-groups", "--bounding-set=-all", "--inh-caps=-all",
            "--ambient-caps=-all", "--no-new-privs", "/usr/bin/env", "-i",
            *[f"{key}={value}" for key, value in environment.items()], *map(str, command)]


def wait_manager(account, tools, evidence, timeout=30, interval=0.1):
    deadline = time.monotonic() + timeout
    report = {"status": "waiting", "attempts": []}
    path = evidence / "manager-readiness.json"
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            result = invoke(user_command(account, [tools / "acceptance", "probe-manager"]),
                            timeout=min(10, remaining), capture=True)
            attempt = {"returncode": result.returncode, "stdout": result.stdout,
                       "stderr": result.stderr}
        except subprocess.TimeoutExpired:
            attempt = {"status": "timeout"}
            result = None
        report["attempts"].append(attempt)
        path.write_text(json.dumps(report, indent=2) + "\n")
        if result is not None and result.returncode == 0:
            value = json.loads(result.stdout)
            if value.get("status") != "ready" or value.get("uid") != account.pw_uid or str(value.get("version", "")).split(".")[0] != "257":
                report["status"] = "failed"
                path.write_text(json.dumps(report, indent=2) + "\n")
                raise RuntimeError("invalid manager readiness evidence")
            report.update(status="ready", manager=value)
            path.write_text(json.dumps(report, indent=2) + "\n")
            return
        time.sleep(min(interval, max(0, deadline - time.monotonic())))
    report["status"] = "failed"
    path.write_text(json.dumps(report, indent=2) + "\n")
    raise RuntimeError("user manager readiness deadline; see manager-readiness.json")


def stop_fixture_units(root, account):
    database = root / "state/state.db"
    if not database.exists():
        return
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        ids = [row[0] for row in connection.execute("SELECT run_id FROM launches")]
    for identity in ids:
        if len(identity) != 64 or any(char not in "0123456789abcdef" for char in identity):
            raise ValueError("invalid retained fixture run ID")
        command = ["/usr/bin/systemctl", "--user", "stop", "--no-block",
                   f"rustward-run-{identity}.service"]
        subprocess.run(user_command(account, command), check=False, timeout=15)


def run(root, tools, user, revision):
    if os.geteuid() != 0:
        raise PermissionError("host setup must run as root; tests use the execution user")
    if Path("/proc/1/comm").read_text().strip() != "systemd":
        raise RuntimeError("PID 1 is not systemd")
    version = checked(["/usr/bin/systemctl", "--version"], capture_output=True).stdout
    if not version.startswith("systemd 257 "):
        raise RuntimeError("systemd 257 is required")
    if not Path("/sys/fs/cgroup/cgroup.controllers").is_file():
        raise RuntimeError("cgroup v2 is required")
    if os.statvfs("/sys/fs/cgroup").f_flag & os.ST_RDONLY:
        raise RuntimeError("the delegated cgroup mount is read-only")
    account = pwd.getpwnam(user)
    if account.pw_uid == 0:
        raise ValueError("execution user must be non-root")
    fixture = prepare(root, tools, user, revision)
    evidence = root / "evidence"
    status = {"status": "failed", "phase": "user-manager"}
    try:
        checked(["/usr/bin/systemctl", "start", f"user@{account.pw_uid}.service"])
        checked(user_command(account, ["/usr/bin/systemctl", "--user", "start", "dbus.socket"]))
        status["phase"] = "manager-readiness"
        wait_manager(account, tools, evidence)
        status["phase"] = "legacy-host-tests"
        for name, count in (("systemd_host", 2), ("runtime_host", 1)):
            extra = {"RUSTWARD_PREPARATION_UID": "0",
                     "RUSTWARD_RUNTIME_CATALOG": str(root / "runtimes"),
                     "RUSTWARD_RUNTIME_ID": fixture["runtime_id"],
                     "RUSTWARD_RUNTIME_PROGRAM": str(root / "runtimes" / fixture["runtime_id"] / "bin/python3"),
                     "RUSTWARD_RUNTIME_CWD": str(root / "runtimes" / fixture["runtime_id"]),
                     "RUSTWARD_RUNTIME_WORK_ROOT": str(root / "work")}
            result = invoke(user_command(account, [tools / name, "--ignored",
                            "--test-threads=1"], extra), timeout=120, capture=True)
            (evidence / f"{name}.stdout").write_text(result.stdout)
            (evidence / f"{name}.stderr").write_text(result.stderr)
            if result.returncode or f"test result: ok. {count} passed;" not in result.stdout:
                raise RuntimeError(f"{name} did not pass its expected test count")
        (evidence / "legacy-host.json").write_text(json.dumps({"status": "passed", "tests": 3}) + "\n")
        status["phase"] = "controller-acceptance"
        checked(user_command(account, [tools / "acceptance", "run", root]), timeout=480)
        result = json.loads((evidence / "result.json").read_text())
        if result.get("status") != "passed" or result.get("cases") != ["A01", "A02"]:
            raise RuntimeError("controller acceptance did not produce its pass evidence")
        status["phase"] = "daemon-acceptance"
        service_case.run(root, tools, account, fixture, user_command)
        status["phase"] = "scheduler-acceptance"
        service_case.run_schedules(root, tools, account, fixture, user_command)
        status["phase"] = "web-acceptance"
        web_case.run(root, tools, account, fixture, user_command)
        status = {"status": "passed", "phase": "complete"}
    finally:
        (evidence / "host-runner.json").write_text(json.dumps(status, sort_keys=True) + "\n")
        try:
            stop_fixture_units(root, account)
            stop_fixture_units(root / "daemon", account)
            stop_fixture_units(root / "scheduled", account)
            stop_fixture_units(root / "web", account)
        finally:
            checked(["/usr/bin/systemctl", "stop", f"user@{account.pw_uid}.service"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--tools", required=True, type=Path)
    parser.add_argument("--user", default="rustward")
    parser.add_argument("--source-revision", required=True)
    options = parser.parse_args()
    try:
        run(options.root, options.tools, options.user, options.source_revision)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Host acceptance failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

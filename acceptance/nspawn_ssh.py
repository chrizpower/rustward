"""Run the shared SSH cases inside the private nspawn network."""
import json
import os
from pathlib import Path
import shutil
import subprocess

from remaining_runner import CASES
from ssh_runner import FIXTURE, Remote
from stopping_runner import ROOTS

SOURCE = Path(__file__).resolve().parent
PYTHON = "/usr/bin/python3"
TARGET_USER = "rustward-target"
HOME = Path("/home") / TARGET_USER
TARGET_STATE = Path("/run/rustward-target")


class GuestRemote(Remote):
    def __init__(self, evidence):
        self.name = "nspawn"
        self.evidence = evidence
        self.target = self.name + "-target"
        self.public_key = None
        self.targets = {}
        self.addresses = {
            self.target: "127.0.0.2",
            self.name + "-stopping-target": "127.0.0.3",
            self.name + "-remaining-target": "127.0.0.4",
        }

    def call(self, command, timeout=120, checked=True, input_data=None):
        result = subprocess.run(command, input=input_data, capture_output=True, timeout=timeout)
        if checked and result.returncode:
            error = result.stderr.decode(errors="replace")
            raise RuntimeError(f"{command[-1]} failed: {error.strip()}")
        return result.returncode, result.stdout.decode()

    def inside(self, operation, *arguments, **keywords):
        return self.call([PYTHON, "-B", SOURCE / "ssh_inside.py", operation, *arguments], **keywords)

    def target_address(self, target):
        return self.addresses[target]

    def start_target(self, target):
        paths = Path("/run") / target
        paths.mkdir(mode=0o700)
        home, state = paths / "home", paths / "state"
        home.mkdir(mode=0o700)
        state.mkdir(mode=0o755)
        owner = HOME.stat()
        os.chown(home, owner.st_uid, owner.st_gid)
        with (paths / "sshd.log").open("wb") as log:
            self.targets[target] = subprocess.Popen([
                "/usr/bin/unshare", "--mount", "--pid", "--fork", "--kill-child", "--mount-proc",
                "/bin/sh", "-ec", """
                mount --bind "$1" "$2"
                mount --bind "$3" "$4"
                shift 4
                exec "$@"
                """, "target", home, HOME, state, TARGET_STATE,
                PYTHON, "-I", "-B", SOURCE / "ssh_target.py", "serve", self.target_address(target),
            ], stdout=log, stderr=log)

    def target_call(self, target, operation, *arguments, **keywords):
        process = self.targets[target]
        if process.poll() is None:
            children = Path(f"/proc/{process.pid}/task/{process.pid}/children")
            pids = children.read_text().split() if children.exists() else []
            if pids:
                return self.call(["/usr/bin/nsenter", "--target", pids[0], "--mount", "--pid",
                                  PYTHON, "-I", "-B", SOURCE / "ssh_target.py", operation, *arguments],
                                 **keywords)
        if keywords.get("checked", True):
            raise RuntimeError(f"SSH target is unavailable: {target}")
        return 1, ""

    def stop_target(self):
        self.stop(self.target)
        (self.evidence / "target-stopped.json").write_text(json.dumps({"status": "passed"}) + "\n")

    def stop(self, target):
        process = self.targets[target]
        if process.poll() is None:
            process.kill()
        process.wait(timeout=10)
        if Path("/run", target, "sshd.log").stat().st_size:
            shutil.copyfile(Path("/run", target, "sshd.log"), self.evidence / f"{target}.log")

    def cleanup(self):
        for target in self.targets:
            self.stop(target)


def run(evidence, revision):
    evidence.mkdir()
    subprocess.run(["/usr/sbin/useradd", "--create-home", "--shell", "/bin/sh",
                    "--password", "", TARGET_USER], check=True)
    TARGET_STATE.mkdir()
    for name in ("hold", "flood"):
        Path(f"/opt/rustward_{name}.py").symlink_to(SOURCE / "fixtures" / f"{name}.py")
    remote = GuestRemote(evidence)
    remote.prepare_credentials()
    try:
        remote.run_cases(revision)
    finally:
        remote.cleanup()
        remote.scan()
        roots = {"ssh-fixture": FIXTURE}
        roots.update({f"stopping-{kind}-fixture": path for kind, path in ROOTS.items()})
        roots.update({f"remaining-{case}-fixture": settings["root"] for case, settings in CASES.items()})
        for name, path in roots.items():
            if Path(path).exists():
                Path(path).rename(evidence / name)

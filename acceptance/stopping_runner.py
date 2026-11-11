"""Own the fresh A05 SSH target and collect independent remote evidence."""
import json
from pathlib import Path
import time

ROOTS = {"cancel": "/opt/rustward-a05-cancel", "timeout": "/opt/rustward-a05-timeout"}


def expected_inputs():
    return {"text": 'He said "hello"; it\'s a test', "count": 7, "enabled": True}


class Stopping:
    def __init__(self, remote):
        self.remote = remote
        self.target = remote.name + "-stopping-target"
        self.created = False
        self.started = []

    def target_call(self, operation, *arguments, **keywords):
        return self.remote.docker.call(["exec", "-i", self.target, "/usr/bin/python3", "-I", "-B",
                                       "/opt/ssh_target.py", operation, *arguments], **keywords)

    def observe(self, kind, label):
        _, output = self.target_call("hold-state", kind)
        value = json.loads(output)
        if value.get("kind") != kind:
            raise RuntimeError("remote stopping marker belongs to another case")
        if kind == "queued":
            valid = value.get("state") == "absent"
        else:
            valid = value.get("state") in ("alive", "ended") and json.dumps(value.get("inputs"), sort_keys=True) == json.dumps(expected_inputs(), sort_keys=True)
            run_id = value.get("run_id", "")
            valid = valid and type(run_id) is str and len(run_id) == 64 and all(char in "0123456789abcdef" for char in run_id)
            valid = valid and type(value.get("pid")) is int and value["pid"] > 0 and type(value.get("start_time")) is str and bool(value["start_time"])
        if not valid:
            raise RuntimeError("remote stopping evidence is missing or invalid")
        report = {"status": "passed", "remote": value, "scope": "Marker presence only" if kind == "queued" else "Remote state after local stopping; no acknowledgement"}
        (self.remote.evidence / f"stopping-{label}-target.json").write_text(json.dumps(report, indent=2) + "\n")

    def run(self, revision):
        self.created = True
        docker = self.remote.docker
        docker.call(["run", "--detach", "--pull=never", "--name", self.target,
                     "--network", self.remote.network, "--tmpfs", "/run:rw,nosuid,nodev,mode=755", self.remote.target_image])
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            code, output = self.target_call("metadata", checked=False)
            if code == 0:
                metadata = json.loads(output)
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("stopping target metadata deadline")
        _, output = docker.call(["inspect", self.target])
        metadata["address"] = json.loads(output)[0]["NetworkSettings"]["Networks"][self.remote.network]["IPAddress"]
        (self.remote.evidence / "stopping-target-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        self.target_call("authorize", input_data=self.remote.public_key.encode())
        self.remote.inside("pin-stopping", input_data=json.dumps(metadata).encode())
        self.observe("queued", "queued-before")
        for kind in ROOTS:
            self.started.append(kind)
            self.remote.inside("stopping", kind, revision, timeout=600)
            self.observe(kind, kind)
        self.observe("queued", "queued-after")
        self.remote.inside("complete-stopping")

    def copy(self):
        copied = True
        for kind in self.started:
            code, _ = self.remote.docker.call(["cp", f"{self.remote.name}:{ROOTS[kind]}/.",
                                               self.remote.evidence / f"stopping-{kind}-fixture"], checked=False)
            copied &= code == 0
        return copied

    def cleanup(self):
        if self.created:
            self.remote.docker.call(["logs", self.target], checked=False)
            self.remote.docker.call(["inspect", self.target], checked=False)
            self.remote.docker.call(["rm", "--force", self.target])


def status(evidence):
    aggregate = evidence / "ssh-fixture/evidence/A05.json"
    if aggregate.exists():
        return json.loads(aggregate.read_text()).get("status", "incomplete")
    result = "not run"
    for kind in ROOTS:
        root = evidence / f"stopping-{kind}-fixture/evidence"
        names = [f"A05-{kind}", f"A05-{kind}-host"] + (["A05-queued"] if kind == "cancel" else [])
        for name in names:
            report = root / f"{name}.json"
            if report.exists():
                if json.loads(report.read_text()).get("status") == "failed":
                    return "failed"
                result = "incomplete"
            elif (root / f"{name}-admission.json").exists():
                result = "incomplete"
    return result


def validate(evidence):
    for kind in ROOTS:
        root = evidence / f"stopping-{kind}-fixture"
        names = [f"A05-{kind}", f"A05-{kind}-host", "host-key-check", "ssh-command-check"]
        if kind == "cancel":
            names.append("A05-queued")
        for name in names:
            if json.loads((root / "evidence" / f"{name}.json").read_text()).get("status") != "passed":
                raise RuntimeError(f"missing stopping pass evidence: {name}")
    for kind in ROOTS:
        local = json.loads((evidence / f"stopping-{kind}-fixture/evidence/A05-{kind}.json").read_text())
        target = json.loads((evidence / f"stopping-{kind}-target.json").read_text())
        if target.get("remote", {}).get("run_id") != local.get("run_id"):
            raise RuntimeError("remote stopping marker belongs to another admitted run")
    for name in ("cancel", "timeout", "queued-before", "queued-after"):
        if json.loads((evidence / f"stopping-{name}-target.json").read_text()).get("status") != "passed":
            raise RuntimeError(f"missing independent stopping evidence: {name}")
    if status(evidence) != "passed":
        raise RuntimeError("A05 has no complete pass evidence")

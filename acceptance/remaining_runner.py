"""Run the remaining isolated acceptance cases on a fresh SSH target."""
import json
from pathlib import Path
import time
from stopping_runner import Stopping, expected_inputs

CASES = {"A06": {"root":"/opt/rustward-a06", "mode":"recovery", "kind":"recovery"},
         "A07": {"root":"/opt/rustward-a07", "mode":"async", "kind":"async"},
         "A08": {"root":"/opt/rustward-a08", "mode":"runtime-a", "kind":"runtime-a"}}


class Remaining(Stopping):
    def __init__(self, remote):
        super().__init__(remote)
        self.target = remote.name + "-remaining-target"

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
            raise RuntimeError("remaining target metadata deadline")
        _, output = docker.call(["inspect", self.target])
        metadata["address"] = json.loads(output)[0]["NetworkSettings"]["Networks"][self.remote.network]["IPAddress"]
        (self.remote.evidence / "remaining-target-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        self.target_call("authorize", input_data=self.remote.public_key.encode())
        self.remote.inside("pin-remaining", input_data=json.dumps(metadata).encode())
        for case, settings in CASES.items():
            self.started.append(case)
            self.remote.inside("scenario", case, revision, timeout=600)
            _, output = self.remote.inside("scenario-report", case)
            report = json.loads(output)
            if report.get("status") != "passed":
                raise RuntimeError(f"scenario has no pass decision: {case}")
            self.observe_case(case, settings, report)

    def observe_case(self, case, settings, report):
        _, output = self.target_call("runtime-result") if case == "A08" else self.target_call("hold-state", settings["kind"])
        value = json.loads(output)
        valid = (value.get("runtime") == "B" if case == "A08" else value.get("state") in ("alive", "ended")) and value.get("run_id") == report.get("run_id")
        if case == "A07":
            valid &= value.get("state") == "alive"
        valid &= json.dumps(value.get("inputs"), sort_keys=True) == json.dumps(expected_inputs(), sort_keys=True)
        if not valid:
            raise RuntimeError(f"independent remote evidence differs: {case}")
        (self.remote.evidence / f"remaining-{case}-target.json").write_text(json.dumps({"status":"passed","remote":value}, indent=2) + "\n")

    def copy(self):
        copied = True
        for case in self.started:
            code, _ = self.remote.docker.call(["cp", f"{self.remote.name}:{CASES[case]['root']}/.",
                                               self.remote.evidence / f"remaining-{case}-fixture"], checked=False)
            copied &= code == 0
        return copied


def status(evidence, case):
    root = evidence / f"remaining-{case}-fixture/evidence"
    report = root / f"{case}.json"
    host = root / f"{case}-host.json"
    if host.exists() and json.loads(host.read_text()).get("status") == "failed":
        return "failed"
    if report.exists():
        value = json.loads(report.read_text()).get("status", "incomplete")
        return value if host.exists() else "incomplete"
    return "incomplete" if (root / f"{case}-admission.json").exists() else "not run"


def validate(evidence):
    for case in CASES:
        root = evidence / f"remaining-{case}-fixture/evidence"
        for name in (case, case + "-host", "host-key-check", "ssh-command-check"):
            report = json.loads((root / f"{name}.json").read_text())
            if report.get("status") != "passed" or (name.endswith("-host") and report.get("writers_stopped") is not True):
                raise RuntimeError(f"missing remaining-case evidence: {name}")
        native = json.loads((root / f"{case}.json").read_text())
        target = json.loads((evidence / f"remaining-{case}-target.json").read_text())
        if target.get("status") != "passed" or target.get("remote", {}).get("run_id") != native.get("run_id"):
            raise RuntimeError(f"remote run identity differs: {case}")

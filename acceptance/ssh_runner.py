"""Own the isolated SSH target and its public evidence."""
import json
import time

FIXTURE = "/opt/rustward-ssh-acceptance"


class Remote:
    def __init__(self, docker, name, evidence):
        self.docker, self.name, self.evidence = docker, name, evidence
        self.target, self.network = name + "-target", name + "-network"
        self.target_created = self.network_created = False
        self.public_key = None
        self.fixture_started = False
        self.stopping = None
        self.target_image = None

    def create_network(self):
        self.network_created = True
        self.docker.call(["network", "create", "--internal", self.network])

    def prepare_credentials(self):
        if self.public_key is not None:
            raise RuntimeError("fixture credentials are already prepared")
        _, self.public_key = self.inside("client-key")

    def inside(self, operation, *arguments, **keywords):
        return self.docker.call(["exec", "-i", self.name, "/usr/bin/python3", "-I", "-B", "-c",
            "import sys;sys.path.insert(0,'/opt/rustward-acceptance-source');"
            "from ssh_inside import main;sys.exit(main())", operation, *arguments], **keywords)

    def target_call(self, operation, **keywords):
        return self.docker.call(["exec", "-i", self.target, "/usr/bin/python3", "-I", "-B",
                                 "/opt/ssh_target.py", operation], **keywords)

    def run(self, context, debian_image, revision):
        if self.public_key is None:
            raise RuntimeError("prepare fixture credentials before running workloads")
        image_file = self.evidence / "target-image-id"
        self.docker.call(["build", "--pull", "--target", "ssh-target", "--iidfile", image_file,
            "--file", "acceptance/Dockerfile", "--build-arg", f"DEBIAN_IMAGE={debian_image}",
            context], timeout=900)
        image = image_file.read_text().strip()
        if not image.startswith("sha256:") or len(image) != 71:
            raise RuntimeError("SSH target build has no fixed image ID")
        if not self.network_created:
            raise RuntimeError("create the internal network before starting the controller")
        self.target_image = image
        self.target_created = True
        self.docker.call(["run", "--detach", "--pull=never", "--name", self.target,
            "--network", self.network, "--tmpfs", "/run:rw,nosuid,nodev,mode=755", image])
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            code, output = self.target_call("metadata", checked=False)
            if code == 0:
                metadata = json.loads(output)
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("SSH target metadata deadline")
        _, output = self.docker.call(["inspect", self.target])
        metadata["address"] = json.loads(output)[0]["NetworkSettings"]["Networks"][self.network]["IPAddress"]
        (self.evidence / "target-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        self.target_call("authorize", input_data=self.public_key.encode())
        self.inside("pin", input_data=json.dumps(metadata).encode())
        self.fixture_started = True
        self.inside("run", revision, timeout=480)
        _, output = self.target_call("result")
        expected = {"text": 'He said "hello"; it\'s a test', "count": 7, "enabled": True}
        if json.dumps(json.loads(output), sort_keys=True) != json.dumps(expected, sort_keys=True):
            raise RuntimeError("independent SSH target result differs")
        (self.evidence / "target-check.json").write_text(json.dumps({"status": "passed", "value": expected}) + "\n")
        self.inside("failure", "task", timeout=240)
        _, output = self.target_call("failure-result")
        if json.dumps(json.loads(output), sort_keys=True) != json.dumps(expected, sort_keys=True):
            raise RuntimeError("independent remote failure proof differs")
        (self.evidence / "failure-target-check.json").write_text(json.dumps({"status": "passed", "value": expected}) + "\n")
        self.stop_target()
        self.inside("failure", "unreachable", timeout=240)
        self.inside("complete-failures")
        from stopping_runner import Stopping
        self.stopping = Stopping(self)
        self.stopping.run(revision)

    def stop_target(self):
        self.docker.call(["stop", "--time", "10", self.target], timeout=30)
        _, output = self.docker.call(["inspect", "--format", "{{json .State}}", self.target])
        state = json.loads(output)
        if state.get("Running") is not False or state.get("Pid") != 0:
            raise RuntimeError("SSH target did not stop before the unreachable case")
        (self.evidence / "target-stopped.json").write_text(json.dumps({"status": "passed", "state": state}) + "\n")

    def scan(self):
        code, output = self.inside("scan", checked=False)
        try:
            report = json.loads(output)
        except ValueError:
            report = {"status": "failed", "reason": "credential scan returned no valid report"}
        (self.evidence / "credential-scan.json").write_text(json.dumps(report, indent=2) + "\n")
        if code or report.get("status") != "passed":
            raise RuntimeError("credential export check failed; raw fixture export refused")

    def cleanup(self):
        if self.stopping is not None:
            self.stopping.cleanup()
        if self.target_created:
            self.docker.call(["rm", "--force", self.target])
        if self.network_created:
            # The stopped controller can be retained when evidence copy fails.
            self.docker.call(["network", "disconnect", "--force", self.network, self.name], checked=False)
            self.docker.call(["network", "rm", self.network])


def validate(fixture, evidence):
    for name in ("A03", "A04", "A05", "A04-task-failure", "A04-unreachable", "A04-task-failure-host", "A04-unreachable-host", "host-key-check", "ssh-command-check", "ssh-host-runner", "result"):
        value = json.loads((fixture / "evidence" / f"{name}.json").read_text())
        if value.get("status") != "passed":
            raise RuntimeError(f"missing SSH pass evidence: {name}")
    for name in ("credential-scan", "target-check", "failure-target-check", "target-stopped"):
        if json.loads((evidence / f"{name}.json").read_text()).get("status") != "passed":
            raise RuntimeError(f"missing SSH pass evidence: {name}")
    result = json.loads((fixture / "evidence/result.json").read_text())
    if result.get("cases") != ["A03"]:
        raise RuntimeError("SSH result has no A03 case")
    from stopping_runner import validate as validate_stopping
    validate_stopping(evidence)


def a04_status(evidence):
    report = evidence / "A04.json"
    if report.exists():
        return json.loads(report.read_text()).get("status", "incomplete")
    status = "not run"
    for case in ("A04-task-failure", "A04-unreachable"):
        report = evidence / f"{case}.json"
        if report.exists():
            if json.loads(report.read_text()).get("status") == "failed":
                return "failed"
            status = "incomplete"
        elif (evidence / f"{case}-admission.json").exists():
            status = "incomplete"
    return status

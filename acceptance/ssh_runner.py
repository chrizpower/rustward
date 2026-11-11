"""Own the isolated SSH target and its public evidence."""
import json
import time

FIXTURE = "/opt/rustward-ssh-acceptance"


class Remote:
    def __init__(self, docker, name, evidence):
        self.docker, self.name, self.evidence = docker, name, evidence
        self.target, self.network = name + "-target", name + "-network"
        self.target_created = self.network_created = False

    def inside(self, operation, *arguments, **keywords):
        return self.docker.call(["exec", "-i", self.name, "/usr/bin/python3", "-I", "-B", "-c",
            "import sys;sys.path.insert(0,'/opt/rustward-acceptance-source');"
            "from ssh_inside import main;sys.exit(main())", operation, *arguments], **keywords)

    def target_call(self, operation, **keywords):
        return self.docker.call(["exec", "-i", self.target, "/usr/bin/python3", "-I", "-B",
                                 "/opt/ssh_target.py", operation], **keywords)

    def run(self, context, debian_image, revision):
        image_file = self.evidence / "target-image-id"
        self.docker.call(["build", "--pull", "--target", "ssh-target", "--iidfile", image_file,
            "--file", "acceptance/Dockerfile", "--build-arg", f"DEBIAN_IMAGE={debian_image}",
            context], timeout=900)
        image = image_file.read_text().strip()
        if not image.startswith("sha256:") or len(image) != 71:
            raise RuntimeError("SSH target build has no fixed image ID")
        self.network_created = True
        self.docker.call(["network", "create", "--internal", self.network])
        self.target_created = True
        self.docker.call(["run", "--detach", "--pull=never", "--name", self.target,
            "--network", self.network, "--tmpfs", "/run:rw,nosuid,nodev,mode=755", image])
        self.docker.call(["network", "connect", self.network, self.name])
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
        _, public = self.inside("client-key")
        self.target_call("authorize", input_data=public.encode())
        self.inside("pin", input_data=json.dumps(metadata).encode())
        self.inside("run", revision, timeout=480)
        _, output = self.target_call("result")
        expected = {"text": 'He said "hello"; it\'s a test', "count": 7, "enabled": True}
        if json.dumps(json.loads(output), sort_keys=True) != json.dumps(expected, sort_keys=True):
            raise RuntimeError("independent SSH target result differs")
        (self.evidence / "target-check.json").write_text(json.dumps({"status": "passed", "value": expected}) + "\n")

    def scan(self):
        code, output = self.inside("scan", checked=False)
        report = json.loads(output)
        (self.evidence / "credential-scan.json").write_text(json.dumps(report, indent=2) + "\n")
        if code or report.get("status") != "passed":
            raise RuntimeError("credential export check failed; raw fixture export refused")

    def cleanup(self):
        if self.target_created:
            self.docker.call(["rm", "--force", self.target])
        if self.network_created:
            # The stopped controller can be retained when evidence copy fails.
            self.docker.call(["network", "disconnect", "--force", self.network, self.name], checked=False)
            self.docker.call(["network", "rm", self.network])


def validate(fixture, evidence):
    for name in ("A03", "host-key-check", "ssh-host-runner", "result"):
        value = json.loads((fixture / "evidence" / f"{name}.json").read_text())
        if value.get("status") != "passed":
            raise RuntimeError(f"missing SSH pass evidence: {name}")
    for name in ("credential-scan", "target-check"):
        if json.loads((evidence / f"{name}.json").read_text()).get("status") != "passed":
            raise RuntimeError(f"missing SSH pass evidence: {name}")
    result = json.loads((fixture / "evidence/result.json").read_text())
    if result.get("cases") != ["A03"]:
        raise RuntimeError("SSH result has no A03 case")

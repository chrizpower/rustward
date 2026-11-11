"""Build and run synthetic acceptance on an isolated Docker host."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "acceptance"))
INSIDE = "/opt/rustward-acceptance-source"
TOOLS = "/opt/rustward-acceptance-tools"
FIXTURE = "/opt/rustward-acceptance"


def excluded(path):
    return path.name == "AI.md" or path.name == "__pycache__" or path.suffix == ".pyc"


def snapshot(repository, destination):
    destination.mkdir()
    files = []

    def copy(path, relative):
        if excluded(path):
            return
        if path.is_symlink():
            raise ValueError("build context source contains a symlink")
        target = destination / relative
        if path.is_dir():
            target.mkdir()
            for entry in sorted(path.iterdir()):
                copy(entry, relative / entry.name)
        elif path.is_file():
            data = path.read_bytes()
            target.write_bytes(data)
            files.append({"path": relative.as_posix(), "sha256": hashlib.sha256(data).hexdigest()})
        else:
            raise ValueError("build context source contains a special file")

    for name in ("Cargo.toml", "Cargo.lock", "src", "acceptance"):
        copy(repository / name, Path(name))
    (destination / "examples").mkdir()
    copy(repository / "examples/acceptance", Path("examples/acceptance"))
    (destination / "tests").mkdir()
    for name in ("systemd_host.rs", "runtime_host.rs"):
        copy(repository / "tests" / name, Path("tests") / name)
    encoded = json.dumps(sorted(files, key=lambda value: value["path"]), sort_keys=True).encode()
    return {"revision": hashlib.sha256(encoded).hexdigest(), "files": files}


def run_arguments(name, image, network="none"):
    return ["run", "--detach", "--name", name, "--pull=never", "--privileged",
            "--cgroupns=private", f"--network={network}", "--tmpfs", "/run:rw,nosuid,nodev,mode=755",
            "--tmpfs", "/run/lock:rw,nosuid,nodev,mode=1777", "--tmpfs", "/tmp:rw,nosuid,nodev,mode=1777",
            "--stop-signal=SIGRTMIN+3", "--stop-timeout=20", image]


class Docker:
    def __init__(self, evidence):
        self.evidence = evidence
        self.sequence = 0

    def call(self, arguments, timeout=120, checked=True, input_data=None):
        self.sequence += 1
        prefix = self.evidence / f"docker-{self.sequence:02}"
        prefix.with_suffix(".command.json").write_text(json.dumps(["docker", *map(str, arguments)]) + "\n")
        with prefix.with_suffix(".stdout").open("wb") as output, prefix.with_suffix(".stderr").open("wb") as error:
            try:
                result = subprocess.run(["docker", *map(str, arguments)], stdout=output,
                                        stderr=error, timeout=timeout, check=False, input=input_data)
            except subprocess.TimeoutExpired:
                prefix.with_suffix(".status.json").write_text('{"status":"timeout"}\n')
                raise
        prefix.with_suffix(".status.json").write_text(json.dumps({"returncode": result.returncode}) + "\n")
        if checked and result.returncode:
            raise RuntimeError(f"Docker step {self.sequence} failed; see {prefix.name} logs")
        return result.returncode, prefix.with_suffix(".stdout").read_text()


def validate_bundle(fixture):
    for name in ("A01", "A02", "local-recovery", "host-runner", "legacy-host"):
        value = json.loads((fixture / "evidence" / f"{name}.json").read_text())
        if value.get("status") != "passed":
            raise RuntimeError(f"missing pass evidence: {name}")
    result = json.loads((fixture / "evidence/result.json").read_text())
    if result.get("status") != "passed" or result.get("cases") != ["A01", "A02"]:
        raise RuntimeError("acceptance bundle has no complete local result")
    metadata = json.loads((fixture / "fixture.json").read_text())
    if not all(metadata.get(key) for key in ("runtime_id", "host_id", "packages", "source_revision")):
        raise RuntimeError("acceptance bundle has no dependency provenance")


def execute(repository, evidence, rust_image, debian_image, remote=False):
    evidence.mkdir(parents=True, exist_ok=False)
    result = {"status": "blocked", "phase": "docker-preflight", "container": None,
              "cases": {"A01": "not run", "A02": "not run"}}
    if remote:
        result["cases"].update(A03="not run", A04="not run", A05="not run")
    (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    if not shutil.which("docker"):
        result["reason"] = "Docker is not installed"
        (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        return 2
    docker = Docker(evidence)
    name = "rustward-acceptance-" + uuid.uuid4().hex
    created = False
    copied = False
    ssh = None
    try:
        _, info = docker.call(["info", "--format", "{{json .}}"])
        details = json.loads(info)
        if details.get("OSType") != "linux" or details.get("CgroupVersion") != "2" or any("rootless" in option for option in details.get("SecurityOptions", [])):
            raise RuntimeError("use an isolated rootful Linux Docker host with cgroup v2")
        result["phase"] = "build"
        source = snapshot(repository, evidence / "context")
        (evidence / "source.json").write_text(json.dumps(source, indent=2) + "\n")
        image_file = evidence / "image-id"
        docker.call(["build", "--pull", "--iidfile", image_file, "--file", "acceptance/Dockerfile",
                     "--build-arg", f"SOURCE_REVISION={source['revision']}",
                     "--build-arg", f"RUST_IMAGE={rust_image}", "--build-arg",
                     f"DEBIAN_IMAGE={debian_image}", evidence / "context"], timeout=2400)
        image = image_file.read_text().strip()
        if not image.startswith("sha256:") or len(image) != 71:
            raise RuntimeError("build did not return a fixed image ID")
        result.update(image=image, source_revision=source["revision"], container=name, phase="boot")
        # The name is unique to this attempt, including when a run reply is lost.
        created = True
        if remote:
            from ssh_runner import Remote
            ssh = Remote(docker, name, evidence)
            ssh.create_network()
        docker.call(run_arguments(name, image, network=ssh.network if ssh else "none"))
        ready = False
        import time
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            code, _ = docker.call(["exec", name, "/usr/bin/systemctl", "is-active", "--quiet", "basic.target"], checked=False)
            if code == 0:
                ready = True
                break
            time.sleep(1)
        if not ready:
            raise RuntimeError("container did not reach basic.target")
        if ssh is not None:
            ssh.prepare_credentials()
        result["phase"] = "host-acceptance"
        code, _ = docker.call(["exec", name, "/usr/bin/python3", "-I", "-B", "-c",
                              f"import sys;sys.path.insert(0,{INSIDE!r});from inside import main;sys.exit(main())",
                              "--root", FIXTURE, "--tools", TOOLS, "--source-revision", source["revision"]], timeout=900, checked=False)
        if code:
            result["reason"] = "local host acceptance failed; see Docker exec logs"
        if code == 0 and remote:
            result["phase"] = "ssh-acceptance"
            ssh.run(evidence / "context", debian_image, source["revision"])
        result["status"] = "passed" if code == 0 else "failed"
        result["phase"] = "complete" if code == 0 else "host-acceptance"
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        result["reason"] = str(error)
        if result["phase"] != "docker-preflight":
            result["status"] = "failed"
    finally:
        if created:
            cleanup_errors = []
            for command in (["logs", name], ["inspect", name],
                            ["exec", name, "/usr/bin/journalctl", "--boot", "--no-pager"]):
                try:
                    docker.call(command, checked=False)
                except (OSError, subprocess.SubprocessError) as error:
                    cleanup_errors.append(str(error))
            try:
                if remote:
                    if ssh is None:
                        raise RuntimeError("SSH fixture setup did not start; raw export refused")
                    ssh.scan()
                code, _ = docker.call(["cp", f"{name}:{FIXTURE}/.", evidence / "fixture"], checked=False)
                copied = code == 0
                if copied:
                    for case in ("A01", "A02"):
                        report = evidence / "fixture/evidence" / f"{case}.json"
                        if report.exists():
                            value = json.loads(report.read_text()).get("status")
                            result["cases"][case] = value if value in ("passed", "failed") else "incomplete"
                        elif (evidence / "fixture/evidence" / f"{case}-admission.json").exists():
                            result["cases"][case] = "incomplete"
                    if result["status"] == "passed":
                        validate_bundle(evidence / "fixture")
                if remote and ssh.fixture_started:
                    from ssh_runner import FIXTURE as SSH_FIXTURE, validate, a04_status
                    code, _ = docker.call(["cp", f"{name}:{SSH_FIXTURE}/.", evidence / "ssh-fixture"], checked=False)
                    copied = copied and code == 0
                    report = evidence / "ssh-fixture/evidence/A03.json"
                    if report.exists():
                        result["cases"]["A03"] = json.loads(report.read_text()).get("status", "incomplete")
                    result["cases"]["A04"] = a04_status(evidence / "ssh-fixture/evidence")
                    if ssh.stopping is not None:
                        copied = ssh.stopping.copy() and copied
                    from stopping_runner import status as a05_status
                    result["cases"]["A05"] = a05_status(evidence)
                    if result["status"] == "passed":
                        validate(evidence / "ssh-fixture", evidence)
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                cleanup_errors.append(str(error))
            stopped = False
            try:
                code, _ = docker.call(["stop", "--time", "20", name], checked=False)
                stopped = code == 0
                if ssh is not None:
                    ssh.cleanup()
                if copied and stopped and not cleanup_errors:
                    docker.call(["rm", name])
            except (OSError, RuntimeError, subprocess.SubprocessError) as error:
                cleanup_errors.append(str(error))
            if not copied or not stopped or cleanup_errors:
                result.update(status="failed", retained_container=name,
                              cleanup_errors=cleanup_errors,
                              reason=result.get("reason", "") + "; evidence or cleanup incomplete; inspect retained container")
        result["evidence_copied"] = copied
        (evidence / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    return 0 if result["status"] == "passed" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--remote", action="store_true", help="also run A03 through A05 on isolated SSH targets")
    parser.add_argument("--rust-image", default="rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07")
    parser.add_argument("--debian-image", default="debian:13-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2")
    options = parser.parse_args()
    name = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8]
    evidence = options.artifacts or Path.home() / "artifacts/rustward/ansible-acceptance" / name
    print(f"Acceptance evidence: {evidence}", flush=True)
    return execute(ROOT, evidence.resolve(), options.rust_image, options.debian_image, remote=options.remote)


if __name__ == "__main__":
    sys.exit(main())

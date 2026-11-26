"""Build current Rustward sources and run acceptance through nspawn."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import uuid

from container import ROOT, snapshot, validate_bundle
from ssh_runner import validate as validate_ssh


def build_tools(context, evidence):
    tools = evidence / "tools"
    tools.mkdir()
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target))
    commands = [
        (["build", "--bin", "rustward", "--bin", "rustward-exec", "--example", "acceptance"],
         ("rustward", "rustward-exec", "acceptance")),
        (["test", "--no-run", "--test", "systemd_host", "--test", "runtime_host"],
         ("systemd_host", "runtime_host")),
        (["build", "--features", "web-preview", "--bin", "rustward"],
         ("rustward-preview",)),
    ]
    with (evidence / "build.log").open("w") as log:
        for arguments, names in commands:
            result = subprocess.run(
                ["cargo", arguments[0], "--locked", "--release", "--message-format=json", *arguments[1:]],
                cwd=context, env=environment, stdout=subprocess.PIPE, stderr=log, text=True, timeout=1800,
            )
            result.check_returncode()
            files = {}
            for line in result.stdout.splitlines():
                entry = json.loads(line)
                if entry.get("executable"):
                    files[entry["target"]["name"]] = entry["executable"]
            for name in names:
                source = "rustward" if name == "rustward-preview" else name
                shutil.copy2(files[source], tools / name)
                subprocess.run(["strip", "--strip-debug", tools / name], check=True, timeout=30)
    return tools


def execute(evidence, runner):
    evidence.mkdir(mode=0o700, parents=True)
    status = {"status": "failed", "phase": "build"}
    try:
        context = evidence / "source"
        source = snapshot(ROOT, context, extra_files=("rust-toolchain.toml",))
        (evidence / "source.json").write_text(json.dumps(source, indent=2) + "\n")
        tools = build_tools(context, evidence)
        revision_file = evidence / "source-revision"
        revision_file.write_text(source["revision"] + "\n")
        status.update(phase="integration", source_revision=source["revision"])
        with tarfile.open(evidence / "input.tar", "w") as archive:
            archive.add(tools, arcname="tools")
            archive.add(context / "acceptance", arcname="acceptance")
            archive.add(revision_file, arcname="source-revision")
        with (evidence / "input.tar").open("rb") as staged, \
                (evidence / "output.tar").open("wb") as retained, \
                (evidence / "runner.log").open("wb") as log:
            result = subprocess.run([runner], stdin=staged, stdout=retained, stderr=log, timeout=3900)
        if (evidence / "output.tar").stat().st_size == 0:
            result.check_returncode()
        with tarfile.open(evidence / "output.tar", "r:") as archive:
            archive.extractall(evidence / "container", filter="data")
        result.check_returncode()
        result_file = evidence / "container/result.json"
        expected = {"status": "passed", "source_revision": source["revision"]}
        if json.loads(result_file.read_text()) != expected:
            raise ValueError("acceptance failed; see container/result.json and runner.log")
        validate_bundle(evidence / "container/fixture", require_daemon=True, require_scheduler=True,
                        require_web=True, require_pilot=True)
        ssh = evidence / "container/ssh"
        validate_ssh(ssh / "ssh-fixture", ssh)
        status.update(status="passed", phase="complete")
    except (OSError, ValueError, KeyError, RuntimeError, tarfile.TarError, subprocess.SubprocessError) as error:
        status["reason"] = str(error)
        print(f"Integration failed: {error}; see {evidence}")
    finally:
        (evidence / "result.json").write_text(json.dumps(status, indent=2) + "\n")
    return 0 if status["status"] == "passed" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path)
    options = parser.parse_args()
    name = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8]
    evidence = options.artifacts or Path.home() / "artifacts/rustward/nspawn" / name
    runner = os.environ.get("RUSTWARD_NSPAWN_RUNNER", str(ROOT / "acceptance/nspawn-runner.sh"))
    print(f"Acceptance evidence: {evidence}", flush=True)
    return execute(evidence.resolve(), runner)


if __name__ == "__main__":
    raise SystemExit(main())

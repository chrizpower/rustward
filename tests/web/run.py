"""Build the synthetic preview and run browser tests."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import uuid

root = Path(__file__).resolve().parents[2]
web = root / "web"
metadata = subprocess.check_output(
    ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root, text=True,
)
checkout = hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
target = Path(json.loads(metadata)["target_directory"]) / f"web-preview-{checkout}"
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
artifacts = Path(os.environ.get("RUSTWARD_WEB_ARTIFACTS",
    Path.home() / "artifacts/rustward/web" / f"{stamp}-{uuid.uuid4().hex[:8]}"))
artifacts = artifacts.resolve()
print(f"Browser test artifacts: {artifacts}", flush=True)
subprocess.run(["cargo", "build", "--locked", "--features", "web-preview", "--bin", "rustward"],
               cwd=root, check=True, env={**os.environ, "CARGO_TARGET_DIR": str(target)})
binary = target / "debug/rustward"
engine = os.environ.get("RUSTWARD_WEB_CONTAINER", "podman" if shutil.which("podman") else "none")


def run_browser_tests(project):
    subprocess.run(["npm", "ci", "--include=dev"], cwd=project, check=True)
    playwright = project / "node_modules/.bin/playwright"
    if engine == "podman":
        artifacts.mkdir(parents=True, exist_ok=True)
        subprocess.run([
            "podman", "run", "--rm", "--network=none", "--userns=keep-id",
            "--user", f"{os.getuid()}:{os.getgid()}", "--shm-size=512m",
            "-v", f"{project}:/web:ro", "-v", f"{binary}:/usr/local/bin/rustward:ro",
            "-v", f"{artifacts}:/results:rw", "-w", "/web",
            "-e", "RUSTWARD_WEB_BINARY=/usr/local/bin/rustward", "-e", "RUSTWARD_WEB_ARTIFACTS=/results/browser",
            "mcr.microsoft.com/playwright:v1.58.2-noble", "/web/node_modules/.bin/playwright", "test",
        ], check=True)
    elif engine == "none":
        subprocess.run([playwright, "install", "chromium"], cwd=project, check=True)
        subprocess.run([playwright, "test"], cwd=project, check=True,
                       env={**os.environ, "RUSTWARD_WEB_BINARY": str(binary),
                            "RUSTWARD_WEB_ARTIFACTS": str(artifacts)})
    else:
        raise SystemExit("RUSTWARD_WEB_CONTAINER must be podman or none")


with tempfile.TemporaryDirectory(prefix="rustward-browser-") as scratch:
    project = Path(scratch)
    for name in ("package.json", "package-lock.json", "playwright.config.ts", "tsconfig.json"):
        shutil.copyfile(web / name, project / name)
    for name in ("src", "tests"):
        shutil.copytree(web / name, project / name)
    run_browser_tests(project)

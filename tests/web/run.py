"""Build the synthetic preview and run browser tests."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid

root = Path(__file__).resolve().parents[2]
tests = Path(__file__).resolve().parent
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
subprocess.run(["npm", "ci"], cwd=tests, check=True)
playwright = tests / "node_modules/.bin/playwright"
binary = target / "debug/rustward"
engine = os.environ.get("RUSTWARD_WEB_CONTAINER", "podman" if shutil.which("podman") else "none")
if engine == "podman":
    artifacts.mkdir(parents=True, exist_ok=True)
    subprocess.run([
        "podman", "run", "--rm", "--network=none", "--userns=keep-id",
        "--user", f"{os.getuid()}:{os.getgid()}", "--shm-size=512m",
        "-v", f"{tests}:/tests:ro", "-v", f"{binary}:/usr/local/bin/rustward:ro",
        "-v", f"{artifacts}:/results:rw", "-w", "/tests",
        "-e", "RUSTWARD_WEB_BINARY=/usr/local/bin/rustward", "-e", "RUSTWARD_WEB_ARTIFACTS=/results/browser",
        "mcr.microsoft.com/playwright:v1.58.2-noble", "/tests/node_modules/.bin/playwright", "test",
    ], check=True)
elif engine == "none":
    subprocess.run([playwright, "install", "chromium"], cwd=tests, check=True)
    subprocess.run([playwright, "test"], cwd=tests, check=True,
                   env={**os.environ, "RUSTWARD_WEB_BINARY": str(binary),
                        "RUSTWARD_WEB_ARTIFACTS": str(artifacts)})
else:
    raise SystemExit("RUSTWARD_WEB_CONTAINER must be podman or none")

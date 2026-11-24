"""Run the synthetic monitoring rehearsal in an isolated container."""
from datetime import datetime, timezone
from pathlib import Path
import json
import shutil
import subprocess
import tempfile
import uuid

from container import ROOT, snapshot


def main():
    engine = shutil.which("podman") or shutil.which("docker")
    if engine is None:
        raise SystemExit("Install Podman or start Docker Desktop")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    evidence = Path.home() / "artifacts/rustward/pilot" / f"{stamp}-{uuid.uuid4().hex[:8]}"
    evidence.mkdir(parents=True, mode=0o700)
    print(f"Pilot evidence: {evidence}", flush=True)
    with tempfile.TemporaryDirectory(prefix="rustward-pilot-") as data:
        context = Path(data) / "context"
        source = snapshot(ROOT, context)
        (evidence / "source.json").write_text(json.dumps(source, indent=2) + "\n")
        image = evidence / "image-id"
        with (evidence / "build.log").open("w") as output:
            subprocess.run([engine, "build", "--target", "pilot-rehearsal", "--iidfile", image,
                            "-f", "acceptance/Dockerfile", "."], cwd=context,
                           stdout=output, stderr=subprocess.STDOUT, check=True)
    subprocess.run([engine, "run", "--rm", "--network=none", "--cap-drop=ALL",
                    "-v", f"{evidence}:/results:rw", image.read_text().strip()], check=True)


if __name__ == "__main__":
    main()

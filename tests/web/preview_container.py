"""Run the synthetic Linux preview with a host loopback port."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "acceptance"))
from container import snapshot


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("port", nargs="?", default=4173, type=int)
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("port must be between 1 and 65535")
    engine = shutil.which("docker") or shutil.which("podman")
    if engine is None:
        parser.error("start Docker Desktop or install Podman with a Linux machine")
    image = "rustward-web-preview:local"
    with tempfile.TemporaryDirectory(prefix="rustward-web-preview-") as data:
        context = Path(data) / "context"
        snapshot(root, context)
        subprocess.run([engine, "build", "--target", "web-preview", "-t", image,
                        "-f", "acceptance/Dockerfile", "."], cwd=context, check=True)
    port = str(args.port)
    os.execv(engine, [engine, "run", "--rm", "--cap-drop=ALL",
                      "-p", f"127.0.0.1:{port}:{port}", image, port])


if __name__ == "__main__":
    main()

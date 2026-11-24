"""Build and package the two Linux executables from a Git revision."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
MACHINES = {"x86_64": 62, "aarch64": 183}
SOURCES = (
    "Cargo.toml", "Cargo.lock", "build.rs", "src", "acceptance/Dockerfile",
    "examples/acceptance", "examples/rustward.toml", "examples/tasks", "examples/schedules",
    "web/index.html", "web/package.json", "web/package-lock.json", "web/tsconfig.json",
    "web/vite.config.ts", "web/src",
)
EXECUTABLES = ("rustward", "rustward-exec")


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def source_context(revision, destination):
    with tempfile.TemporaryFile() as source:
        subprocess.run(["git", "archive", revision, "--", *SOURCES],
                       cwd=ROOT, stdout=source, check=True)
        source.seek(0)
        with tarfile.open(fileobj=source) as archive:
            archive.extractall(destination, filter="data")


def check_executable(path, architecture):
    with path.open("rb") as source:
        header = source.read(20)
    if (len(header) != 20 or header[:7] != b"\x7fELF\x02\x01\x01"
            or struct.unpack_from("<H", header, 18)[0] != MACHINES[architecture]):
        raise ValueError(f"{path.name} is not a Linux executable for {architecture}")


def add_bytes(archive, name, data, mode):
    entry = tarfile.TarInfo(name)
    entry.size = len(data)
    entry.mode = mode
    archive.addfile(entry, io.BytesIO(data))


def package(binaries, output, architecture, revision, version):
    for name in EXECUTABLES:
        check_executable(binaries / name, architecture)
    output.mkdir(parents=True, exist_ok=True)
    bundle = output / f"rustward-linux-{architecture}.tar.gz"
    manifest = json.dumps({
        "architecture": architecture,
        "executables": EXECUTABLES,
        "os": "linux",
        "revision": revision,
        "version": version,
    }, indent=2, sort_keys=True).encode() + b"\n"
    license_path = subprocess.run(["git", "cat-file", "-e", f"{revision}:LICENSE"],
                                  cwd=ROOT, stderr=subprocess.DEVNULL, check=False)
    with bundle.open("wb") as stream, gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
            for name in EXECUTABLES:
                add_bytes(archive, name, (binaries / name).read_bytes(), 0o755)
            add_bytes(archive, "BUILD-INFO.json", manifest, 0o644)
            if license_path.returncode == 0:
                add_bytes(archive, "LICENSE", git("show", f"{revision}:LICENSE"), 0o644)
    with bundle.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    (output / f"{bundle.name}.sha256").write_text(f"{digest}  {bundle.name}\n", encoding="ascii")
    print(bundle)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD")
    parser.add_argument("--architecture", required=True, choices=MACHINES)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--engine", choices=("docker", "podman"))
    args = parser.parse_args()
    engine = args.engine or shutil.which("podman") or shutil.which("docker")
    if engine is None:
        parser.error("Install Podman or Docker")
    revision = git("rev-parse", "--verify", "--end-of-options", args.revision + "^{commit}").decode().strip()
    version = tomllib.loads(git("show", f"{revision}:Cargo.toml").decode())["package"]["version"]
    platform = {"x86_64": "amd64", "aarch64": "arm64"}[args.architecture]
    with tempfile.TemporaryDirectory(prefix="rustward-release-") as directory:
        staging = Path(directory)
        context = staging / "source"
        context.mkdir()
        source_context(revision, context)
        binaries = staging / "binaries"
        subprocess.run([
            engine, "build", "--platform", f"linux/{platform}", "--target", "release-files",
            "--output", f"type=local,dest={binaries}", "-f", "acceptance/Dockerfile", ".",
        ], cwd=context, check=True)
        package(binaries, args.output, args.architecture, revision, version)


if __name__ == "__main__":
    main()

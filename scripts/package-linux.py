"""Build and package the two Linux executables from a Git revision."""
import argparse
import gzip
import hashlib
import io
import json
import re
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


def tag_for_version(version, revision):
    if not isinstance(version, str) or not re.fullmatch(
        r"\d+\.\d+\.\d+-[0-9A-Za-z.-]+\.g" + revision + r"(?:\+[0-9A-Za-z.-]+)?", version
    ):
        raise ValueError("Artifact version does not identify the source commit")
    return f"v{version}"


def tag_for_downloads(downloads, revision):
    tags = set()
    for architecture in MACHINES:
        name = f"rustward-linux-{architecture}"
        bundle = downloads / name / f"{name}.tar.gz"
        checksum = bundle.with_name(bundle.name + ".sha256")
        with bundle.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if checksum.read_text(encoding="ascii").strip() != f"{digest}  {bundle.name}":
            raise ValueError(f"Checksum differs for {architecture}")
        with tarfile.open(bundle) as archive:
            with archive.extractfile("BUILD-INFO.json") as info:
                build = json.load(info)
        if (build["architecture"], build["os"], build["revision"]) != (architecture, "linux", revision):
            raise ValueError(f"Build metadata differs for {architecture}")
        tags.add(tag_for_version(build["version"], revision))
    if len(tags) != 1:
        raise ValueError("Linux archives report different versions")
    return tags.pop()


def package(binaries, output, architecture, revision, package_version):
    version = (binaries / "rustward-version").read_text(encoding="ascii").strip()
    tag_for_version(version, revision)
    for name in EXECUTABLES:
        check_executable(binaries / name, architecture)
    output.mkdir(parents=True, exist_ok=True)
    bundle = output / f"rustward-linux-{architecture}.tar.gz"
    manifest = json.dumps({
        "architecture": architecture,
        "executables": EXECUTABLES,
        "os": "linux",
        "package_version": package_version,
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
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--architecture", choices=MACHINES)
    mode.add_argument("--tag-for", type=Path, metavar="DOWNLOADED_ARTIFACTS")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--engine", choices=("docker", "podman"))
    args = parser.parse_args()
    revision = git("rev-parse", "--verify", "--end-of-options", args.revision + "^{commit}").decode().strip()
    if args.tag_for is not None:
        print(tag_for_downloads(args.tag_for, revision))
        return
    if args.output is None:
        parser.error("--architecture requires --output")
    engine = args.engine or shutil.which("podman") or shutil.which("docker")
    if engine is None:
        parser.error("Install Podman or Docker")
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
            "--build-arg", f"RUSTWARD_SOURCE_REVISION={revision}",
            "--output", f"type=local,dest={binaries}", "-f", "acceptance/Dockerfile", ".",
        ], cwd=context, check=True)
        package(binaries, args.output, args.architecture, revision, version)


if __name__ == "__main__":
    main()

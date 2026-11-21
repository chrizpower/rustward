"""Save mounted A10 output before scanning and copying the fixture."""
import hashlib
import json
from pathlib import Path
import re
import stat


def read(path):
    return json.loads(path.read_text())


def copy_directory(source, destination, files, relative=Path()):
    if not stat.S_ISDIR(source.lstat().st_mode):
        raise RuntimeError("storage evidence source is not a directory")
    destination.mkdir(mode=0o700)
    for entry in sorted(source.iterdir()):
        mode = entry.lstat().st_mode
        path = relative / entry.name
        target = destination / entry.name
        if stat.S_ISDIR(mode):
            copy_directory(entry, target, files, path)
        elif stat.S_ISREG(mode):
            digest = hashlib.sha256()
            size = 0
            with entry.open("rb") as source_file, target.open("xb") as output:
                for block in iter(lambda: source_file.read(65536), b""):
                    output.write(block)
                    digest.update(block)
                    size += len(block)
            files[path.as_posix()] = {"bytes": size, "sha256": digest.hexdigest()}
        else:
            raise RuntimeError("storage evidence contains a link or special file")


def export(root):
    root = Path(root)
    host = read(root / "evidence/A10-host.json")
    if host.get("writers_stopped") is not True:
        raise RuntimeError("storage evidence writers are not stopped")
    files = {}
    copy_directory(root / "logs", root / "exported-logs", files)
    report = {"status": "copied", "files": files}
    with (root / "evidence/A10-export.json").open("x") as output:
        json.dump(report, output, indent=2)
    native = root / "evidence/A10.json"
    if native.exists() and read(native).get("status") == "passed":
        validate(root)


def validate(root):
    root = Path(root)
    native = read(root / "evidence/A10.json")
    exported = read(root / "evidence/A10-export.json")
    run_id = native.get("run_id")
    if type(run_id) is not str or re.fullmatch("[a-f0-9]{64}", run_id) is None:
        raise RuntimeError("storage evidence has an invalid run ID")
    if exported.get("status") != "copied":
        raise RuntimeError("storage output was not exported")
    directory = root / "exported-logs"
    for path in (directory, directory / run_id):
        if not stat.S_ISDIR(path.lstat().st_mode):
            raise RuntimeError("storage evidence directory is missing or is a link")
    record = native["record"]
    retained = 0
    for stream in ("stdout", "stderr"):
        name = f"{run_id}/{stream}.log"
        path = directory / name
        if not stat.S_ISREG(path.lstat().st_mode):
            raise RuntimeError("storage output is missing or is not a regular file")
        expected = record["output_evidence"][stream]["retained_bytes"]
        if type(expected) is not int or expected < 0:
            raise RuntimeError("storage output has an invalid byte count")
        digest = hashlib.sha256()
        size = 0
        with path.open("rb") as source:
            for block in iter(lambda: source.read(65536), b""):
                digest.update(block)
                size += len(block)
        if size != expected or exported["files"].get(name) != {"bytes": size, "sha256": digest.hexdigest()}:
            raise RuntimeError("exported storage output differs from its record")
        retained += size
    if retained != record["log_bytes"]:
        raise RuntimeError("exported storage output total differs from its record")

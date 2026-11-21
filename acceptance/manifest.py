"""Fixture-only file copies and host dependency manifests."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            value.update(block)
    return value.hexdigest()


def copy_tree(source, target, ancestors=()):
    source, target = Path(source), Path(target)
    resolved = source.resolve(strict=True)
    if resolved in ancestors:
        raise ValueError("source directory link cycle")
    target.mkdir(mode=0o755)
    for entry in sorted(source.iterdir()):
        if entry.name == "__pycache__" or entry.suffix == ".pyc":
            continue
        output = target / entry.name
        if entry.is_dir():
            copy_tree(entry, output, (*ancestors, resolved))
        elif entry.is_file():
            shutil.copyfile(entry, output)
            output.chmod(0o755 if entry.stat().st_mode & 0o111 else 0o644)
        else:
            raise ValueError("source contains a special file")


def tree_manifest(root):
    directories, files = [], {}
    for directory, folders, names in os.walk(root):
        for name in sorted(folders):
            path = Path(directory) / name
            if path.is_symlink():
                raise ValueError("runtime directory is a symlink")
            directories.append(path.relative_to(root).as_posix())
        for name in sorted(names):
            path = Path(directory) / name
            if path.is_symlink() or not path.is_file():
                raise ValueError("runtime entry is not a regular file")
            files[path.relative_to(root).as_posix()] = {
                "sha256": digest(path), "executable": bool(path.stat().st_mode & 0o111)}
    return sorted(directories), files


def source(manifest):
    text = f'version = {manifest["version"]}\n'
    for key in ("host", "launcher", "directories"):
        if key in manifest:
            text += f"{key} = {json.dumps(manifest[key], ensure_ascii=True)}\n"
    for path, specification in sorted(manifest["files"].items()):
        text += f"\n[files.{json.dumps(path)}]\n"
        text += f'sha256 = "{specification["sha256"]}"\n'
        text += f'executable = {str(specification["executable"]).lower()}\n'
    if manifest.get("links"):
        text += "\n[links]\n"
        for path, target in sorted(manifest["links"].items()):
            text += f"{json.dumps(path)} = {json.dumps(target)}\n"
    return text


class Host:
    def __init__(self, launcher):
        self.value = {"version": 1, "launcher": str(Path(launcher).resolve(strict=True)),
                      "files": {}, "links": {}}

    def pin(self, logical):
        path = Path(logical)
        if not path.is_absolute():
            raise ValueError("host dependency path must be absolute")
        current = Path("/")
        parts = list(path.parts[1:])
        count = 0
        while parts:
            current /= parts.pop(0)
            if current.is_symlink():
                count += 1
                if count > 32:
                    raise ValueError("host link cycle or limit")
                target = os.readlink(current)
                self.value["links"][str(current)] = target
                replacement = Path(os.path.normpath(
                    target if Path(target).is_absolute() else current.parent / target))
                parts = list(replacement.parts[1:]) + parts
                current = Path("/")
        metadata = current.stat()
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("host dependency is not a regular file")
        self.value["files"][str(current)] = {
            "sha256": digest(current), "executable": bool(metadata.st_mode & 0o111)}

    def libraries(self, executable):
        result = subprocess.run(["/usr/bin/ldd", str(executable)], check=False,
                                capture_output=True, text=True, timeout=30)
        output = result.stdout + result.stderr
        if result.returncode and not any(marker in output for marker in
                                        ("not a dynamic executable", "statically linked")):
            raise RuntimeError("could not inspect fixture ELF dependencies")
        if "not found" in output:
            raise RuntimeError("fixture ELF dependency is missing")
        for line in result.stdout.splitlines():
            field = line.split("=>", 1)[-1].strip().split()
            if field and field[0].startswith("/"):
                path = Path(field[0])
                if not hasattr(self, "runtime") or not path.resolve().is_relative_to(self.runtime):
                    self.pin(path)


def host_manifest(runtime, launcher):
    host = Host(launcher)
    host.runtime = Path(runtime).resolve()
    host.pin(launcher)
    host.libraries(launcher)
    host.pin("/bin/sh")
    host.libraries(Path("/bin/sh").resolve(strict=True))
    inspected = set()
    for directory, _, names in os.walk(runtime):
        for name in names:
            path = Path(directory) / name
            with path.open("rb") as candidate:
                elf = candidate.read(4) == b"\x7fELF"
            if elf:
                checksum = digest(path)
                if checksum not in inspected:
                    host.libraries(path)
                    inspected.add(checksum)
    for path in ("/etc/ld.so.cache", "/etc/nsswitch.conf", "/etc/passwd", "/etc/group",
                 "/usr/share/zoneinfo/UTC"):
        if Path(path).exists():
            host.pin(path)
    locale = Path("/usr/lib/locale/C.utf8")
    if locale.exists():
        for path in sorted(locale.rglob("*")):
            if path.is_file():
                host.pin(path)
    return host.value

"""Ephemeral synthetic client credentials and export checks."""
import json
import os
from pathlib import Path
import stat
import subprocess

from fixtures.ssh_policy import CREDENTIALS, ALIAS


def create_key(path):
    subprocess.run(["/usr/bin/ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "",
                    "-f", str(path)], check=True, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=15)
    public = " ".join(path.with_suffix(".pub").read_text().split()[:2])
    lines = path.read_bytes().splitlines()[1:-1]
    marker = lines[len(lines) // 2]
    if len(marker) < 64 or marker in public.encode():
        raise RuntimeError("could not select a private credential marker")
    return public, marker


def client_key(uid=1000, gid=1000):
    CREDENTIALS.mkdir(mode=0o711)
    public, marker = create_key(CREDENTIALS / "client_key")
    key = CREDENTIALS / "client_key"
    os.chown(key, uid, gid)
    key.chmod(0o400)
    marker_file = CREDENTIALS / "marker"
    marker_file.write_bytes(marker)
    marker_file.chmod(0o600)
    (CREDENTIALS / "wrong_known_hosts").write_text(f"{ALIAS} {public}\n")
    (CREDENTIALS / "wrong_known_hosts").chmod(0o644)
    return {"public_key": public}


def scan(roots, marker):
    if len(marker) < 64:
        raise ValueError("invalid credential marker")
    findings = []
    checked = 0
    for root in map(Path, roots):
        if root.is_symlink():
            findings.append({"file": "<root>", "reason": "unsafe export root"})
            continue
        if not root.exists():
            continue
        for directory, folders, files in os.walk(root, followlinks=False):
            for name in [*folders, *files]:
                path = Path(directory) / name
                relative = str(path.relative_to(root))
                label = "<redacted path>" if marker in relative.encode() else relative
                metadata = path.lstat()
                if stat.S_ISLNK(metadata.st_mode) or not (stat.S_ISREG(metadata.st_mode) or stat.S_ISDIR(metadata.st_mode)):
                    if name in folders:
                        folders.remove(name)
                    findings.append({"file": label, "reason": "unsafe export entry"})
                    continue
                if not stat.S_ISREG(metadata.st_mode):
                    continue
                checked += 1
                found = marker in relative.encode()
                tail = b""
                with path.open("rb") as source:
                    for block in iter(lambda: source.read(65536), b""):
                        value = tail + block
                        found |= marker in value
                        tail = value[-(len(marker) - 1):]
                if found:
                    findings.append({"file": label, "reason": "credential marker found"})
    return {"status": "passed" if not findings else "failed", "checked_files": checked,
            "findings": findings, "scope": "literal private-key marker in all fixture files"}


def export_status(roots):
    marker = (CREDENTIALS / "marker").read_bytes()
    report = scan(roots, marker)
    (CREDENTIALS / "export-status.json").write_text(json.dumps(report, sort_keys=True) + "\n")
    return report

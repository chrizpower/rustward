"""Fixed SSH policy for the synthetic fixture; no production secret interface."""
from pathlib import Path

CREDENTIALS = Path("/run/rustward-credentials")
ALIAS = "rustward-a03-target"
USER = "rustward-target"
TARGET = "/home/rustward-target/result.json"


def options(known_hosts=None):
    known_hosts = known_hosts or CREDENTIALS / "known_hosts"
    return ["-F", "/dev/null", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
            "-o", f"UserKnownHostsFile={known_hosts}", "-o", "GlobalKnownHostsFile=/dev/null",
            "-o", f"HostKeyAlias={ALIAS}", "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none",
            "-o", "ControlMaster=no", "-o", "ControlPersist=no", "-o", "ControlPath=none", "-o", "ForwardAgent=no",
            "-o", "ClearAllForwardings=yes", "-o", "PasswordAuthentication=no",
            "-o", "KbdInteractiveAuthentication=no", "-o", "ConnectTimeout=5"]


def credentials(mode="success"):
    if mode in ("queued", "cancel", "timeout"):
        return CREDENTIALS / "stopping"
    if mode in ("recovery", "async", "runtime-a", "runtime-b", "flood"):
        return CREDENTIALS / "remaining"
    return CREDENTIALS

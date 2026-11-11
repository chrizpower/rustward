"""Read one remote process marker through the pinned fixture SSH connection."""
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).parent))
from ssh_policy import CREDENTIALS, USER, credentials, options


def read(runtime, kind):
    if kind not in ("queued", "cancel", "timeout", "recovery", "async", "runtime-a"):
        raise ValueError("unknown stopping case")
    directory = credentials(kind)
    connection = json.loads((directory / "connection.json").read_text())
    command = [str(runtime / "bin/ssh"), *options(directory / "known_hosts"),
               "-i", str(CREDENTIALS / "client_key"), "-p", "2222", f"{USER}@{connection['address']}",
               "/usr/bin/python3", "-I", "-B", "/opt/rustward_hold.py", "state", kind]
    try:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10)
    except subprocess.TimeoutExpired:
        return {"state": "probe-failed", "kind": kind}
    if result.returncode:
        return {"state": "probe-failed", "kind": kind, "exit_code": result.returncode}
    return json.loads(result.stdout)


if __name__ == "__main__":
    print(json.dumps(read(Path(sys.executable).parent.parent, sys.argv[1])))

"""Release only the matching A08 fixture command; never retry a lost reply."""
import json
from pathlib import Path
import subprocess
import sys
sys.path.insert(0, str(Path(__file__).parent))
from ssh_policy import CREDENTIALS, USER, credentials, options

if __name__ == "__main__":
    kind, run_id = sys.argv[1:]
    if kind != "runtime-a" or len(run_id) != 64 or any(char not in "0123456789abcdef" for char in run_id):
        raise ValueError("invalid runtime release")
    runtime = Path(sys.executable).parent.parent
    directory = credentials(kind)
    connection = json.loads((directory / "connection.json").read_text())
    command = [str(runtime / "bin/ssh"), *options(directory / "known_hosts"), "-i", str(CREDENTIALS / "client_key"),
               "-p", "2222", f"{USER}@{connection['address']}", "/usr/bin/python3", "-I", "-B",
               "/opt/rustward_hold.py", "release", kind, run_id]
    result = subprocess.run(command, stdin=subprocess.DEVNULL, check=True, capture_output=True, text=True, timeout=10)
    print(result.stdout, end="")

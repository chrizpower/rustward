import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).parent))
from common import begin, save, check_privileges

mode = sys.argv[1]
if mode not in ("complete", "hold"):
    raise ValueError("invalid fixture mode")
work, values = begin(sys.argv[2:])
runtime = Path(sys.executable).parent.parent
expected = {"PATH": str(runtime / "bin"), "HOME": str(work), "TMPDIR": str(work),
            "RUSTWARD_WORK_DIRECTORY": str(work), "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8", "TZ": "UTC"}
if dict(os.environ) != expected:
    raise ValueError("execution environment differs")
save(work / "environment.json", dict(os.environ))
save(work / "privileges.json", check_privileges())
child = subprocess.Popen([sys.executable, "-I", "-B", str(runtime / "child.py")],
                         start_new_session=True, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
deadline = time.monotonic() + 10
while not (work / "child.json").exists():
    if child.poll() is not None or time.monotonic() >= deadline:
        raise RuntimeError("child did not reach its synchronization point")
    time.sleep(0.01)
save(work / "target.json", values)
print("A02_STDOUT", flush=True)
print("A02_STDERR", file=sys.stderr, flush=True)
save(work / "ready.json", {"mode": mode})
if mode == "hold":
    while True:
        time.sleep(0.1)

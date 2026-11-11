"""Synthetic remote work. Local stopping does not prove this process ended."""
import json
import os
from pathlib import Path
import sys
import time


def identity(pid):
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return None
    return {"state": fields[0], "start_time": fields[19]}


def marker(kind):
    if kind not in ("queued", "cancel", "timeout", "recovery", "async", "runtime-a"):
        raise ValueError("unknown holding case")
    return Path(f"/home/rustward-target/a05-{kind}.json")


def state(kind):
    path = marker(kind)
    if not path.exists():
        return {"kind": kind, "state": "absent"}
    value = json.loads(path.read_text())
    current = identity(value["pid"])
    alive = current is not None and current["start_time"] == value["start_time"] and current["state"] not in ("Z", "X")
    return dict(value, state="alive" if alive else "ended")


def publish(path, value):
    pending = path.with_suffix(path.suffix + ".pending")
    with pending.open("x") as output:
        json.dump(value, output)
        output.flush()
        os.fsync(output.fileno())
    os.link(pending, path)
    pending.unlink()


def run(kind, values, run_id):
    if len(run_id) != 64 or any(char not in "0123456789abcdef" for char in run_id):
        raise ValueError("invalid run ID")
    current = identity(os.getpid())
    value = {"run_id": run_id, "kind": kind, "pid": os.getpid(), "start_time": current["start_time"], "inputs": values}
    publish(marker(kind), value)
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        release = marker(kind).with_suffix(".release")
        if kind == "runtime-a" and release.exists() and json.loads(release.read_text()).get("run_id") == run_id:
            return
        time.sleep(0.2)
    raise SystemExit(24)


if __name__ == "__main__":
    if sys.argv[1] == "release":
        kind, run_id = sys.argv[2:4]
        current = state(kind)
        if current.get("run_id") != run_id or current.get("state") != "alive":
            raise ValueError("release does not match live remote work")
        publish(marker(kind).with_suffix(".release"), {"run_id":run_id})
        print(json.dumps({"status":"released","run_id":run_id}))
    elif sys.argv[1] == "state":
        print(json.dumps(state(sys.argv[2])))
    else:
        run(sys.argv[1], json.loads(sys.argv[2]), sys.argv[3])

import json
import os
from pathlib import Path
import sys

TEXT = 'He said "hello"; it\'s a test'


def save(path, value):
    path = Path(path)
    pending = path.with_suffix(path.suffix + ".pending")
    with pending.open("x", encoding="utf-8") as output:
        json.dump(value, output, sort_keys=True)
        output.flush()
        os.fsync(output.fileno())
    os.link(pending, path)
    pending.unlink()


def process():
    value = Path("/proc/self/stat").read_text()
    return {"pid": os.getpid(), "start_time": value.rsplit(")", 1)[1].split()[19]}


def begin(arguments):
    text, number, enabled = arguments
    if enabled not in ("true", "false"):
        raise ValueError("invalid boolean")
    values = {"text": text, "count": int(number), "enabled": enabled == "true"}
    if values != {"text": TEXT, "count": 7, "enabled": True}:
        raise ValueError("unexpected fixture input")
    work = Path(os.environ["RUSTWARD_WORK_DIRECTORY"])
    with (work / "starts.jsonl").open("a", encoding="utf-8") as output:
        output.write(json.dumps(process(), sort_keys=True) + "\n")
        output.flush()
        os.fsync(output.fileno())
    save(work / "inputs.json", values)
    return work, values


def dependencies():
    runtime = Path(sys.executable).parent.parent
    sys.path.insert(0, str(runtime / "lib" / "python3" / "dist-packages"))
    return runtime


def check_privileges():
    fields = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines() if ":" in line)
    names = ("CapEff", "CapPrm", "CapInh", "CapAmb")
    if os.getuid() == 0 or os.getuid() != os.geteuid():
        raise RuntimeError("workload is not an unprivileged execution user")
    if any(int(fields[name].strip(), 16) != 0 for name in names) or fields["NoNewPrivs"].strip() != "1":
        raise RuntimeError("workload can gain privileges or holds capabilities")
    return {name: fields[name].strip() for name in (*names, "NoNewPrivs")}

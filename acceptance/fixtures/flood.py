"""Bounded synthetic output; publish proof only after both streams finish."""
import json
from pathlib import Path
import sys


def run(run_id, values, destination):
    if len(run_id) != 64 or any(char not in "0123456789abcdef" for char in run_id):
        raise ValueError("invalid flood run ID")
    block = b"x" * 65536
    for stream in (sys.stdout.buffer, sys.stderr.buffer):
        for _ in range(16): stream.write(block)
        stream.flush()
    with Path(destination).open("x") as output:
        json.dump({"run_id":run_id,"inputs":values,"stdout_bytes":1048576,"stderr_bytes":1048576},output)


if __name__ == "__main__":
    run(sys.argv[1],json.loads(sys.argv[2]),sys.argv[3])

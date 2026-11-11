"""Emit bounded local output only after the native fixture holds the DB lock."""
import os
import threading
import time


def start(work):
    def emit():
        deadline = time.monotonic() + 120
        while not (work / "write-output").exists():
            if time.monotonic() >= deadline: return
            time.sleep(0.02)
        try:
            for _ in range(32):
                pending = memoryview(b"x" * 65536)
                while pending:
                    pending = pending[os.write(1, pending):]
        except BrokenPipeError:
            return
    threading.Thread(target=emit, name="fixture-output", daemon=True).start()

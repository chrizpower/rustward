#!/usr/bin/env python3
"""Run acceptance on a prepared disposable Debian VM with KVM."""

import fcntl
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


def qmp(path, command):
    with socket.socket(socket.AF_UNIX) as monitor:
        monitor.settimeout(5)
        monitor.connect(str(path))
        with monitor.makefile("rwb") as stream:
            json.loads(stream.readline())
            for name in ("qmp_capabilities", command):
                stream.write(json.dumps({"execute": name}).encode() + b"\n")
                stream.flush()
                while True:
                    reply = json.loads(stream.readline())
                    if "return" in reply:
                        break
                    if "error" in reply:
                        raise RuntimeError(f"QEMU monitor command {name} failed: {reply['error']}")
    return reply["return"]


def ssh(state, command):
    return [
        "ssh", "-F", "/dev/null", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3",
        "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes", "-o",
        f"UserKnownHostsFile={state / 'known-hosts'}", "-o", "HostKeyAlias=rustward-qemu",
        "-i", state / "access-key", "-p", "22286", "nspawn-runner@127.0.0.1", command,
    ]


def wait_for_ssh(state, guest):
    deadline = time.monotonic() + 90
    while guest.poll() is None and time.monotonic() < deadline:
        try:
            result = subprocess.run(
                ssh(state, "systemctl --user is-active --quiet default.target"),
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, timeout=10,
            )
            if result.returncode == 0:
                return
        except subprocess.TimeoutExpired:
            pass
        time.sleep(1)
    raise RuntimeError("The QEMU runner did not become ready")


def stop(guest, monitor):
    try:
        if guest.poll() is None:
            qmp(monitor, "system_powerdown")
            guest.wait(timeout=30)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        guest.terminate()
        try:
            guest.wait(timeout=10)
        except subprocess.TimeoutExpired:
            guest.kill()
            guest.wait()
        raise RuntimeError("The QEMU runner did not shut down cleanly") from error


def run(state, monitor, logs):
    command = [
        "qemu-system-x86_64", "-machine", "q35", "-accel", "kvm", "-cpu", "host",
        "-smp", "4", "-m", "4096", "-display", "none", "-monitor", "none",
        "-qmp", f"unix:{monitor},server=on,wait=off", "-serial", f"file:{logs / 'serial.log'}",
        "-drive", "file=runner.qcow2,if=virtio,format=qcow2",
        "-drive", "file=seed.iso,media=cdrom,readonly=on",
        "-nic", "user,model=virtio-net-pci,restrict=on,hostfwd=tcp:127.0.0.1:22286-:22",
    ]
    with (logs / "qemu.log").open("wb") as log:
        guest = subprocess.Popen(command, cwd=state, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=log)
        try:
            wait_for_ssh(state, guest)
            acceleration = qmp(monitor, "query-kvm")
            if not acceleration["enabled"]:
                raise RuntimeError("The QEMU runner did not enable KVM")
            (logs / "kvm.json").write_text(json.dumps(acceleration) + "\n")
            script = Path(__file__).with_name("nspawn-runner.sh")
            subprocess.run(
                ssh(state, 'umask 077; cat > "$HOME/.rustward-nspawn-runner.sh"'),
                input=script.read_bytes(), check=True, timeout=15,
            )
            result = subprocess.run(
                ssh(state, 'RUSTWARD_NSPAWN_BASE_IMAGE=/var/lib/rustward-runner/debian-13-base.raw '
                           'exec /bin/sh "$HOME/.rustward-nspawn-runner.sh"'),
                timeout=3700,
            )
        finally:
            stop(guest, monitor)
        if guest.returncode:
            raise RuntimeError(f"QEMU exited with status {guest.returncode}")
        return result.returncode


def main():
    try:
        state = Path(os.environ["RUSTWARD_QEMU_STATE_DIR"]).resolve(strict=True)
        with (state / "acceptance.lock").open("a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise RuntimeError("Another acceptance run is using this QEMU state directory") from error
            logs = Path(tempfile.mkdtemp(prefix="acceptance-qemu-", dir=state))
            print(f"QEMU runner logs: {logs}", file=sys.stderr, flush=True)
            with tempfile.TemporaryDirectory(prefix="rustward-qmp-") as scratch:
                return run(state, Path(scratch) / "monitor.sock", logs)
    except (OSError, KeyError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"QEMU runner failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

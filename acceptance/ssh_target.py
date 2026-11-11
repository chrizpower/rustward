"""Disposable SSH target. Only public metadata and target data can be exported."""
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path("/run/rustward-target")
PRIVILEGE_DIRECTORY = Path("/run/sshd")
RESULT = Path("/home/rustward-target/result.json")


def serve():
    ROOT.mkdir(mode=0o755)
    subprocess.run(["/usr/bin/ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "",
                    "-f", str(ROOT / "host_key")], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    (ROOT / "authorized_keys").write_text("")
    (ROOT / "authorized_keys").chmod(0o644)
    configuration = "\n".join([
        "Port 2222", "ListenAddress 0.0.0.0", f"HostKey {ROOT / 'host_key'}",
        f"AuthorizedKeysFile {ROOT / 'authorized_keys'}", "AuthenticationMethods publickey",
        "PasswordAuthentication no", "KbdInteractiveAuthentication no", "PermitRootLogin no",
        "PermitEmptyPasswords no", "UsePAM no", "StrictModes yes", "AllowUsers rustward-target",
        "AllowAgentForwarding no", "AllowTcpForwarding no", "X11Forwarding no",
        "PermitTunnel no", "PermitTTY no", "Subsystem sftp internal-sftp", "LogLevel ERROR", ""])
    (ROOT / "sshd_config").write_text(configuration)
    PRIVILEGE_DIRECTORY.mkdir(mode=0o755, exist_ok=True)
    subprocess.run(["/usr/sbin/sshd", "-t", "-f", str(ROOT / "sshd_config")], check=True)
    public = " ".join((ROOT / "host_key.pub").read_text().split()[:2])
    packages = subprocess.run(["/usr/bin/dpkg-query", "-W", "-f=${binary:Package}\t${Version}\n",
                               "openssh-server", "python3"], check=True, capture_output=True, text=True).stdout
    (ROOT / "ready.json").write_text(json.dumps({"host_key": public, "port": 2222,
                                               "packages": packages}) + "\n")
    os.execv("/usr/sbin/sshd", ["/usr/sbin/sshd", "-D", "-e", "-f", str(ROOT / "sshd_config")])


def command(arguments):
    if arguments == ["serve"]:
        serve()
    elif arguments == ["metadata"]:
        print((ROOT / "ready.json").read_text(), end="")
    elif arguments == ["authorize"]:
        value = json.loads(sys.stdin.read())
        fields = value["public_key"].split()
        if len(fields) != 2 or fields[0] != "ssh-ed25519":
            raise ValueError("invalid synthetic public key")
        (ROOT / "authorized_keys").write_text(" ".join(fields) + "\n")
    elif len(arguments) == 2 and arguments[0] == "hold-state":
        from runpy import run_path
        state = run_path("/opt/rustward_hold.py")["state"]
        print(json.dumps(state(arguments[1])))
    elif arguments == ["runtime-result"]:
        print(RESULT.with_name("runtime.json").read_text(), end="")
    elif arguments == ["failure-result"]:
        print(RESULT.with_name("failure.json").read_text(), end="")
    elif arguments == ["result"]:
        print(RESULT.read_text(), end="")
    else:
        raise ValueError("unknown target operation")


if __name__ == "__main__":
    command(sys.argv[1:])

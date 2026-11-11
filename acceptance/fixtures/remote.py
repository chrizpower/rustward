import ipaddress
import json
from pathlib import Path
import shlex
import sys

sys.path.insert(0, str(Path(__file__).parent))
from common import begin, dependencies, check_privileges, save
from ssh_policy import CREDENTIALS, USER, TARGET, options


def arguments(runtime, work, values, connection, mode="success"):
    if mode not in ("success", "failure", "unreachable"):
        raise ValueError("unknown remote fixture mode")
    address = ipaddress.IPv4Address(connection["address"])
    if not address.is_private or address.is_multicast or address.is_unspecified:
        raise ValueError("fixture target must use a private IPv4 address")
    if connection["port"] != 2222:
        raise ValueError("unexpected fixture SSH port")
    values = dict(values, proof_path=TARGET, ansible_connection="ssh",
                  ansible_host=str(address), ansible_port=2222, ansible_user=USER,
                  ansible_python_interpreter="/usr/bin/python3",
                  ansible_ssh_private_key_file=str(CREDENTIALS / "client_key"),
                  ansible_ssh_executable=str(runtime / "bin/ssh"),
                  ansible_sftp_executable=str(runtime / "bin/sftp"),
                  ansible_ssh_transfer_method="sftp", ansible_ssh_args="", ansible_ssh_use_tty=False,
                  ansible_ssh_common_args=shlex.join(options()),
                  ansible_remote_tmp="/home/rustward-target/.ansible-tmp")
    inventory = work / "inventory.ini"
    with inventory.open("x") as output:
        output.write("[fixture]\nssh-target\n")
    playbook = "failure.yml" if mode == "failure" else "playbook.yml"
    return ["ansible-playbook", "-i", str(inventory), str(runtime / playbook),
            "--extra-vars", json.dumps(values)]


def main():
    runtime = dependencies()
    mode = sys.argv[1] if len(sys.argv) == 5 else "success"
    inputs = sys.argv[2:] if len(sys.argv) == 5 else sys.argv[1:]
    work, values = begin(inputs)
    save(work / "privileges.json", check_privileges())
    connection = json.loads((CREDENTIALS / "connection.json").read_text())
    marker = {"success": "A03", "failure": "A04_TASK_FAILURE", "unreachable": "A04_UNREACHABLE"}[mode]
    print(f"{marker}_STDOUT", flush=True)
    print(f"{marker}_STDERR", file=sys.stderr, flush=True)
    from ansible.cli.playbook import main as playbook
    playbook(arguments(runtime, work, values, connection, mode))


if __name__ == "__main__":
    main()

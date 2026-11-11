import ipaddress
import json
from pathlib import Path
import shlex
import sys

sys.path.insert(0, str(Path(__file__).parent))
from common import begin, dependencies, check_privileges, save
from ssh_policy import CREDENTIALS, USER, TARGET, options


def arguments(runtime, work, values, connection):
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
    return ["ansible-playbook", "-i", str(inventory), str(runtime / "playbook.yml"),
            "--extra-vars", json.dumps(values)]


def main():
    runtime = dependencies()
    work, values = begin(sys.argv[1:])
    save(work / "privileges.json", check_privileges())
    connection = json.loads((CREDENTIALS / "connection.json").read_text())
    print("A03_STDOUT", flush=True)
    print("A03_STDERR", file=sys.stderr, flush=True)
    from ansible.cli.playbook import main as playbook
    playbook(arguments(runtime, work, values, connection))


if __name__ == "__main__":
    main()

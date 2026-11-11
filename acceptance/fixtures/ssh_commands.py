"""Check the installed Ansible SSH and SFTP command builders without connecting."""
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).parent))
from common import TEXT, dependencies
from remote import arguments
from ssh_policy import credentials


def check(runtime, work, connection, mode="success"):
    from ansible.plugins.loader import init_plugin_loader, connection_loader
    from ansible.playbook.play_context import PlayContext

    work.mkdir()
    values = {"text": TEXT, "count": 7, "enabled": True}
    variables = json.loads(arguments(runtime, work, values, connection, mode)[-1])
    forbidden = work / ("control-sockets-" + "x" * 128)
    variables["ansible_control_path_dir"] = str(forbidden)
    init_plugin_loader()
    plugin = connection_loader.get("ssh", PlayContext(), None)
    plugin.set_options(var_options=variables)
    commands = {}
    for binary in ("ssh", "sftp"):
        command = [value.decode() for value in plugin._build_command(str(runtime / "bin" / binary), binary)]
        paths = [value for value in command if "controlpath" in value.lower()]
        if paths != ["ControlPath=none"] or forbidden.exists():
            raise RuntimeError("Ansible generated an SSH control-socket path")
        commands[binary] = command
    result = subprocess.run([*commands["ssh"], "-G", connection["address"]],
                            check=True, capture_output=True, text=True, timeout=15)
    effective = dict(line.split(None, 1) for line in result.stdout.splitlines() if " " in line)
    if effective.get("controlpath", "none") != "none":
        raise RuntimeError("SSH still has a control-socket path")
    for key, allowed in {"controlmaster": {"false", "no"}, "controlpersist": {"no", "0", "false"},
                         "stricthostkeychecking": {"true", "yes"}, "batchmode": {"true", "yes"}}.items():
        if effective.get(key) not in allowed:
            raise RuntimeError(f"unexpected effective SSH setting: {key}")
    return {"status": "passed", "commands": commands, "control_socket_created": False}


if __name__ == "__main__":
    runtime = dependencies()
    mode = sys.argv[2] if len(sys.argv) == 3 else "success"
    connection = json.loads((credentials(mode) / "connection.json").read_text())
    print(json.dumps(check(runtime, Path(sys.argv[1]), connection, mode)))

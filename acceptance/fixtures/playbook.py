import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from common import begin, dependencies

def arguments(runtime, work, values, python):
    import json
    values = dict(values, proof_path=str(work / "target.json"),
                  ansible_python_interpreter=python,
                  ansible_remote_tmp=str(work / "ansible-remote"))
    return ["ansible-playbook", "-i", str(runtime / "inventory.ini"),
            str(runtime / "playbook.yml"), "--extra-vars", json.dumps(values)]


def main():
    runtime = dependencies()
    work, values = begin(sys.argv[1:])
    print("A01_STDOUT", flush=True)
    print("A01_STDERR", file=sys.stderr, flush=True)
    from ansible.cli.playbook import main as playbook
    playbook(arguments(runtime, work, values, sys.executable))


if __name__ == "__main__":
    main()

"""Prepare only synthetic acceptance data before controller execution."""
import argparse
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import sys
import sysconfig

from manifest import copy_tree, host_manifest, source, tree_manifest


def write(path, value):
    with Path(path).open("x", encoding="utf-8") as output:
        output.write(value)
    Path(path).chmod(0o644)


def identify(tool, kind, path):
    result = subprocess.run([str(tool), "id", kind, str(path)], check=True,
                            capture_output=True, text=True, timeout=60)
    identity = result.stdout.strip()
    if len(identity) != 64 or any(char not in "0123456789abcdef" for char in identity):
        raise ValueError("invalid manifest identity")
    return identity


def configuration(root, runtime, remote=False, directory="configuration"):
    definitions = root / directory
    definitions.mkdir(mode=0o755)
    (definitions / "tasks").mkdir(mode=0o755)
    (definitions / "schedules").mkdir(mode=0o755)
    write(definitions / "rustward.toml", "version=2\n[storage]\n"
          f"state_database={json.dumps(str(root / 'state/state.db'))}\n"
          f"log_directory={json.dumps(str(root / 'logs'))}\n")
    tasks = [("local-playbook", "playbook.py", None), ("ordinary", "ordinary.py", "complete"),
             ("recovery", "ordinary.py", "hold")]
    if remote:
        tasks += [("remote-playbook", "remote.py", None),
                  ("remote-failure", "remote.py", "failure"),
                  ("remote-unreachable", "remote.py", "unreachable"),
                  ("remote-queued", "remote.py", "queued"), ("remote-cancel", "remote.py", "cancel"),
                  ("remote-timeout", "remote.py", "timeout"),
                  ("remote-recovery", "remote.py", "recovery"),
                  ("remote-async", "remote.py", "async"),
                  ("remote-runtime-a", "remote.py", "runtime-a"), ("remote-runtime-b", "remote.py", "runtime-b")]
    for name, script, mode in tasks:
        arguments = [{"literal": "-I"}, {"literal": "-B"}, {"literal": script}]
        if mode:
            arguments.append({"literal": mode})
        arguments += [{"input": name} for name in ("text", "count", "enabled")]
        arguments = ",".join("{" + ",".join(f"{key}={json.dumps(value)}"
                            for key, value in argument.items()) + "}" for argument in arguments)
        write(definitions / "tasks" / f"{name}.toml",
              f"label={json.dumps(name)}\nexecutable={json.dumps(str(runtime / 'bin/python3'))}\n"
              f"working_directory={json.dumps(str(runtime))}\narguments=[{arguments}]\n"
              f"timeout_seconds={60 if mode == 'timeout' else 180}\ncleanup_grace_seconds=2\n"
              "inputs=[{id='text',type='string',required=true},"
              "{id='count',type='integer',required=true},"
              "{id='enabled',type='boolean',required=true}]\n")


def create_root(root, account):
    root.mkdir(mode=0o755)
    for name in ("runtimes", "hosts"):
        (root / name).mkdir(mode=0o755)
    for name in ("work", "logs", "state", "evidence"):
        path = root / name
        path.mkdir(mode=0o700)
        os.chown(path, account.pw_uid, account.pw_gid)


def prepare_shared(root, source_root, user, revision):
    os.umask(0o022)
    account = pwd.getpwnam(user)
    if os.geteuid() != 0 or account.pw_uid == 0:
        raise PermissionError("fixture preparation requires root and a non-root execution user")
    root, source_root = Path(root), Path(source_root)
    if not root.is_absolute() or root != root.resolve() or root.exists():
        raise ValueError("use a new absolute fixture root without symlinks")
    fixture = json.loads((source_root / "fixture.json").read_text())
    if fixture["source_revision"] != revision or fixture["execution_uid"] != account.pw_uid:
        raise ValueError("shared fixture selection changed")
    create_root(root, account)
    configuration(root, source_root / "runtimes" / fixture["runtime_id"], remote=True)
    fixture["catalog_root"] = str(source_root)
    write(root / "fixture.json", json.dumps(fixture, indent=2) + "\n")
    return fixture


def prepare(root, tools, user, revision, remote=False):
    if os.geteuid() != 0:
        raise PermissionError("fixture preparation requires root ownership")
    os.umask(0o022)
    root, tools = Path(root), Path(tools).resolve(strict=True)
    if not root.is_absolute() or root != root.resolve() or root.exists():
        raise ValueError("use a new absolute fixture root without symlinks")
    account = pwd.getpwnam(user)
    if account.pw_uid == 0:
        raise ValueError("execution user must be non-root")
    create_root(root, account)
    stage = root / "staging"
    stage.mkdir(mode=0o755)
    (stage / "bin").mkdir(mode=0o755)
    shutil.copyfile(Path(sys.executable).resolve(strict=True), stage / "bin/python3")
    (stage / "bin/python3").chmod(0o755)
    (stage / "lib").mkdir(mode=0o755)
    stdlib = Path(sysconfig.get_path("stdlib"))
    copy_tree(stdlib, stage / "lib" / stdlib.name)
    packages = stage / "lib/python3"
    if not packages.exists():
        packages.mkdir(mode=0o755)
    copy_tree(Path("/usr/lib/python3/dist-packages"), packages / "dist-packages")
    programs = ["mkdir", "chmod", "rm", "sleep", "cat"]
    if remote:
        programs += ["ssh", "sftp"]
    for name in programs:
        shutil.copyfile(Path("/usr/bin") / name, stage / "bin" / name)
        (stage / "bin" / name).chmod(0o755)
    for path in sorted((Path(__file__).parent / "fixtures").iterdir()):
        if path.name == "AI.md" or path.is_dir():
            continue
        shutil.copyfile(path, stage / path.name)
        (stage / path.name).chmod(0o644)
    (stage / "collections").mkdir(mode=0o755)
    host = root / "host.pending.toml"
    write(host, source(host_manifest(stage, tools / "rustward-exec")))
    host_id = identify(tools / "acceptance", "host", host)
    host.rename(root / "hosts" / f"{host_id}.toml")
    directories, files = tree_manifest(stage)
    write(stage / "runtime.toml", source({"version": 1, "host": host_id,
          "directories": directories, "files": files}))
    runtime_id = identify(tools / "acceptance", "runtime", stage / "runtime.toml")
    runtime = root / "runtimes" / runtime_id
    stage.rename(runtime)
    configuration(root, runtime, remote=remote)
    ansible = subprocess.run([str(runtime / "bin/python3"), "-I", "-B", "-c",
                             "import sys;sys.path.insert(0,'lib/python3/dist-packages');"
                             "from ansible.release import __version__;print(__version__)"],
                            cwd=runtime, check=True, capture_output=True, text=True).stdout.strip()
    packages = subprocess.run(["/usr/bin/dpkg-query", "-W",
                               "-f=${binary:Package}\t${Version}\n"], check=True,
                              capture_output=True, text=True).stdout
    fixture = {"version": 1, "runtime_id": runtime_id, "host_id": host_id,
               "preparation_uid": 0, "execution_uid": account.pw_uid,
               "source_revision": revision, "packages": packages,
               "python_version": sys.version, "ansible_version": ansible}
    write(root / "fixture.json", json.dumps(fixture, sort_keys=True, indent=2) + "\n")
    return fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--user", default="rustward")
    parser.add_argument("--source-revision", required=True)
    options = parser.parse_args()
    fixture = prepare(options.root, options.tools, options.user, options.source_revision)
    print(json.dumps({key: fixture[key] for key in ("runtime_id", "host_id")}))


if __name__ == "__main__":
    main()

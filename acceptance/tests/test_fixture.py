import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

ACCEPTANCE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ACCEPTANCE))
sys.path.insert(0, str(ACCEPTANCE / "fixtures"))
import common
import manifest
import prepare
import playbook
import inside


class FixtureTests(unittest.TestCase):
    def test_wrapper_preserves_types_quotes_and_argument_boundaries(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            with patch.dict(os.environ, {"RUSTWARD_WORK_DIRECTORY": str(work)}):
                result, values = common.begin([common.TEXT, "7", "true"])
            self.assertEqual(result, work)
            self.assertIs(type(values["count"]), int)
            self.assertIs(type(values["enabled"]), bool)
            argv = playbook.arguments(Path("/runtime=a"), work, values, "/runtime=a/bin/python3")
            self.assertEqual(argv[:3], ["ansible-playbook", "-i", "/runtime=a/inventory.ini"])
            typed = json.loads(argv[-1])
            self.assertEqual(typed["text"], common.TEXT)
            self.assertIs(typed["enabled"], True)
            self.assertEqual(len((work / "starts.jsonl").read_text().splitlines()), 1)
            with self.assertRaises(FileExistsError):
                common.save(work / "inputs.json", {})
        for bad in ("FALSE", "yes", "1"):
            with self.assertRaises(ValueError):
                common.begin([common.TEXT, "7", bad])

    def test_configuration_and_manifests_use_production_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / "runtime with space=1"
            prepare.configuration(root, runtime)
            task = tomllib.loads((root / "configuration/tasks/local-playbook.toml").read_text())
            self.assertEqual(task["executable"], str(runtime / "bin/python3"))
            self.assertEqual(task["arguments"][-3:], [{"input": name} for name in ("text", "count", "enabled")])
            self.assertNotIn("id", task)
            self.assertEqual(task["cleanup_grace_seconds"], 2)
            value = {"version": 1, "host": "a" * 64, "directories": ["bin"],
                     "files": {"bin/task": {"sha256": "b" * 64, "executable": True}}}
            self.assertEqual(tomllib.loads(manifest.source(value)), value)
            host = {"version": 1, "launcher": "/launcher", "files": value["files"], "links": {"/lib": "usr/lib"}}
            self.assertEqual(tomllib.loads(manifest.source(host)), host)

    def test_runtime_copy_dereferences_links_and_rejects_cycles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / "file").write_text("synthetic")
            (source / "alias").symlink_to("file")
            (source / "__pycache__").mkdir()
            (source / "__pycache__/excluded.pyc").write_bytes(b"disposable")
            target = root / "target"
            manifest.copy_tree(source, target)
            self.assertFalse((target / "alias").is_symlink())
            self.assertFalse((target / "__pycache__").exists())
            folders, files = manifest.tree_manifest(target)
            self.assertEqual(folders, [])
            self.assertEqual(files["alias"]["sha256"], files["file"]["sha256"])
            (source / "cycle").symlink_to(source, target_is_directory=True)
            with self.assertRaises(ValueError):
                manifest.copy_tree(source, root / "cycle-output")
            (target / "link").symlink_to(target, target_is_directory=True)
            with self.assertRaises(ValueError):
                manifest.tree_manifest(target)

    def test_host_dependencies_include_explicit_aliases_and_reject_missing_libraries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "binary"
            binary.write_bytes(b"synthetic")
            binary.chmod(0o755)
            alias = root / "alias"
            alias.symlink_to("binary")
            host = manifest.Host(binary)
            host.pin(alias)
            self.assertEqual(host.value["links"][str(alias)], "binary")
            self.assertIn(str(binary), host.value["files"])
            result = subprocess.CompletedProcess([], 0, f"libtest => {alias} (0x123)\n", "")
            with patch("manifest.subprocess.run", return_value=result):
                host.libraries(binary)
            missing = subprocess.CompletedProcess([], 0, "libtest => not found\n", "")
            with patch("manifest.subprocess.run", return_value=missing), self.assertRaises(RuntimeError):
                host.libraries(binary)
            broken = subprocess.CompletedProcess([], 1, "", "inspection failed")
            with patch("manifest.subprocess.run", return_value=broken), self.assertRaises(RuntimeError):
                host.libraries(binary)

    def test_privilege_drop_keeps_the_service_non_root_and_does_not_use_a_shell(self):
        import pwd
        account = pwd.getpwuid(os.geteuid())
        argv = inside.user_command(account, ["/synthetic/tool", "two words", "$HOME;*"])
        self.assertIn("--bounding-set=-all", argv)
        self.assertIn("--ambient-caps=-all", argv)
        self.assertIn("--no-new-privs", argv)
        self.assertEqual(argv[-3:], ["/synthetic/tool", "two words", "$HOME;*"])

    def test_no_docker_or_root_is_needed_for_mechanical_fixture_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch("prepare.os.geteuid", return_value=1000), self.assertRaises(PermissionError):
                prepare.prepare(Path(directory) / "new", Path(directory), "unused", "synthetic")
        with patch("prepare.subprocess.run", return_value=subprocess.CompletedProcess([], 0, "invalid", "")), self.assertRaises(ValueError):
            prepare.identify(Path("/tool"), "runtime", Path("/manifest"))

class PrivilegeTests(unittest.TestCase):
    def test_workload_requires_no_new_privileges_and_zero_held_capabilities(self):
        fields = {name: "0" for name in ("CapEff", "CapPrm", "CapInh", "CapAmb")}
        fields["NoNewPrivs"] = "1"
        for change in (None, "CapEff", "CapPrm", "CapInh", "CapAmb", "NoNewPrivs"):
            value = dict(fields)
            if change:
                value[change] = "0" if change == "NoNewPrivs" else "1"
            text = "\n".join(f"{key}: {field}" for key, field in value.items())
            with patch("common.Path.read_text", return_value=text), patch("common.os.getuid", return_value=1000), patch("common.os.geteuid", return_value=1000):
                if change:
                    with self.assertRaises(RuntimeError):
                        common.check_privileges()
                else:
                    self.assertEqual(common.check_privileges(), value)

class ProcessCleanupTests(unittest.TestCase):
    def test_harness_children_do_not_survive_their_controller_process(self):
        import time
        code = "import subprocess,sys;child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);print(child.pid,flush=True)"
        result = inside.invoke([sys.executable, "-I", "-B", "-c", code], timeout=5, capture=True)
        self.assertEqual(result.returncode, 0)
        pid = int(result.stdout.strip())
        deadline = time.monotonic() + 3
        while True:
            try:
                state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
            except (FileNotFoundError, ProcessLookupError):
                break
            if state in ("Z", "X"):
                break
            self.assertLess(time.monotonic(), deadline, "harness child survived cleanup")
            time.sleep(0.01)


if __name__ == "__main__":
    unittest.main()

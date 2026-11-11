import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import container


class RunnerTests(unittest.TestCase):
    def test_context_excludes_git_and_unrelated_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository = root / "repository"
            repository.mkdir()
            for name in ("Cargo.toml", "Cargo.lock"):
                (repository / name).write_text("synthetic")
            for name in ("src", "acceptance", "examples/acceptance", "tests"):
                (repository / name).mkdir(parents=True)
            (repository / "src/lib.rs").write_text("synthetic")
            (repository / "outside").write_text("synthetic unrelated data")
            for name in ("systemd_host.rs", "runtime_host.rs"):
                (repository / "tests" / name).write_text("synthetic test")
            source = container.snapshot(repository, root / "context")
            self.assertFalse((root / "context/outside").exists())
            self.assertEqual(len(source["revision"]), 64)
            self.assertEqual(len(source["files"]), 5)
            self.assertTrue(container.excluded(Path("__pycache__")))
            self.assertTrue(container.excluded(Path("AI.md")))
            (repository / "src/alias").symlink_to("lib.rs")
            with self.assertRaises(ValueError):
                container.snapshot(repository, root / "other-context")

    def test_container_has_private_namespaces_and_no_host_mounts_or_network(self):
        args = container.run_arguments("synthetic", "sha256:" + "a" * 64)
        self.assertIn("--privileged", args)
        self.assertIn("--cgroupns=private", args)
        self.assertIn("--network=none", args)
        self.assertIn("--pull=never", args)
        self.assertNotIn("--volume", args)
        self.assertNotIn("--mount", args)
        self.assertNotIn("--pid=host", args)
        self.assertNotIn("--cgroupns=host", args)

    def test_absent_docker_reports_not_run_and_returns_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "evidence"
            with patch("container.shutil.which", return_value=None):
                status = container.execute(Path(directory), evidence, "rust", "debian")
            self.assertNotEqual(status, 0)
            report = json.loads((evidence / "result.json").read_text())
            self.assertEqual(report["status"], "blocked")
            self.assertEqual(report["cases"], {"A01": "not run", "A02": "not run"})

    def test_docker_commands_are_argument_vectors_with_durable_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            docker = container.Docker(Path(directory))
            with patch("container.subprocess.run", return_value=subprocess.CompletedProcess([], 0)) as run:
                docker.call(["inspect", "two words", "$(synthetic)"])
            self.assertEqual(run.call_args.args[0], ["docker", "inspect", "two words", "$(synthetic)"])
            self.assertFalse(run.call_args.kwargs.get("shell", False))
            self.assertTrue((Path(directory) / "docker-01.command.json").is_file())
            with patch("container.subprocess.run", return_value=subprocess.CompletedProcess([], 1)), self.assertRaises(RuntimeError):
                docker.call(["failure"])

class CleanupTests(unittest.TestCase):
    def attempt(self, directory, harness_code=0, complete=True, logs_timeout=False, remote=False, scan_failed=False):
        root = Path(directory)
        evidence = root / "evidence"
        calls = []

        class FakeDocker:
            def __init__(self, unused):
                pass

            def call(self, args, **unused):
                args = list(map(str, args))
                calls.append(args)
                if args[0] == "info":
                    return 0, json.dumps({"OSType": "linux", "CgroupVersion": "2", "SecurityOptions": []})
                if args[0] == "build":
                    Path(args[args.index("--iidfile") + 1]).write_text("sha256:" + "a" * 64)
                if args[0] == "exec" and "--root" in args:
                    return harness_code, ""
                if args[0] == "logs" and logs_timeout:
                    raise subprocess.TimeoutExpired(args, 1)
                if args[0] == "cp":
                    output = evidence / "fixture/evidence"
                    output.mkdir(parents=True)
                    if complete:
                        for name in ("A01", "A02", "local-recovery", "host-runner", "legacy-host"):
                            (output / f"{name}.json").write_text('{"status":"passed"}')
                        (output / "result.json").write_text(json.dumps({"status": "passed", "cases": ["A01", "A02"]}))
                        (evidence / "fixture/fixture.json").write_text(json.dumps({"runtime_id": "a" * 64, "host_id": "b" * 64, "packages": "synthetic", "source_revision": "c" * 64}))
                return 0, ""

        class FakeRemote:
            def __init__(self, *args):
                pass
            def run(self, *args):
                pass
            def scan(self):
                calls.append(["credential-scan"])
                if scan_failed:
                    raise RuntimeError("credential export check failed")
            def cleanup(self):
                calls.append(["ssh-cleanup"])
        with patch("container.shutil.which", return_value="/synthetic/docker"), patch("container.Docker", FakeDocker), patch("container.snapshot", return_value={"revision": "c" * 64}), patch("ssh_runner.Remote", FakeRemote):
            code = container.execute(root, evidence, "rust", "debian", remote=remote)
        return code, json.loads((evidence / "result.json").read_text()), calls

    def test_success_requires_collected_pass_evidence_and_stops_before_removal(self):
        with tempfile.TemporaryDirectory() as directory:
            code, result, calls = self.attempt(directory)
        self.assertEqual(code, 0)
        self.assertEqual(result["cases"], {"A01": "passed", "A02": "passed"})
        operations = [args[0] for args in calls]
        self.assertLess(operations.index("cp"), operations.index("stop"))
        self.assertLess(operations.index("stop"), operations.index("rm"))

    def test_exit_zero_without_pass_evidence_never_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            code, result, calls = self.attempt(directory, complete=False)
        self.assertNotEqual(code, 0)
        self.assertEqual(result["cases"], {"A01": "not run", "A02": "not run"})
        self.assertIn("retained_container", result)
        self.assertIn("stop", [args[0] for args in calls])
        self.assertNotIn("rm", [args[0] for args in calls])

    def test_credential_scan_failure_blocks_raw_copy_and_keeps_stopped_controller(self):
        with tempfile.TemporaryDirectory() as directory:
            code, result, calls = self.attempt(directory, remote=True, scan_failed=True)
        self.assertNotEqual(code, 0)
        operations = [args[0] for args in calls]
        self.assertIn("credential-scan", operations)
        self.assertNotIn("cp", operations)
        self.assertNotIn("rm", operations)
        self.assertIn("stop", operations)
        self.assertIn("ssh-cleanup", operations)
        self.assertIn("retained_container", result)

    def test_diagnostic_timeout_still_collects_data_and_stops_the_container(self):
        with tempfile.TemporaryDirectory() as directory:
            code, result, calls = self.attempt(directory, harness_code=1, logs_timeout=True)
        self.assertNotEqual(code, 0)
        self.assertIn("cp", [args[0] for args in calls])
        self.assertIn("stop", [args[0] for args in calls])
        self.assertIn("retained_container", result)


if __name__ == "__main__":
    unittest.main()

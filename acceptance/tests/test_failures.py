import json
from pathlib import Path
import sys
import tempfile
import unittest
import subprocess
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures.remote import arguments
from ssh_runner import Remote, a04_status
import ssh_inside


class FailureTests(unittest.TestCase):
    def test_failure_modes_use_fixed_connection_policy_and_separate_playbooks(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for mode, playbook in (("failure", "failure.yml"), ("unreachable", "playbook.yml")):
                work = root / mode
                work.mkdir()
                argv = arguments(Path("/runtime"), work, {"text": "quoted 'text'", "count": 7, "enabled": True},
                                 {"address": "172.19.0.2", "port": 2222}, mode)
                self.assertEqual(argv[3], f"/runtime/{playbook}")
                values = json.loads(argv[-1])
                self.assertIs(values["enabled"], True)
                self.assertEqual(values["ansible_port"], 2222)
                self.assertIn("StrictHostKeyChecking=yes", values["ansible_ssh_common_args"])
                self.assertIn("ControlPath=none", values["ansible_ssh_common_args"])
                self.assertNotIn("ansible_password", values)
            with self.assertRaises(ValueError):
                arguments(Path("/runtime"), root, {}, {}, "unknown")

    def test_unreachable_setup_requires_docker_to_confirm_no_target_process(self):
        for state, accepted in (({"Running": False, "Pid": 0}, True),
                                ({"Running": True, "Pid": 42}, False),
                                ({"Running": False, "Pid": 42}, False), ({}, False)):
            with tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                calls = []
                class Docker:
                    def call(self, args, **kwargs):
                        calls.append(args)
                        return (0, json.dumps(state)) if args[0] == "inspect" else (0, "")
                runner = Remote(Docker(), "test", root)
                if accepted:
                    runner.stop_target()
                    self.assertEqual(json.loads((root / "target-stopped.json").read_text())["status"], "passed")
                else:
                    with self.assertRaises(RuntimeError):
                        runner.stop_target()
                    self.assertFalse((root / "target-stopped.json").exists())
                self.assertEqual([args[0] for args in calls], ["stop", "inspect"])

    def test_a04_needs_both_case_reports_and_host_cleanup_reports(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            evidence = root / "evidence"
            evidence.mkdir()
            with patch("ssh_inside.ROOT", root):
                with self.assertRaises(OSError):
                    ssh_inside.complete_failures()
                for name in ("A04-task-failure", "A04-unreachable", "A04-task-failure-host", "A04-unreachable-host"):
                    (evidence / f"{name}.json").write_text('{"status":"passed"}')
                report = evidence / "A04-unreachable.json"
                report.write_text('{"status":"failed"}')
                with self.assertRaises(RuntimeError):
                    ssh_inside.complete_failures()
                self.assertFalse((evidence / "A04.json").exists())
                report.write_text('{"status":"passed"}')
                ssh_inside.complete_failures()
                self.assertEqual(a04_status(evidence), "passed")
                with self.assertRaises(FileExistsError):
                    ssh_inside.complete_failures()

    def test_host_cleanup_failure_cannot_leave_a_pass_report(self):
        for cleanup_fails in (False, True):
            with tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                (root / "evidence").mkdir()
                account = SimpleNamespace(pw_uid=1000, pw_gid=1000, pw_dir="/home/synthetic")
                def checked(command, **kwargs):
                    if cleanup_fails and command[:2] == ["/usr/bin/systemctl", "stop"]:
                        raise subprocess.CalledProcessError(1, command)
                with patch("ssh_inside.ROOT", root), patch("ssh_inside.pwd.getpwnam", return_value=account), patch("ssh_inside.checked", side_effect=checked), patch("ssh_inside.wait_manager"), patch("ssh_inside.stop_fixture_units"):
                    if cleanup_fails:
                        with self.assertRaises(subprocess.CalledProcessError):
                            ssh_inside.run_failure("task")
                    else:
                        ssh_inside.run_failure("task")
                report = json.loads((root / "evidence/A04-task-failure-host.json").read_text())
                self.assertEqual(report["status"], "failed" if cleanup_fails else "passed")

    def test_partial_or_failed_a04_is_never_reported_as_passed(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.assertEqual(a04_status(root), "not run")
            (root / "A04-task-failure-admission.json").write_text('{}')
            self.assertEqual(a04_status(root), "incomplete")
            (root / "A04-task-failure.json").write_text('{"status":"passed"}')
            self.assertEqual(a04_status(root), "incomplete")
            (root / "A04-unreachable.json").write_text('{"status":"failed"}')
            self.assertEqual(a04_status(root), "failed")


if __name__ == "__main__":
    unittest.main()

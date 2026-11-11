import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import inside


class ReadinessTests(unittest.TestCase):
    def account(self):
        return SimpleNamespace(pw_uid=1000, pw_gid=1000, pw_dir="/home/synthetic")

    def ready(self, uid=1000, version="257.13"):
        return subprocess.CompletedProcess([], 0, json.dumps({"status": "ready", "uid": uid,
                                                            "version": version}), "")

    def test_waits_for_authenticated_manager_before_starting_any_tests(self):
        unavailable = subprocess.CompletedProcess([], 1, "", "authenticated user bus: Unavailable")
        now = [0.0]
        with tempfile.TemporaryDirectory() as directory:
            with patch("inside.time.monotonic", side_effect=lambda: now[0]), patch("inside.time.sleep", side_effect=lambda delay: now.__setitem__(0, now[0] + delay)), patch("inside.invoke", side_effect=[unavailable, self.ready()]) as invoke:
                inside.wait_manager(self.account(), Path("/tools"), Path(directory))
            report = json.loads((Path(directory) / "manager-readiness.json").read_text())
        self.assertEqual(report["status"], "ready")
        self.assertEqual(len(report["attempts"]), 2)
        self.assertIn("Unavailable", report["attempts"][0]["stderr"])
        for call in invoke.call_args_list:
            self.assertEqual(call.args[0][-2:], ["/tools/acceptance", "probe-manager"])
            self.assertNotIn("--ignored", call.args[0])
            self.assertNotIn("run", call.args[0])

    def test_timeout_keeps_failure_evidence_and_does_not_pass(self):
        now = [0.0]
        with tempfile.TemporaryDirectory() as directory:
            with patch("inside.time.monotonic", side_effect=lambda: now[0]), patch("inside.time.sleep", side_effect=lambda delay: now.__setitem__(0, now[0] + delay)), patch("inside.invoke", return_value=subprocess.CompletedProcess([], 1, "", "unavailable")), self.assertRaises(RuntimeError):
                inside.wait_manager(self.account(), Path("/tools"), Path(directory), timeout=1, interval=0.5)
            report = json.loads((Path(directory) / "manager-readiness.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(len(report["attempts"]), 2)

    def test_wrong_uid_or_version_cannot_be_treated_as_ready(self):
        for reply in (self.ready(uid=1001), self.ready(version="256")):
            with tempfile.TemporaryDirectory() as directory:
                with patch("inside.invoke", return_value=reply), self.assertRaises(RuntimeError):
                    inside.wait_manager(self.account(), Path("/tools"), Path(directory))
                report = json.loads((Path(directory) / "manager-readiness.json").read_text())
                self.assertEqual(report["status"], "failed")

    def test_hung_probe_is_bounded_and_recorded(self):
        now = [0.0]

        def invoke(*unused, **options):
            now[0] += options["timeout"]
            raise subprocess.TimeoutExpired("probe-manager", options["timeout"])

        with tempfile.TemporaryDirectory() as directory:
            with patch("inside.time.monotonic", side_effect=lambda: now[0]), patch("inside.time.sleep"), patch("inside.invoke", side_effect=invoke), self.assertRaises(RuntimeError):
                inside.wait_manager(self.account(), Path("/tools"), Path(directory), timeout=1)
            report = json.loads((Path(directory) / "manager-readiness.json").read_text())
        self.assertEqual(now[0], 1)
        self.assertEqual(report["attempts"], [{"status": "timeout"}])


if __name__ == "__main__":
    unittest.main()

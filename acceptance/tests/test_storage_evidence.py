import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import remaining_runner
import ssh_inside
import storage_evidence
from ssh_credentials import scan


def fixture(root, stdout=b"out", stderr=b"err"):
    (root / "evidence").mkdir(parents=True)
    run_id = "a" * 64
    logs = root / "logs" / run_id
    logs.mkdir(parents=True)
    (logs / "stdout.log").write_bytes(stdout)
    (logs / "stderr.log").write_bytes(stderr)
    value = {"status":"passed", "run_id":run_id, "record":{
        "log_bytes":len(stdout)+len(stderr), "output_evidence":{
            "stdout":{"retained_bytes":len(stdout)}, "stderr":{"retained_bytes":len(stderr)}}}}
    (root / "evidence/A10.json").write_text(json.dumps(value))
    (root / "evidence/A10-host.json").write_text('{"status":"passed","writers_stopped":true}')
    return logs


class StorageEvidenceTests(unittest.TestCase):
    def test_export_preserves_bytes_and_does_not_depend_on_the_mounted_source(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            logs = fixture(root, b"x" * 258048, b"A10_STDERR\n")
            storage_evidence.export(root)
            for path in logs.iterdir(): path.unlink()
            logs.rmdir()
            storage_evidence.validate(root)
            value = storage_evidence.read(root / "evidence/A10-export.json")
            self.assertEqual(value["files"]["a" * 64 + "/stdout.log"]["bytes"], 258048)

    def test_export_refuses_active_writers_and_links(self):
        for kind in ("active", "file-link", "directory-link", "pipe"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                logs = fixture(root)
                if kind == "active":
                    (root / "evidence/A10-host.json").write_text('{"writers_stopped":false}')
                elif kind == "directory-link":
                    (root / "logs/link").symlink_to(logs, target_is_directory=True)
                else:
                    path = logs / "stdout.log"
                    path.unlink()
                    if kind == "file-link": path.symlink_to(logs / "stderr.log")
                    else: os.mkfifo(path)
                with self.assertRaises(RuntimeError): storage_evidence.export(root)
                self.assertFalse((root / "evidence/A10-export.json").exists())

    def test_bundle_rejects_missing_short_changed_and_linked_output(self):
        for kind in ("missing", "short", "changed", "linked", "wrong-total", "wrong-run", "missing-manifest"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                root = Path(folder) / "remaining-A10-fixture"
                fixture(root)
                storage_evidence.export(root)
                path = root / "exported-logs" / ("a" * 64) / "stdout.log"
                if kind == "missing": path.unlink()
                elif kind == "short": path.write_bytes(b"o")
                elif kind == "changed": path.write_bytes(b"bad")
                elif kind == "linked":
                    path.unlink()
                    path.symlink_to(path.parent / "stderr.log")
                elif kind == "missing-manifest": (root / "evidence/A10-export.json").unlink()
                else:
                    value = storage_evidence.read(root / "evidence/A10.json")
                    if kind == "wrong-total": value["record"]["log_bytes"] = 100
                    else: value["run_id"] = "../logs"
                    (root / "evidence/A10.json").write_text(json.dumps(value))
                with self.assertRaises((OSError, RuntimeError)):
                    storage_evidence.validate(root)
                self.assertEqual(remaining_runner.status(root.parent, "A10"), "incomplete")

    def test_empty_stream_is_required_even_when_no_bytes_were_retained(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            fixture(root, b"", b"")
            storage_evidence.export(root)
            storage_evidence.validate(root)
            (root / "exported-logs" / ("a" * 64) / "stderr.log").unlink()
            with self.assertRaises(OSError): storage_evidence.validate(root)

    def test_failed_case_still_copies_available_diagnostic_output(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            fixture(root)
            (root / "evidence/A10.json").unlink()
            storage_evidence.export(root)
            self.assertTrue((root / "exported-logs" / ("a" * 64) / "stdout.log").is_file())

    def test_scan_checks_the_saved_copy_after_writer_confirmation(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            marker = b"synthetic-private-marker-" * 4
            fixture(root, marker)
            calls = []
            def confirmed(): calls.append("stopped")
            def checked_scan(roots):
                self.assertEqual(calls, ["stopped"])
                self.assertTrue((root / "evidence/A10-export.json").is_file())
                return scan([root], marker)
            with patch.object(ssh_inside.os, "geteuid", return_value=0), \
                 patch.object(ssh_inside.sys, "argv", ["fixture", "scan"]), \
                 patch("stop_inside.confirm_stopped", side_effect=confirmed), \
                 patch.object(remaining_runner, "CASES", {"A10":{"root":str(root)}}), \
                 patch.object(ssh_inside, "export_status", side_effect=checked_scan), \
                 redirect_stdout(io.StringIO()) as output:
                self.assertEqual(ssh_inside.main(), 1)
            findings = json.loads(output.getvalue())["findings"]
            self.assertTrue(any(item["file"].startswith("exported-logs/") for item in findings))
            self.assertNotIn(marker.decode(), output.getvalue())

import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import nspawn


class NspawnTests(unittest.TestCase):
    def run_case(self, root, report, exit_code=0, ssh_error=None):
        def snapshot(_repository, context, extra_files):
            self.assertEqual(extra_files, ("rust-toolchain.toml",))
            (context / "acceptance").mkdir(parents=True)
            return {"revision": "a" * 64}

        def build_tools(_context, evidence):
            tools = evidence / "tools"
            tools.mkdir()
            return tools

        def run(arguments, stdin, stdout, stderr, timeout):
            self.assertEqual(arguments, ["synthetic-runner"])
            with tarfile.open(fileobj=stdin, mode="r:") as input_archive:
                self.assertEqual(input_archive.extractfile("source-revision").read(), b"a" * 64 + b"\n")
            with tarfile.open(fileobj=stdout, mode="w") as output:
                data = json.dumps(report).encode()
                entry = tarfile.TarInfo("result.json")
                entry.size = len(data)
                output.addfile(entry, io.BytesIO(data))
            return subprocess.CompletedProcess(arguments, exit_code)

        with patch.object(nspawn, "snapshot", snapshot), patch.object(nspawn, "build_tools", build_tools), \
                patch.object(nspawn, "validate_bundle") as validate, patch.object(nspawn, "validate_ssh", side_effect=ssh_error) as validate_ssh, \
                patch.object(nspawn.subprocess, "run", run):
            result = nspawn.execute(root, "synthetic-runner")
        return result, validate, validate_ssh

    def test_pass_requires_the_built_revision_and_complete_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "evidence"
            status, validate, validate_ssh = self.run_case(root, {"status": "passed", "source_revision": "a" * 64})
            self.assertEqual(status, 0)
            validate.assert_called_once_with(root / "container/fixture", require_daemon=True,
                                             require_scheduler=True, require_web=True, require_pilot=True)
            ssh = root / "container/ssh"
            validate_ssh.assert_called_once_with(ssh / "ssh-fixture", ssh)

    def test_missing_ssh_evidence_blocks_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "evidence"
            report = {"status": "passed", "source_revision": "a" * 64}
            status, _, validate_ssh = self.run_case(root, report, ssh_error=FileNotFoundError("A03.json"))
            self.assertEqual(status, 1)
            validate_ssh.assert_called_once()
            self.assertIn("A03.json", json.loads((root / "result.json").read_text())["reason"])

    def test_wrong_revision_or_failed_runner_blocks_push_and_keeps_evidence(self):
        for revision, exit_code in (("b" * 64, 0), ("a" * 64, 1)):
            with self.subTest(revision=revision, exit_code=exit_code), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "evidence"
                report = {"status": "passed", "source_revision": revision}
                status, validate, validate_ssh = self.run_case(root, report, exit_code)
                self.assertEqual(status, 1)
                validate.assert_not_called()
                validate_ssh.assert_not_called()
                self.assertEqual(json.loads((root / "container/result.json").read_text()), report)


if __name__ == "__main__":
    unittest.main()

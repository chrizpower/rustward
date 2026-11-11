import json
import io
from contextlib import redirect_stdout
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import hold, remote, remote_state
import prepare
import stop_inside
import ssh_inside
import stopping_runner
from stopping_runner import Stopping, ROOTS, expected_inputs


class StoppingTests(unittest.TestCase):
    def test_marker_checks_process_start_time_and_treats_zombies_as_ended(self):
        with tempfile.TemporaryDirectory() as folder:
            marker = Path(folder) / "marker.json"
            with patch.object(hold, "marker", return_value=marker):
                self.assertEqual(hold.state("cancel"), {"kind": "cancel", "state": "absent"})
                marker.write_text(json.dumps({"kind": "cancel", "pid": 42, "start_time": "100", "inputs": expected_inputs()}))
                for process, expected in (({"start_time": "100", "state": "S"}, "alive"),
                                          ({"start_time": "101", "state": "S"}, "ended"),
                                          ({"start_time": "100", "state": "Z"}, "ended"), (None, "ended")):
                    with patch.object(hold, "identity", return_value=process):
                        self.assertEqual(hold.state("cancel")["state"], expected)
            with self.assertRaises(ValueError):
                hold.marker("../invalid")

    def test_holding_marker_cannot_be_overwritten_by_another_start(self):
        with tempfile.TemporaryDirectory() as folder:
            marker = Path(folder) / "marker.json"
            with patch.object(hold, "marker", return_value=marker), patch.object(hold, "identity", return_value={"start_time":"100"}), patch.object(hold.time, "monotonic", side_effect=[0, 301]):
                with self.assertRaises(SystemExit): hold.run("cancel", expected_inputs(), "a" * 64)
            before = marker.read_bytes()
            self.assertEqual(json.loads(before)["run_id"], "a" * 64)
            with patch.object(hold, "marker", return_value=marker), patch.object(hold, "identity", return_value={"start_time":"101"}):
                with self.assertRaises(FileExistsError): hold.run("cancel", expected_inputs(), "b" * 64)
            self.assertEqual(marker.read_bytes(), before)

    def test_stopping_arguments_keep_separate_host_pin_and_fixed_private_key(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for kind in ("queued", "cancel", "timeout"):
                work = root / kind
                work.mkdir()
                argv = remote.arguments(Path("/runtime"), work, expected_inputs(), {"address":"172.19.0.2","port":2222}, kind)
                self.assertEqual(argv[3], "/runtime/hold.yml")
                values = json.loads(argv[-1])
                self.assertEqual(values["hold_kind"], kind)
                self.assertEqual(values["ansible_ssh_private_key_file"], "/run/rustward-credentials/client_key")
                self.assertIn("UserKnownHostsFile=/run/rustward-credentials/stopping/known_hosts", values["ansible_ssh_common_args"])
                self.assertIn("ControlPath=none", values["ansible_ssh_common_args"])

    def test_marker_probe_uses_read_only_pinned_ssh_and_reports_errors(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "connection.json").write_text('{"address":"172.19.0.2","port":2222}')
            with patch.object(remote_state, "credentials", return_value=root):
                for response, expected in ((SimpleNamespace(returncode=0, stdout='{"kind":"cancel","state":"absent"}'), "absent"),
                                           (SimpleNamespace(returncode=255, stdout=""), "probe-failed")):
                    with patch.object(remote_state.subprocess, "run", return_value=response) as run:
                        self.assertEqual(remote_state.read(Path("/runtime"), "cancel")["state"], expected)
                        command = run.call_args.args[0]
                        self.assertEqual(command[-3:], ["/opt/rustward_hold.py", "state", "cancel"])
                        self.assertIn("StrictHostKeyChecking=yes", command)
                        self.assertFalse(run.call_args.kwargs.get("shell", False))
                with patch.object(remote_state.subprocess, "run", side_effect=subprocess.TimeoutExpired([], 10)):
                    self.assertEqual(remote_state.read(Path("/runtime"), "timeout")["state"], "probe-failed")
            with self.assertRaises(ValueError):
                remote_state.read(Path("/runtime"), "invalid")

    def test_shared_fixture_preserves_catalog_ids_and_uses_separate_private_roots(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "source"
            source.mkdir()
            metadata = {"version":1,"source_revision":"source","execution_uid":1000,"preparation_uid":0,
                        "runtime_id":"a"*64,"host_id":"b"*64}
            (source / "fixture.json").write_text(json.dumps(metadata))
            account = SimpleNamespace(pw_uid=1000, pw_gid=1000)
            with patch.object(prepare.os, "geteuid", return_value=0), patch.object(prepare.pwd, "getpwnam", return_value=account), patch.object(prepare.os, "chown"):
                for kind in ("cancel", "timeout"):
                    destination = root / kind
                    value = prepare.prepare_shared(destination, source, "synthetic", "source")
                    self.assertEqual(value["runtime_id"], metadata["runtime_id"])
                    self.assertEqual(value["host_id"], metadata["host_id"])
                    self.assertEqual(value["catalog_root"], str(source))
                    for name in ("work", "logs", "state", "evidence"):
                        self.assertEqual((destination / name).stat().st_mode & 0o777, 0o700)
                    task = tomllib.loads((destination / "configuration/tasks/remote-timeout.toml").read_text())
                    self.assertEqual(task["timeout_seconds"], 60)
                    self.assertEqual(task["cleanup_grace_seconds"], 2)
                    self.assertTrue(task["executable"].startswith(str(source)))
                with self.assertRaises(ValueError):
                    prepare.prepare_shared(root / "changed", source, "synthetic", "other-source")
                self.assertFalse((root / "changed").exists())

    def test_post_stop_remote_evidence_can_be_alive_but_never_absent(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            parent = SimpleNamespace(name="test", evidence=root)
            runner = Stopping(parent)
            for state in ("alive", "ended"):
                value = {"kind":"cancel","state":state,"inputs":expected_inputs(),"pid":42,"start_time":"100","run_id":"a"*64}
                with patch.object(runner, "target_call", return_value=(0,json.dumps(value))):
                    runner.observe("cancel", state)
                    report = json.loads((root / f"stopping-{state}-target.json").read_text())
                    self.assertEqual(report["remote"]["state"], state)
            with patch.object(runner, "target_call", return_value=(0,'{"kind":"timeout","state":"absent"}')):
                with self.assertRaises(RuntimeError):
                    runner.observe("timeout", "invalid")

    def test_a05_aggregation_requires_both_isolated_cases_without_rewriting_reports(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "source"
            (source / "evidence").mkdir(parents=True)
            roots = {kind: root / kind for kind in ROOTS}
            with patch.object(stop_inside, "SOURCE", source), patch.object(stop_inside, "ROOTS", roots):
                with self.assertRaises(OSError):
                    stop_inside.complete()
                for kind, path in roots.items():
                    evidence = path / "evidence"
                    evidence.mkdir(parents=True)
                    names = [f"A05-{kind}", f"A05-{kind}-host", "host-key-check", "ssh-command-check"]
                    if kind == "cancel": names.append("A05-queued")
                    for name in names:
                        (evidence / f"{name}.json").write_text('{"status":"passed","writers_stopped":true}')
                report = roots["timeout"] / "evidence/A05-timeout.json"
                report.write_text('{"status":"failed"}')
                with self.assertRaises(RuntimeError): stop_inside.complete()
                self.assertFalse((source / "evidence/A05.json").exists())
                report.write_text('{"status":"passed"}')
                stop_inside.complete()
                with self.assertRaises(FileExistsError): stop_inside.complete()

    def test_fresh_target_pin_does_not_replace_the_previous_host_pin(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            with patch.object(ssh_inside, "CREDENTIALS", root):
                ssh_inside.pin({"host_key":"ssh-ed25519 original","address":"172.19.0.2"})
                before = (root / "known_hosts").read_bytes()
                (root / "wrong_known_hosts").write_text("synthetic wrong public key")
                value = io.StringIO('{"host_key":"ssh-ed25519 stopping","address":"172.19.0.3"}')
                with patch.object(ssh_inside.os, "geteuid", return_value=0), patch.object(ssh_inside.sys, "argv", ["fixture","pin-stopping"]), patch.object(ssh_inside.sys, "stdin", value):
                    ssh_inside.main()
                self.assertEqual((root / "known_hosts").read_bytes(), before)
                self.assertEqual(json.loads((root / "connection.json").read_text())["address"], "172.19.0.2")
                self.assertEqual(json.loads((root / "stopping/connection.json").read_text())["address"], "172.19.0.3")

    def test_export_scan_includes_all_separate_stopping_stores(self):
        with patch.object(ssh_inside.os, "geteuid", return_value=0), patch.object(ssh_inside.sys, "argv", ["fixture","scan"]), patch.object(ssh_inside, "export_status", return_value={"status":"passed"}) as scan, patch.object(stop_inside, "confirm_stopped"), redirect_stdout(io.StringIO()):
            self.assertEqual(ssh_inside.main(), 0)
        roots = list(map(str, scan.call_args.args[0]))
        self.assertIn("/opt/rustward-acceptance", roots)
        self.assertIn("/opt/rustward-ssh-acceptance", roots)
        for path in ROOTS.values(): self.assertIn(path, roots)

    def test_complete_bundle_keeps_remote_and_local_run_ids_matched(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            aggregate = root / "ssh-fixture/evidence"
            aggregate.mkdir(parents=True)
            (aggregate / "A05.json").write_text('{"status":"passed"}')
            for kind, run_id in (("cancel", "a"*64), ("timeout", "b"*64)):
                evidence = root / f"stopping-{kind}-fixture/evidence"
                evidence.mkdir(parents=True)
                names = [f"A05-{kind}", f"A05-{kind}-host", "host-key-check", "ssh-command-check"]
                if kind == "cancel": names.append("A05-queued")
                for name in names:
                    (evidence / f"{name}.json").write_text(json.dumps({"status":"passed","run_id":run_id}))
                (root / f"stopping-{kind}-target.json").write_text(json.dumps({"status":"passed","remote":{"run_id":run_id}}))
            for name in ("queued-before", "queued-after"):
                (root / f"stopping-{name}-target.json").write_text('{"status":"passed"}')
            stopping_runner.validate(root)
            (root / "stopping-cancel-target.json").write_text(json.dumps({"status":"passed","remote":{"run_id":"b"*64}}))
            with self.assertRaisesRegex(RuntimeError, "another admitted run"):
                stopping_runner.validate(root)

    def test_host_cleanup_failure_does_not_report_a_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            case_root = root / "cancel"
            (case_root / "evidence").mkdir(parents=True)
            account = SimpleNamespace(pw_uid=1000, pw_gid=1000, pw_dir="/home/synthetic")
            def checked(command, **kwargs):
                if command[:2] == ["/usr/bin/systemctl", "stop"]:
                    raise subprocess.CalledProcessError(1, command)
            with patch.object(stop_inside, "ROOTS", {"cancel":case_root}), patch.object(stop_inside, "prepare_shared", return_value={"runtime_id":"a"*64}), patch.object(stop_inside.pwd, "getpwnam", return_value=account), patch.object(stop_inside, "checked", side_effect=checked), patch.object(stop_inside, "wait_manager"), patch.object(stop_inside, "probe"), patch.object(stop_inside, "check_commands"), patch.object(stop_inside, "stop_fixture_units"):
                with self.assertRaises(subprocess.CalledProcessError): stop_inside.run("cancel", "source")
            report = json.loads((case_root / "evidence/A05-cancel-host.json").read_text())
            self.assertEqual(report["status"], "failed")

    def test_writer_confirmation_is_required_before_export_even_after_a_case_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            evidence = root / "evidence"
            evidence.mkdir()
            with patch.object(stop_inside, "ROOTS", {"cancel": root}):
                with self.assertRaises(OSError): stop_inside.confirm_stopped()
                report = evidence / "A05-cancel-host.json"
                report.write_text('{"status":"passed","writers_stopped":false}')
                with self.assertRaises(RuntimeError): stop_inside.confirm_stopped()
                report.write_text('{"status":"failed","writers_stopped":true}')
                stop_inside.confirm_stopped()

    def test_partial_stopping_evidence_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.assertEqual(stopping_runner.status(root), "not run")
            evidence = root / "stopping-cancel-fixture/evidence"
            evidence.mkdir(parents=True)
            (evidence / "A05-cancel-admission.json").write_text('{}')
            self.assertEqual(stopping_runner.status(root), "incomplete")
            (evidence / "A05-cancel.json").write_text('{"status":"failed"}')
            self.assertEqual(stopping_runner.status(root), "failed")
            with self.assertRaises(RuntimeError): stopping_runner.validate(root)


if __name__ == "__main__":
    unittest.main()

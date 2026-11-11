import json
import shutil
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from ssh_credentials import scan, create_key
from fixtures import remote, ssh_policy
from ssh_runner import Remote, validate
import ssh_inside
import ssh_target
from types import SimpleNamespace


class CredentialTests(unittest.TestCase):
    def test_marker_across_read_boundary_is_detected_without_disclosure(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            marker = b"synthetic-private-marker-" * 4
            (root / "state.db").write_bytes(b"x" * 65500 + marker + b"tail")
            report = scan([root], marker)
            self.assertEqual(report["status"], "failed")
            self.assertNotIn(marker.decode(), json.dumps(report))
            (root / "state.db").write_bytes(b"ordinary synthetic data")
            self.assertEqual(scan([root], marker)["status"], "passed")

    def test_export_rejects_links_and_special_files(self):
        import os
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "link").symlink_to("missing")
            os.mkfifo(root / "pipe")
            report = scan([root], b"x" * 64)
            self.assertEqual(len(report["findings"]), 2)
            self.assertEqual(report["checked_files"], 0)
            alias = root / "root-link"
            alias.symlink_to(root, target_is_directory=True)
            self.assertEqual(scan([alias], b"x" * 64)["status"], "failed")

    def test_generated_key_marker_is_private_and_key_is_not_exported(self):
        with tempfile.TemporaryDirectory() as folder:
            key = Path(folder) / "key"
            public, marker = create_key(key)
            self.assertTrue(public.startswith("ssh-ed25519 "))
            self.assertGreaterEqual(len(marker), 64)
            self.assertNotIn(marker, public.encode())
            self.assertIn(marker, key.read_bytes())


class TargetTests(unittest.TestCase):
    def test_target_uses_ephemeral_keys_and_absolute_daemon_path(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder) / "target"
            def setup(args, **kwargs):
                if args[0] == "/usr/bin/ssh-keygen":
                    (root / "host_key.pub").write_text("ssh-ed25519 synthetic-public")
                return SimpleNamespace(stdout="synthetic package versions")
            with patch("ssh_target.ROOT", root), patch("ssh_target.PRIVILEGE_DIRECTORY", Path(folder) / "sshd"), patch("ssh_target.subprocess.run", side_effect=setup), patch("ssh_target.os.execv") as launch:
                ssh_target.serve()
            executable, arguments = launch.call_args.args
            self.assertEqual(executable, arguments[0])
            self.assertTrue(Path(arguments[0]).is_absolute())
            configuration = (root / "sshd_config").read_text()
            for rule in ("PasswordAuthentication no", "PermitRootLogin no", "AllowTcpForwarding no",
                         "AuthenticationMethods publickey", f"HostKey {root / 'host_key'}"):
                self.assertIn(rule, configuration)
            self.assertEqual(json.loads((root / "ready.json").read_text())["port"], 2222)


class PolicyTests(unittest.TestCase):
    def test_inventory_uses_fixed_paths_and_strict_ssh_options(self):
        with tempfile.TemporaryDirectory() as folder:
            args = remote.arguments(Path("/runtime"), Path(folder),
                {"text": "two 'quoted' words", "count": 7, "enabled": True},
                {"address": "172.19.0.2", "port": 2222})
            values = json.loads(args[-1])
            self.assertIs(values["enabled"], True)
            self.assertEqual(values["count"], 7)
            self.assertEqual(values["ansible_ssh_args"], "")
            self.assertIs(values["ansible_ssh_use_tty"], False)
            self.assertEqual(values["ansible_ssh_private_key_file"], str(ssh_policy.CREDENTIALS / "client_key"))
            policy = values["ansible_ssh_common_args"]
            for option in ("StrictHostKeyChecking=yes", "BatchMode=yes", "IdentityAgent=none",
                           "ControlMaster=no", "ControlPath=none", "IdentitiesOnly=yes", "GlobalKnownHostsFile=/dev/null"):
                self.assertIn(option, policy)
            self.assertEqual((Path(folder) / "inventory.ini").read_text(), "[fixture]\nssh-target\n")
        with self.assertRaises(ValueError):
            remote.arguments(Path("/runtime"), Path("/unused"), {}, {"address": "8.8.8.8", "port": 2222})

    @unittest.skipUnless(shutil.which("ssh"), "OpenSSH client is not installed")
    def test_openssh_disables_control_sockets_even_with_a_later_long_path(self):
        command = [shutil.which("ssh"), *ssh_policy.options(), "-o", "ControlPath=/" + "x" * 200,
                   "-G", "172.19.0.2"]
        result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=10)
        effective = dict(line.split(None, 1) for line in result.stdout.splitlines() if " " in line)
        self.assertIn(effective["controlmaster"], ("no", "false"))
        self.assertIn(effective["controlpersist"], ("no", "false", "0"))
        self.assertEqual(effective.get("controlpath", "none"), "none")

    def test_remote_lifecycle_uses_internal_network_and_public_control_channel(self):
        with tempfile.TemporaryDirectory() as folder:
            evidence = Path(folder)
            calls = []
            class Docker:
                def call(self, args, **kwargs):
                    args = list(map(str, args))
                    calls.append((args, kwargs))
                    if args[0] == "build":
                        Path(args[args.index("--iidfile") + 1]).write_text("sha256:" + "a" * 64)
                    if args[0] == "inspect" and "--format" in args:
                        return 0, '{"Running":false,"Pid":0}'
                    if args[0] == "inspect":
                        return 0, json.dumps([{"NetworkSettings": {"Networks": {"test-network": {"IPAddress": "172.19.0.2"}}}}])
                    if args[-1] == "metadata":
                        return 0, '{"host_key":"ssh-ed25519 public","port":2222}'
                    if args[-1] == "client-key":
                        return 0, '{"public_key":"ssh-ed25519 public"}'
                    if args[-1] in ("result", "failure-result"):
                        return 0, json.dumps({"text": 'He said "hello"; it\'s a test', "count": 7, "enabled": True})
                    if args[-1] == "scan":
                        return 0, '{"status":"passed"}'
                    return 0, ""
            runner = Remote(Docker(), "test", evidence)
            runner.create_network()
            runner.prepare_credentials()
            runner.run(evidence, "debian", "revision")
            runner.scan()
            runner.cleanup()
            self.assertIn(["network", "create", "--internal", "test-network"], [args for args, _ in calls])
            self.assertFalse(any(args[:2] == ["network", "connect"] for args, _ in calls))
            target = next(args for args, _ in calls if args[0] == "run")
            self.assertNotIn("--privileged", target)
            self.assertNotIn("--publish", target)
            self.assertNotIn("--volume", target)
            self.assertEqual(json.loads((evidence / "target-check.json").read_text())["status"], "passed")
            self.assertTrue(any("input_data" in options for _, options in calls))
            actions = [args for args, _ in calls]
            failed_task = next(args for args in actions if args[-2:] == ["failure", "task"])
            unreachable = next(args for args in actions if args[-2:] == ["failure", "unreachable"])
            stop = next(args for args in actions if args[0] == "stop")
            stopped = next(args for args in actions if args[0] == "inspect" and "--format" in args)
            self.assertLess(actions.index(failed_task), actions.index(stop))
            self.assertLess(actions.index(stop), actions.index(stopped))
            self.assertLess(actions.index(stopped), actions.index(unreachable))
            self.assertEqual(sum(args[-1] == "client-key" for args, _ in calls), 1)
            with self.assertRaisesRegex(RuntimeError, "already prepared"):
                runner.prepare_credentials()

    def test_probe_requires_correct_key_success_and_wrong_key_rejection(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "connection.json").write_text('{"address":"172.19.0.2"}')
            account = SimpleNamespace(pw_uid=1000, pw_gid=1000, pw_dir="/home/synthetic")
            for code, error, accepted in [(255, "Host key verification failed.", True),
                                          (0, "", False), (255, "Connection refused", False)]:
                replies = [SimpleNamespace(returncode=0, stderr=""), SimpleNamespace(returncode=code, stderr=error)]
                with patch("ssh_inside.CREDENTIALS", root), patch("ssh_inside.invoke", side_effect=replies) as invoke:
                    if accepted:
                        ssh_inside.probe(account, Path("/runtime"), root)
                    else:
                        with self.assertRaises(RuntimeError):
                            ssh_inside.probe(account, Path("/runtime"), root)
                    self.assertEqual(invoke.call_count, 2)
                report = json.loads((root / "host-key-check.json").read_text())
                self.assertEqual(report["status"], "passed" if accepted else "failed")

    def test_failed_scan_refuses_export_and_validation_requires_all_proofs(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            class Docker:
                def call(self, *args, **kwargs):
                    return 1, '{"status":"failed","findings":[]}'
            with self.assertRaisesRegex(RuntimeError, "export refused"):
                Remote(Docker(), "test", root).scan()
            with self.assertRaises(OSError):
                validate(root, root)

    def test_bundle_requires_the_installed_command_builder_pass_report(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            evidence = root / "evidence"
            evidence.mkdir()
            for name in ("A03", "A04", "A04-task-failure", "A04-unreachable", "A04-task-failure-host", "A04-unreachable-host", "host-key-check", "ssh-host-runner", "credential-scan", "target-check", "failure-target-check", "target-stopped"):
                (evidence / f"{name}.json").write_text('{"status":"passed"}')
            (evidence / "result.json").write_text('{"status":"passed","cases":["A03"]}')
            with self.assertRaises(OSError):
                validate(root, evidence)
            report = evidence / "ssh-command-check.json"
            report.write_text('{"status":"passed"}')
            validate(root, evidence)
            report.write_text('{"status":"failed"}')
            with self.assertRaisesRegex(RuntimeError, "ssh-command-check"):
                validate(root, evidence)

    def test_missing_scan_report_still_refuses_export_with_a_clear_reason(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            class Docker:
                def call(self, *args, **kwargs):
                    return 1, ""
            with self.assertRaisesRegex(RuntimeError, "raw fixture export refused"):
                Remote(Docker(), "test", root).scan()
            report = json.loads((root / "credential-scan.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertIn("no valid report", report["reason"])


if __name__ == "__main__":
    unittest.main()

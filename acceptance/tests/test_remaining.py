import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import remaining_runner
from fixtures import remote_state, ssh_policy
from types import SimpleNamespace
import scenario_inside
import variant_prepare
from stopping_runner import expected_inputs


class RemainingTests(unittest.TestCase):
    def test_storage_pass_requires_exhaustion_and_cleanup_before_unlock(self):
        with tempfile.TemporaryDirectory() as folder:
            evidence = Path(folder)
            root = evidence / 'remaining-A10-fixture/evidence'
            root.mkdir(parents=True)
            for name in ('host-key-check', 'ssh-command-check'):
                (root / f'{name}.json').write_text('{"status":"passed"}')
            (root / 'A10.json').write_text(json.dumps({'status':'passed', 'run_id':'a'*64,
                'storage_gate_closed':True, 'stopped_while_sqlite_locked':True}))
            host = {'status':'passed', 'writers_stopped':True, 'log_filesystem':{'free_bytes':0}}
            (root / 'A10-host.json').write_text(json.dumps(host))
            before = {'sqlite_write_lock_held':True, 'gate_failed':True,
                'local_cleanup_confirmed':True, 'durable_result':None}
            (root / 'A10-before-unlock.json').write_text(json.dumps(before))
            (evidence / 'remaining-A10-target.json').write_text(json.dumps({'status':'passed', 'remote':{'run_id':'a'*64}}))
            with patch.object(remaining_runner, 'CASES', {'A10':{}}):
                remaining_runner.validate(evidence)
                for key in ('sqlite_write_lock_held', 'gate_failed', 'local_cleanup_confirmed', 'durable_result'):
                    changed = dict(before, **{key:'invalid'})
                    (root / 'A10-before-unlock.json').write_text(json.dumps(changed))
                    with self.assertRaises(RuntimeError): remaining_runner.validate(evidence)
                (root / 'A10-before-unlock.json').write_text(json.dumps(before))
                host['log_filesystem']['free_bytes'] = 4096
                (root / 'A10-host.json').write_text(json.dumps(host))
                with self.assertRaises(RuntimeError): remaining_runner.validate(evidence)

    def test_storage_fault_mount_is_limited_to_the_case_logs(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'logs').mkdir()
            account = SimpleNamespace(pw_uid=1000, pw_gid=1000)
            with patch.object(scenario_inside, 'checked') as mount:
                scenario_inside.prepare_storage(root, account)
            args = mount.call_args.args[0]
            self.assertEqual(args[-1], root / 'logs')
            self.assertEqual(args[2], 'tmpfs')
            self.assertIn('size=1048576', args[4])
            self.assertIn('nosuid,nodev', args[4])
            self.assertEqual((root / 'logs/.ballast').stat().st_size, 786432)
            self.assertFalse((root / 'state').exists())
            self.assertEqual(ssh_policy.credentials('storage').name, 'remaining')

    def test_created_case_requires_stopped_writer_evidence(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            (root/'evidence').mkdir()
            with patch.object(scenario_inside, 'CASES', {'A06':{'root':str(root)}}):
                with self.assertRaises(OSError): scenario_inside.confirm_stopped()
                report=root/'evidence/A06-host.json'
                report.write_text('{"status":"passed","writers_stopped":false}')
                with self.assertRaises(RuntimeError): scenario_inside.confirm_stopped()
                report.write_text('{"status":"failed","writers_stopped":true}')
                scenario_inside.confirm_stopped()

    def test_no_pass_from_partial_case_or_failed_cleanup(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            self.assertEqual(remaining_runner.status(root,'A06'),'not run')
            evidence=root/'remaining-A06-fixture/evidence';evidence.mkdir(parents=True)
            (evidence/'A06-admission.json').write_text('{}')
            self.assertEqual(remaining_runner.status(root,'A06'),'incomplete')
            (evidence/'A06.json').write_text('{"status":"passed"}')
            self.assertEqual(remaining_runner.status(root,'A06'),'incomplete')
            (evidence/'A06-host.json').write_text('{"status":"failed"}')
            self.assertEqual(remaining_runner.status(root,'A06'),'failed')
            with self.assertRaises((OSError,RuntimeError)): remaining_runner.validate(root)

    def test_recovery_probe_uses_the_separate_remaining_pin(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'connection.json').write_text('{"address":"172.19.0.4","port":2222}')
            self.assertEqual(ssh_policy.credentials('recovery').name,'remaining')
            with patch.object(remote_state,'credentials',return_value=root), patch.object(remote_state.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout='{"state":"absent","kind":"recovery"}')):
                self.assertEqual(remote_state.read(Path('/runtime'),'recovery')['kind'],'recovery')

    def test_async_post_recovery_evidence_must_still_be_alive(self):
        with tempfile.TemporaryDirectory() as folder:
            parent=SimpleNamespace(name='test',evidence=Path(folder))
            runner=remaining_runner.Remaining(parent)
            settings=remaining_runner.CASES['A07']
            for state,accepted in [('alive',True),('ended',False)]:
                value={'state':state,'kind':'async','run_id':'a'*64,'inputs':expected_inputs()}
                with patch.object(runner,'target_call',return_value=(0,json.dumps(value))):
                    if accepted:runner.observe_case('A07',settings,{'run_id':'a'*64})
                    else:
                        with self.assertRaises(RuntimeError):runner.observe_case('A07',settings,{'run_id':'a'*64})

    def test_variants_publish_different_ids_without_changing_the_shared_source(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);original=root/'original';runtime=original/'runtimes'/'source';runtime.mkdir(parents=True)
            (original/'hosts').mkdir();(original/'hosts'/('c'*64+'.toml')).write_text('synthetic host')
            (original/'fixture.json').write_text(json.dumps({'runtime_id':'source','host_id':'c'*64,'source_revision':'revision'}))
            (runtime/'runtime.toml').write_text('original manifest');(runtime/'program').write_text('synthetic program')
            destination=root/'case'
            account=SimpleNamespace(pw_uid=1000,pw_gid=1000)
            def identify(tool,kind,path):return ('a' if (path.parent/'selection.txt').read_text().strip()=='A' else 'b')*64
            with patch.object(variant_prepare.pwd,'getpwnam',return_value=account),patch('prepare.os.chown'),patch.object(variant_prepare,'identify',side_effect=identify):
                metadata=variant_prepare.prepare(destination,original,Path('/tools'),'revision')
            self.assertNotEqual(metadata['runtime_id'],metadata['runtime_b_id'])
            self.assertEqual((runtime/'runtime.toml').read_text(),'original manifest')
            self.assertEqual((destination/'runtimes'/('a'*64)/'selection.txt').read_text(),'A\n')
            self.assertTrue((destination/'configuration-b/tasks/runtime-selection.toml').exists())

    def test_flood_proof_requires_both_streams_and_matching_run_identity(self):
        with tempfile.TemporaryDirectory() as folder:
            runner=remaining_runner.Remaining(SimpleNamespace(name='test',evidence=Path(folder)))
            value={'run_id':'a'*64,'inputs':expected_inputs(),'stdout_bytes':1048576,'stderr_bytes':1048576}
            with patch.object(runner,'target_call',return_value=(0,json.dumps(value))):
                runner.observe_case('A09',remaining_runner.CASES['A09'],{'run_id':'a'*64})
            value['stderr_bytes']=0
            with patch.object(runner,'target_call',return_value=(0,json.dumps(value))):
                with self.assertRaises(RuntimeError):runner.observe_case('A09',remaining_runner.CASES['A09'],{'run_id':'a'*64})

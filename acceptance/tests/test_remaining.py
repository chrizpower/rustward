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
from types import SimpleNamespace
from stopping_runner import expected_inputs


class RemainingTests(unittest.TestCase):
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

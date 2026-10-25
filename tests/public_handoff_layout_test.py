"""Exercise signed-engine normalization while preserving the receipt workflow identity."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]


class HandoffLayoutTests(unittest.TestCase):
    def run_layout(self, mode, valid=True):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'plan').mkdir()
            handoffs = root / 'handoffs'
            handoffs.mkdir()
            requests = [{'request_id': 'a' * 64, 'proposal': 'b' * 40}]
            if mode == 'multiple':
                requests.append({'request_id': 'c' * 64, 'proposal': 'd' * 40})
            (root / 'plan/plan.json').write_text(json.dumps({'requests': requests}))
            target = handoffs
            if mode == 'multiple':
                target = handoffs / ('csb-public-handoff-' + requests[0]['request_id'])
                target.mkdir()
            result = {'request': requests[0] if mode != 'wrong_request' else {'request_id': 'f' * 64}}
            (target / 'result.json').write_text(json.dumps(result))
            (target / 'budget.json').write_text('{}')
            (target / 'handoff.tar.gz').write_bytes(b'untouched archive')
            if mode == 'linked':
                (target / 'budget.json').unlink()
                (target / 'budget.json').symlink_to(target / 'result.json')
            if mode == 'extra':
                (target / 'unexpected').write_text('reject')
            code = "import sys,json,pathlib; sys.path.insert(0, sys.argv[1]); import public_ci; root=pathlib.Path(sys.argv[2]); public_ci.normalize_handoff_layout(json.loads((root/'plan/plan.json').read_text()), root/'handoffs')"
            run = subprocess.run([sys.executable, '-c', code, str(ROOT / 'scripts'), directory], env={**os.environ, 'RUNNER_TEMP': directory}, capture_output=True)
            self.assertEqual(run.returncode == 0, valid, run.stderr.decode())
            if valid:
                output = handoffs / ('csb-public-handoff-' + requests[0]['request_id'])
                self.assertEqual((output / 'handoff.tar.gz').read_bytes(), b'untouched archive')
                self.assertEqual(json.loads((output / 'result.json').read_text()), result)
            else:
                self.assertFalse((handoffs / ('csb-public-handoff-' + requests[0]['request_id'])).exists())

    def test_single_artifact_restores_expected_directory(self):
        self.run_layout('single')

    def test_multiple_artifacts_keep_named_directories(self):
        self.run_layout('multiple')

    def test_invalid_flat_handoffs_are_rejected_before_moving(self):
        for mode in ['linked', 'extra', 'wrong_request']:
            with self.subTest(mode=mode):
                self.run_layout(mode, valid=False)

    def test_controller_keeps_original_receipt_identity(self):
        import hashlib
        workflow = ROOT / '.github/workflows/csb-public-controller.yml'
        self.assertEqual(hashlib.sha256(workflow.read_bytes()).hexdigest(),
                         'a93f79d5b6d440988c302c173ce8d5cf5ae812b84843597a39b625638e2aa02d')

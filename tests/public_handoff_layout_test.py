"""Exercise the actual trusted workflow normalization, not a duplicate helper."""
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
            workflow = (ROOT / '.github/workflows/csb-public-controller.yml').read_text()
            block = workflow.split('      - name: Normalize single-artifact handoff layout\n', 1)[1].split('      - name:', 1)[0]
            code = textwrap.dedent(block.split("          python3 - <<'PY'\n", 1)[1].rsplit('          PY', 1)[0])
            run = subprocess.run([sys.executable, '-c', code], env={**os.environ, 'RUNNER_TEMP': directory}, capture_output=True)
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

    def test_template_and_installed_normalization_match(self):
        def block(path):
            return path.read_text().split('      - name: Normalize single-artifact handoff layout\n', 1)[1].split('      - name:', 1)[0]
        self.assertEqual(block(ROOT / '.github/workflows/csb-public-controller.yml'), block(ROOT / 'templates/public-ci/controller.yml'))

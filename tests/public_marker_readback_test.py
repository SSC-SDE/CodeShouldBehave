"""Only the exact empty deployment marker can be reconstructed during readback."""
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from public_ci_test import ci, configuration


class MarkerReadbackTests(unittest.TestCase):
    def restore(self, entries, get_file):
        config = configuration()
        data = ci.canonical({'schema_version': 'csb-public-site-v1',
            'observed_at': '2026-10-26T05:00:00Z', 'files': entries})
        state = dict(ci.empty_state(config), site_generation=1, site_manifest_sha256=ci.digest(data))
        calls = []
        def get(url, cap):
            if url.endswith('site-manifest.json'):
                return data
            calls.append(url)
            return get_file(url, cap)
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'site'
            def verify(root, sha):
                self.assertEqual(sha, ci.digest(data))
                for entry in entries:
                    self.assertEqual(ci.digest((root / entry['path']).read_bytes()), entry['sha256'])
            with patch.dict(sys.modules, public_site=SimpleNamespace(verify_site=verify)):
                result = ci.restore_public_site(config, state, output, get=get)
            self.assertEqual(result, output)
        return calls

    def entry(self, path, data=b''):
        return {'path': path, 'bytes': len(data), 'sha256': ci.digest(data)}

    def test_exact_empty_marker_not_fetched_but_report_is(self):
        calls = self.restore([self.entry('.nojekyll'), self.entry('report.json', b'{}')], lambda *_: b'{}')
        self.assertEqual(len(calls), 1)
        self.assertTrue(calls[0].endswith('/report.json'))

    def test_nonempty_marker_refused(self):
        with self.assertRaisesRegex(ci.Refusal, 'invalid deployment marker'):
            self.restore([self.entry('.nojekyll', b'not-empty')], lambda *_: b'not-empty')

    def test_wrong_marker_digest_refused(self):
        marker = self.entry('.nojekyll'); marker['sha256'] = 'a' * 64
        with self.assertRaisesRegex(ci.Refusal, 'invalid deployment marker'):
            self.restore([marker], lambda *_: b'')

    def test_missing_report_or_other_hidden_file_still_refused(self):
        for path in ('report.json', '.other', 'nested/.nojekyll'):
            with self.subTest(path=path), self.assertRaises(urllib.error.HTTPError):
                self.restore([self.entry('.nojekyll'), self.entry(path)],
                    lambda *_: (_ for _ in ()).throw(urllib.error.HTTPError('redacted', 404, 'missing', {}, None)))

    def test_corrupt_report_still_refused(self):
        with self.assertRaisesRegex(ci.Refusal, 'content mismatch'):
            self.restore([self.entry('.nojekyll'), self.entry('report.json', b'{}')], lambda *_: b'[]')


class PagesConvergenceTests(unittest.TestCase):
    def scenario(self, manifests, *, expected, previous):
        config = configuration()
        state = dict(ci.empty_state(config), site_generation=2,
                     site_manifest_sha256=ci.digest(expected))
        calls, waits = [], []
        remaining = iter(manifests)
        def get(url, cap):
            calls.append(url)
            self.assertTrue(url.endswith('site-manifest.json'))
            return next(remaining)
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'site'
            with patch.dict(sys.modules, public_site=SimpleNamespace(verify_site=lambda root, sha: None)):
                ci.restore_public_site(config, state, output, get=get,
                                       lagging_manifest_sha256=previous, sleep=waits.append)
            self.assertEqual((output / 'site-manifest.json').read_bytes(), expected)
        return calls, waits

    def test_signed_previous_generation_can_converge_without_deploying_again(self):
        expected = ci.canonical({'schema_version': 'csb-public-site-v1',
            'observed_at': '2026-10-27T05:00:00Z', 'files': []})
        old = b'previous-signed-manifest'
        calls, waits = self.scenario([old, old, expected], expected=expected, previous=ci.digest(old))
        self.assertEqual(len(calls), 3)
        self.assertEqual(waits, [5, 5])

    def test_unknown_generation_is_refused_without_waiting(self):
        with self.assertRaisesRegex(ci.Refusal, 'indeterminate live Pages'):
            self.scenario([b'unknown'], expected=b'expected', previous=ci.digest(b'old'))

    def test_lag_is_bounded_and_does_not_clear_intent(self):
        old = b'previous'
        with self.assertRaisesRegex(ci.Refusal, 'indeterminate live Pages'):
            self.scenario([old] * 12, expected=b'expected', previous=ci.digest(old))

    def test_ordinary_restore_does_not_accept_lag(self):
        with self.assertRaisesRegex(ci.Refusal, 'indeterminate live Pages'):
            self.scenario([b'old'], expected=b'expected', previous=None)

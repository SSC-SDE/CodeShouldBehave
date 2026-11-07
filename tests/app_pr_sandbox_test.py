"""Offline tests for the networkless PR launcher, not host qualification."""
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('app_pr_sandbox', ROOT / 'scripts/app_pr_sandbox.py')
sandbox = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sandbox)

class PrSandboxTests(unittest.TestCase):
    def test_unqualified_host_has_no_fallback(self):
        with patch.object(sandbox.sys, 'platform', 'darwin'):
            with self.assertRaises(ValueError): sandbox.command(Path('/engine'))
        with patch.object(sandbox.sys, 'platform', 'linux'), patch.object(sandbox.shutil, 'which', return_value=None):
            with self.assertRaises(ValueError): sandbox.command(Path('/engine'))

    def test_sandbox_command_has_no_network_credentials_or_host_repository(self):
        with patch.object(sandbox.sys, 'platform', 'linux'), patch.object(sandbox.shutil, 'which', return_value='/usr/bin/tool'):
            command = sandbox.command(Path('/operator/verified'))
        for flag in ['--unshare-all', '--clearenv', '--new-session', '--die-with-parent', '--cpu=60', '--as=1073741824']:
            self.assertIn(flag, command)
        self.assertNotIn('--share-net', command)
        self.assertNotIn('/home', command)
        self.assertEqual(command[-1], '/engine/pr-analysis')

    def test_verified_copy_and_sanitized_environment(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / 'engine'
            binary.write_bytes(b'fixture pinned executable')
            digest = hashlib.sha256(binary.read_bytes()).hexdigest()
            def invoke(command, **kwargs):
                pinned = Path(command[0])
                self.assertNotEqual(pinned, binary)
                self.assertEqual(hashlib.sha256(pinned.read_bytes()).hexdigest(), digest)
                self.assertEqual(kwargs['input'], b'{}')
                self.assertEqual(set(kwargs['env']), {'PATH', 'LANG'})
                self.assertEqual(kwargs['timeout'], 90)
                kwargs['stdout'].write(b'{"version":"fixture"}')
                return subprocess.CompletedProcess(command, 0)
            with patch.object(sandbox, 'command', side_effect=lambda p: [str(p)]), patch.object(sandbox.subprocess, 'run', side_effect=invoke):
                self.assertEqual(sandbox.run(binary, digest, b'{}'), b'{"version":"fixture"}')
            with patch.object(sandbox.subprocess, 'run') as runner:
                with self.assertRaises(ValueError): sandbox.run(binary, 'a' * 64, b'{}')
                runner.assert_not_called()
            link = Path(tmp) / 'linked'
            link.symlink_to(binary)
            with self.assertRaises(ValueError): sandbox.run(link, digest, b'{}')

    def test_timeout_and_oversized_output_never_return_partial_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / 'engine'; binary.write_bytes(b'fixture')
            digest = hashlib.sha256(binary.read_bytes()).hexdigest()
            with patch.object(sandbox, 'command', return_value=['sandbox']), patch.object(sandbox.subprocess, 'run', side_effect=subprocess.TimeoutExpired('worker',45)):
                with self.assertRaises(subprocess.TimeoutExpired): sandbox.run(binary, digest, b'{}')
            def oversized(command, **kwargs):
                kwargs['stdout'].write(b'x' * (sandbox.OUTPUT_LIMIT + 1))
                return subprocess.CompletedProcess(command, 0)
            with patch.object(sandbox, 'command', return_value=['sandbox']), patch.object(sandbox.subprocess, 'run', side_effect=oversized):
                with self.assertRaises(ValueError): sandbox.run(binary, digest, b'{}')

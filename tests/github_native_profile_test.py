"""Operator-only native build selection; no Cargo or network activity."""
import contextlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest import mock
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import github_mvp_server as host


class NativeProfileTest(unittest.TestCase):
    def test_profile_and_binary_paths_cannot_escape_fixed_project_targets(self):
        for profile in ("debug", "release"):
            for name in ("csb-onboarding", "flowdiff"):
                self.assertEqual(host.native_binary(profile, name), host.ROOT / "target" / profile / name)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "uncreated"
            for invalid in ("../release", "/tmp", "release/flowdiff", "", None, []):
                with self.subTest(profile=invalid), self.assertRaises(ValueError):
                    host.State(root, output, native_profile=invalid)
                self.assertFalse(output.exists(), "Invalid profile must fail before mkdir/lock")
            for invalid in ("../flowdiff", "/tmp/flowdiff", "other-binary"):
                with self.subTest(name=invalid), self.assertRaises(ValueError):
                    host.native_binary("release", invalid)

    def test_missing_binaries_build_locked_selected_profile_from_trusted_manifest(self):
        for profile in ("debug", "release"):
            with self.subTest(profile=profile), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)

                def completed_build(command, **kwargs):
                    self.assertEqual(kwargs, {"cwd": root, "check": True})
                    expected = ["cargo", "build", "--locked", "--features", "server",
                                "--bin", "csb-onboarding", "--bin", "flowdiff",
                                "--manifest-path", str(root / "Cargo.toml")]
                    if profile == "release": expected.append("--release")
                    self.assertEqual(command, expected)
                    directory = root / "target" / profile
                    directory.mkdir(parents=True)
                    for name in ("csb-onboarding", "flowdiff"):
                        (directory / name).write_bytes(b"inert-test-binary")
                    return subprocess.CompletedProcess(command, 0)

                with mock.patch.object(host, "ROOT", root), mock.patch.object(host.subprocess, "run", side_effect=completed_build) as build:
                    host.ensure_native_binaries(profile)
                    host.ensure_native_binaries(profile)
                    self.assertEqual(build.call_count, 1, "Existing selected binaries must not rebuild on every read")
                    other_profile = "release" if profile == "debug" else "debug"
                    self.assertFalse((root / "target" / other_profile).exists())

    def test_failed_or_redirected_build_is_not_accepted_as_selected_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "target" / "release"
            directory.mkdir(parents=True)
            external = root / "external"
            external.write_bytes(b"inert")
            (directory / "flowdiff").symlink_to(external)
            with mock.patch.object(host, "ROOT", root), mock.patch.object(host.subprocess, "run") as build:
                with self.assertRaises(ValueError): host.ensure_native_binaries("release")
                build.assert_not_called()
                (directory / "flowdiff").unlink()
                with self.assertRaises(ValueError): host.ensure_native_binaries("release")
                self.assertEqual(build.call_count, 1, "Success without fixed outputs must fail")
                build.side_effect = subprocess.CalledProcessError(1, ["cargo"])
                with self.assertRaises(subprocess.CalledProcessError): host.ensure_native_binaries("release")

    def test_constructor_default_and_status_report_actual_selected_profile(self):
        for profile in (None, "release"):
            with self.subTest(profile=profile), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                state = host.State(root, root, **({} if profile is None else {"native_profile": profile}))
                try:
                    state.provider_status = lambda: {"authenticated": False}
                    handler = object.__new__(host.handler(state))
                    handler.path = "/api/status"
                    handler.loopback_origin_allowed = lambda: True
                    with mock.patch.object(handler, "json") as response:
                        handler.do_GET()
                    self.assertEqual(response.call_args.args[0]["native_profile"], profile or "debug")
                    with mock.patch.object(host, "ROOT", root), mock.patch.object(host.subprocess, "run") as native:
                        status, _ = state._native_onboarding_request("defaults", "active", {}, "owner/repo", "reviewer", False)
                        self.assertEqual(status, 503)
                        native.assert_not_called()
                finally:
                    state._host_lock.close()

    def test_direct_server_defaults_to_release_and_passes_only_declared_profile(self):
        for extra, expected in (([], "release"), (["--native-profile", "debug"], "debug")):
            with self.subTest(profile=expected), mock.patch.object(sys, "argv", ["server", "--repo", ".", "--output", "/unused", "--auth-profile", "gh-cli", *extra]), \
                    mock.patch.object(host, "ensure_native_binaries") as ensure, \
                    mock.patch.object(host, "State") as state, \
                    mock.patch.object(host, "ThreadingHTTPServer") as server, contextlib.redirect_stdout(io.StringIO()):
                host.main()
                ensure.assert_called_once_with(expected)
                self.assertEqual(state.call_args.args[-1], expected)
                server.return_value.serve_forever.assert_called_once_with()
        with mock.patch.object(sys, "argv", ["server", "--repo", ".", "--output", "/unused", "--native-profile", "../release"]), \
                mock.patch.object(host, "ensure_native_binaries") as ensure, \
                mock.patch.object(host, "State") as state, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as rejected: host.main()
            self.assertEqual(rejected.exception.code, 2)
            ensure.assert_not_called()
            state.assert_not_called()

    def test_launcher_rejects_invalid_profile_before_any_runtime_or_output_creation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output, runtime = root / "output", root / "runtime"
            result = subprocess.run(["sh", str(host.ROOT / "scripts" / "start_github_mvp.sh"), str(root)],
                env={**os.environ, "CSB_NATIVE_PROFILE": "../release", "CSB_MVP_OUTPUT": str(output), "TMPDIR": str(runtime)},
                capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertIn("CSB_NATIVE_PROFILE must be debug or release", result.stderr)
            self.assertFalse(output.exists())
            self.assertFalse(runtime.exists())


if __name__ == "__main__":
    unittest.main()

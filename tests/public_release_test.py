"""Release trust/redistribution boundaries; requires the documented Python 3.11+."""
import gzip
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
try:
    import tomllib
except ImportError:
    tomllib = None

if tomllib is not None:
    SPEC = importlib.util.spec_from_file_location("public_release", Path(__file__).resolve().parents[1] / "scripts/build_public_release.py")
    release = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(release)


@unittest.skipIf(tomllib is None, "Release packaging requires Python 3.11+")
class PublicReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.registry = self.root / "registry/src"
        self.registry.mkdir(parents=True)
        cache = self.root / "registry/cache/index"
        cache.mkdir(parents=True)
        data = io.BytesIO()
        with gzip.GzipFile(fileobj=data, mode="wb", mtime=0) as compressor:
            with tarfile.open(fileobj=compressor, mode="w") as archive:
                for name, content in {"fixture-1.0.0/Cargo.toml": b'[package]\nname="fixture"\nversion="1.0.0"\nlicense="MIT OR Apache-2.0"\n', "fixture-1.0.0/LICENSE-MIT": b"original upstream attribution", "fixture-1.0.0/NOTICE": b"original upstream notice"}.items():
                    info = tarfile.TarInfo(name); info.size = len(content)
                    archive.addfile(info, io.BytesIO(content))
        self.crate = cache / "fixture-1.0.0.crate"
        self.crate.write_bytes(data.getvalue())
        self.empty_notices = {"licenses/native/sources.json": (b"[]", 0o644), "licenses/javascript/sources.json": (b"[]", 0o644)}
        self.lock = ('version=4\n[[package]]\nname="fixture"\nversion="1.0.0"\nsource="registry+https://github.com/rust-lang/crates.io-index"\nchecksum="' + release.sha(data.getvalue()) + '"\n').encode()

    def tearDown(self):
        self.temp.cleanup()

    def test_original_notices_come_from_exact_locked_archive_not_modified_source_cache(self):
        fake_source = self.registry / "index/fixture-1.0.0"
        fake_source.mkdir(parents=True)
        (fake_source / "LICENSE-MIT").write_text("modified source cache")
        packages, notices = release.license_inventory(self.lock, self.registry)
        self.assertEqual(len(packages), 1)
        self.assertEqual(notices["licenses/rust/fixture-1.0.0/LICENSE-MIT"][0], b"original upstream attribution")
        self.crate.write_bytes(b"substituted archive")
        with self.assertRaisesRegex(ValueError, "Cargo.lock checksum"):
            release.license_inventory(self.lock, self.registry)

    def test_package_is_deterministic_and_never_overwrites_existing_output(self):
        files = dict(self.empty_notices, **{"Cargo.lock": (self.lock, 0o644), "README.md": (b"public fixture", 0o644)})
        with patch.object(release, "distribution_files", return_value=files):
            first = release.build(self.root / "first", "a" * 40, registry=self.registry)
            second = release.build(self.root / "second", "a" * 40, registry=self.registry)
            self.assertEqual(first, second)
            asset = first["assets"][0]["path"]
            self.assertEqual((self.root / "first" / asset).read_bytes(), (self.root / "second" / asset).read_bytes())
            with self.assertRaisesRegex(ValueError, "new directory"):
                release.build(self.root / "first", "a" * 40, registry=self.registry)
        with tarfile.open(self.root / "first" / asset) as archive:
            names = archive.getnames()
            self.assertIn("sbom.cdx.json", names)
            self.assertIn("licenses/rust/fixture-1.0.0/NOTICE", names)
            self.assertNotIn(".git", names)
            self.assertNotIn("docs/evidence", names)

    def test_supplemental_notice_cannot_silently_survive_a_dependency_update(self):
        data = b"original upstream license"
        record = {"path": "licenses/native/fixture-LICENSE", "bytes": len(data),
                  "sha256": release.sha(data), "package": "fixture", "package_version": "1.0.0",
                  "registry_checksum_sha256": "a" * 64}
        files = dict(self.empty_notices)
        files["licenses/native/fixture-LICENSE"] = (data, 0o644)
        files["licenses/native/sources.json"] = (json.dumps([record]).encode(), 0o644)
        packages = [{"name": "fixture", "version": "1.0.0", "registry_checksum_sha256": "a" * 64}]
        release.verify_notice_sources(files, packages)
        packages[0]["version"] = "2.0.0"
        with self.assertRaisesRegex(ValueError, "locked dependency"):
            release.verify_notice_sources(files, packages)
        with self.assertRaisesRegex(ValueError, "supplemental notice is missing"):
            release.verify_notice_sources(self.empty_notices, [{"name": "tree-sitter", "version": "0.25.10"}])

    def test_sensitive_findings_never_echo_credential_bytes_and_block_package(self):
        credential = b"ghp_" + b"A" * 36
        files = dict(self.empty_notices, **{"Cargo.lock": (self.lock, 0o644), "config/unsafe.json": (credential, 0o644)})
        findings = release.scan(files)
        self.assertEqual(findings[0]["rule"], "github-token")
        self.assertNotIn(credential.decode(), json.dumps(findings))
        with patch.object(release, "distribution_files", return_value=files):
            with self.assertRaisesRegex(ValueError, "Potential sensitive strings") as caught:
                release.build(self.root / "blocked", "a" * 40, registry=self.registry)
            self.assertNotIn(credential.decode(), str(caught.exception))
        self.assertFalse((self.root / "blocked").exists())


if __name__ == "__main__":
    unittest.main()

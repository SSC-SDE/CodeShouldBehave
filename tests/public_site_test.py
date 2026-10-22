"""Publication boundaries for local static-site assembly, no network or CI writes."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("public_site", Path(__file__).resolve().parents[1] / "scripts/public_site.py")
site = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(site)
NOW = "2026-10-17T06:00:00Z"


class PublicSiteTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.semantic_verifier = patch.object(site, "verify_producer", side_effect=lambda root, *_: site.bundle(root)[0])
        self.semantic_verifier.start()
        self.project = self.root / "project"
        (self.project / "src").mkdir(parents=True)
        (self.project / "docs/prototypes/vendor").mkdir(parents=True)
        (self.project / "src/workspace.html").write_text('<meta name="color-scheme" content="dark"><link href="/assets/workspace.css"><a href="/workspace">CSB</a><script src="/assets/workspace.js"></script>')
        for name in ("workspace.js", "workspace.css", "public-workspace.js", "workspace-preferences.js", "workspace-registry.json"):
            (self.project / "src" / name).write_text("/* trusted UI fixture */")
        (self.project / "src/fonts").mkdir()
        for name in ("inter.ttf", "jetbrains-mono.ttf", "inter-OFL.txt", "jetbrains-mono-OFL.txt"):
            (self.project / "src/fonts" / name).write_text("trusted font fixture")
        for name in ("mermaid.tiny-12.1.0.js", "mermaid-LICENSE.txt"):
            (self.project / "docs/prototypes/vendor" / name).write_text("trusted asset fixture")
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.md"):
            (self.project / name).write_text("fixture notice")
        (self.project / "licenses/native").mkdir(parents=True)
        (self.project / "licenses/native/NOTICE.txt").write_text("original native notice")

    def tearDown(self):
        self.semantic_verifier.stop()
        self.temp.cleanup()

    def bundle(self, name="bundle", text="safe"):
        root = self.root / name
        root.mkdir()
        data = site.canonical({"text": text})
        manifest = {"schema_version": "csb-public-report-v1", "analysis_review_id": "a" * 64,
                    "repository": {"id": 1, "owner": "fixture", "name": "repo"}, "pr_number": 1,
                    "selected_baseline": "a" * 40, "baseline": "a" * 40, "proposal": "b" * 40,
                    "scope_version": "scope1", "policy_digest": "c" * 64,
                    "engine": {"package": "flowdiff", "version": "0.1.0", "hld_generator": "fixture", "lld_generator": "fixture", "lld_parser": "fixture", "public_projection": "v1"},
                    "files": [{"path": "artifact.json", "sha256": site.digest(data), "bytes": len(data), "kind": "artifact"}],
                    "routes": {"artifact": "artifact.json", "hld": None, "lld_catalog": None, "lld_details": [], "sources": [], "file_sources": [], "exports": []},
                    "coverage": {"complete": False, "gaps": ["synthetic"], "deferred_details": 0}, "capabilities": {}}
        identity = site.digest(site.canonical(manifest))
        manifest.update(review_id=identity, public_report_id=identity)
        (root / "manifest.json").write_bytes(site.canonical(manifest))
        (root / "artifact.json").write_bytes(data)
        return root, identity

    def build(self, bundles, name="site", **kwargs):
        output = self.root / name
        result = site.assemble_site(bundles, output, NOW, project_root=self.project, **kwargs)
        return output, result

    def test_project_prefix_and_independent_manifest_inventory(self):
        bundle, identity = self.bundle()
        output, result = self.build([bundle])
        self.assertEqual(site.verify_site(output, result["site_sha256"])["site_sha256"], result["site_sha256"])
        html = (output / "workspace/index.html").read_text()
        self.assertEqual((output / "licenses/native/NOTICE.txt").read_text(), "original native notice")
        self.assertIn('content="../index.json"', html)
        self.assertIn('src="../assets/public-workspace.js"', html)
        self.assertNotIn('src="/assets/', html)
        index = json.loads((output / "index.json").read_text())
        self.assertEqual(index["reports"][0]["public_report_id"], identity)
        self.assertNotIn("state", index["reports"][0], "Assembler must not invent provider freshness")
        self.assertEqual(index["reports"][0]["manifest_bytes"], (output / "reviews" / identity / "manifest.json").stat().st_size)
        (output / "assets/workspace.js").write_text("tampered")
        with self.assertRaisesRegex(site.SiteError, "hash/length"):
            site.verify_site(output, result["site_sha256"])

    def test_private_input_extra_file_and_hash_change_rejected_before_output(self):
        bundle, _ = self.bundle()
        (bundle / "private.json").write_text('"retained private IR"')
        with self.assertRaisesRegex(site.SiteError, "Unexpected"):
            self.build([bundle])
        self.assertFalse((self.root / "site").exists())
        (bundle / "private.json").unlink()
        (bundle / "artifact.json").write_text("different bytes")
        with self.assertRaisesRegex(site.SiteError, "hash/length|byte limit"):
            self.build([bundle])

    def test_symlink_and_traversal_are_rejected(self):
        bundle, _ = self.bundle()
        (bundle / "artifact.json").unlink()
        (bundle / "artifact.json").symlink_to(self.project / "LICENSE")
        with self.assertRaisesRegex(site.SiteError, "Symlink"):
            self.build([bundle])
        for value in ("../secret", "safe/../../secret", "/secret", "a\\b", "./a", "a//b", "https:bad", "a%2Fb", "a?b", "a#b", ".git/config"):
            with self.subTest(value=value), self.assertRaises(site.SiteError):
                site.relative_path(value)

    def test_standalone_assembly_requires_a_pinned_semantic_verifier(self):
        bundle, _ = self.bundle()
        self.semantic_verifier.stop()
        with self.assertRaisesRegex(site.SiteError, "semantic verifier required"):
            self.build([bundle], "unverified")
        verifier = self.root / "verifier"
        verifier.write_bytes(b"not a trusted executable")
        with self.assertRaisesRegex(site.SiteError, "verifier digest mismatch"):
            site.verify_producer(bundle, verifier, "0" * 64)
        with patch.object(site.subprocess, "run", return_value=type("Result", (), {"returncode": 1, "stdout": b""})()):
            with self.assertRaisesRegex(site.SiteError, "semantic verification failed"):
                site.verify_producer(bundle, verifier, site.digest(verifier.read_bytes()))
        self.assertFalse((self.root / "unverified").exists())
        self.semantic_verifier.start()

    def test_withdrawal_applies_to_recovery_and_cannot_restore_old_public_bytes(self):
        bundle, identity = self.bundle()
        first, old = self.build([bundle], "first")
        with self.assertRaisesRegex(site.SiteError, "independent trusted"):
            self.build([], "untrusted", previous=first)
        with self.assertRaisesRegex(site.SiteError, "receipt hash"):
            self.build([], "bad-proof", previous=first, previous_sha256="0" * 64)
        second, newer = self.build([], "second", previous=first, previous_sha256=old["site_sha256"], withdrawn=[identity])
        self.assertFalse((second / "reviews" / identity).exists())
        self.assertEqual(json.loads((second / "expired" / (identity + ".json")).read_text())["reason"], "withdrawn")
        third, _ = self.build([bundle], "third", previous=second, previous_sha256=newer["site_sha256"])
        self.assertFalse((third / "reviews" / identity).exists(), "A new input cannot resurrect a withdrawal")
        restored, _ = self.build([], "restore-old", previous=first, previous_sha256=old["site_sha256"], withdrawn=[identity])
        self.assertFalse((restored / "reviews" / identity).exists(), "Current withdrawal ledger must override an older archive")

    def test_site_must_fit_the_publishers_restore_manifest_and_file_limits(self):
        bundle, _ = self.bundle()
        output, _ = self.build([bundle])
        manifest_bytes = (output / "site-manifest.json").stat().st_size
        with patch.object(site, "MAX_SITE_MANIFEST_BYTES", manifest_bytes - 1):
            with self.assertRaisesRegex(site.SiteError, "bounded regular file"):
                site.verify_site(output)
            with self.assertRaises(site.SiteError):
                self.build([bundle], "oversize-manifest")
            self.assertFalse((self.root / "oversize-manifest").exists())
        with patch.object(site, "MAX_FILES", 2):
            with self.assertRaisesRegex(site.SiteError, "cap exceeded"):
                self.build([bundle], "too-many-files")
            self.assertFalse((self.root / "too-many-files").exists())
        self.assertTrue(output.is_dir())

    def test_resource_exhaustion_does_not_overwrite_previous_generation(self):
        bundle, _ = self.bundle()
        first, old = self.build([bundle], "first")
        with patch.object(site, "MAX_SITE_BYTES", 32), self.assertRaises(site.SiteError):
            self.build([bundle], "too-large")
        self.assertFalse((self.root / "too-large").exists())
        self.assertEqual(site.verify_site(first, old["site_sha256"])["site_sha256"], old["site_sha256"])
        self.assertFalse(list(self.root.glob(".csb-site-*")))
        with self.assertRaisesRegex(site.SiteError, "new directory"):
            site.assemble_site([bundle], first, NOW, project_root=self.project)

    def test_duplicate_json_keys_and_non_finite_metadata_are_rejected(self):
        for data in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":Infinity}'):
            with self.subTest(data=data), self.assertRaises(site.SiteError):
                site.parse_json(data)

    def test_provider_observation_changes_index_without_rewriting_report(self):
        bundle, identity = self.bundle()
        first, proof = self.build([bundle], "first", observations=[{"repository_id": 1, "pr_number": 1, "state": "draft", "head": "b" * 40, "base": "a" * 40, "observed_at": NOW}])
        record = json.loads((first / "index.json").read_text())["reports"][0]
        self.assertEqual(record["state"], "draft")
        self.assertFalse(record["historical"])
        second, _ = self.build([], "second", previous=first, previous_sha256=proof["site_sha256"], observations=[{"repository_id": 1, "pr_number": 1, "state": "closed", "head": "c" * 40, "base": "a" * 40, "observed_at": NOW}])
        new_record = json.loads((second / "index.json").read_text())["reports"][0]
        self.assertEqual(new_record["state"], "closed")
        self.assertTrue(new_record["historical"])
        self.assertEqual((first / "reviews" / identity / "manifest.json").read_bytes(), (second / "reviews" / identity / "manifest.json").read_bytes())
        with self.assertRaisesRegex(site.SiteError, "observation"):
            self.build([], "invalid", observations=[{"repository_id": 1, "pr_number": 1, "state": "open", "head": "b" * 40, "base": "a" * 40, "observed_at": NOW, "arbitrary_title": "not authorized metadata"}])


if __name__ == "__main__":
    unittest.main()

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
WORKSPACE_ASSETS = (
    "workspace-focus-model.js",
    "workspace-focus.js", "workspace-focus.css", "workspace-moria.js",
    "workspace-moria.css", "workspace-moria-palette.css", "workspace-moria.json",
    "workspace-standard.js", "workspace-standard.css",
    "workspace-ring.js", "workspace-ring.css",
)


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
        with (self.project / "src/workspace.html").open("a") as stream:
            for name in WORKSPACE_ASSETS:
                if name.endswith(".js"):
                    stream.write(f'<script src="/assets/{name}"></script>')
                elif name.endswith(".css"):
                    stream.write(f'<link href="/assets/{name}">')
        (self.project / "src/landing.html").write_text('<!doctype html><link href="assets/landing.css"><a href="workspace/">Workspace</a><img src="assets/landing/architecture.webp">')
        (self.project / "src/landing.css").write_text("body { color: #bfc4ce; }")
        (self.project / "src/landing").mkdir()
        for name in ("architecture.webp", "detail.webp", "source.webp", "favicon.png"):
            (self.project / "src/landing" / name).write_bytes(b"trusted image fixture: " + name.encode())
        for name in ("workspace.js", "workspace.css", "public-workspace.js", "workspace-preferences.js", "workspace-registry.json") + WORKSPACE_ASSETS:
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

    def test_workspace_assets_ship_exact_bytes_with_subpath_safe_links(self):
        (self.project / "src/workspace-unpublished.js").write_text("not approved")
        output, result = self.build([])
        html = (output / "workspace/index.html").read_text()
        manifest = json.loads((output / "site-manifest.json").read_text())
        inventory = {item["path"]: item for item in manifest["files"]}
        for name in WORKSPACE_ASSETS:
            with self.subTest(name=name):
                expected = (self.project / "src" / name).read_bytes()
                self.assertEqual((output / "assets" / name).read_bytes(), expected)
                self.assertEqual(inventory[f"assets/{name}"]["sha256"], site.digest(expected))
                if not name.endswith(".json"):
                    self.assertIn(f'"../assets/{name}"', html)
        self.assertNotIn('href="/assets/', html)
        self.assertNotIn('src="/assets/', html)
        self.assertFalse((output / "assets/workspace-unpublished.js").exists())
        self.assertEqual(site.verify_site(output)["site_sha256"], result["site_sha256"])

    def test_missing_workspace_assets_fail_before_publishing(self):
        for name in WORKSPACE_ASSETS:
            with self.subTest(name=name):
                path = self.project / "src" / name
                original = path.read_bytes()
                path.unlink()
                with self.assertRaises(site.SiteError):
                    self.build([])
                self.assertFalse((self.root / "site").exists())
                self.assertFalse(list(self.root.glob(".csb-site-*")))
                path.write_bytes(original)

    def test_landing_does_not_publish_unapproved_neighboring_assets(self):
        (self.project / "src/landing/unpublished.webp").write_bytes(b"not a public asset")
        output, _ = self.build([])
        self.assertFalse((output / "assets/landing/unpublished.webp").exists())

    def test_landing_inputs_must_be_present_bounded_regular_files(self):
        for source, limit in (("src/landing.html", 512 * 1024), ("src/landing.css", 4 * 1024 * 1024), ("src/landing/source.webp", 4 * 1024 * 1024)):
            path = self.project / source
            original = path.read_bytes()
            for kind in ("missing", "symlink", "oversize"):
                with self.subTest(source=source, kind=kind):
                    path.unlink()
                    if kind == "symlink":
                        path.symlink_to(self.project / "LICENSE")
                    elif kind == "oversize":
                        with path.open("wb") as stream:
                            stream.truncate(limit + 1)
                    with self.assertRaises(site.SiteError):
                        self.build([])
                    self.assertFalse((self.root / "site").exists())
                    self.assertFalse(list(self.root.glob(".csb-site-*")))
                    if path.exists() or path.is_symlink():
                        path.unlink()
                    path.write_bytes(original)

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

    def test_site_cap_refusal_names_retained_and_new_bytes(self):
        bundle, _ = self.bundle()
        first, _ = self.build([bundle], "first")
        total = sum(
            item["bytes"]
            for item in json.loads((first / "site-manifest.json").read_text())["files"]
        )
        with patch.object(site, "MAX_SITE_BYTES", total - 1), self.assertRaisesRegex(
            site.SiteError, "retained bytes"
        ) as raised:
            self.build([bundle], "over-cap")
        message = str(raised.exception)
        self.assertIn("new bundle bytes", message)
        self.assertIn(str(total - 1), message)
        self.assertFalse((self.root / "over-cap").exists())

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

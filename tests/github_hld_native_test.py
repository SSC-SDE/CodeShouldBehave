"""HTTP → real Rust → staged SQLite HLD tests, with an inert GitHub fixture.

Build all server binaries first. Only provider responses are synthetic; HLD
generation, source pins, authorization, correction and acceptance are native.
No reviewed code, hooks, deployment or remote provider calls are executed.
"""
import copy
import http.client
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import github_mvp_server as host
from phase4_browser_fixture import build


class HldNativeHttpTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="csb-hld-native-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.fixture = build(Path(cls.temporary.name) / "corpus")

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="csb-hld-http-")
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        shutil.copytree(self.fixture["output"], root / "output")
        self.state = host.State(Path(self.fixture["repo"]), root / "output")
        self.addCleanup(self.state._host_lock.close)
        self.identity = "fixture-user"
        self.permissions = {"pull": True, "push": True}
        self.heads = {number: value["head"] for number, value in self.fixture["reviews"].items()}
        command = self.state.command

        def fixture_provider(args, **kwargs):
            if args[0] != "gh":
                return command(args, **kwargs)
            if args[1:3] == ["api", "user"]:
                value = self.identity
            elif args[1:3] == ["api", "repos/" + self.fixture["slug"]]:
                value = json.dumps(self.permissions)
            elif args[1] == "api" and "/pulls/" in args[2]:
                number = args[2].rsplit("/", 1)[1]
                value = json.dumps({"base": {"sha": self.fixture["baseline"]}, "head": {"sha": self.heads[number]}})
            elif args[1:3] == ["pr", "list"]:
                value = "[]"
            else:
                raise AssertionError("Unexpected synthetic provider request")
            return subprocess.CompletedProcess(args, 0, value, "")

        self.state.command = fixture_provider
        self.server = host.ThreadingHTTPServer(("127.0.0.1", 0), host.handler(self.state))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop_server)

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def request(self, number, suffix="", body=None, authenticated=True):
        review = self.fixture["reviews"][str(number)]["review_id"]
        connection = http.client.HTTPConnection(*self.server.server_address, timeout=15)
        headers = {"Content-Type": "application/json"}
        if authenticated:
            headers["Authorization"] = "Bearer " + self.state.session
        try:
            connection.request("POST" if body is not None else "GET", "/v1/reviews/" + review + "/hld" + suffix,
                               json.dumps(body) if body is not None else None, headers)
            response = connection.getresponse()
            raw = response.read()
            return response.status, json.loads(raw) if response.headers.get("Content-Type", "").startswith("application/json") else {}
        finally:
            connection.close()

    def view(self, number=99):
        status, value = self.request(number)
        self.assertEqual(status, 200, value)
        return value

    def preview(self, number=99, interpretation="as_built", side="baseline", corrections=None):
        value = self.view(number)
        status, preview = self.request(number, "/preview", {
            "design_id": value[side]["id"], "interpretation": interpretation,
            "corrections": corrections or [], "expected_revision": 0})
        self.assertEqual(status, 200, preview)
        return value, preview

    def accept(self, preview, number=99, gaps=True, activate=True, **overrides):
        return self.request(number, "/accept", {
            "preview_id": preview["preview_id"], "expected_revision": preview["expected_revision"],
            "input_fingerprint": preview["input_fingerprint"], "known_gaps_accepted": gaps,
            "activate_default": activate, **overrides})

    def test_native_hld_and_exact_source_export_are_bound_to_real_git_snapshots(self):
        value = self.view()
        self.assertEqual(value["delta"]["outcome"], "change_detected")
        self.assertEqual(value["baseline"]["snapshot"], self.fixture["baseline"])
        self.assertEqual(value["proposal"]["snapshot"], self.fixture["proposal"])
        self.assertTrue(value["delta"]["changes"])
        self.assertNotIn("PRIVATE_", json.dumps(value))
        evidence = next(item for item in value["proposal"]["evidence"] if item["path"] == "domain/src/lib.rs")
        status, source = self.request(99, "/source?design_id=" + value["proposal"]["id"] + "&evidence_id=" + evidence["id"])
        self.assertEqual(status, 200, source)
        self.assertEqual((source["commit"], source["blob_id"], source["path"], source["start_line"]),
                         (evidence["commit"], evidence["blob"], evidence["path"], evidence["start_line"]))
        self.assertIn("Result", source["text"])
        status, exported = self.request(99, "/export?view=diff")
        self.assertEqual(status, 200)
        self.assertIn(self.fixture["baseline"], exported["markdown"])
        self.assertIn(self.fixture["proposal"], exported["markdown"])
        self.assertIn("```mermaid", exported["markdown"])
        draft = json.loads((Path(self.fixture["root"]) / "check-draft.json").read_text())
        self.assertIn("HLD: change_detected. LLD: unavailable", draft["output"]["summary"])
        self.assertEqual(draft["conclusion"], "neutral")
        self.assertTrue(draft["details_url"].endswith("&design=hld"))
        self.assertEqual(self.request(99, "/source?design_id=unknown&evidence_id=" + evidence["id"])[0], 404)

    def test_preview_pins_explicit_gaps_and_acceptance_survive_native_reload(self):
        before = self.view()
        component = next(item for item in before["baseline"]["elements"] if item["kind"] == "component")
        correction = {"id": "explicit-role", "kind": "responsibility", "target_id": component["id"],
                      "value": "Reviewer-confirmed payment responsibility", "paths": [], "related_ids": [],
                      "constraint_kind": None, "reference_design_id": None, "correspondence_targets": []}
        _, preview = self.preview(corrections=[correction])
        self.assertEqual(self.accept(preview, gaps=False)[0], 404)
        self.assertEqual(self.accept(preview, input_fingerprint="0" * 64)[0], 409)
        self.assertIsNone(self.view()["accepted"])
        status, accepted = self.accept(preview)
        self.assertEqual(status, 200, accepted)
        loaded = self.view()["accepted_record"]
        self.assertEqual(loaded["accepted_id"], accepted["accepted"]["accepted_id"])
        self.assertEqual(loaded["observed_design"]["elements"], before["baseline"]["elements"])
        self.assertEqual(loaded["review"]["confirmation"]["actor"], "github:fixture-user")
        self.assertTrue(loaded["active_default"])
        self.assertNotEqual(loaded["design"]["id"], loaded["observed_design"]["id"])
        # Each HTTP operation launches and closes the real adapter/SQLite
        # connection, so this is a durable native reload, not browser memory.
        self.assertEqual(self.view()["accepted"]["snapshot"], self.fixture["baseline"])

    def test_intended_default_remains_separate_and_other_pr_cannot_accept_preview(self):
        _, preview = self.preview(interpretation="intended", side="proposal")
        self.assertEqual(self.accept(preview, number=100)[0], 404)
        status, accepted = self.accept(preview)
        self.assertEqual(status, 200, accepted)
        value = self.view()
        self.assertIsNone(value["accepted"])
        self.assertEqual(value["accepted_intended"]["snapshot"], self.fixture["proposal"])
        self.assertEqual(self.request(99, "/export?view=accepted")[0], 404)

    def test_force_push_after_native_preview_rolls_back_live_acceptance(self):
        _, preview = self.preview()
        self.heads["99"] = self.fixture["helper"]
        self.assertEqual(self.accept(preview)[0], 404)
        value = self.view()
        self.assertIsNone(value["accepted"])
        # The private preview remains retryable; failed publication did not
        # commit a hidden confirmation into the live database.
        self.heads["99"] = self.fixture["proposal"]
        self.assertEqual(self.accept(preview)[0], 200)

    def test_revocation_after_native_work_denies_publication_and_source(self):
        _, preview = self.preview()
        native = self.state.native_onboarding_request

        def revoke(*args, **kwargs):
            if args[0] != "hld-accept":
                return native(*args, **kwargs)
            original_fence = kwargs["publication_fence"]

            def fence():
                self.permissions["pull"] = False
                original_fence()

            return native(*args, **{**kwargs, "publication_fence": fence})

        self.state.native_onboarding_request = revoke
        self.assertEqual(self.accept(preview)[0], 404)
        self.assertEqual(self.request(99)[0], 404)
        self.state.native_onboarding_request = native
        self.permissions["pull"] = True
        self.assertIsNone(self.view()["accepted"])

    def test_private_helper_and_actual_merge_have_independent_designs_and_drift(self):
        helper = self.view(100)
        self.assertEqual(helper["delta"]["changes"], [])
        self.assertEqual(helper["delta"]["outcome"], "incomplete")
        self.assertTrue(helper["proposal"]["coverage"]["gaps"])
        _, preview = self.preview(side="proposal")
        self.assertEqual(self.accept(preview)[0], 200)
        accepted_id = self.view()["accepted_record"]["accepted_id"]
        merged = self.view(101)
        self.assertEqual(merged["proposal"]["snapshot"], self.fixture["merged"])
        self.assertNotEqual(merged["proposal"]["snapshot"], self.fixture["proposal"])
        self.assertEqual(merged["accepted_record"]["accepted_id"], accepted_id)
        self.assertEqual(merged["accepted"]["snapshot"], self.fixture["proposal"])
        self.assertTrue(merged["accepted_drift"]["changes"])
        self.assertNotEqual(merged["proposal"]["id"], helper["proposal"]["id"])

    def test_kill_switch_keeps_ordinary_reviews_history_and_disables_new_confirmation(self):
        _, preview = self.preview()
        self.assertEqual(self.accept(preview)[0], 200)
        self.state.hld_enabled = False
        value = self.view()
        self.assertFalse(value["can_accept"])
        self.assertIsNotNone(value["accepted"])
        self.assertEqual(self.request(99, "/preview", {"design_id": value["proposal"]["id"], "interpretation": "as_built", "corrections": [], "expected_revision": 0})[0], 503)
        self.assertEqual(self.request(102)[0], 404)
        ordinary = self.state.report(self.fixture["reviews"]["102"]["review_id"])
        self.assertTrue(ordinary["files"])
        self.assertIsNone(ordinary["artifact"].get("hld"))
        self.assertEqual(ordinary["artifact"]["lld_status"], "unavailable")

    def test_browser_cannot_override_operator_path_actor_or_compare_identity(self):
        self.assertEqual(self.request(99, authenticated=False)[0], 401)
        value = self.view()
        for field, injected in (("artifact_path", "/etc/passwd"), ("principal", "other"), ("comparison_id", "pr-100"), ("create_preview", True)):
            status, _ = self.request(99, "/preview", {"design_id": value["baseline"]["id"], "interpretation": "as_built",
                                                      "corrections": [], "expected_revision": 0, field: injected})
            self.assertEqual(status, 400)
        self.assertIsNone(self.view()["accepted"])

    def test_explicit_document_intent_has_exact_source_and_stale_doc_question(self):
        before = self.view()
        component = next(item for item in before["baseline"]["elements"] if item["kind"] == "component")
        candidate = next(item for item in before["baseline"]["document_candidates"]
                         if any(evidence["id"] == item["evidence_id"] and evidence["path"] == "docs/architecture.md"
                                for evidence in before["baseline"]["evidence"]))
        correction = {"id": "selected-doc", "kind": "document_reference", "target_id": component["id"],
                      "value": "Reviewer-selected intent: API depends on the domain, verify the changed contract",
                      "paths": [], "related_ids": [], "constraint_kind": None, "reference_design_id": None,
                      "correspondence_targets": [], "document_evidence_id": candidate["evidence_id"]}
        _, preview = self.preview(corrections=[correction])
        self.assertEqual(self.accept(preview)[0], 200)
        loaded = self.view()
        self.assertEqual(loaded["accepted"]["decision_records"][0]["resolution"], "user_assertion")
        self.assertEqual(loaded["accepted_record"]["observed_design"]["decision_records"], [])
        source_status, source = self.request(99, "/source?design_id=" + loaded["accepted"]["id"] + "&evidence_id=" + candidate["evidence_id"])
        self.assertEqual(source_status, 200)
        self.assertEqual(source["path"], "docs/architecture.md")
        self.assertIn("Intent: Payments API calls", source["text"])
        merged = self.view(101)
        finding = next(item for item in merged["delta"]["findings"] if item["constraint_id"] == "selected-doc")
        self.assertEqual(finding["resolution"], "unknown")
        self.assertIn("document changed", finding["message"])
        self.assertTrue(finding["evidence_ids"])


if __name__ == "__main__":
    unittest.main()

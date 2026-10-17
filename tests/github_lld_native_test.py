"""Real HTTP → Rust → SQLite LLD boundaries; provider responses are inert fixtures."""
import http.client
import json
from pathlib import Path
import sys
import tempfile
import unittest
from urllib.parse import urlencode

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import github_mvp_server as host
import github_hld_native_test as hld_test
from phase5_browser_fixture import build


class LldNativeHttpTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="csb-lld-native-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.fixture = build(Path(cls.temporary.name) / "corpus")

    setUp = hld_test.HldNativeHttpTest.setUp
    stop_server = hld_test.HldNativeHttpTest.stop_server

    def request(self, number=99, operation="catalog", body=None, query=None, authenticated=True):
        item = self.fixture["reviews"][str(number)]
        pins = {"parent_hld": item.get("proposal_design", "unavailable")}
        if query:
            pins.update(query)
        connection = http.client.HTTPConnection(*self.server.server_address, timeout=15)
        headers = {"Content-Type": "application/json"}
        if authenticated:
            headers["Authorization"] = "Bearer " + self.state.session
        try:
            target = "/v1/reviews/" + item["review_id"] + "/lld/" + operation
            if body is None:
                target += "?" + urlencode(pins)
            else:
                body = {**pins, **body}
            connection.request("POST" if body is not None else "GET", target,
                               json.dumps(body) if body is not None else None, headers)
            response = connection.getresponse()
            raw = response.read()
            return response.status, json.loads(raw) if response.headers.get("Content-Type", "").startswith("application/json") else {}
        finally:
            connection.close()

    def catalog(self, number=99, **query):
        status, value = self.request(number, query=query)
        self.assertEqual(status, 200, value)
        return value

    def detail(self, number=99, name="handle"):
        entry = next(row["proposal"] or row["baseline"] for row in self.catalog(number)["entries"]
                     if (row["proposal"] or row["baseline"])["name"] == name)
        status, value = self.request(number, "detail", query={"entry": entry["id"]})
        self.assertEqual(status, 200, value)
        return value

    def preview(self, number=99, name="handle", interpretation="as_built"):
        detail = self.detail(number, name)
        selected = detail["proposal"] or detail["baseline"]
        status, preview = self.request(number, "preview", body={
            "entry": detail["entry_id"], "design_id": selected["id"],
            "expected_revision": 0, "interpretation": interpretation,
            "corrections": [{"id": "reviewer-invariant", "kind": "lld_invariant",
                             "target_id": selected["entry"]["id"], "value": "Reject invalid payment inputs; requires verification",
                             "paths": [], "related_ids": [], "constraint_kind": None,
                             "reference_design_id": None, "correspondence_targets": []}]})
        self.assertEqual(status, 200, preview)
        return detail, preview

    def accept(self, detail, preview, number=99, **overrides):
        return self.request(number, "accept", body={"entry": detail["entry_id"],
            "preview_id": preview["preview_id"], "expected_revision": preview["expected_revision"],
            "input_fingerprint": preview["input_fingerprint"], "known_gaps_accepted": True,
            "activate_default": True, **overrides})

    def test_real_hld_lld_function_and_exact_source_handoff_has_no_private_ir(self):
        catalog = self.catalog()
        self.assertEqual(catalog["delta"]["outcome"], "change_detected")
        self.assertNotIn("inputs", json.dumps(catalog))
        self.assertNotIn("PRIVATE_", json.dumps(catalog))
        self.assertEqual(catalog["baseline"]["snapshot"], self.fixture["baseline"])
        detail = self.detail()
        self.assertEqual(detail["delta"]["outcome"], "change_detected")
        proposal = detail["proposal"]
        self.assertEqual(proposal["label"], "Static local control flow")
        self.assertEqual(proposal["coverage"]["runtime_validation"], "unavailable")
        self.assertTrue(any(edge["kind"] == "propagate_residual" for edge in proposal["graph"]["edges"]))
        self.assertTrue(any(change["kind"] == "condition_changed" for change in detail["delta"]["changes"]))
        evidence = next(item for item in proposal["evidence"] if item["relationship_type"] == "lld_declaration")
        status, source = self.request(operation="source", query={"entry": detail["entry_id"], "evidence": evidence["id"]})
        self.assertEqual(status, 200, source)
        self.assertEqual((source["commit"], source["path"], source["blob_id"], source["start_line"]),
                         (evidence["commit"], evidence["path"], evidence["blob"], evidence["start_line"]))
        self.assertIn("charge()?", source["text"])

    def test_private_behavior_changes_are_independent_from_hld(self):
        detail = self.detail(100, "private_helper")
        self.assertEqual(detail["delta"]["outcome"], "change_detected")
        artifact = self.state.report(self.fixture["reviews"]["100"]["review_id"])["artifact"]
        self.assertEqual(artifact["hld"]["delta"]["changes"], [])
        self.assertEqual(artifact["hld_status"], "incomplete")
        self.assertTrue(any(change["kind"] == "call_observation_changed" for change in detail["delta"]["changes"]))

    def test_source_outside_evidence_and_arbitrary_operator_paths_are_rejected(self):
        detail = self.detail()
        evidence = detail["proposal"]["evidence"][0]
        self.assertEqual(self.request(operation="source", query={"entry": detail["entry_id"],
            "evidence": evidence["id"], "start_line": evidence["start_line"], "end_line": evidence["end_line"] + 1})[0], 404)
        self.assertEqual(self.request(query={"artifact_path": "/tmp/other.json"})[0], 400)
        self.assertEqual(self.request(operation="source", query={"entry": detail["entry_id"], "evidence": "forged"})[0], 404)
        self.assertEqual(self.request(operation="detail", query={"entry": "forged"})[0], 404)
        self.assertEqual(self.request(authenticated=False)[0], 401)

    def test_cursor_cannot_cross_comparison_or_component_and_limit_is_bounded(self):
        catalog = self.catalog(limit=1)
        self.assertEqual(len(catalog["entries"]), 1)
        self.assertTrue(catalog["next_cursor"])
        self.assertEqual(self.request(100, query={"cursor": catalog["next_cursor"]})[0], 409)
        self.assertEqual(self.request(query={"cursor": catalog["next_cursor"], "component": "unknown"})[0], 404)
        self.assertEqual(self.request(query={"limit": 101})[0], 400)
        second = self.catalog(cursor=catalog["next_cursor"], limit=1)
        self.assertNotEqual(catalog["entries"], second["entries"])

    def test_invariant_acceptance_pins_body_and_does_not_replace_hld_default(self):
        detail, preview = self.preview()
        self.assertEqual(self.accept(detail, preview, input_fingerprint="0" * 64)[0], 409)
        status, accepted = self.accept(detail, preview)
        self.assertEqual(status, 200, accepted)
        stored = self.detail()["accepted"]
        self.assertEqual(stored["accepted_id"], accepted["accepted"]["accepted_id"])
        data = stored["design"]["lld"]
        self.assertEqual(data["invariants"][0]["body_fingerprint"], stored["observed_design"]["lld"]["behavior_fingerprint"])
        self.assertEqual(data["invariants"][0]["resolution"], "user_assertion")
        self.assertEqual(stored["observed_design"]["lld"]["invariants"], [])
        report = self.state.report(self.fixture["reviews"]["99"]["review_id"])
        status, hld = self.state.hld_request(report)
        self.assertEqual(status, 200)
        self.assertIsNone(hld["accepted"])
        self.assertIsNone(hld["accepted_intended"])

    def test_intended_and_as_built_are_distinct_and_changed_body_is_not_approved(self):
        detail, preview = self.preview(100, "private_helper", "intended")
        self.assertEqual(self.accept(detail, preview, 100)[0], 200)
        loaded = self.detail(100, "private_helper")
        self.assertIsNone(loaded["accepted"])
        self.assertIsNotNone(loaded["accepted_intended"])
        other = self.detail(99, "private_helper")
        self.assertNotEqual(other["proposal"]["behavior_fingerprint"], loaded["proposal"]["behavior_fingerprint"])
        self.assertEqual(other["accepted_intended"]["observed_design"]["snapshot"], self.fixture["helper"])
        self.assertEqual(self.accept(detail, preview, 99)[0], 404)

    def test_force_push_after_preview_never_publishes_a_default(self):
        detail, preview = self.preview()
        self.heads["99"] = self.fixture["helper"]
        self.assertEqual(self.accept(detail, preview)[0], 404)
        self.assertIsNone(self.detail()["accepted"])

    def test_origin_changed_after_staged_acceptance_never_publishes_a_default(self):
        detail, preview = self.preview()
        original_command, original_origin = self.state.command, self.state.repository_slug
        staged_accepts = []

        def change_origin_after_native(args, **kwargs):
            result = original_command(args, **kwargs)
            if Path(args[0]).name == "csb-onboarding" and kwargs.get("input_text"):
                payload = json.loads(kwargs["input_text"])
                if payload["action"] == "lld-accept":
                    staged_accepts.append(payload["action"])
                    self.state.repository_slug = lambda: "fixture/other-repository"
            return result

        self.state.command = change_origin_after_native
        try:
            self.assertEqual(self.accept(detail, preview)[0], 404)
        finally:
            self.state.repository_slug = original_origin
            self.state.command = original_command
        self.assertEqual(staged_accepts, ["lld-accept"])
        self.assertIsNone(self.detail()["accepted"])
        self.assertEqual(list((self.state.output / ".onboarding").glob(".authority-*")), [])

    def test_changed_actor_after_native_detail_cannot_deliver_previous_actor_source(self):
        detail = self.detail()
        evidence = detail["proposal"]["evidence"][0]
        original_command = self.state.command
        requests = []

        def change_actor_after_native(args, **kwargs):
            result = original_command(args, **kwargs)
            if Path(args[0]).name == "csb-onboarding" and kwargs.get("input_text"):
                payload = json.loads(kwargs["input_text"])
                if payload["action"] == "lld-source":
                    requests.append(payload["action"])
                    self.identity = "other-fixture-user"
            return result

        self.state.command = change_actor_after_native
        status, source = self.request(operation="source", query={"entry": detail["entry_id"], "evidence": evidence["id"]})
        self.assertEqual(status, 404)
        self.assertEqual(requests, ["lld-source"])
        self.assertNotIn("text", source)

    def test_viewer_can_read_but_cannot_confirm_and_revocation_blocks_cached_parents(self):
        self.catalog()
        self.permissions["push"] = False
        detail = self.detail()
        self.assertFalse(detail["can_accept"])
        self.assertEqual(self.request(operation="preview", body={"entry": detail["entry_id"],
            "design_id": detail["proposal"]["id"], "interpretation": "intended", "corrections": [], "expected_revision": 0})[0], 404)
        self.permissions["pull"] = False
        self.assertEqual(self.request()[0], 404)
        self.assertEqual(self.request(operation="detail", query={"entry": detail["entry_id"]})[0], 404)

    def test_export_and_generation_switch_do_not_expand_authority(self):
        detail = self.detail()
        status, exported = self.request(operation="export", query={"entry": detail["entry_id"], "view": "diff"})
        self.assertEqual(status, 200, exported)
        self.assertIn("HLD outcome: change_detected", exported["markdown"])
        self.assertIn("LLD outcome: change_detected", exported["markdown"])
        self.state.lld_enabled = False
        self.assertEqual(self.request(operation="detail", query={"entry": detail["entry_id"]})[0], 503)
        self.assertEqual(self.request()[0], 200)
        self.assertEqual(self.request(102)[0], 404)

    def test_kill_switch_preserves_authorized_immutable_lld_history(self):
        detail, preview = self.preview()
        self.assertEqual(self.accept(detail, preview)[0], 200)
        self.state.lld_enabled = False
        status, history = self.request(operation="history", query={"entry": detail["entry_id"]})
        self.assertEqual(status, 200, history)
        self.assertFalse(history["generation_enabled"])
        self.assertEqual(history["accepted"]["observed_design"]["snapshot"], self.fixture["proposal"])
        self.assertTrue(history["accepted"]["observed_design"]["lld"]["detail"]["graph"]["nodes"])
        self.permissions["pull"] = False
        self.assertEqual(self.request(operation="history", query={"entry": detail["entry_id"]})[0], 404)

    def test_empty_counterpart_cannot_turn_duplicate_declarations_into_proven_additions(self):
        catalog = self.catalog(104)
        occurrences = [row["proposal"] for row in catalog["entries"] if row.get("proposal") and row["proposal"]["name"] == "private_helper"]
        self.assertEqual(len(occurrences), 2)
        for override in ({}, {"baseline_entry": ""}):
            status, detail = self.request(104, "detail", query={"entry": occurrences[0]["id"], **override})
            self.assertEqual(status, 200, detail)
            self.assertEqual(detail["delta"]["outcome"], "incomplete")
            self.assertFalse(any(change["classification"] == "added" for change in detail["delta"]["changes"]))
        self.assertEqual(self.request(104, "preview", body={"entry": occurrences[0]["id"],
            "design_id": "forged", "interpretation": "intended", "corrections": [],
            "expected_revision": 0, "baseline_entry": []})[0], 400)


if __name__ == "__main__":
    unittest.main()

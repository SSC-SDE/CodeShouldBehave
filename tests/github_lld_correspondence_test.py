"""Independent native correspondence and current-context regressions.

Only provider replies are inert fixtures. HTTP, immutable Git source analysis,
shared Rust interpretations and staged bundled SQLite remain real. No reviewed
code, Git hooks, remote provider requests or deployments are executed.
"""
import copy
import json
from pathlib import Path
import tempfile
import unittest

import github_lld_native_test as native


class LldCorrespondenceNativeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="csb-lld-correspondence-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.fixture = native.build(Path(cls.temporary.name) / "corpus")

    # Compose transport/setup helpers without inheriting or rerunning the
    # original class's acceptance cases as if they were independent evidence.
    setUp = native.LldNativeHttpTest.setUp
    stop_server = native.LldNativeHttpTest.stop_server
    request = native.LldNativeHttpTest.request
    catalog = native.LldNativeHttpTest.catalog
    detail = native.LldNativeHttpTest.detail

    def rename_pair(self):
        rows = self.catalog(103, limit=100)["entries"]
        before = next(row["baseline"] for row in rows if row["baseline"]
                      and row["baseline"]["kind"] == "function"
                      and row["baseline"]["name"] == "private_helper")
        after = next(row["proposal"] for row in rows if row["proposal"]
                     and row["proposal"]["kind"] == "function"
                     and row["proposal"]["name"] == "guard_payment")
        return before, after

    def paired_detail(self):
        before, after = self.rename_pair()
        status, value = self.request(103, "detail", query={
            "entry": after["id"], "baseline_entry": before["id"]})
        self.assertEqual(status, 200, value)
        self.assertEqual(value["baseline"]["entry"]["id"], before["id"])
        self.assertEqual(value["proposal"]["entry"]["id"], after["id"])
        return value

    def correction(self, detail):
        return {"id": "declared-function-rename", "kind": "correspondence",
                "target_id": detail["proposal"]["entry"]["id"], "value": "rename",
                "paths": [], "related_ids": [detail["baseline"]["entry"]["id"]],
                "constraint_kind": None, "reference_design_id": detail["baseline"]["id"],
                "correspondence_targets": [detail["proposal"]["entry"]["id"]]}

    def preview_rename(self, detail=None, correction=None, **overrides):
        detail = detail or self.paired_detail()
        status, value = self.request(103, "preview", body={
            "entry": detail["proposal"]["entry"]["id"],
            "baseline_entry": detail["baseline"]["entry"]["id"],
            "design_id": detail["proposal"]["id"], "interpretation": "as_built",
            "expected_revision": 0, "corrections": [correction or self.correction(detail)],
            **overrides})
        return status, value, detail

    def accept_rename(self, preview, detail, **overrides):
        return self.request(103, "accept", body={
            "entry": detail["proposal"]["entry"]["id"],
            "baseline_entry": detail["baseline"]["entry"]["id"],
            "preview_id": preview["preview_id"], "expected_revision": preview["expected_revision"],
            "input_fingerprint": preview["input_fingerprint"], "known_gaps_accepted": True,
            "activate_default": True, **overrides})

    def test_declared_rename_is_exact_source_traceability_and_not_equivalence(self):
        detail = self.paired_detail()
        self.assertEqual(detail["baseline"]["entry"]["body_fingerprint"],
                         detail["proposal"]["entry"]["body_fingerprint"])
        self.assertTrue(any("do not establish behavioral equivalence" in gap
                            for gap in detail["delta"]["limitations"]))
        for side, name in (("baseline", "private_helper"), ("proposal", "guard_payment")):
            evidence = next(item for item in detail[side]["evidence"]
                            if item["relationship_type"] == "lld_declaration")
            status, source = self.request(103, "source", query={
                "entry": detail["proposal"]["entry"]["id"],
                "baseline_entry": detail["baseline"]["entry"]["id"], "evidence": evidence["id"]})
            self.assertEqual(status, 200, source)
            self.assertEqual(source["commit"], evidence["commit"])
            self.assertIn(name, source["text"])
        status, preview, _ = self.preview_rename(detail)
        self.assertEqual(status, 200, preview)
        assertion = preview["design"]["correspondence"][0]
        self.assertEqual(assertion["kind"], "rename")
        self.assertEqual(assertion["resolution"], "user_assertion")
        self.assertEqual(assertion["from_ids"], [detail["baseline"]["entry"]["id"]])
        self.assertEqual(assertion["to_ids"], [detail["proposal"]["entry"]["id"]])
        self.assertEqual(preview["design"]["lld"]["detail"], detail["proposal"])
        status, accepted = self.accept_rename(preview, detail)
        self.assertEqual(status, 200, accepted)
        loaded = self.paired_detail()["accepted"]
        self.assertEqual(loaded["accepted_id"], accepted["accepted"]["accepted_id"])
        self.assertEqual(loaded["observed_design"]["correspondence"], [])
        self.assertEqual(loaded["observed_design"]["lld"]["invariants"], [])
        self.assertEqual(loaded["design"]["correspondence"], [assertion])
        self.assertEqual(loaded["review"]["confirmation"]["actor"], "github:fixture-user")

    def test_wrong_kind_component_unbound_reference_and_cross_comparison_are_denied(self):
        before, after = self.rename_pair()
        rows = self.catalog(103, limit=100)["entries"]
        wrong_kind = next(row["baseline"] for row in rows if row["baseline"]
                          and row["baseline"]["kind"] == "configuration")
        self.assertEqual(self.request(103, "detail", query={
            "entry": after["id"], "baseline_entry": wrong_kind["id"]})[0], 404)
        outside_component = next(row["baseline"] for row in rows if row["baseline"]
                                 and row["baseline"]["kind"] == "function"
                                 and row["baseline"]["name"] == "charge")
        self.assertEqual(self.request(103, "detail", query={
            "entry": after["id"], "baseline_entry": outside_component["id"],
            "component": after["parent"]["component_id"]})[0], 404)
        other = self.detail(99, "handle")
        self.assertEqual(self.request(103, "detail", query={
            "entry": after["id"], "baseline_entry": other["proposal"]["entry"]["id"]})[0], 404)
        detail = self.paired_detail()
        correction = self.correction(detail)
        correction["reference_design_id"] = other["proposal"]["id"]
        self.assertEqual(self.preview_rename(detail, correction)[0], 404)
        correction = self.correction(detail)
        correction["reference_design_id"] = detail["proposal"]["id"]
        correction["related_ids"] = [detail["proposal"]["entry"]["id"]]
        self.assertEqual(self.preview_rename(detail, correction)[0], 404)
        correction = self.correction(detail)
        correction["correspondence_targets"] = [before["id"]]
        self.assertEqual(self.preview_rename(detail, correction)[0], 404)

    def test_optimistic_preview_force_push_and_other_comparison_cannot_transfer_receipt(self):
        status, preview, detail = self.preview_rename()
        self.assertEqual(status, 200, preview)
        self.assertEqual(self.accept_rename(preview, detail,
                         expected_revision=preview["expected_revision"] + 1)[0], 409)
        self.assertEqual(self.accept_rename(preview, detail, input_fingerprint="0" * 64)[0], 409)
        other = self.detail(100, "private_helper")
        status, _ = self.request(100, "accept", body={
            "entry": other["entry_id"], "preview_id": preview["preview_id"],
            "expected_revision": preview["expected_revision"], "input_fingerprint": preview["input_fingerprint"],
            "known_gaps_accepted": True, "activate_default": True})
        self.assertEqual(status, 404)
        self.heads["103"] = self.fixture["proposal"]
        self.assertEqual(self.accept_rename(preview, detail)[0], 404)
        self.assertIsNone(self.paired_detail()["accepted"])

    def test_default_isolation_preserves_hld_as_built_intended_and_distinct_functions(self):
        other_detail, other_preview = native.LldNativeHttpTest.preview(self, 99, "handle", "intended")
        status, other_accepted = native.LldNativeHttpTest.accept(self, other_detail, other_preview, 99)
        self.assertEqual(status, 200, other_accepted)
        status, preview, detail = self.preview_rename()
        self.assertEqual(status, 200, preview)
        self.assertEqual(self.accept_rename(preview, detail)[0], 200)
        renamed = self.paired_detail()
        self.assertIsNotNone(renamed["accepted"])
        self.assertIsNone(renamed["accepted_intended"])
        original = self.detail(99, "handle")
        self.assertIsNone(original["accepted"])
        self.assertEqual(original["accepted_intended"]["accepted_id"],
                         other_accepted["accepted"]["accepted_id"])
        self.assertEqual(original["accepted_intended"]["observed_design"]["lld"]["logical_key"],
                         other_detail["proposal"]["entry"]["logical_key"])
        report = self.state.report(self.fixture["reviews"]["103"]["review_id"])
        status, hld = self.state.hld_request(report)
        self.assertEqual(status, 200, hld)
        self.assertIsNone(hld["accepted"])
        self.assertIsNone(hld["accepted_intended"])

    def test_fresh_viewer_revocation_denies_cached_correspondence_and_source(self):
        detail = self.paired_detail()
        evidence = detail["proposal"]["evidence"][0]
        self.permissions["pull"] = False
        self.assertEqual(self.request(103, "detail", query={
            "entry": detail["proposal"]["entry"]["id"],
            "baseline_entry": detail["baseline"]["entry"]["id"]})[0], 404)
        self.assertEqual(self.request(103, "source", query={
            "entry": detail["proposal"]["entry"]["id"],
            "baseline_entry": detail["baseline"]["entry"]["id"], "evidence": evidence["id"]})[0], 404)
        self.assertEqual(self.preview_rename(detail)[0], 404)

    def test_accepted_context_narrowing_rechecks_historical_catalog_and_source_authority(self):
        detail = self.paired_detail()
        evidence = detail["proposal"]["evidence"][0]
        session_id = "narrow-lld-correspondence"
        identity = {"security_domain": "local", "provider": "github", "owner": "fixture", "name": "lld-review"}
        status, session = self.state.onboarding_request("create", session_id, {
            "id": session_id, "reference": self.fixture["baseline"],
            "scope_version": "narrow-lld-context", "scope": {
                "primary_repository": identity, "primary_include": ["api"],
                "primary_exclude": [], "contexts": [], "contracts": [],
                "export_allowed": True, "model_destinations": []}})
        self.assertEqual(status, 200, session)

        def mutate(action, **fields):
            nonlocal session
            status, value = self.state.onboarding_request(action, session_id, {
                "security_domain": "local", "expected_revision": session["revision"], **fields})
            self.assertEqual(status, 200, (action, value))
            session = value

        for _ in range(32):
            mutate("scan")
            if session["inventory"]["complete"]:
                break
        self.assertTrue(session["inventory"]["complete"])
        mutate("detect")
        mutate("inspect")
        for original in session["components"]:
            component = copy.deepcopy(original)
            component["decision"] = "corrected"
            mutate("decisions", component=component)
        for index, original in enumerate(session["providers"]):
            provider = copy.deepcopy(original)
            provider["decision"] = "confirmed"
            mutate("provider-decisions", provider_index=index, provider=provider)
        for original in session["integrations"]:
            mutate("integration-decisions", integration_id=original["id"], decision="include")
        mutate("confirm", proposal_fingerprint=session["proposal_fingerprint"],
               known_gaps_accepted=True, activate_defaults=True)
        self.assertEqual(self.request(103)[0], 404)
        self.assertEqual(self.request(103, "source", query={
            "entry": detail["proposal"]["entry"]["id"],
            "baseline_entry": detail["baseline"]["entry"]["id"], "evidence": evidence["id"]})[0], 404)
        self.assertEqual(self.preview_rename(detail)[0], 404)


if __name__ == "__main__":
    unittest.main()

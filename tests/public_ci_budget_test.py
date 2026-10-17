"""Application REST tickets; entirely offline, with no provider or native code."""
from concurrent.futures import ThreadPoolExecutor
import copy
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("test_public_ci_budget_module", ROOT / "scripts/public_ci_budget.py")
budget = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(budget)


def reserve_child(directory, manifest, sender):
    stage = budget.Budget(directory, manifest, "plan")
    successes = 0
    for _ in range(8):
        try:
            stage.reserve("GET", "repos/fixture/repo")
            successes += 1
        except budget.Refusal:
            pass
    sender.send(successes)
    sender.close()


class RestPassBudgetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="csb-budget-test-")
        self.root = Path(self.temporary.name)
        self.config = {"repository": {"id": 424242}, "workflow_sha256": "f" * 64}
        self.manifest = budget.make_manifest(self.config, 999, 1, ["a" * 64, "b" * 64, "c" * 64])
        self.environment = patch.dict(os.environ, {"GITHUB_ACTIONS": "false"})
        self.environment.start()

    def tearDown(self):
        self.environment.stop()
        self.temporary.cleanup()

    def test_disjoint_allocations_sum100_and_cannot_be_expanded(self):
        self.assertEqual(sum(self.manifest["allocations"].values()), 100)
        for role in budget.LIMITS:
            invalid = copy.deepcopy(self.manifest)
            invalid["allocations"][role] += 1
            invalid["pass_id"] = budget.sha(budget.canonical(budget.pass_context(invalid)))
            with self.subTest(role=role), self.assertRaises(budget.Refusal):
                budget.validate_manifest(invalid)
        invalid = copy.deepcopy(self.manifest)
        invalid["allocations"]["plan"] = 25.0
        with self.assertRaises(budget.Refusal):
            budget.validate_manifest(invalid)

    def test_request_assignment_does_not_reset_plan_and_cannot_be_reassigned(self):
        manifest = budget.make_manifest(self.config, 999, 1)
        first = budget.Budget(self.root, manifest, "plan")
        first.reserve("GET", "repos/fixture/repo")
        assigned = budget.assign_requests(manifest, ["a" * 64])
        self.assertEqual(manifest["pass_id"], assigned["pass_id"])
        self.assertEqual(budget.Budget(self.root, assigned, "plan").snapshot()["used"], 1)
        self.assertEqual(budget.analysis_role(assigned, "a" * 64), "analyze-0")
        for requests in (["b" * 64], ["a" * 64] * 2, [str(number) * 64 for number in range(4)]):
            with self.subTest(requests=requests), self.assertRaises(budget.Refusal):
                budget.assign_requests(assigned, requests)

    def test_new_cli_instances_reuse_counter_and_refuse_before_transport(self):
        transports = []
        for _ in range(25):
            stage = budget.Budget(self.root, self.manifest, "plan")
            stage.reserve("GET", "repos/fixture/repo/pulls")
            transports.append(True)
        with self.assertRaises(budget.Refusal):
            budget.Budget(self.root, self.manifest, "plan").reserve("POST", "repos/fixture/repo/deployments")
            transports.append(True)
        self.assertEqual(len(transports), 25)
        self.assertEqual(stage.snapshot()["remaining"], 0)

    def test_concurrent_reservations_never_overspend_role(self):
        stage = budget.Budget(self.root, self.manifest, "plan")
        def reserve(_):
            try:
                stage.reserve("GET", "repos/fixture/repo")
                return True
            except budget.Refusal:
                return False
        with ThreadPoolExecutor(max_workers=8) as workers:
            outcomes = list(workers.map(reserve, range(60)))
        self.assertEqual(sum(outcomes), 25)
        self.assertEqual(stage.snapshot()["used"], 25)

    def test_deleted_ledger_or_reset_used_counter_cannot_create_fresh_budget(self):
        stage = budget.Budget(self.root, self.manifest, "plan")
        stage.reserve("GET", "repos/fixture/repo")
        original = stage.path.read_bytes()
        value = json.loads(original)
        value["used"] = 0
        payload = {key: item for key, item in value.items() if key != "ledger_sha256"}
        value["ledger_sha256"] = budget.sha(budget.canonical(payload))
        stage.path.write_bytes(budget.canonical(value))
        with self.assertRaises(budget.Refusal):
            budget.Budget(self.root, self.manifest, "plan")
        stage.path.write_bytes(original)
        stage.path.unlink()
        with self.assertRaises(budget.Refusal):
            budget.Budget(self.root, self.manifest, "plan")

    def test_separate_processes_share_the_same_durable_role_limit(self):
        stage = budget.Budget(self.root, self.manifest, "plan")
        context = multiprocessing.get_context("fork")
        children, readers = [], []
        try:
            for _ in range(5):
                reader, writer = context.Pipe(duplex=False)
                child = context.Process(target=reserve_child, args=(self.root, self.manifest, writer))
                child.start()
                writer.close()
                children.append(child)
                readers.append(reader)
            self.assertTrue(all(reader.poll(5) for reader in readers))
            self.assertEqual(sum(reader.recv() for reader in readers), 25)
            for child in children:
                child.join(5)
                self.assertEqual(child.exitcode, 0)
            self.assertEqual(stage.snapshot()["used"], 25)
        finally:
            for child in children:
                if child.is_alive():
                    child.terminate(); child.join()
            for reader in readers:
                reader.close()

    def test_cross_run_config_request_and_actual_job_are_denied(self):
        budget.Budget(self.root, self.manifest, "plan").reserve("GET", "repos/fixture/repo")
        for manifest in (budget.make_manifest(self.config, 999, 2),
                         budget.make_manifest(dict(self.config, changed=True), 999, 1)):
            with self.assertRaises(budget.Refusal):
                budget.Budget(self.root, manifest, "plan")
        with self.assertRaises(budget.Refusal):
            budget.Budget(self.root, self.manifest, "analyze-0", "b" * 64)
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_RUN_ID": "999",
                                    "GITHUB_RUN_ATTEMPT": "1", "GITHUB_JOB": "publish"}):
            with self.assertRaises(budget.Refusal):
                budget.Budget(self.root, self.manifest, "analyze-0", "a" * 64)

    def test_failed_transport_keeps_conservative_reservation(self):
        stage = budget.Budget(self.root, self.manifest, "analyze-0", "a" * 64)
        with self.assertRaises(OSError):
            stage.reserve("GET", "repos/fixture/repo/pulls/1")
            raise OSError("simulated connection closed after dispatch")
        self.assertEqual(budget.Budget(self.root, self.manifest, "analyze-0", "a" * 64).snapshot()["used"], 1)

    def test_all_roles_sum100_and_missing_duplicate_corrupt_snapshots_fail(self):
        snapshots = []
        for role, cap in budget.LIMITS.items():
            request_id = self.manifest["analysis_requests"][int(role[-1])] if role.startswith("analyze-") else None
            stage = budget.Budget(self.root, self.manifest, role, request_id)
            for _ in range(cap):
                stage.reserve("GET", "repos/fixture/repo")
            snapshots.append(stage.snapshot())
        self.assertEqual(budget.aggregate(self.manifest, snapshots)["application_rest_calls"], 100)
        for invalid in (snapshots[:-1], snapshots + [snapshots[0]], [dict(snapshots[0], schema_version="fake"), *snapshots[1:]]):
            with self.assertRaises(budget.Refusal):
                budget.aggregate(self.manifest, invalid)

    def test_duplicate_json_keys_and_linked_state_are_denied(self):
        stage = budget.Budget(self.root, self.manifest, "plan")
        data = stage.path.read_bytes()
        stage.path.write_bytes(b'{"used":0,' + data[1:])
        with self.assertRaises(budget.Refusal):
            stage.snapshot()
        stage.path.unlink()
        target = self.root / "outside.json"
        target.write_bytes(data)
        stage.path.symlink_to(target)
        with self.assertRaises(budget.Refusal):
            stage.snapshot()


if __name__ == "__main__":
    unittest.main()

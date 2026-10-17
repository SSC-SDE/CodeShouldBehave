"""Offline oracle tests only; no native engine, provider or cloud traffic."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("csb_public_profile_replay", ROOT / "scripts/public_profile_replay.py")
replay = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = replay
SPEC.loader.exec_module(replay)


class OfflineProfileTests(unittest.TestCase):
    def test_qualifying_corpus_has_exact_window_revision_and_burst_counts(self):
        for seed in replay.SEEDS:
            events = replay.corpus(seed)
            arrival = [item for item in events if item["phase"] == "arrival"]
            burst = [item for item in events if item["phase"] == "burst"]
            self.assertEqual(len(arrival), 500)
            self.assertEqual(len(burst), 20)
            self.assertTrue(all(0 <= item["at"] < 28800 for item in arrival))
            self.assertEqual({item["at"] for item in burst}, {28860})
            self.assertEqual(len({item["pr_number"] for item in burst}), 20)
            for number in range(1, 101):
                self.assertEqual([item["revision"] for item in arrival if item["pr_number"] == number], [1, 2, 3, 4, 5])
            self.assertEqual(events, replay.corpus(seed))

    def test_small_replay_uses_real_state_machine_without_network_or_execution(self):
        # Small diagnostic corpus is deliberately not the qualifying scale gate.
        with patch.object(replay.ci, "GitHub", side_effect=AssertionError("Provider used")), \
                patch.object(replay.ci.subprocess, "run", side_effect=AssertionError("Subprocess used")):
            result = replay.run_seed(1729, count=12, revisions=3, burst_count=6, arrival_seconds=1200)
        self.assertEqual(result["latest_terminal_count"], 12)
        self.assertLessEqual(result["max_parallel_simulated"], 2)
        self.assertLessEqual(result["max_attempts_per_comparison"], 3)
        self.assertTrue(result["stale_completion_probe"]["old_completion_rejected"])
        self.assertEqual(result["source_heads"], 42)
        self.assertEqual(sum(result["terminal_history"].values()) + result["superseded_without_terminal"], 42)

    def test_oracle_detects_missing_stale_or_false_clean_terminal(self):
        config = replay.configuration()
        event = replay.corpus(1729, count=1, revisions=1, burst_count=0, arrival_seconds=60)[0]
        current = {1: replay.observation(config, event)}
        key = replay.comparison(config, current[1])
        state = replay.ci.plan(config, replay.ci.empty_state(config), list(current.values()), replay.stamp(0))["state"]
        state = replay.ci.finish_head(state, 1, event["proposal"], replay.expected_terminal(1729, 1, 1))
        replay.assert_converged(config, current, state, 1729, {key: 1})
        for mutation in ("missing", "stale", "wrong_outcome"):
            invalid = copy.deepcopy(state)
            if mutation == "missing":
                invalid["completed"] = []
            elif mutation == "stale":
                invalid["completed"][0]["comparison_key"] = "0" * 64
            else:
                invalid["completed"][0]["outcome"] = "unsupported" if invalid["completed"][0]["outcome"] != "unsupported" else "published"
            with self.subTest(mutation=mutation), self.assertRaises(replay.ReplayFailure):
                replay.assert_converged(config, current, invalid, 1729, {key: 1})

    def test_example_copy_cannot_be_mistaken_for_live_budget_authority(self):
        config = replay.configuration()
        self.assertEqual(config["repository"]["owner"], "fixture")
        self.assertTrue(all(value is False for value in config["operations"].values()))
        self.assertEqual(config["budget"]["proof_mode"], "unavailable")
        with self.assertRaises(replay.ci.Refusal):
            replay.ci.load_config(config, runtime=True)

    def test_incomplete_current_head_cannot_pass_as_published(self):
        config = replay.configuration()
        seed = 1729
        number = next(number for number in range(1, 101)
                      if replay.expected_terminal(seed, number, 1) == "unsupported")
        event = next(event for event in replay.corpus(seed)
                     if event["pr_number"] == number and event["revision"] == 1)
        current = {number: replay.observation(config, event)}
        key = replay.comparison(config, current[number])
        state = replay.ci.plan(config, replay.ci.empty_state(config), list(current.values()), replay.stamp(0))["state"]
        state = replay.ci.finish_head(state, number, event["proposal"], "unsupported")
        replay.assert_converged(config, current, state, seed, {key: 1})
        state["completed"][0]["outcome"] = "published"
        with self.assertRaises(replay.ReplayFailure):
            replay.assert_converged(config, current, state, seed, {key: 1})


if __name__ == "__main__":
    unittest.main()

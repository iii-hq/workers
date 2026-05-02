#!/usr/bin/env python3
import unittest

import collect_worker_interface


class CollectWorkerInterfaceTests(unittest.TestCase):
    def test_count_worker_matches_by_name_or_id(self):
        workers_json = {
            "workers": [
                {"name": "image-resize", "id": "worker-1"},
                {"name": "other", "id": "mcp"},
                {"name": "ignored"},
            ]
        }

        self.assertEqual(
            collect_worker_interface.count_worker_matches(workers_json, "image-resize"),
            1,
        )
        self.assertEqual(
            collect_worker_interface.count_worker_matches(workers_json, "mcp"),
            1,
        )
        self.assertEqual(
            collect_worker_interface.count_worker_matches(workers_json, "missing"),
            0,
        )

    def test_wait_for_worker_zero_wait_returns_latest_snapshot(self):
        original_run_iii = collect_worker_interface.run_iii
        try:
            snapshot = {"workers": [{"name": "mcp"}]}
            collect_worker_interface.run_iii = lambda _function_id, _payload: snapshot

            self.assertIs(
                collect_worker_interface.wait_for_worker("mcp", 0),
                snapshot,
            )
        finally:
            collect_worker_interface.run_iii = original_run_iii


if __name__ == "__main__":
    unittest.main()

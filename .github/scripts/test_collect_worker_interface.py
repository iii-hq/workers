#!/usr/bin/env python3
import contextlib
import io
import subprocess
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

    def test_collect_triggers_returns_none_when_engine_listing_fails(self):
        original_run_iii = collect_worker_interface.run_iii
        try:
            collect_worker_interface.run_iii = lambda _function_id, _payload: (_ for _ in ()).throw(
                subprocess.CalledProcessError(1, ["iii"])
            )

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                self.assertIsNone(collect_worker_interface.collect_triggers())
            self.assertIn("publishing triggers=[]", stderr.getvalue())
        finally:
            collect_worker_interface.run_iii = original_run_iii


if __name__ == "__main__":
    unittest.main()

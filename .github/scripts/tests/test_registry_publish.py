import urllib.error
import unittest
from unittest.mock import patch

import registry_publish
from registry_release import RegistryError


PAYLOAD = {"worker": "iii-directory", "version": "1.2.0"}


class RegistryPublishTests(unittest.TestCase):
    def test_publish_accepts_a_normal_success(self):
        with patch.object(registry_publish, "request_json", return_value=(200, {"ok": True})):
            result = registry_publish.publish(
                "https://registry", "secret", PAYLOAD, "iii-directory", "1.2.0", "next"
            )

        self.assertFalse(result["reconciled"])
        self.assertEqual(result["registry_response"], {"ok": True})

    def test_publish_reconciles_duplicate_or_ambiguous_transport_failure(self):
        failures = [
            (409, {"error": "already exists"}),
            urllib.error.URLError("timed out"),
        ]
        for failure in failures:
            with self.subTest(failure=failure):
                if isinstance(failure, Exception):
                    request = patch.object(registry_publish, "request_json", side_effect=failure)
                else:
                    request = patch.object(registry_publish, "request_json", return_value=failure)
                with request, patch.object(registry_publish, "resolve_version", return_value="1.2.0"):
                    result = registry_publish.publish(
                        "https://registry", "secret", PAYLOAD, "iii-directory", "1.2.0", "next"
                    )

                self.assertTrue(result["reconciled"])
                self.assertEqual(result["resolved_version"], "1.2.0")

    def test_publish_reports_unknown_state_when_transport_fails_and_channel_does_not_match(self):
        with (
            patch.object(registry_publish, "request_json", side_effect=urllib.error.URLError("timed out")),
            patch.object(registry_publish, "resolve_version", return_value="1.1.2"),
            self.assertRaisesRegex(RegistryError, "publication state is unknown"),
        ):
            registry_publish.publish(
                "https://registry", "secret", PAYLOAD, "iii-directory", "1.2.0", "next"
            )

    def test_publish_reports_unknown_state_when_reconciliation_also_times_out(self):
        with (
            patch.object(registry_publish, "request_json", side_effect=urllib.error.URLError("publish timed out")),
            patch.object(registry_publish, "resolve_version", side_effect=urllib.error.URLError("resolve timed out")),
            self.assertRaisesRegex(RegistryError, "reconciliation also failed"),
        ):
            registry_publish.publish(
                "https://registry", "secret", PAYLOAD, "iii-directory", "1.2.0", "next"
            )


if __name__ == "__main__":
    unittest.main()

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("provision-potion.py")
SPEC = importlib.util.spec_from_file_location("provision_potion", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


class ProvisionPotionTest(unittest.TestCase):
    def test_config_and_manifest_contract(self):
        MODULE.validate_config({"model_type": "model2vec", "hidden_dim": 256, "normalize": True})
        with self.assertRaises(ValueError):
            MODULE.validate_config({"model_type": "bert", "hidden_dim": 256, "normalize": True})
        manifest = MODULE.model_manifest()
        self.assertEqual(manifest["revision"], MODULE.REVISION)
        self.assertEqual(manifest["dimensions"], 256)

    def test_checksum_and_nonempty_destination_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = root / "payload"
            payload.write_bytes(b"wrong")
            with self.assertRaises(ValueError):
                MODULE.verify_file(payload, 5, "0" * 64)
            destination = root / "model"
            destination.mkdir()
            (destination / "existing").write_text("keep")
            with self.assertRaises(ValueError):
                MODULE.require_empty_destination(destination)


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path

from build_publish_payload import (
    build_payload,
    derive_registry_function_name,
    normalize_dependencies,
    normalize_worker_interface,
)


class PublishPayloadTests(unittest.TestCase):
    def test_normalize_dependencies_accepts_map(self):
        self.assertEqual(
            normalize_dependencies({"helper-worker": "^1.0.0"}),
            [{"name": "helper-worker", "version": "^1.0.0"}],
        )

    def test_normalize_dependencies_accepts_wire_list(self):
        deps = [{"name": "helper-worker", "version": "^1.0.0"}]
        self.assertEqual(normalize_dependencies(deps), deps)

    def test_normalize_dependencies_rejects_scalar(self):
        with self.assertRaisesRegex(ValueError, "dependencies"):
            normalize_dependencies("helper-worker")

    def test_derive_registry_function_name_prefers_metadata_registry_name(self):
        self.assertEqual(
            derive_registry_function_name(
                "image_resize::resize",
                {"registry_name": "resize_image", "name": "ignored"},
            ),
            "resize_image",
        )

    def test_derive_registry_function_name_falls_back_to_final_function_segment(self):
        self.assertEqual(derive_registry_function_name("image_resize::resize", {}), "resize")

    def test_normalize_worker_interface_converts_function_schema_fields(self):
        workers = {
            "workers": [
                {
                    "id": "worker-1",
                    "name": "image-resize",
                    "functions": ["image_resize::resize", "image_resize::ping"],
                }
            ]
        }
        functions = {
            "functions": [
                {
                    "function_id": "image_resize::resize",
                    "description": "Resize an image",
                    "request_format": {"type": "object"},
                    "response_format": {"type": "object"},
                    "metadata": {"tags": ["image", "transform"]},
                },
                {
                    "function_id": "image_resize::ping",
                    "description": None,
                    "request_format": None,
                    "response_format": None,
                    "metadata": None,
                },
                {
                    "function_id": "other::fn",
                    "description": "Not this worker",
                    "request_format": {"type": "object"},
                    "response_format": None,
                    "metadata": {},
                },
            ]
        }
        triggers = {
            "triggers": [
                {
                    "id": "t1",
                    "trigger_type": "http",
                    "function_id": "image_resize::resize",
                    "config": {"api_path": "/resize"},
                    "metadata": {"public": True},
                },
                {
                    "id": "t2",
                    "trigger_type": "http",
                    "function_id": "other::fn",
                    "config": {"api_path": "/other"},
                },
            ]
        }

        interface = normalize_worker_interface(
            worker_name="image-resize",
            workers_json=workers,
            functions_json=functions,
            triggers_json=triggers,
        )

        self.assertEqual(
            interface["functions"],
            [
                {
                    "name": "resize",
                    "description": "Resize an image",
                    "request_schema": {"type": "object"},
                    "response_schema": {"type": "object"},
                    "metadata": {"tags": ["image", "transform"]},
                },
                {
                    "name": "ping",
                    "description": "",
                    "request_schema": {},
                    "response_schema": {},
                    "metadata": {},
                },
            ],
        )
        self.assertEqual(
            interface["triggers"],
            [
                {
                    "name": "resize",
                    "description": "",
                    "invocation_schema": {},
                    "return_schema": {},
                    "metadata": {
                        "public": True,
                        "engine_id": "t1",
                        "trigger_type": "http",
                        "function_id": "image_resize::resize",
                        "config": {"api_path": "/resize"},
                    },
                }
            ],
        )

    def test_normalize_worker_interface_rejects_missing_worker(self):
        with self.assertRaisesRegex(ValueError, "expected exactly one worker"):
            normalize_worker_interface(
                worker_name="missing",
                workers_json={"workers": []},
                functions_json={"functions": []},
                triggers_json={"triggers": []},
            )

    def test_normalize_worker_interface_accepts_no_triggers_source(self):
        interface = normalize_worker_interface(
            worker_name="image-resize",
            workers_json={
                "workers": [
                    {
                        "id": "worker-1",
                        "name": "image-resize",
                        "functions": ["image_resize::resize"],
                    }
                ]
            },
            functions_json={
                "functions": [
                    {
                        "function_id": "image_resize::resize",
                        "description": "Resize",
                        "request_format": None,
                        "response_format": None,
                        "metadata": None,
                    }
                ]
            },
            triggers_json=None,
        )
        self.assertEqual(interface["triggers"], [])

    def test_build_binary_payload_has_registry_shape_and_binaries(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "image-resize"
            root.mkdir()
            (root / "iii.worker.yaml").write_text(
                "description: Resize images from a URL.\n"
                "dependencies:\n"
                "  helper-worker: '^1.0.0'\n",
                encoding="utf-8",
            )
            (root / "README.md").write_text("# image-resize\n", encoding="utf-8")
            (root / "config.yaml").write_text("{}\n", encoding="utf-8")

            payload = build_payload(
                repo_root=Path(tmp),
                worker="image-resize",
                version="0.1.0",
                registry_tag="latest",
                deploy="binary",
                repo_url="https://github.com/example/image-resize",
                interface={
                    "functions": [
                        {
                            "name": "resize",
                            "description": "Resize",
                            "request_schema": {"type": "object"},
                            "response_schema": None,
                            "metadata": {},
                        }
                    ],
                    "triggers": [],
                },
                binaries={
                    "x86_64-unknown-linux-gnu": {
                        "url": "https://example.com/releases/image-resize-x86_64-unknown-linux-gnu.tar.gz",
                        "sha256": "b" * 64,
                    }
                },
                image_tag="",
            )

        self.assertEqual(payload["worker_name"], "image-resize")
        self.assertEqual(payload["version"], "0.1.0")
        self.assertEqual(payload["tag"], "latest")
        self.assertEqual(payload["type"], "binary")
        self.assertEqual(payload["readme"], "# image-resize\n")
        self.assertEqual(payload["repo"], "https://github.com/example/image-resize")
        self.assertEqual(payload["description"], "Resize images from a URL.")
        self.assertEqual(
            payload["dependencies"],
            [{"name": "helper-worker", "version": "^1.0.0"}],
        )
        self.assertEqual(payload["config"], {})
        self.assertEqual(payload["functions"][0]["name"], "resize")
        self.assertIn("request_schema", payload["functions"][0])
        self.assertIn("response_schema", payload["functions"][0])
        self.assertEqual(payload["functions"][0]["response_schema"], {})
        self.assertEqual(payload["triggers"], [])
        self.assertIn("binaries", payload)
        self.assertNotIn("image_tag", payload)

    def test_build_image_payload_has_image_tag(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "hello-worker"
            root.mkdir()
            (root / "iii.worker.yaml").write_text("description: Demo worker\n", encoding="utf-8")

            payload = build_payload(
                repo_root=Path(tmp),
                worker="hello-worker",
                version="1.0.0",
                registry_tag="latest",
                deploy="image",
                repo_url="https://github.com/example/workers",
                interface={"functions": [], "triggers": []},
                binaries={},
                image_tag="ghcr.io/example/hello-worker:1.0.0",
            )

        self.assertEqual(payload["type"], "image")
        self.assertEqual(payload["image_tag"], "ghcr.io/example/hello-worker:1.0.0")
        self.assertNotIn("binaries", payload)


if __name__ == "__main__":
    unittest.main()

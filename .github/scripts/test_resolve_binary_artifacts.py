import unittest

from resolve_binary_artifacts import build_binary_artifact_map


class ResolveBinaryArtifactsTests(unittest.TestCase):
    def test_builds_binary_urls_and_sha_values(self):
        checksums = {
            "https://github.com/example/workers/releases/download/image-resize/v0.1.0/image-resize-x86_64-unknown-linux-gnu.sha256": "b" * 64
        }

        result = build_binary_artifact_map(
            repo_url="https://github.com/example/workers",
            tag="image-resize/v0.1.0",
            bin_name="image-resize",
            targets=["x86_64-unknown-linux-gnu"],
            read_checksum=lambda url: checksums[url],
        )

        self.assertEqual(
            result,
            {
                "x86_64-unknown-linux-gnu": {
                    "url": "https://github.com/example/workers/releases/download/image-resize/v0.1.0/image-resize-x86_64-unknown-linux-gnu.tar.gz",
                    "sha256": "b" * 64,
                }
            },
        )


if __name__ == "__main__":
    unittest.main()

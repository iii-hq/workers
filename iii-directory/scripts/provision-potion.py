#!/usr/bin/env python3
"""Explicitly install iii-directory's pinned local Potion model."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import urllib.request


REPOSITORY = "minishlab/potion-multilingual-128M"
REVISION = "a28f4eebecd4dc585034f605e52d414878a0417c"
MODEL_SHA256 = "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397"
TOKENIZER_SHA256 = "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1"
FILES = {
    "config.json": (None, None),
    "tokenizer.json": (18_616_131, TOKENIZER_SHA256),
    "model.safetensors": (512_361_560, MODEL_SHA256),
}


def verify_file(path: Path, expected_size: int, expected_sha256: str) -> None:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            size += len(chunk)
            digest.update(chunk)
    if size != expected_size or digest.hexdigest() != expected_sha256:
        raise ValueError(f"{path.name} failed size/checksum verification")


def validate_config(config: dict) -> None:
    if (
        config.get("model_type") != "model2vec"
        or config.get("hidden_dim") != 256
        or config.get("normalize") is not True
    ):
        raise ValueError("config.json does not describe the pinned 256d normalized Model2Vec model")


def model_manifest() -> dict:
    return {
        "repo": REPOSITORY,
        "revision": REVISION,
        "model_sha256": MODEL_SHA256,
        "tokenizer_sha256": TOKENIZER_SHA256,
        "dimensions": 256,
    }


def require_empty_destination(destination: Path) -> None:
    if not destination.exists():
        return
    if not destination.is_dir() or any(destination.iterdir()):
        raise ValueError(f"destination must not exist or must be empty: {destination}")


def provision(destination: Path) -> None:
    destination = destination.resolve()
    require_empty_destination(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{destination.name}.", dir=destination.parent))
    try:
        base = f"https://huggingface.co/{REPOSITORY}/resolve/{REVISION}"
        for name in FILES:
            with urllib.request.urlopen(f"{base}/{name}") as response, (temporary / name).open("wb") as output:
                shutil.copyfileobj(response, output)
        validate_config(json.loads((temporary / "config.json").read_text()))
        for name, (size, checksum) in FILES.items():
            if size is not None:
                verify_file(temporary / name, size, checksum)
        (temporary / "iii-model.json").write_text(json.dumps(model_manifest(), indent=2, sort_keys=True) + "\n")
        if destination.exists():
            destination.rmdir()
        os.replace(temporary, destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    provision(args.destination)


if __name__ == "__main__":
    main()

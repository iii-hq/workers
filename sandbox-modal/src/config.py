"""Config loader for sandbox-modal. Reads a flat key:value YAML-shaped file.

Avoids a YAML dependency by parsing the small set of keys the worker
actually uses; matches the shape of `iii.worker.yaml`'s `config:` block.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Config:
    max_concurrent_sandboxes: int = 10
    default_idle_timeout_secs: int = 300
    default_cpus: int = 1
    default_memory_mb: int = 512
    image_allowlist: list[str] = field(default_factory=list)

    @classmethod
    def load(cls, path: str | os.PathLike[str]) -> Config:
        cfg = cls()
        try:
            raw = Path(path).read_text(encoding="utf-8")
        except OSError:
            return cfg

        list_key: str | None = None
        for line in raw.splitlines():
            stripped = line.split("#", 1)[0].rstrip()
            if not stripped.strip():
                continue
            if stripped.startswith("  - ") and list_key is not None:
                value = stripped[4:].strip().strip("\"'")
                getattr(cfg, list_key).append(value)
                continue
            if ":" not in stripped:
                continue
            key, _, val = stripped.partition(":")
            key = key.strip()
            val = val.strip()
            if not hasattr(cfg, key):
                list_key = None
                continue
            if val == "":
                # nested list follows
                if isinstance(getattr(cfg, key), list):
                    list_key = key
                continue
            list_key = None
            if val == "[]":
                setattr(cfg, key, [])
            elif val.lstrip("-").isdigit():
                setattr(cfg, key, int(val))
            else:
                setattr(cfg, key, val.strip("\"'"))
        return cfg

    def image_allowed(self, image: str) -> bool:
        return not self.image_allowlist or image in self.image_allowlist

"""Small shared constants/helpers for the .github/scripts/tests/ suite.

Kept separate from conftest.py because pytest treats conftest as a fixture
module — re-imports of its top-level symbols from sibling test files are
fragile in unusual collection paths. Putting plain Python constants here
keeps both conftest and tests reading from one source.
"""
from __future__ import annotations

import os

GIT_HERMETIC_ENV = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": "/dev/null",
    "GIT_CONFIG_SYSTEM": "/dev/null",
}

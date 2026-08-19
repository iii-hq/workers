# rustpython-sre_engine compatibility fork

Forked from `rustpython-sre_engine` 0.5.0 for the browser worker's frozen
CPython 3.12 regular-expression contract. Upstream source is MIT licensed;
see `LICENSE`.

The local `compiler` module is the worker-owned Rust port of the matching
parts of CPython 3.12's `Lib/re/_parser.py` and `Lib/re/_compiler.py`.

# Frozen Scrapling oracle

`requirements.lock` is the hashed Python 3.12 resolution used to capture the
standalone worker's observable contract. Regenerate it with:

```sh
uv pip compile oracle/requirements.in \
  --python-version 3.12 \
  --python-platform linux \
  --generate-hashes \
  --output-file oracle/requirements.lock \
  --custom-compile-command 'scripts/update_oracle.sh'
```

`manifest.json` records the worker source, runtime, browser, and host data that
can change observable output. Build and verify the oracle with:

```sh
uv venv --python 3.12.13 .oracle
uv pip sync --python .oracle/bin/python --require-hashes oracle/requirements.lock
PYTHONHASHSEED=0 .oracle/bin/python scripts/verify_oracle.py
```

`scripts/gen_goldens.py` runs that verification before writing anything. A
release verification also passes `--archive-dir DIR`; `DIR` must contain the
six archive filenames recorded in `manifest.json`. `--write` is reserved for
an intentional oracle refresh and also requires the archive directory.

Pull-request differentials install the same hashed lock into a fresh virtual
environment and use `verify_oracle.py --parser-runtime`. That mode still hashes
the standalone source, every immutable package file, and every parser data
asset; it excludes only browser archives and host-specific executable, font,
locale, timezone, and CA-bundle records. Run the public-wrapper comparators with:

```sh
PYTHONHASHSEED=0 .oracle/bin/python scripts/differential_parser.py --oracle-check parser-runtime --cases 10000
PYTHONHASHSEED=0 .oracle/bin/python scripts/differential_http.py --oracle-check parser-runtime
```

CI also regenerates every schema and behavior fixture in a temporary directory
and byte-compares it with the committed goldens:

```sh
PYTHONHASHSEED=0 .oracle/bin/python scripts/gen_goldens.py check --parser-runtime
```

The scheduled certification job raises each HTML/CSS/XPath and regex grammar
to one million deterministic cases.

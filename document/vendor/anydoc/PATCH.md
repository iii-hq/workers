# Temporary anydoc dependency patch

This directory contains the published `anydoc 0.1.9` crate from crates.io,
with its original MIT license. Its archive SHA-256 was verified against the
previous Cargo.lock entry before copying:

`93a8b0dbecf79be9329111093b493d256db2a573ed3f1cc827c96bdbbe32a936`

Upstream: https://github.com/firecrawl/anydoc
Published crate: https://crates.io/crates/anydoc/0.1.9

## Local changes

- Upgrade `zip` from 8.6.0 to 9.0.5.
- Upgrade `calamine` from 0.36.1 to 0.37.0, the release that uses ZIP 9.
- Keep the converter implementation and public API unchanged.
- Express one existing multiline test string with newline escapes, preserving
  its exact expected bytes while avoiding a trailing-whitespace diff error.

These updates remove ZIP 8 from the document worker's dependency graph and
address RUSTSEC-2026-0367, RUSTSEC-2026-0368, and RUSTSEC-2026-0369 without
suppressing security advisories. The latest inspected upstream release,
`anydoc 0.2.4`, still depends on ZIP 8 and cannot resolve the findings alone.

The copied files are Cargo.toml, LICENSE, README.md, src/, and the published
example. `rustfmt.toml` comes from the upstream `v0.1.9` tag so CI checks
preserve the original source formatting. Registry bookkeeping and the upstream lockfile are omitted; the
worker's document/Cargo.lock is authoritative.

Remove this local copy and return to a registry dependency after an upstream
release adopts ZIP 9 and passes the document conversion regression tests.

# Native WhatsApp worker verification

Verified on 2026-09-30 with rustc/Cargo 1.97.1 on Linux x86_64.
Source is maintained in the isolated `feat/whatsapp-native` worktree. The user
requested a local commit and removal of the implementation's duplicate files
from the original checkout; unrelated changes remain there. No push, Registry
publication, account pairing or real WhatsApp send was performed.

## Results

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --all-features` | 52 passed; one engine integration test intentionally opt-in |
| Opt-in engine integration using the final release worker binary | 1 passed; all 145 function request/response schemas matched; event trigger schema, central configuration, offline metadata RPC and SIGTERM passed |
| `cargo build --release --locked` | Passed |
| Release `--manifest` / `--catalog` | Passed offline |
| Repository `validate_worker.py` | Passed inside the template test container |
| Deployment descriptor compiler | Passed inside the template test container; 82 publishable descriptors including WhatsApp |
| Source copy comparison | Identical worker files, excluding build/data directories |

The release binary is `whatsapp/target/release/whatsapp`, approximately 35 MiB,
ELF x86_64 GNU/Linux. SHA-256:

```text
771ebf395ff469f72003e6e55b7525a1ad2f232416af0184e369a57c2c19dca8
```

`ldd` reports only `libgcc_s`, `libm`, `libc`, the system loader and Linux vDSO.
Node, Chromium, external SQLite and codec processes are not runtime dependencies.
This is a dynamically linked Linux binary; static musl and other-platform artifacts
were not built or declared for this initial release target.

## Coverage and limits

The worker registers 145 typed functions and trigger type `whatsapp::events`.
The complete OpenWA snapshot contains 202 operations: 71 native equivalents,
23 partial mappings and 108 unimplemented application/API features. Native
equivalence means an action exists under the worker's distinct iii schema;
it does not mean HTTP compatibility or live-tested feature parity.

Offline regressions cover lifecycle/persistence, concurrent operations, expired
pairing data, shutdown bounds, callback generations, event limits/cursors,
private paths, secret-safe logging, JIDs, strict schemas, message construction,
timestamp units, recipient keys, media bounds and status audience requirements.
The engine test never starts a WhatsApp session. Live interoperability still
requires an operator to pair an account and exercise the desired operations.

## Reproduce the engine test

```sh
cd whatsapp
cargo build --release --locked
III_BINARY=/absolute/path/to/iii \
WHATSAPP_BINARY="$PWD/target/release/whatsapp" \
cargo test --locked --test engine_smoke -- --ignored --nocapture
```

The engine starts on a temporary loopback port with an isolated configuration
directory. The test verifies registration and offline operations through actual
RPC and terminates both processes when it finishes.

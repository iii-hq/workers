# Native WhatsApp worker verification

Verified on 2026-09-30 with rustc/Cargo 1.97.1 on Linux x86_64.
Source is maintained in the isolated `feat/whatsapp-native` worktree. The user
requested a local commit and removal of the implementation's duplicate files
from the original checkout; unrelated changes remain there. No push, Registry
publication or real WhatsApp message send was performed. The user's already
paired account was preserved and reconnected for read-only inbox/group checks.

## Results

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --all-features` | 72 passed; one engine integration test intentionally opt-in |
| Opt-in engine integration using the final release worker binary | 1 passed; all 148 function request/response schemas matched; event trigger, central configuration, offline inbox pagination, live retention and SIGTERM passed |
| `cargo build --release --locked` | Passed |
| Release `--manifest` / `--catalog` | Passed offline |
| Repository `validate_worker.py` | Passed inside the template test container |
| Deployment descriptor compiler | Passed inside the template test container; 82 publishable descriptors including WhatsApp |
| ADE UI tests | 57 passed; TypeScript/build and strict UI lint passed without warnings |
| Browser validation | Mock conversation reading, pagination, history request, groups/members, navigation, disconnected cache and desktop/mobile layouts passed without page errors |
| Live deployment | Only WhatsApp restarted; existing session reconnected, 3 available messages imported, 63 participating groups listed; served page matches the build |
| Live history request | Phone returned 50 older messages; the archive contains 53 messages in the available conversation, with history progress 100 |

The release binary is `whatsapp/target/release/whatsapp`, approximately 35 MiB,
ELF x86_64 GNU/Linux. SHA-256:

```text
acccfe5206414e4076db770b03317d63fdbac4683538d47eb7f9a2d3fcae5a35
```

`ldd` reports only `libgcc_s`, `libm`, `libc`, the system loader and Linux vDSO.
Node, Chromium, external SQLite and codec processes are not runtime dependencies.
This is a dynamically linked Linux binary; static musl and other-platform artifacts
were not built or declared for this initial release target.

## Coverage and limits

The worker registers 148 typed functions and trigger type `whatsapp::events`.
The complete OpenWA snapshot contains 202 operations: 71 native equivalents,
25 partial mappings and 106 unimplemented application/API features. Native
equivalence means an action exists under the worker's distinct iii schema;
it does not mean HTTP compatibility or live-tested feature parity.

Offline regressions cover lifecycle/persistence, concurrent operations, expired
pairing data, shutdown bounds, callback generations, event limits/cursors,
private paths, secret-safe logging, JIDs, strict schemas, message construction,
timestamp units, recipient keys, media bounds and status audience requirements.
The engine test never starts a WhatsApp session. Live checks use only the
existing account's groups and stored messages; an explicit older-history
request returned a streamed batch of 50 messages. Sending messages, group mutations
and complete history recovery remain outside the live validation performed.

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

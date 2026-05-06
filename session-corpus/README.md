# session-corpus

Dataset publishing pipeline for completed sessions on the iii bus. Scans
session JSONL for secrets, runs caller-supplied review, redacts, and
publishes to a configured target under `corpus::*`.

## Installation

```bash
iii worker add session-corpus
```

## Run

```bash
iii-session-corpus --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

`corpus::scan`, `corpus::redact`, `corpus::review`, `corpus::publish`.

## Build

```bash
cargo build --release
```

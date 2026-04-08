# iii-sensor

The code your AI writes today is the context it reads tomorrow. Every session silently degrades your architecture unless you measure it. iii-sensor scans your codebase, computes a quality score across 5 dimensions (complexity, coupling, cohesion, size, duplication), saves baselines before agent sessions, and flags when quality drops. It's the feedback sensor in the harness engineering loop.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-sensor --url ws://your-engine:49134`. It registers 6 functions. Call `sensor::baseline` before a coding session, then `sensor::compare` after to see what changed. Wire `sensor::gate` into your CI to reject PRs that degrade quality below your threshold.

## Functions

| Function ID | Description |
|---|---|
| `sensor::scan` | Walk a directory and compute per-file code quality metrics |
| `sensor::score` | Aggregate quality score (0-100) from scan results using weighted power mean |
| `sensor::baseline` | Save a named baseline snapshot for later comparison |
| `sensor::compare` | Compare a fresh scan against a saved baseline to detect degradation |
| `sensor::gate` | CI quality gate returning pass/fail on score thresholds |
| `sensor::history` | Retrieve historical scores and compute trend direction |

## iii Primitives Used

- **State** -- baselines, score history, latest scan results (keyed by path hash)
- **HTTP** -- all functions exposed as REST endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`

## Build

```bash
cargo build --release
```

## Usage

```bash
./target/release/iii-sensor --url ws://127.0.0.1:49134 --config ./config.yaml
```

```
Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```

## Configuration

```yaml
scan_extensions: ["rs", "ts", "py", "js", "go"]  # file extensions to include
max_file_size_kb: 512                              # skip files larger than this
score_weights:
  complexity: 0.25
  coupling: 0.25
  cohesion: 0.20
  size: 0.15
  duplication: 0.15
thresholds:
  degradation_pct: 10.0    # dimension drop % that flags degradation
  min_score: 60.0           # minimum passing score for quality gate
```

## Tests

```bash
cargo test
```

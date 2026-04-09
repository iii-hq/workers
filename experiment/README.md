# iii-experiment

Karpathy ran 700 experiments in 2 days and got 11% speedup on an already-optimized codebase. Shopify's CEO ran it overnight and got 53% faster rendering. The pattern is simple: edit, run, measure, keep or discard, repeat. iii-experiment makes this a first-class iii primitive. Point it at any function + any metric, set a direction (minimize latency, maximize quality), and let it run. It's completely generic — works on API latency, code quality scores, LLM prompts, config tuning, or anything with a number.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-experiment --url ws://your-engine:49134`. It registers 7 functions. Call `experiment::create` with your target function, metric function, and budget. Then call `experiment::loop` and watch it optimize. Progress streams in real-time via iii Streams.

## Functions

| Function ID | Description |
|---|---|
| `experiment::create` | Create an experiment with target function, metric function, and direction |
| `experiment::propose` | Generate a parameter variation proposal from the current best payload |
| `experiment::run` | Execute one iteration: propose, run target, measure, decide keep/discard |
| `experiment::decide` | Pure comparison of a score against the current best |
| `experiment::loop` | Full optimization loop running budget iterations with stop-signal checks |
| `experiment::status` | Read experiment definition, run state, and iteration history |
| `experiment::stop` | Set stop signal so the loop halts before the next iteration |

## iii Primitives Used

- **State** -- experiment definitions, run state, per-iteration results, best payloads, proposals
- **Streams** -- progress events published to `experiment:progress` group
- **HTTP** -- all functions exposed as POST endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`

## Build

```bash
cargo build --release
```

## Usage

```bash
./target/release/iii-experiment --url ws://127.0.0.1:49134 --config ./config.yaml
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
default_budget: 20         # default iterations per experiment
max_budget: 100            # hard cap on iterations
timeout_per_run_ms: 30000  # timeout for each target function call
```

## Tests

```bash
cargo test
```

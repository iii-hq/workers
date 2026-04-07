# iii-experiment — Generic Optimization Loop Worker

## Overview

A generic optimization loop worker for the III engine. Given any `target_function` and any `metric_function`, it runs propose-run-measure-keep/discard cycles to optimize measurable outcomes.

The worker knows nothing about what the target or metric functions do. It is 100% generic.

## Concept

1. Call `metric_function` to get a baseline score
2. Propose a change (parameter variation on the target payload)
3. Apply change by calling `target_function` with modified payload
4. Call `metric_function` again to get new score
5. If better (per direction) -> keep, if worse -> discard
6. Repeat until budget exhausted or stopped

## Functions (7)

### experiment::create
- **Input:** `{ target_function, metric_function, metric_path, direction, budget?, description?, target_payload?, metric_payload? }`
- **Behavior:** Validates functions exist via `iii.list_functions()`, generates experiment ID, calls metric_function for baseline, stores definition
- **Output:** `{ experiment_id, baseline_score, status: "created" }`

### experiment::propose
- **Input:** `{ experiment_id }`
- **Behavior:** Reads best payload from state, applies random parameter variation (numeric +-50%, boolean flip), stores proposal
- **Output:** `{ proposal_id, hypothesis, modified_payload }`

### experiment::run
- **Input:** `{ experiment_id, proposal_id? }`
- **Behavior:** Calls propose if no proposal_id, executes target_function with proposed payload, measures via metric_function, decides keep/discard, streams progress
- **Output:** `{ iteration, score, baseline_score, best_score, kept, improvement_pct }`

### experiment::decide
- **Input:** `{ experiment_id, score, iteration, current_best?, direction? }`
- **Behavior:** Pure comparison — compares score against current best based on direction
- **Output:** `{ experiment_id, iteration, kept, reason, improvement_pct }`

### experiment::loop
- **Input:** `{ experiment_id }`
- **Behavior:** Full optimization loop running budget iterations. Checks for stop signal before each iteration. Streams progress events.
- **Output:** `{ experiment_id, total_runs, kept_count, best_score, baseline_score, total_improvement_pct, status }`

### experiment::status
- **Input:** `{ experiment_id }`
- **Behavior:** Reads definition, run state, and all iteration results from state
- **Output:** `{ experiment_id, status, iterations_completed, budget, best_score, baseline_score, improvement_pct, history }`

### experiment::stop
- **Input:** `{ experiment_id }`
- **Behavior:** Sets run status to "stopped"; loop checks this before each iteration
- **Output:** `{ stopped: true, experiment_id, iterations_completed }`

## State Scopes

| Scope | Key Pattern | Contents |
|-------|-------------|----------|
| `experiment:definitions` | `{experiment_id}` | Experiment config (target, metric, direction, budget, payloads) |
| `experiment:runs` | `{experiment_id}` | Running state (status, current_iteration, best_score) |
| `experiment:results` | `{experiment_id}:{iteration}` | Per-iteration results (score, kept, payload) |
| `experiment:best` | `{experiment_id}` | Current best payload |
| `experiment:proposals` | `{experiment_id}:{proposal_id}` | Proposals with modified payloads |

## HTTP Triggers

| Method | Path | Function |
|--------|------|----------|
| POST | `/experiment/create` | `experiment::create` |
| POST | `/experiment/propose` | `experiment::propose` |
| POST | `/experiment/run` | `experiment::run` |
| POST | `/experiment/decide` | `experiment::decide` |
| POST | `/experiment/loop` | `experiment::loop` |
| POST | `/experiment/status` | `experiment::status` |
| POST | `/experiment/stop` | `experiment::stop` |

## Streams

Progress events are published to `experiment:progress` stream with group_id = experiment_id:

```json
{
  "score": 42.5,
  "kept": true,
  "iteration": 3,
  "best_score": 42.5
}
```

## Configuration (config.yaml)

```yaml
default_budget: 20
max_budget: 100
timeout_per_run_ms: 30000
```

## Parameter Variation (v1)

Simple strategy for generating proposals:
- **Numeric fields:** Randomly vary by -50% to +50%
- **Boolean fields:** Flip with ~40% probability
- **Nested objects/arrays:** Recurse into children
- **Strings:** Unchanged

## Usage Example

```bash
# Create an experiment to minimize p99 latency
curl -X POST http://localhost:3111/experiment/create -d '{
  "target_function": "order.process",
  "metric_function": "eval::metrics",
  "metric_path": "p99_ms",
  "direction": "minimize",
  "budget": 20,
  "target_payload": { "batch_size": 100, "timeout_ms": 5000, "retry": true },
  "metric_payload": { "function_id": "order.process" }
}'

# Run the full optimization loop
curl -X POST http://localhost:3111/experiment/loop -d '{
  "experiment_id": "<id-from-create>"
}'

# Check status mid-run
curl -X POST http://localhost:3111/experiment/status -d '{
  "experiment_id": "<id>"
}'

# Stop early
curl -X POST http://localhost:3111/experiment/stop -d '{
  "experiment_id": "<id>"
}'
```

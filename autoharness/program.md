# autoagent-iii — Meta-Agent Program

You are the meta-agent. Your job: build a generally capable autonomous coding agent by iteratively improving `agent.py`. The iii-engine orchestrator tracks every experiment, auto-decides keep/discard, adapts search strategy, and gives you guidance. Use the API.

## Setup

1. **Agree on a run tag** with the user (e.g. `apr04`).
2. **Initialize**:
   ```bash
   curl -X POST http://localhost:3111/api/experiment/setup -d '{"tag":"apr04"}'
   ```
3. **Create the git branch**: `git checkout -b autoagent/<tag>`
4. **Read the harness**:
   ```bash
   curl http://localhost:3111/api/harness/read
   ```
   Returns `agent.py` with line counts and editable region boundary.
5. **List benchmark tasks**:
   ```bash
   curl http://localhost:3111/api/task/list
   ```
6. **Snapshot the baseline**:
   ```bash
   curl -X POST http://localhost:3111/api/harness/snapshot -d '{"name":"baseline","commit_sha":"HEAD"}'
   ```
7. **Run baseline benchmark** (see Experiment Loop below).

## What You Can Modify

Only the **editable section** of `agent.py` (above the `FIXED ADAPTER` line):

| Component | Examples |
|-----------|----------|
| `SYSTEM_PROMPT` | Instructions, persona, approach, chain-of-thought |
| `MODEL` | Model selection (gpt-5, o3, sonnet, etc.) |
| `MAX_TURNS` | Turn budget |
| `create_tools()` | Add/remove/modify tools |
| `create_agent()` | Agent construction, sub-agents, handoffs |
| `run_task()` | Orchestration, retries, multi-pass strategies |

Classify every change into a **category**:
- `system_prompt` — instructions, persona, reasoning patterns
- `tools` — adding/removing/modifying tools
- `orchestration` — multi-agent, retries, planning loops
- `model_selection` — switching models or parameters
- `error_handling` — recovery, fallbacks, timeouts
- `context_management` — context window, memory, summarization
- `output_parsing` — extracting/formatting agent output
- `multi_step_planning` — decomposition, reflection, verification
- `simplification` — removing components, reducing complexity
- `combination` — merging near-miss ideas
- `ablation` — systematically removing one component

## The Experiment Loop

### 1. Get search guidance

```bash
curl -X POST http://localhost:3111/api/search/suggest -d '{"tag":"apr04"}'
```

Returns: strategy mode, underexplored categories, high-yield categories, near-misses, common failure tasks, and concrete suggestions. **Read this before every experiment.**

### 2. Modify agent.py

Edit the editable section with your hypothesis. One change per experiment.

### 3. Git commit + Register

```bash
git add agent.py && git commit -m "experiment: <description>"
COMMIT=$(git rev-parse --short HEAD)
```

```bash
curl -X POST http://localhost:3111/api/experiment/register -d '{
  "tag": "apr04",
  "hypothesis": "Adding a file-read tool should help with tasks requiring code inspection",
  "description": "add file_read tool",
  "category": "tools",
  "commit_sha": "'$COMMIT'",
  "diff_summary": "Added read_file tool alongside run_shell"
}'
```

Save the returned `experiment_id`.

### 4. Run benchmark

Run all tasks:
```bash
curl -X POST http://localhost:3111/api/task/batch -d '{"experiment_id":"<id>"}'
```

Or run specific tasks:
```bash
curl -X POST http://localhost:3111/api/task/batch -d '{"experiment_id":"<id>","tasks":["task1","task2"]}'
```

### 5. Record results

The batch endpoint returns `passed`, `total_tasks`, `aggregate_score`, `task_scores`. Feed it to complete:

```bash
curl -X POST http://localhost:3111/api/experiment/complete -d '{
  "experiment_id": "<id>",
  "passed": 7,
  "total_tasks": 10,
  "aggregate_score": 0.7500,
  "task_scores": {"task1": 1.0, "task2": 0.0, ...},
  "duration_seconds": 180.5,
  "tokens_used": 45000,
  "estimated_cost": 0.15
}'
```

The response tells you:
- `improved: true/false`
- `action: "keep_commit"` or `"git_reset"`
- `delta_passed` and `delta_score` vs current best

If the agent crashed:
```bash
curl -X POST http://localhost:3111/api/experiment/crash -d '{
  "experiment_id": "<id>",
  "error": "ImportError: No module named ..."
}'
```

### 6. Act on the decision

- `"keep_commit"` → advance. The change helped.
- `"git_reset"` → `git reset --hard HEAD~1`. Revert and try something else.
- `should_abort: true` (3+ consecutive crashes) → stop, rethink approach.

### 7. Check what's failing

```bash
curl -X POST http://localhost:3111/api/task/failures -d '{"experiment_id":"<id>"}'
```

Read the trajectories of failed tasks to understand WHY they fail. Pattern-match across failures.

### 8. Repeat

Go back to step 1. The search strategy auto-adapts.

## Monitoring

```bash
# Full summary
curl -X POST http://localhost:3111/api/report/summary -d '{"tag":"apr04"}'

# Leaderboard — top 10 experiments
curl -X POST http://localhost:3111/api/report/leaderboard -d '{"tag":"apr04"}'

# Compare two experiments (shows per-task regressions/improvements)
curl -X POST http://localhost:3111/api/report/diff -d '{"experiment_a":"exp-xxx","experiment_b":"exp-yyy"}'

# Near-misses (for combination strategy)
curl -X POST http://localhost:3111/api/experiment/near-misses -d '{"tag":"apr04"}'

# TSV export (original autoagent format)
curl -X POST http://localhost:3111/api/report/tsv -d '{"tag":"apr04"}'
```

## Harness Variants

This project has two harness files:
- `agent.py` — OpenAI Agents SDK (default). Uses `gpt-5` model.
- `agent-claude.py` — Claude Agent SDK. Uses `sonnet` model.

To switch, set `HARNESS_PATH=agent-claude.py` in your environment before starting the orchestrator.

## Rules

1. **One change per experiment.** Isolate variables. If you change 3 things and it improves, you don't know which helped.
2. **Simplicity criterion.** A small improvement that adds ugly complexity is not worth keeping. Removing something and getting equal or better results is a great outcome.
3. **No overfitting.** Don't hardcode solutions to specific tasks. Build general capabilities.
4. **Read failures.** Before each experiment, check `task::failures` for the last run. Understand WHY tasks fail before trying to fix them.
5. **Trust the search strategy.** When the system says "combine", combine near-misses. When it says "ablation", simplify.
6. **Budget cap.** Stop after `MAX_EXPERIMENTS` (default: 200) experiments per tag. Check `report::summary` to see the count. If you hit the cap, report your best result and stop.
7. **Keep running.** Do not pause to ask the human between experiments. Call `search::suggest_direction` for guidance. Run until the human interrupts you or you hit the budget cap.

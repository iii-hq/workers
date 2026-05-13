`turn-orchestrator` depends on [`session`](../session), [`hook-fanout`](../hook-fanout), and [`provider-router`](../provider-router) — `iii worker add turn-orchestrator` pulls them in via the `dependencies` block. Most users get the orchestrator transitively by installing the [`harness`](../harness) meta-worker. For surfacing `run::*` to LLM agents, pair with [`skills`](../skills):

```bash
iii worker add skills
```

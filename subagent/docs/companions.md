`subagent` depends on [`turn-orchestrator`](../turn-orchestrator) for `run::*` — `iii worker add subagent` already pulls it in via the `dependencies` block in `iii.worker.yaml`. For surfacing `subagent::*` to LLM agents, pair with [`skills`](../skills):

```bash
iii worker add skills
```

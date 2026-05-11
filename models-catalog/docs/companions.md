For per-model cost tracking and spend caps that consume the catalog's pricing fields, pair with the [`llm-budget`](../llm-budget) worker. For surfacing `models::*` to LLM agents, pair with [`skills`](../skills):

```bash
iii worker add llm-budget
iii worker add skills
```

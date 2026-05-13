For cost-per-token data used to estimate LLM call costs before `budget::check`, pair with the [`models-catalog`](../models-catalog) worker. For surfacing `budget::*` to LLM agents, pair with [`skills`](../skills):

```bash
iii worker add models-catalog
iii worker add skills
```

For sandbox-targeted execution and `shell::fs::*` forwarding, install [`iii-sandbox`](../iii-sandbox); `iii worker add shell` does not currently pull it in. For surfacing `shell::*` to LLM agents, pair with [`skills`](../skills):

```bash
iii worker add iii-sandbox
iii worker add skills
```

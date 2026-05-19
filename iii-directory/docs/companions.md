`directory::skills::download` pulls bundles from the public workers registry (`api.workers.iii.dev` by default). For self-hosted setups, repoint `registry_url` in `config.yaml` at your own registry. The `directory::registry::*` proxies share their envelope with `directory::engine::workers::*`, so a single parser handles both local and remote worker discovery.

```bash
iii worker add iii-directory
```

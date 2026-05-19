# iii-directory

Engine introspection, workers registry proxy, and filesystem-backed skill + prompt reader for the iii engine. Every public function sits under a single `directory::*` namespace, split into four MCP-agnostic surfaces (`skills`, `prompts`, `engine`, and `registry`), so callers learn one envelope across local files, the running engine, and the public workers registry.

Skills and prompts are sourced from a single configured folder on disk (`skills_folder`). The only write path is `directory::skills::download`, which pulls markdown into `skills_folder` from either the workers registry or a GitHub repo. `directory::skills::list` returns one row per markdown file with `title` (preferring the YAML frontmatter `title:` over the body H1) and `type` lifted from frontmatter. `directory::skills::get` accepts a bare id, a `<id>.md` file-path form, or the legacy `iii://<id>` URI. `SKILLS.md` is aliased to `index.md` at scan time so the new convention round-trips through both filesystem and parser. `directory::skills::index` renders a short per-worker overview that emits both relative file-path pointers (`Read [<ns>/index.md](<ns>/index.md)`) and the legacy `iii://<ns>/index` form side by side for back-compat.

`directory::skills::download` pulls bundles from the public workers registry (`api.workers.iii.dev` by default). For self-hosted setups, repoint `registry_url` in `config.yaml` at your own registry. The `directory::registry::*` proxies share their envelope with `directory::engine::workers::*`, so a single parser handles both local and remote worker discovery.

```bash
iii worker add iii-directory
```

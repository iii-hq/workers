# openwiki

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/iii-hq/workers/main/openwiki/assets/openwiki-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/iii-hq/workers/main/openwiki/assets/openwiki-light.png">
    <img alt="OpenWiki browser UI" src="https://raw.githubusercontent.com/iii-hq/workers/main/openwiki/assets/openwiki-light.png" width="100%">
  </picture>
</p>

Builds and maintains a source-grounded, interlinked markdown wiki for a code
repository, and serves a browser UI to read and search it. Point it at a git
repo: an agent reads the source, plans a hierarchical index, writes one cited
page per topic, and keeps the wiki current from git diffs on a per-wiki
schedule. Pages persist in iii-state; the engine serves the UI and JSON API
under `/openwiki`.

## Install

```bash
iii worker add openwiki
```

This pulls `state`, `cron`, and `llm-router` transitively. Add a model provider
through the console's onboarding (anthropic, openai, codex, ...) and pages are
model-written; the provider credential lives in the `llm-router` config, never
in this worker.

For the best tier, agent-orchestrated pages written by one sub-agent per page
with line citations, add the harness stack as well:

```bash
iii worker add harness
```

`harness` transitively pulls `session-manager`, `context-manager`, `shell`
(jailed git for clone/diff), and the model providers. openwiki degrades
gracefully when a worker is absent:

| Present | Pages are |
|---|---|
| `harness` + a configured provider | agent-orchestrated, line-cited (best) |
| `llm-router` only | model-written from pre-selected files |
| neither | heuristic, built from file headers, always works |

## Quickstart

Open the browser UI on the engine's HTTP port:

```text
http://localhost:3111/openwiki
```

Or drive it from the CLI:

```bash
iii trigger openwiki::generate --json '{"repo_url":"https://github.com/owner/repo"}'
# -> { "wiki_id": "<wiki_id>", "status": "started" }

iii trigger openwiki::status --json '{"id":"<wiki_id>"}'    # poll until phase = ready
iii trigger openwiki::page   --json '{"id":"<wiki_id>","slug":"overview"}'
iii trigger openwiki::search --json '{"id":"<wiki_id>","q":"config"}'
```

`openwiki::refresh { id }` is incremental: it pulls the clone, diffs against
the recorded commit, and regenerates only the pages whose source changed.
`openwiki::set-schedule { id, schedule }` puts that refresh on a per-wiki
cadence (`off` | `3h` | `6h` | `12h` | `daily` | `weekly` | a cron string); a
content-hash gate keeps an unchanged repo from churning the wiki.

The full function catalogue (generation, scoped source readers for writer
sub-agents, cited Q&A via `openwiki::ask`, Mermaid diagrams, `AGENTS.md`
export, lint) is one `iii worker info openwiki` away. HTTP triggers mirror the
read/generate functions under `/openwiki/api/*`, generation progress streams
live over SSE, and page citations deep-link to source at the pinned commit.

openwiki also registers `openwiki::read-wiki-structure`,
`openwiki::read-wiki-contents`, and `openwiki::ask-question`, which the
[mcp](https://github.com/iii-hq/workers/tree/main/mcp) worker advertises to any
MCP client:

```bash
iii worker add mcp
```

## How generation works

1. Clone the repo (through the `shell` worker, with a local `git` fallback),
   inventory its files, and record the commit so citations deep-link to exact
   source.
2. A lead agent explores the clone through openwiki's scoped readers
   (`openwiki::src::read` / `src::list` / `src::grep`) and plans a
   reading-ordered index. The model decides how many pages the repo needs and
   follows the repo's own docs index (`llms.txt`, a `docs/` tree) when present.
3. The lead spawns one writer sub-agent per page in parallel. Each writer reads
   its focused files and stores its finished page with `openwiki::write-page`;
   openwiki turns citations into pinned-commit source links and rejects a page
   that comes back too thin.
4. Pages stream into the UI as each writer lands; the lead submits only the
   table of contents.

## Configuration

- **Model**: pick one in the browser UI's generate form (populated from the
  router's live catalog, grouped by provider), pass `model` to
  `openwiki::generate`, or set `OPENWIKI_MODEL`. Any model the router
  advertises works. Default `claude-haiku-4-5-20251001`.
- **`refresh_default`**: the auto-refresh cadence new wikis start with (`off`
  by default; each wiki overrides it in the UI). Editable in the console like
  the other openwiki config, or seed it with `OPENWIKI_REFRESH_DEFAULT`.
- **`OPENWIKI_DATA`**: wiki store and clone directory (default
  `/tmp/openwiki-data`). Must resolve inside the shell worker's
  `fs.host_roots` when git runs through `shell`.
- **`OPENWIKI_MAX_PARALLEL`**: concurrent page writers (default `3`).

## License

Apache-2.0

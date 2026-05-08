# project-rules — provenance

The contents of this directory and `../styles/` are a snapshot of the canonical iii project rules and Vale styles, originally authored at:

- `iii-hq/ideal-docs/project-rules/`
- `iii-hq/ideal-docs/styles/`

The snapshot is checked in here so the workers monorepo is self-contained for validation: `iii-skill-check` reads `./project-rules/` and `./styles/` directly, with no dependency on a sibling clone.

## Future migration

When the canonical home moves to `iii-hq/iii`, `iii-skill-check` will gain a `rules.source: git` mode that does a shallow, sparse-checkout clone:

```bash
git clone --depth=1 --filter=blob:none --sparse https://github.com/iii-hq/iii ~/.cache/iii-skill-check/iii
git -C ~/.cache/iii-skill-check/iii sparse-checkout set docs/project-rules docs/styles
```

The cache will be keyed by commit SHA with a configurable TTL.

## How to refresh the snapshot today

Until the migration lands, the snapshot is refreshed by hand:

```bash
cp /path/to/ideal-docs/project-rules/*.md ./project-rules/
cp -R /path/to/ideal-docs/styles/Diataxis ./styles/
cp -R /path/to/ideal-docs/styles/Terminology ./styles/
```

## What is local-only

`_skill-check-prompt.md` in this directory is the system prompt for the `iii-skill-check` AI layer. It lives with the rules so changing the prompt belongs in the same change as changing the rules. It is local to this validator and not part of the upstream `ideal-docs` set.

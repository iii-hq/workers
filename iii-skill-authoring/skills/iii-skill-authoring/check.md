# Running iii-skill-check locally

`iii-skill-check` is the validator that renders, lints, and AI-reviews worker artifacts. It runs on every PR via GitHub Actions and on every commit via the pre-commit hook, but it can also run directly during authoring.

## Render

```bash
cargo run --manifest-path iii-skill-check/Cargo.toml -- render --write <worker>
```

Reads `iii.worker.yaml.name`, `config.yaml`, and the partials under `<worker>/docs/`. Writes `<worker>/README.md`, `<worker>/skill.md`, and `<worker>/skills/*.md`.

Drop the `--write` flag to render to memory only — useful for previewing the rendered output without touching the on-disk artifacts.

## Verify all layers

```bash
cargo run --manifest-path iii-skill-check/Cargo.toml -- verify <worker>
```

Runs three layers in order, accumulating violations:

1. **Structure** — section presence and order in README, install command parity with `iii.worker.yaml.name`, no source-build blocks, llm-only marker balance, every `iii://<name>/<leaf>` link resolves.
2. **Vale** — every rendered artifact lints clean against `styles/Diataxis` (HowTo subset) and `styles/Terminology` (slop, forbidden terms).
3. **AI** — one Claude API call per artifact with the project rules concatenated as context. Requires `ANTHROPIC_API_KEY`.

Subset the layers with `--layers structure,vale` to skip the AI call locally.

## Verify rendered artifacts match source

```bash
cargo run --manifest-path iii-skill-check/Cargo.toml -- verify-rendered <worker>
```

Re-renders the worker in memory and diffs against the on-disk `README.md`, `skill.md`, and `skills/*.md`. Non-zero exit means an artifact drifted from the partials. Re-run `render --write` to fix.

## Reading violations

Output format is `<file>:<line> — <message>`. Structure and Vale layers report inline; AI failures appear under `[AI] <path>` blocks at the end with the model's full violation list.

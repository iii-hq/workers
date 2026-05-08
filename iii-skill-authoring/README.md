# iii-skill-authoring

Filesystem-backed skill bundle for authoring worker README and skill content in the iii workers monorepo. Read individual topics directly via `skillkit read iii-skill-authoring/<topic>`, or surface the bundle through MCP by adding it to your iii deployment's `skills:` glob:

```yaml
# config.yaml for the iii engine
skills:
  - workers/iii-skill-authoring/skills/**/*.md
```

The bundle covers:

- Directory layout and renderer slot order.
- Voice and terminology rules (the subset of `styles/Terminology/` that Vale enforces).
- Per-function leaf authoring.
- llm-only comment block round-trip.
- Running `iii-skill-check` locally.
- Why `ideal-docs/project-rules` is canonical and existing workers are not.

This bundle is not itself a worker — there is no `iii.worker.yaml`, no Rust source, no `iii-skill-check render` step. The markdown under `skills/` is the source of truth.

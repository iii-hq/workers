# Worker docs source layout

A worker that uses `iii-skill-check` keeps narrative source under `docs/`. The renderer combines those partials with `iii.worker.yaml.name` and `config.yaml` to produce the three rendered artifacts.

## Files the author edits

```
<worker>/
├── iii.worker.yaml          # name, description (renderer reads name)
├── config.yaml              # default runtime config (rendered verbatim under ## Configuration)
├── docs/
│   ├── intro.md             # paragraph(s) shown after the H1 in README and skill.md
│   ├── quickstart.md        # body of ## Quickstart in README only
│   ├── companions.md        # appended inside ## Install when this worker pairs with a sibling (optional, README only)
│   ├── migration.md         # body of ## Migration notes (optional, README only)
│   └── leaves/
│       └── <leaf>.md        # body of skills/<leaf>.md
```

## Files the renderer produces

```
<worker>/
├── README.md                # published to the registry, rendered on iii.dev
├── skill.md                 # body for iii://<worker> (LLM-facing)
└── skills/
    └── <leaf>.md            # body for iii://<worker>/<leaf>
```

Always run `iii-skill-check render --write <worker>` before committing — the rendered files carry a generated banner and should not be hand-edited.

## Slot order in README.md

1. Generated banner.
2. `# <name>` (from `iii.worker.yaml.name`).
3. `intro.md` (llm-only blocks kept as HTML comments).
4. `## Install` + `iii worker add <name>` boilerplate, optionally followed by `companions.md` (no new H2).
5. `## Quickstart` + `quickstart.md`.
6. `## Configuration` + fenced `config.yaml`.
7. `## Migration notes` + `migration.md` (only if present).

## Slot order in skill.md

1. Generated banner.
2. `# <name>`.
3. `intro.md` (llm-only blocks unwrapped).

## Slot order in skills/<leaf>.md

1. Generated banner.
2. `docs/leaves/<leaf>.md` (llm-only blocks unwrapped).

The leaf author chooses the H1 — typically a topical phrase like `# Sizing text before provider calls`, not the function id.

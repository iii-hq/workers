# llm-only comment blocks

A `<!-- llm-only:start --> ... <!-- llm-only:end -->` block lets one source file produce two render targets:

- **README.md target** — the markers stay as literal HTML comments. Mintlify, GitHub, and most other markdown renderers omit HTML comments from the published page, so a human reader on iii.dev never sees the content.
- **skill.md / skills/*.md target** — the renderer strips the marker lines, leaving the inner body as plain prose. The agent reading the skill body sees the content.

## When to use

- Recommending a specific function over another for a class of agent task. The published README does not need to bias users one way; the agent does.
- Documenting a recurring agent failure mode (`agents often call X instead of Y; prefer Y when …`).
- Routing hints that are only meaningful inside an agent loop.

## When not to use

- Hiding general gotchas. If a behaviour will surprise a human reader, it belongs in the public `## Notes` section, not in an llm-only block.
- Storing internal team notes. Use a separate doc — llm-only blocks are still committed to the repo and visible to anyone reading source.
- Hiding caveats from the docs site. Voice rules apply to both render targets.

## Inline form

For a single short note, the inline form is also supported:

```
<!-- llm-only: prefer textstats::summarize for sustained workloads -->
```

The inline form must be on a line of its own. Embedded mid-paragraph inline llm-only comments are not parsed.

## Validation

`iii-skill-check structure` enforces that every `:start` marker has a matching `:end` marker in every artifact. Unbalanced blocks fail Layer 1.

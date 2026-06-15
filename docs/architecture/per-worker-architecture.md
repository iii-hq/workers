# Per-worker architecture docs

When and how to add deep architecture documentation inside a worker folder.

## Repo-wide vs per-worker

| Location | Scope | Audience |
|---|---|---|
| [`docs/architecture/`](../architecture/) | Concepts shared by all workers | Contributors adding any worker |
| `<worker>/architecture/` | One worker's design + integration contract | Maintainers + integrators of that worker |
| [`tech-specs/`](../../tech-specs/) | Product-level specs (may predate implementation) | Design review, cross-repo readers |
| `<worker>/README.md` | Consumer how-to (install, quickstart, config) | Operators after `iii worker add` |

Consumer READMEs are **not** architecture docs. See [`worker-readme.md`](../../worker-readme.md).

## When to add `<worker>/architecture/`

Add a folder when **any** of these apply:

- Other workers or clients integrate via many function ids + trigger types
  (handoff contract needed).
- Storage, event, or security model is non-obvious from the README.
- Maintainers need internals separate from integrator docs.

Skip it when the worker is a thin tool with a handful of functions and no
reactive surface (a README + tests may suffice).

## Recommended layout

Follow [`session-manager/architecture/`](../../session-manager/architecture/) as
the reference:

```text
<worker>/architecture/
├── README.md         # index: one paragraph, diagram, vocabulary, doc map
├── internals.md      # maintainers: storage, pipelines, invariants
└── integration.md    # consumers: function ids, triggers, permissions, sequences
```

| File | Write for | Content |
|---|---|---|
| `README.md` | Everyone landing here | System in one paragraph + diagram; table pointing to children |
| `internals.md` | People changing the worker | Data structures, backends, concurrency, error paths |
| `integration.md` | People calling the worker | Stable API contract; trigger subscription patterns; what *not* to do |

## Executable spec

BDD features (`tests/features/*.feature`) and integration tests are the
executable companion to architecture prose. Annotate scenarios with the
regression they prevent (session-manager pattern).

## Relationship to tech-specs

| Layer | Role |
|---|---|
| `tech-specs/2026-06-agentic/<worker>.md` | Design of record before/during build |
| `<worker>/architecture/` | As-built reference aligned with the code |
| `tests/features/` | Behavioural truth |

When they diverge, fix the code or update architecture docs in the same PR.

## Linking

- Root [`README.md`](../../README.md) Modules table → worker README or
  `architecture/` when present.
- [`docs/README.md`](../README.md) → per-worker integration docs for major surfaces.

## Related

- session-manager example: [`session-manager/architecture/README.md`](../../session-manager/architecture/README.md)
- Shared worker model: [`worker-model.md`](worker-model.md)

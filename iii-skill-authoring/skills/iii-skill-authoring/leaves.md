# Authoring per-function skill leaves

Each registered function gets one file at `docs/leaves/<leaf>.md`. The leaf name is the function id's suffix after the last `::` — `textstats::analyze` corresponds to `docs/leaves/analyze.md`, `auth::list_providers` to `docs/leaves/list_providers.md`.

## What goes in the leaf file

- A topical H1 (optional). Choose a phrase that names the *task*, not the function — `# Sizing text before provider calls`, not `# textstats::analyze`.
- A `## When to use` section with three to five bullets covering the realistic call sites.
- A `## Notes` section with gotchas, edge cases, and behaviour an agent will trip on.
- Optional `<!-- llm-only:start --> ... <!-- llm-only:end -->` blocks for content the agent should see but the published page should hide.

## What does not go in the leaf file

- The function id as the H1. The auto-gen system publishes the API surface at a separate URI.
- The function signature. Same reason.
- A description duplicating the worker's `RegisterFunction::new(...).description("...")` text.
- Cross-references between functions in tabular form. Use prose links.

The renderer prepends a generated banner comment to each leaf and unwraps any llm-only blocks before writing `skills/<leaf>.md`. Beyond that, the file is copied verbatim — what you write is what the agent reads.

## When you remove a function

Delete the corresponding `docs/leaves/<leaf>.md` and re-render. Orphan partials are dead code per `project-rules/general.md` — they leak old behaviour into `iii://<worker>/<leaf>` and clutter the source tree. Same rule applies to any other partial whose referent has been removed (`docs/migration.md` after a release where the migration is no longer load-bearing, `docs/companions.md` if the sibling worker is retired, …).

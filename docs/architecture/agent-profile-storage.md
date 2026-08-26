# Agent profile storage

## Decision

Agent profiles live in a dedicated root as one Markdown file per agent:

```text
agents/
  code-reviewer.md
```

`iii-directory` exposes the profile as the flat id `code-reviewer`; the file
stem is the canonical id.

This matches the established custom-agent shape used by
[Claude Code](https://code.claude.com/docs/en/sub-agents) and
[GitHub Copilot](https://docs.github.com/en/copilot/reference/custom-agents-configuration):
a dedicated agents directory containing frontmatter-backed Markdown files.
The exact vendor roots and filename suffixes differ, so iii keeps its root
configurable and retains the existing `<id>.md` filename.

The draft [`.agents` protocol](https://dotagentsprotocol.com/) instead gives
each agent a directory containing `agent.md` and optional sidecars. Harness
does not consume agent sidecars or per-agent resources, so that extra nesting
is not adopted.

## Configuration

`iii-directory` has a restart-required `agents_folder` setting whose default
is `agents`. It supports the same absolute, `~`-prefixed, and
compose-relative path forms as `skills_folder`.

The existing `agents_skills_folder` setting remains `.agents/skills`. It is a
read-only source of skills installed by agent tooling and is unrelated to the
read-write agent profile root.

A second catalog root exists: `global_agents_folder` (default
`~/.iii/agents`), the user-global side shared by every project on the
machine. It uses the same direct `<id>.md` scan; an id present under
`agents_folder` shadows the same id there, and a missing directory is
treated as empty (the worker never creates the directory itself). Unlike the
skills roots — external tooling's territory, kept read-only — this is iii's
own directory: profiles resolved here are updated and deleted in place.
Automatic vendor-directory discovery stays out of scope.

## Discovery and identity

The scanner reads only direct `<agents_folder>/<id>.md` files. Nested files
are ignored. Agent ids keep the current lowercase ASCII, digit, hyphen, and
underscore validation.

The current required frontmatter and non-empty body validation stays in
place. Unknown frontmatter keys remain harmless, so fields that iii does not
consume do not prevent a profile from loading.

Agent list/get/update/delete resolve against the merged project +
user-global scan; only create is anchored to the project root:

- create writes `<agents_folder>/<id>.md`, and refuses an id already served
  by either root (a global collision names the global file);
- update replaces the resolved file atomically, in whichever root it lives;
- delete removes the resolved file, in whichever root it lives;
- list and get do not fall back to profiles under a skills root.

There is no automatic migration or compatibility read for the old
`<skills_folder>/**/agents/*.md` layout. Existing profiles must be moved to
the canonical layout.

## Skills boundary

`agents` remains a reserved path segment for skill ids and downloaded bundle
classification. A request to create a skill whose id contains that segment
returns a normal validation error and directs the caller to
`directory::agents::create`; it must never panic.

Skill and system-prompt scans do not inspect `agents_folder`. Agent scans do
not inspect `skills_folder`, `local_skills_folder`, or
`agents_skills_folder`.

## Downloads

Registry bundle entries shaped as `agents/<id>.md` are written to
`<agents_folder>/<id>.md` and reported in `agents_written`. They are not
materialized below the worker's skills namespace. Entries that do not match
that exact shape are not treated as agent profiles.

The registry's existing skills snapshot transport may carry those entries;
that wire name does not determine their destination. Repository skill
downloads continue to copy only the requested skill and do not install an
unrelated top-level agent catalog.

Installing a bundle may atomically replace a profile with the same id, just
as reinstalling a bundle may replace its skills. Agent ids are therefore a
shared catalog namespace.

The download response and console renderer show all three result families:
`skills_written`, `system_prompts_written`, and `agents_written`.

## Change notifications

The filesystem watcher treats each root according to its role rather than
inferring every role from path segments:

- Markdown changes under skills roots use the skill/system-prompt classifier;
- a direct `<id>.md` change under `agents_folder` emits
  `directory::agents::on-change`;
- changes under the read-only `agents_skills_folder` emit
  `directory::skills::on-change`.

Worker-mediated writes keep their precise create/update/delete/download
events and suppress the corresponding external-write event.

## Compatibility with harness

Harness remains storage-agnostic. `harness::send` and `harness::spawn` keep
passing a flat agent id to `directory::agents::get`, so session resolution,
delegation, model selection, and frozen identity behavior do not change.

Tests must cover the default/configured root, exact scanner shape, CRUD
destinations, lack of legacy fallback, bundle routing, root-aware watch
events, the reserved skill-id error, and all download result lists in the
console.

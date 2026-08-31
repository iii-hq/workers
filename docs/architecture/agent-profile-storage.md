# Agent profile storage

## Decision

Agent profiles live in a dedicated root as one Markdown file per profile:

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
are ignored. Agent profile ids keep the current lowercase ASCII, digit, hyphen, and
underscore validation.

The current required frontmatter and non-empty body validation stays in
place. Unknown frontmatter keys remain harmless, so fields that iii does not
consume do not prevent a profile from loading. The keys iii consumes are
`name`, `description`, `logo`, `skills`, `model`, `reasoning_effort`, `icon`,
`color` and `extends`.

## Inheritance

`extends: <id>` names one parent profile; chains are allowed up to eight
hops. The directory resolves the chain on every read — `list` and `get`
serve resolved values, the harness never composes:

- `system_prompt` = each ancestor's body root-first, then the profile's own
  body, joined by a blank line. A profile without `extends` serves its own
  body byte-for-byte.
- `skills`, `model` and `reasoning_effort` fall back to the nearest ancestor
  that sets them when the profile omits them. A non-empty `skills` list
  replaces the parent's filter (no union); an empty list means "not narrowed
  here", never "no skills".
- `name`, `description`, `logo`, `icon` and `color` are always the
  profile's own.

A chain that does not resolve — unknown parent, loop, too deep — is
reported fail-soft, mirroring `unknown_skills`: writes are not gated, and
`list`/`get` serve the profile from its own file with `inheritance_error`
(the `D415 invalid_input` text) set, so the editor can open and fix it; the
harness refuses to run it until the chain resolves. A local `iii.md` that
extends `iii` is a self-loop (the bundled copy it shadows is not in the
catalog to extend).

## Bundled base profiles `iii` and `iii-minimal`

The worker binary embeds two agent profiles: `iii`, whose body is the harness
default identity verbatim (a unit test pins the two copies to each other), and
`iii-minimal`, the minimal directory-first identity (the same embedded file
that serves as the bundled `iii-minimal` system prompt). Both follow the
bundled system-prompt contract: always present in `list`/`get` with
`builtin: true` and an empty `modified_at`, shadowed by a local
`<agents_folder>/<id>.md`, `update` copy-on-writes that local file, `create`
of the id writes the same shadow, `delete` of the local file falls back to
the bundled copy, and nothing is ever seeded on disk. `extends: iii` builds
on the full iii doctrine; `extends: iii-minimal` on the compact one.

`model` and `reasoning_effort` are optional, verbatim catalog selections. A
catalog key may include its provider (`provider::model`); the harness splits
that key when routing. Directory deliberately does not reject a retired model
or effort: resolution against the live catalog happens when the profile is
used, so the profile remains editable and the send returns the authoritative
resolution error instead of silently choosing another model.

Agent-profile list/get/update/delete operations resolve against the merged project +
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

Skill and system-prompt scans do not inspect `agents_folder`. Agent-profile scans do
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
unrelated top-level agent-profile catalog.

Installing a bundle may atomically replace a profile with the same id, just
as reinstalling a bundle may replace its skills. Agent profile ids are therefore a
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

Harness remains storage-agnostic. `harness::send` and `harness::spawn` pass a
flat agent profile id to `directory::agents::get` and freeze its resolved
prompt, skills, model, reasoning effort, display name, icon, and color. The
resolved prompt IS the session identity — the harness puts no built-in prompt
underneath it and adds no prefix; only the per-send `mode` paragraph goes in
front, then the usual per-step runtime context. A profile served with
`inheritance_error` is refused as an invalid request. When a profile declares
(or inherits) a model, that model and its effort are authoritative for the
session.
The harness also writes the frozen display/configuration snapshot to
`SessionMeta.metadata.agent_profile`, allowing the Console sidebar and panel
header to identify the session without re-reading the mutable Directory
catalog. Existing sessions are therefore unaffected by later profile edits.

Tests must cover the default/configured root, exact scanner shape, CRUD
destinations, lack of legacy fallback, bundle routing, root-aware watch
events, the reserved skill-id error, and all download result lists in the
console.

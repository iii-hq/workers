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

The current required frontmatter validation stays in place; the body — the
system prompt — may be empty. Unknown frontmatter keys remain harmless, so fields that iii does not
consume do not prevent a profile from loading. The keys iii consumes are
`name`, `description`, `logo`, `skills`, `functions`, `model`,
`reasoning_effort`, `icon`, `color` and `extends`.

## Preloaded functions

`functions:` is a list of engine function ids (`coder::tree`,
`coder::search`, …) the profile uses routinely — its *preloaded* functions.
The harness resolves their contracts once, when the session starts as this
profile, and freezes them into the system prompt as a `<preloaded_functions>`
block (id, one-line description, compacted request schema), so the model
calls them on the first step instead of spending a
`directory::search_functions` and an `engine::functions::info` round-trip
per session. Every other function still goes through normal discovery.

The directory stores the ids verbatim; the only validation at write time is
that each entry is a non-empty id without whitespace (duplicates collapse,
first occurrence wins, order is preserved — it is the order the block is
rendered in). Whether an id is *registered* is checked where it is used:
`get` reports the ids the engine does not currently know in
`unknown_functions` (a warning, like `unknown_skills`; empty when the engine
could not be asked), and the harness renders such ids as unavailable
instead of failing the session. `list` carries `function_count`.

Besides the full-file `create`/`update`, two targeted verbs edit only this
field: `directory::agents::functions::add { id, functions }` appends ids the
profile's own list lacks, `directory::agents::functions::remove { id,
functions }` drops the ones present. Both rewrite just the `functions:`
frontmatter field in block style, leave every other byte of the file alone,
write atomically, copy-on-write a bundled profile's shadow like `update`,
and fan out `directory::agents::on-change` as an `update` — a request that
changes nothing writes nothing. They edit the profile's OWN list: a list
inherited through `extends` is replaced by setting the child's list, never
edited on the parent.

## Inheritance

`extends: <id>` names one parent profile; chains are allowed up to eight
hops. The directory resolves the chain on every read — `list` and `get`
serve resolved values, the harness never composes:

- `system_prompt` = each ancestor's body root-first, then the profile's own
  body, joined by a blank line. A blank body contributes nothing, so a
  profile with no prompt of its own serves its parent chain unchanged (and
  the empty string when it has no parent). A profile with a non-blank body and
  no `extends` serves that body byte-for-byte.
- `skills`, `functions`, `model` and `reasoning_effort` fall back to the
  nearest ancestor that sets them when the profile omits them. A non-empty
  `skills` list replaces the parent's filter (no union); an empty list means
  "not narrowed here", never "no skills". `functions` follows the same rule:
  the nearest non-empty list wins outright (no union), an empty list means
  "nothing declared here".
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
prompt, skills, preloaded functions, model, reasoning effort, display name,
icon, and color. The resolved prompt IS the session identity — the harness
puts no built-in prompt underneath it and adds no prefix; only the per-send
`mode` paragraph goes in front, then the usual per-step runtime context.
When the profile declares (or inherits) `functions`, the harness appends the
`<preloaded_functions>` block — each id's current description and compacted
request schema, taken from its cached registry snapshot with one
`engine::functions::info` batch for ids the snapshot cannot vouch for — to
that frozen prompt; ids the engine does not know are named as unavailable.
The declared ids also travel in `SessionMeta.metadata.agent_profile.functions`. A profile served with
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

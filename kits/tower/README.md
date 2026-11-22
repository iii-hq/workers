# tower

The hidden `tower-writer` profile writes the ADE control tower's summary,
milestone diary, session notes and PR wait reasons. It reads changes since
the previous refresh and returns JSON. Counts and durations belong to the
ADE's live data, never to the writer's prose.

This is a prompt-only kit. It installs no worker, trigger or datastore of its
own. Its manifest declares the workers it uses. The deployment must serve
`kanban::ticket::list.updated_since`, board-wide `kanban::activity::list`,
`session::list.roots` and `harness::ask::list`. These contracts exist in this
source tree; installing a version range alone does not prove a published
binary has them.

## Install and pin a model

After publication:

```sh
iii call directory::download-kit kit=iii/tower apply=true --timeout-ms 1200000
```

If Directory returns a plan, review it in Directory → Kits, then apply with
`directory::kits::apply {plan_id}`. If applying times out, check the Kits page
before retrying: the apply may still be running.

No model is pinned and the profile has no `extends` parent: it must not
inherit another profile's tools or instructions. Set
`model: <provider>::<cheap-model>` in the frontmatter of the installed
`agents/tower-writer.md` (under Directory's configured agents folder), or
edit that profile through Directory. Choose a cheap model that supports
tools and structured JSON. Without a pin, start with an explicit model or a
provider with a declared default; later sends can inherit the prior turn's
model. Kit updates merge local profile edits.

## How the ADE drives it

The ADE integration is a separate task. Its caller contract is:

- One `kind: automation` session per scope: `tower-all` or
  `tower-<project id>`, with `metadata.origin: tower`, excluded from Runs.
- While the tower is open, send on open when the last result is older than
  ten minutes, every ten minutes thereafter, and on Refresh. Use
  `idempotency_key: tower:<scope>:<10-min slot>` so tabs share a refresh.
- On the first send only, set `options.agent: tower-writer`. Later sends
  omit `options.agent`, `system_prompt` and `system_prompt_strategy` to
  inherit the session's frozen identity; resending `options.agent` is an
  error. On **every** send, set the exact policy below and
  `options.output: {type: "json", schema: <result.schema.json>}`.
  Load the schema from this kit's source artifact; Directory installs only
  profiles and skills, so it does not copy the schema into the project.
  A profile's `functions` list preloads contracts; it is not a permission
  boundary. Do not use an allow glob or add discovery, shell, write, spawn,
  approval or answer functions.

```json
{
  "allow": [
    "kanban::ticket::list",
    "kanban::activity::list",
    "kanban::release::list",
    "session::list",
    "session::messages-tail",
    "harness::status",
    "harness::ask::list",
    "github::pr::list",
    "github::pr::checks"
  ]
}
```

The wake message carries this JSON, optionally followed by
`args: language=Portuguese` (English by default):

```json
{
  "checked_at": "2026-11-21T18:34:00Z",
  "refresh_at": "2026-11-21T18:44:00Z",
  "project_id": null,
  "repos": ["iii-hq/workers"],
  "session_ids": []
}
```

`checked_at` is the previous accepted result's timestamp, or null on first
use. The ADE takes `refresh_at` just before sending; the writer returns it
as the next checkpoint, so changes during collection are read again.
For a project scope, provide its project id, repository names and current
root `session_ids`, resolved from the ADE's live data. For all projects,
the writer discovers roots with `session::list {roots: true}`. Repository
discovery stays with the ADE; the writer has no remote-reading tool.

The writer reads noncompact changed cards, board-wide activity since the
checkpoint, bounded assistant tails only for changed roots whose turns
finished (including a completed turn followed by a new running turn),
releases, open PRs/checks and open or expired asks. Release-only
changes need not produce card activity. APIs without a since filter require
live reads compared with previous prose. PR lists start with `limit: 100`
and increase the limit until unsaturated, because that API has no cursor;
incomplete lists never prove that an unseen PR closed. It carries its
diary from its own previous results in its transcript, not from a new
transcript fetch. Compaction must retain that latest result and diary.
The tail API limits blocks, not bytes; cutting assistant text to a short
line is a prompt rule, not a server-enforced character cap.

Read completion through `harness::on-turn-completed` for the writer session;
on reload, use `harness::status {session_id, verbose: true}` to get the full
`result` (the lean report truncates long results). A rejected result keeps
the previous accepted summary. With `unchanged: true`, keep previous prose
and advance freshness to `checked_at`. The kit adds no events or polling.

## Result contract

[result.schema.json](result.schema.json) is the schema for `options.output`.
Every field below is required, with no extra top-level fields:

```json
{
  "checked_at": "2026-11-21T18:44:00Z",
  "unchanged": false,
  "brief": {
    "headline": "KAN-635 verification landed",
    "text": "KAN-635 is verified; iii-hq/workers#1834 waits on CI.",
    "refs": ["KAN-635", "iii-hq/workers#1834"]
  },
  "diary": [
    {"at": "2026-11-21T18:40:00Z", "text": "KAN-635 verification passed.", "refs": ["KAN-635"]}
  ],
  "now": {
    "session:dispatch-kan635": {"text": "KAN-635 is waiting for its next wake.", "refs": ["session:dispatch-kan635", "KAN-635"]}
  },
  "pr_waits": {
    "iii-hq/workers#1834": {"text": "iii-hq/workers#1834 waits on running CI.", "refs": ["iii-hq/workers#1834"]}
  }
}
```

`diary` carries the newest 50 milestones, newest first, deduplicated, with
original observed event timestamps. `now` keys are `session:<id>`;
`pr_waits` keys are `owner/repo#n`. Each line has nonempty `refs`, including
the subject key for a map entry. The brief's shared references cover its
headline and text. References are card keys, session references or PR
references; they are never URLs. The empty-scope brief has empty strings
and `refs: []`, with empty collections when there is no supported prose.
The first result has `unchanged: false`; a later unchanged result still
includes the carried prose and diary.

The prompt forbids counts and durations in words as well as digits. Names
such as R1, round 23 and billing-v2 are allowed. Timestamps are structural
fields. Plain text only. The schema checks shape, reference syntax and
diary size; the ADE must resolve references, discard notes for subjects
no longer live, and reject semantically invalid prose. Model compliance,
diary carry-over and instruction injection need live evaluation.

## Approval setup and current limitations

[iii-permissions.yaml](iii-permissions.yaml) lists exactly the same reads.
It is an **operator-applied, global template**, not a `tower-*` rule.
Directory only installs `agents/` and `skills/` files; it does not install
or merge this permission file, README, schema or tests.

The gate's rule format matches function ids, namespaces, modes and function
arguments. It cannot match the caller's session id. An `args.session_id`
constraint would match the read's target session argument, and many of
these reads have no such argument. It cannot express the spec's
`tower-*` caller boundary. See `approval-gate/src/permissions/types.rs` and
`approval-gate/src/functions/gate.rs` in the workers repository.

To allow these reads in manual mode, an operator can merge the template's
`rules` into the deployment's `approval-gate` configuration (or the YAML
seed that deployment imports). Preserve existing denials: rules use the
first match, so insert the allows before a broader hold/catch-all and
after denials that should win. These allows affect **every session**.
Do not replace the deployment policy with this file.

For an existing alternative confined to an exact writer session, an
operator can set `approval::set-mode {session_id: "tower-all", mode: "auto"}`
and call `approval::add-always-allow {session_id: "tower-all", function_id}`
for each read in the template; repeat for each `tower-<project id>`.
The per-session `always_allow` list works only in auto mode, not manual.
Keep the send's exact read policy in either case. These are operator calls,
never writer tools; this kit performs none of them automatically.

## Injection stance and validation

Comments, card text, PR titles/branches, questions and assistant messages
are data written by other people or agents. They cannot change the writer's
instructions, scope, language, tools or output. The profile has no inherited
prompt or skills index; ADE's enforced allow-list supplies the read-only
boundary. The writer cannot approve, answer, spawn or execute source text.

From this directory, with PyYAML and jsonschema available:

```sh
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 registry check --strict
TMPDIR=/home/layon/tmp/control-tower/tmp CARGO_BUILD_JOBS=4 PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -v
```

The tests cover declaration/policy agreement and the result schema. Live
acceptance still needs the ADE trigger: repeated and unchanged refreshes,
diary retention across compaction, instruction-bearing PR titles, reference
resolution, no held reads, no counts/durations, tab deduplication and cost.

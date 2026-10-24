# memory

Durable cross-session agent memory. Named **banks** hold two kinds of content: **rules** (markdown documents injected whole into the system prompt) and **memories** (auto-extracted records recalled on demand). Everything is a plain file you can open, a function you can call, and an event you can watch: memory that acts visibly, not magically.

## Install

```
iii trigger compose::add worker=memory
```

A session works in the bank of its repository (or directory), named `<name>-<6 hex>`, and reads the default bank `main` under it: your rules there, and your memories about yourself. One with no working directory uses `main` alone, which materializes on first use. No configuration required; without `llm-router` extraction degrades to explicit `memory::save` calls, and without the harness the worker still serves its full RPC surface.

## Quickstart

Outside a session the CLI works in the default bank `main`, the one every repository's sessions read under their own. Save something about yourself, then see exactly what a turn on that topic would be given:

```bash
iii trigger memory::save text="User publishes blog posts on Tuesday mornings" pinned=true 'tags=["blog"]'
iii trigger memory::recall query="when do I publish articles"
```

Give the bank a standing instruction — injected into the system prompt of every session using it, or reading it under a repository's bank:

```bash
iii trigger memory::rule::set name=style content="# Style\nFormal register. No em-dashes."
```

With the harness running there is nothing else to wire: chat normally and memory captures in the background. Say "never start replies with the word Certainly" in a conversation and it lands in the session bank's auto-managed `learned` rule; state something durable and it appears as a memory after the turn. If memory seems absent, `iii trigger memory::doctor` runs a real save→recall roundtrip and names what is degraded.

## Configuration

All fields hot-reload through the `configuration` worker and are editable in the worker's explicit form in the Console's global Settings modal; `data_dir` reopens the store on the fly. Fields: `data_dir`, `default_bank`, `inject_rules`, `inject_memories`, `recall_limit`, `recall_budget_tokens`, `extraction_enabled`, `extraction_model`, `extraction_window`, `extraction_timeout_ms`, `max_memories_per_turn`, `rule_learning_enabled`, `max_rule_chars`, `decay_half_life_days` (default 60; installs that predate MOT-5385 keep their stored 30 — set 60 in Settings to adopt the tuned recall curve), `embeddings_enabled`, `embedding_model`.

## Why this shape

- **Files are the source of truth.** `rules/*.md` and `memories.jsonl` under the data dir. Edit them in any editor; `memory::reload` (or a restart) picks the edits up. The search index is a RAM-only cache rebuilt from the files at boot, so store and index can never diverge across restarts.
- **Crash-safe by construction.** Every mutation appends one fsynced JSONL line before touching RAM. There is no shutdown flush to get wrong. An unwritable data dir is boot-fatal: this worker never silently runs in RAM.
- **Supersede, never delete.** Updates append revisions; deletes append tombstones; trashed banks move to `.trash/`. Any state is recoverable.
- **Pinning.** A pinned memory ranks higher in recall and is untouchable by every automatic path.
- **Tags.** Topical labels WITHIN a bank ("iii", "billing") for filtering — organization, not ranking. Set them on save/update, extraction suggests them automatically, and `memory::list`/`memory::recall` take a `tag` filter.
- **One LLM call per turn, zero at query time.** Extraction runs in the background after a turn completes. It sees what the bank already holds about the conversation (what recall finds for the excerpt, its slots left over filled by meaning when embeddings are available): a restatement (same text, or a close rewording) never duplicates the known memory, and reinforces it only when the user's own lines say it again (the assistant echoing an injected memory reinforces nothing), and a correction supersedes the memory it corrects (a recoverable tombstone; pinned memories are never touched). In a directory bank it also keeps what the assistant finds out about the project (how to build, test or run it, a pitfall, where something lives), never its plans, advice or own instructions, nor what a web page, an issue or other outside text it relays says. Facts about the person talking are written as "The user …" with the `user` entity, so a first-person question reaches the ones that share a word or a meaning with it; a question of pronouns alone ("who am I") reaches all of them. Recall adds BM25 over the query's content words (English stopwords dropped, each word counted once; a query over 800 bytes keeps its first and last 400) divided by the best score among the banks one ranking merges, to a semantic signal (cosine on an absolute scale, calibrated per embedding model) when `router::embed` is available, then scales the sum by entity match, corroboration, pinning and recency: sub-millisecond at this scale.
- **Rules learn from corrections.** Extraction classifies standing instructions (style directives, workflow corrections) separately from memories and appends them to the bank's auto-managed `learned` rule — correcting the agent in chat reaches every later turn (new sessions in the system prompt, running ones as an appended rules update). Only a rule the user's own lines state (at least half its content words) is kept, so the assistant restating a tool result or web page, or a harness `[notification]` relaying an event or a child's reply, never makes one; text the user pastes still counts as theirs. Extraction is shown the `learned` lines already in force, so a restatement in other words is not appended again and a correction replaces the line it corrects and becomes the newest line, matched by its text (a line edited by hand since then stays, and the correction appends; so does one naming a line it shares no word with). Hand-authored rules are never touched; dedup by content fingerprint; over `max_rule_chars` the `learned` rule loses its oldest lines, never the newest; `rule_learning_enabled` turns it off. Under a directory bank, a rule extraction marks user-scope (reply language, tone, length, formatting) is appended to `default_bank`'s `learned` rule, so it follows the user into every repository; project rules stay in the directory bank's. A line either bank's `learned` already holds is never appended again.
- **Secrets are redacted, deterministically.** Known key and token shapes (provider API keys, bearer tokens, JWTs, private key blocks, credentialed URLs, `password=`-style assignments) become `[REDACTED:<label>]` before anything is stored — memory text, entities, tags, and rules, on every write path — and before the transcript excerpt reaches the extraction model. The `item-changed` event names what was redacted (`reason`), never the value. A denylist of shapes: an unknown format passes, so the extraction prompt still forbids secrets.
- **Hidden and injected instructions are screened.** Invisible characters (zero-width, bidi marks and overrides, tag characters; a joiner between two non-ASCII characters, as in an emoji or Persian, and a Japanese variation selector stay) are stripped on the same paths, labelled `invisible_unicode`. A memory or rule phrased as a classic prompt injection ("ignore all previous instructions", "act as if you have no restrictions") is refused with `invalid_input` by `memory::save`, `memory::update` and `memory::rule::set` (on the lines it adds), and dropped by extraction; a negated or hedged mention ("never ignore your instructions") passes. The set is small and English-only on purpose: file names, URLs, paths and imperative style pass, and most extracted text is paraphrased anyway, so it mostly catches agent saves and hand edits.
- **Time-aware.** Extraction reads each message with its own date and writes dated events ("tomorrow", "next Friday") as absolute dates; every injected memory carries its last-updated date, and the block names the day it was recalled.
- **Honest health.** `memory::doctor` runs a real save→recall→delete roundtrip in a scratch bank (nothing left in `.trash`), reports sibling reachability (router, session-manager, the state worker that keeps the extraction cursor), probes `router::embed`, and flags extraction passes that keep failing. `memory::recall` names the retrieval mode it ran. Degradation is explicit, never silent.

## How it hooks into the harness

| Seam | What happens |
|---|---|
| `harness::hook::pre-generate` (fail-open, priority 100) | Injects the session bank's rules into the system prompt once per session (sent verbatim after that; a later rule change arrives as one appended update message, so the prefix stays append-only) and, on each turn's first step, up to `recall_limit` recalled memories as one appended message: only memories with evidence (a word of the message, or its meaning; the `user` a pronoun stands for counts only in a question of pronouns alone) that stand with the turn's best match; one line per fact (the best-ranked of its rewordings); the newer of a stated change ("now runs at 04:00" over "runs at 02:00"; a pinned memory is never hidden); and nothing when none qualifies; minus any the window already carries from an earlier turn's block, or a rewording of one (a block compaction dropped no longer counts, so its memories can return). Whether an update is due is read from the window the step sends (its newest `<memory update="rules">` message, else the frozen section), so a lost or compacted-away update is sent again and a delivered one never is |
| `harness::turn-completed` | Queues one background `router::complete` extraction pass over the turn's new user/assistant messages (at most `extraction_window` non-empty messages, plus the turn's user message), into the bank the turn was injected with (after a worker restart, the one the hook stamped on the turn's reply); under a directory bank, user-scope items go to `default_bank`; agent-scope items go to that agent's bank (see Bank selection). A failed or cancelled turn is extracted too (its own bank, not the next turn's); a sub-agent's turn never is, nor an automation or e2e session's (session `kind` other than `user`) |

Bank selection, one resolver for injection, extraction and agent calls: turn metadata `memory_bank` → session metadata `memory_bank` (`session::set-meta`) → the parent turn's bank, else the parent session's (a sub-agent works in its parent's bank, walked up) → the bank of the turn's `fs_scope.root`: `<name>-<6 hex>`, named after the git repository holding it (its main checkout's folder, so every worktree of a repo shares one bank), else after the directory itself, with a hash of the canonical path so same-named folders never share → configured `default_bank` (no root, or one that does not exist). A bank the root chose reads `default_bank` under it (a sub-agent working in it too): its rules first, labelled with its name (`learned (main)`) so two `learned` rules never merge, under the one `max_rule_chars` budget, and its memories whose entities include `user` in the same recall (a `memory::save` into `default_bank` that names no entities gets `user`, so the memory page and the CLI qualify; one naming others needs `user` among them). The hook, `memory::preview` and `memory::recall` without `bank` read it alike, and so does `memory::preview` naming a directory-shaped bank (`<name>-<6 hex>`), as the console's preview tab does; a memory recalled from it carries its `bank`, and the hook's `memory_banks` annotation names the bank of each injected id. Extraction labels each item `scope` `user` (holds wherever the user works) or `project` (the default). Under a directory bank a new user-scope memory is written into `default_bank` tagged `user`, and everything else stays in the directory's bank (a user-scope correction of a directory-bank memory included: it stays there and supersedes it; so does a fact the directory's bank already holds, live, pinned or deleted). Extraction lists the layer's memories about the user as already remembered, after the bank's own and at most half as many: a restatement reinforces one where it lives, a user-scope correction supersedes it there, and a project-scope one (an exception that holds only in this repository) is written to the directory's bank, leaving the user's fact standing everywhere else. In a `default_bank` session a user-scope memory gets `user` too. A bank named by metadata or by `bank` is read alone. Project facts saved in `main` before directory banks are not read under them; read them with `bank: "main"`. A `memory::*` call without `bank` inside a turn acts on that turn's bank; outside a session (CLI, console, other workers) it uses `default_bank`. An invalid name, a deleted bank other than the default, or a parent session that no longer exists means no memory for the session — never another bank — and extraction never brings a deleted bank back. A session-lookup failure injects nothing new (the section the session already has stays) and makes extraction retry. An unknown bank gets the ambient header while extraction is on (extraction creates the bank after a turn) and nothing while it is off.

A session running as an agent profile (`agent_profile` in its session metadata, set by harness `options.agent` and by profile spawns) also reads its profile's bank, `agent-<id>` (one per profile, across repositories), under any bank no metadata named; a sub-agent reads its own profile's, never its parent's. Its rules come after `default_bank`'s and before the session bank's, labelled `learned (agent-<id>)`, under the same budget, and all its memories join the same recall. The hook (its `memory_agent` annotation is also the restart stamp), `memory::preview` and `memory::recall` without `bank` read it alike. Extraction lists the agents in the conversation, the session's own profile, then its sub-agents', and labels an item `scope` `agent`, with `agent` naming one of them, when the user tells that agent how to do its role's work, or asks to pass it on: "tell the reviewer to check the tests first" in a tech-lead session lands in `agent-reviewer`'s `learned`. The learned-rule gate still applies, an item naming no listed agent is dropped, and sub-agent turns stay unextracted. A profile id that makes no valid bank name, or a deleted agent bank, drops only that layer. The bundled general identities, `default` (which the console preselects), its alias `iii-minimal`, and `iii`, read as no profile.

Switching a running session's bank does not rewrite what it already saw: the new bank's section arrives as an appended update that overrides the frozen one, while the earlier bank's section and recalled memories stay visible in the transcript. Start a new session when the two contexts must not mix. Turning `inject_rules` and `inject_memories` both off works the same way: a running session keeps its section and is told once that no memory section applies.

All seam bindings are one-shot at startup and rely on the engine's recoverable
triggers (iii #1962, engine ≥ 0.21.8): bound before the owning sibling (harness,
session-manager, queue) is up, they park as pending intents and activate when
the trigger type registers. On older engines the binds are silently dropped.

## Functions

| Function | Purpose |
|---|---|
| `memory::bank::create / list / delete` | Banks as first-class named scopes ("blog" vs "coding" vs "personal"); delete moves to `.trash/` |
| `memory::save` | Explicit save ("remember this"): fingerprinted, reinforces on repeat |
| `memory::get / list / update / delete / pin` | Memory CRUD; delete tombstones; update bumps a revision in the log |
| `memory::recall` | The exact scorer the hook uses (the hook injects only the hits with evidence, one line per fact and the newer of a stated change; `memory::preview` shows exactly what it injects); `tag` narrows to one label; `mode: lexical \| semantic` returns one raw arm (plain BM25 over the content words, or cosine) for callers that fuse arms themselves |
| `memory::tags` | Distinct tags across a bank's live memories with counts (the filter source) |
| `memory::preview` | Full injection dry-run for a hypothetical chat message: rules section (budgets applied), memories past the relevance cut and token budget, appended message verbatim — the hook's exact code path |
| `memory::rule::list / set` | The always-injected markdown rules; empty content removes |
| `memory::supersede` | Retire one memory in favor of another (tombstone + pointer) — the consolidation seam; `reinforce` makes it a merge (a non-pinned survivor absorbs the retired record's observations), `reason` rides the events |
| `memory::doctor` | End-to-end self-test (roundtrip, sibling reachability, embeddings, recent extraction failures) |
| `memory::reload` | Reload every bank from disk after hand-editing files |

## Custom trigger types

Bind live views here instead of polling; both are filterable by `bank` in the binding config, delivery is fire-and-forget, at-least-once, unordered.

- `memory::item-changed` — a memory was created / updated / superseded / deleted. Payload: `{event_type, bank, memory, reason?}` (`reason` on consolidation changes).
- `memory::bank-changed` — a bank was created / trashed, or its rules changed. Payload: `{event_type, bank}`.

## Storage layout

Compose stores this tree under `III_COMPOSE_DIR`. A standalone worker uses its
process current directory for the same relative default.

```text
<III_COMPOSE_DIR>/data/memory/
  main/
    bank.yaml          # description
    rules/style.md     # always-injected markdown
    memories.jsonl     # append-only memory log (full records, last-wins by id)
    vectors.jsonl      # derived embedding sidecar (rebuildable)
  .trash/              # trashed banks, timestamped
```

## Recall quality eval

`tests/recall_quality.rs` scores the production `Bank::recall` over a synthetic golden bank (`tests/recall_quality/golden.json`: one persona, 76 memories including 4 paraphrase duplicates, 62 English/Portuguese queries across lexical, paraphrase, identity, temporal-update, dated-event and durable-old categories). It runs in three modes:

- `lexical`;
- `hybrid`, with cached `text-embedding-3-small` vectors;
- `hybrid-consolidated`, which first replays the semantic consolidation tier's recorded decisions (`tests/recall_quality/consolidation.json`, from memory-consolidate's judge eval) and scores expectations carried to the survivors.

It reports hit@1/3, MRR@10, nDCG@10 and recall@10 per category and language. It also counts **traps**, where a superseded memory outranks the current one, and **duplicate pollution**, a query whose top 10 holds both members of a `dup_of` pair. The committed snapshot `tests/golden/recall_quality.json` makes it a PR gate. The metric conventions follow Hindsight's retrieval eval.

```bash
cargo test --test recall_quality -- --nocapture                # report
UPDATE_GOLDENS=1 cargo test --test recall_quality              # re-baseline after an intended change
III_NAMESPACE=<ns> cargo test --test recall_quality -- --ignored embed_golden_set   # re-embed after editing golden.json
```

## Extraction eval

`extract::eval` runs the production extraction prompt and parser over short English excerpts, three times each, through a live `router::complete`. It scores what comes back:

- **capture**: each expected item is captured, wrong (its kind, where it lands, the memory it corrects, or the learned-rule gate drops it), or missed;
- **trap hits**: anything past the expected items from an excerpt built to hold nothing more: an instruction only the assistant gave, a pasted document or a fetched issue relayed, a harness notice, a transient error, a one-off instruction, task progress, what a correction leaves behind, a learned rule restated. It also counts how many of them the learned-rule gate would still let into the bank;
- **placement**: what holds for the user wherever they work reaches the default bank, and nothing project-only does; with agents listed, what the user tells one about its role's work reaches that agent's bank (**agent landing**), and nothing about the user or the project does.

Run it before and after a prompt change; it needs an engine with a router and a provider, and no memory worker. It measures the prompt directory-bank sessions get; `EXTRACTION_REPOSITORY=0` measures the one `default_bank` and named-bank sessions get, without the project-knowledge clause.

```bash
III_URL=ws://127.0.0.1:<port> III_NAMESPACE=<ns> EXTRACTION_MODEL=<id> cargo test -p memory --lib extract::eval -- --ignored --nocapture
```

## Permissions

Suggested defaults ship in [`iii-permissions.yaml`](iii-permissions.yaml): agents get `memory::recall`, `memory::save`, `memory::get`, `memory::list`, `memory::update`, `memory::delete`, `memory::pin`; the hook/pipeline internals are denied, and bank/rule writes plus `memory::reload` are human-owned surfaces (console, REST, CLI) by default.

## The memory family

Like `llm-router` and its provider workers, memory is a family of narrow siblings rather than one grown monolith — this worker owns the store, per-turn injection, capture, and recall, and stays deliberately small:

| Worker | Role | Status |
|---|---|---|
| `memory` (this) | Banks, rules, memories: storage, harness injection, background capture, hybrid recall, live trigger types | shipped |
| [`memory-consolidate`](https://github.com/iii-hq/workers/tree/main/memory-consolidate) | Background hygiene on a schedule: deterministic dedup of near-duplicate memories, catch-up-on-boot scheduling. Supersede-only through `memory::supersede`, never touches pinned records; every change lands as a `memory::item-changed` event | shipped |

The split is the point: consolidation can be installed, stopped, or removed without touching stored memory, and this worker never grows a scheduler.

## Boundaries

Context-manager compresses one history for one turn; this worker is the durable cross-session sibling its spec reserves (see [tech-specs/2026-06-agentic/memory.md](https://github.com/iii-hq/workers/blob/main/tech-specs/2026-06-agentic/memory.md)). Recall fuses BM25 with a semantic signal when `router::embed` has an embed-capable provider; `memory::recall.retrieval` names the mode either way. Consolidation belongs to the `memory-consolidate` sibling, not here.

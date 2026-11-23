# memory-consolidate

Scheduled hygiene sibling of the [`memory`](https://github.com/iii-hq/workers/tree/main/memory) worker. Finds near-duplicate memories in every bank and merges them: the duplicate retires with a `superseded_by` pointer (never a delete), the survivor absorbs the observation as corroboration. A memory from the user always survives over a note an agent saved from a turn, and that note never retires it. An alias tier retags memories whose entity handle is another spelling of a handle the bank already uses. An opt-in verify tier checks notes against their source transcript and replaces a note the transcript contradicts. Every write goes through memory's supersede, update and correct seams (`memory::supersede`, `memory::update`, `memory::correct`), pinned memories untouchable, every change visible as a `memory::on-item-changed` event. Install, stop, or remove it without touching stored memory.

## Install

```
iii call compose::add worker=memory-consolidate
```

Requires the `memory` worker. Scheduling reuses the cron worker: `memory-consolidate::on-tick` observes `cron::on-tick` with args `{ expression: "0 0 * * * *" }`, an hourly heartbeat, and the tick runs a pass only when `interval_hours` (default 24) have elapsed since the last one — the last completed pass persists in the state worker. **Catch-up-on-boot**: a pass missed while this worker was down runs shortly after boot instead of waiting for the next heartbeat; a slim backstop loop also keeps the schedule alive on rigs without the cron worker.

## Quickstart

Plan without writing; the `run` response is the plan:

```bash
iii call memory-consolidate::run dry_run=true
```

A dry run is read-only: it reports the deterministic groups and the alias pairs it would merge, skips the LLM, semantic and verify tiers and the alias judge (no recall, embedding, transcript, model or rule calls), and writes no memory, cursor, pass report or `last_run`, so `memory-consolidate::status` keeps showing the last real pass. The configured `dry_run: true` (scheduled planning mode) makes the same read-only plan but records it in `reports` and checkpoints `last_run`, so the schedule still waits `interval_hours` between passes.

Apply for one bank only:

```bash
iii call memory-consolidate::run bank=blog
```

The report names every group: the surviving memory, the retired duplicates, and anything skipped because it is pinned. The retired records stay on disk and remain queryable with `include_superseded: true`.

## Functions

| Function | Input | What it does |
|---|---|---|
| `memory-consolidate::run` | `{ bank?, dry_run? }` | One consolidation pass on demand, over every configured bank or one bank; returns the per-bank report |
| `memory-consolidate::status` | `{}` | The schedule and the last passes: `enabled`, `interval_hours`, `last_run`, `due`, and `reports`, the last 30 pass reports newest first (`{ ms, dry_run, banks }`, each bank with its LLM calls, tokens and cost; only the newest keeps its banks' `groups`, `aliases` and `semantic` lists) |
| `memory-consolidate::alias-undo` | `{ bank, from }` | Undo an alias merge: the pair turns `distinct` in the ledger, then each memory the merge retagged that is live, unpinned, still carries the canonical handle and does not already carry `from` gets it back; returns `{ bank, from, to, restored, skipped }`. Operator-only, like `run` |
| `memory-consolidate::on-tick` | the `cron::on-tick` event | Internal: the schedule heartbeat |
| `memory-consolidate::on-config-change` | the `configuration::on-updated` event | Internal: hot-reloads the configuration |
| `memory-consolidate::rest` | the http worker's `HttpRequest` | Internal: serves the `run`, `status` and `alias-undo` REST routes |

The worker hosts no observable events of its own: every change it makes is a `memory::on-item-changed` call of the memory worker. It observes:

- `cron::on-tick` with args `{ expression: "0 0 * * * *" }`, target `memory-consolidate::on-tick`;
- `configuration::on-updated`, filtered on its own entry id and `configuration:updated`, target `memory-consolidate::on-config-change`;
- `http::on-request` with args `{ method: "POST", path }` for `run`, `status` and `alias-undo` (`POST /memory-consolidate/run`, `POST /memory-consolidate/status`, `POST /memory-consolidate/alias-undo`), target `memory-consolidate::rest` (internal). It receives the `HttpRequest`, runs the function with the request's JSON body as its input (an empty body is `{}`), and returns the `HttpResponse`: `200` with the function's result, or `{ error }` with `400` (the body does not fit the input), `404` or `500` (the function failed, a bank outside the allowlist included). `curl -X POST localhost:3111/memory-consolidate/run -H 'content-type: application/json' -d '{"bank":"blog","dry_run":true}'` plans one bank.

Each observer stays `pending` until its function registers, so the cron, configuration and http workers can boot in any order.

## Configuration

All fields hot-reload through the `configuration` worker: `enabled`, `interval_hours`, `dry_run` (scheduled passes plan-only), `banks` (allowlist, empty = all), `max_supersedes_per_run` (safety cap; remainder waits for the next pass), `max_llm_calls_per_run` (200: `router::complete` calls per pass, the judge, alias, semantic and verify tiers of every bank together; the rest waits for the next pass with a `deferred` note; 0 = no model calls), `alias_merge_enabled` (on by default, see the alias tier below), `verify_enabled` (off by default) and `verify_max_per_run` (20), see the verify tier below.

To run without model calls, set `max_llm_calls_per_run` to 0 (spelling aliases still merge, near pairs wait with a note), or turn off `alias_merge_enabled` and `semantic_enabled`; `llm_assist_enabled` and `verify_enabled` are off by default and call the model only when turned on.

### The model

`llm_model` serves the judge, alias, semantic and verify tiers. A pinned `provider::id` is used as written, and the router is not asked. Left empty, the model is automatic, by the same rule as memory's `extraction_model`:

1. A provider is usable when it is available and signed in, or owns its sign-in (no credential env var).
2. Tier 1: the default model of the first usable provider, by provider id.
3. Tier 2, only when no usable provider declares a chat default: the first chat model `router::models::list` lists for a usable provider, by provider and then model id.
4. An id that contains `embed` or `rerank` is never picked. With nothing left, the tier stops with a note that names each provider and why it was skipped.

The pick is kept for 60 s, so a pass does not ask the router before every model call. It is logged at INFO when it changes, as `automatic consolidation model model=<provider::id> reason=<provider_default|first_listed_model>`. A failed `router::complete` call and a provider error reply both end in `(model <id>)`, so the pass notes name the model. On the judge tier, such an error is noted as `llm tier skipped`. The pick can change shortly after a restart, until every provider has finished its first model refresh, and a provider whose sign-in has expired can still be picked. Pin `llm_model` to rule this out.

## The alias tier (on by default)

The extraction model can write one thing under two entity handles: `build cache` on some memories and `build-cache` on others, or `dentista` beside `dentist`. Recall and the hook compare handles as strings, so the two read as two subjects. `alias_merge_enabled` (on by default) runs this tier on each bank right after the deterministic pass, on the same scan less the memories that pass retires. It retags in place: each memory carrying the alias gets the canonical handle through `memory::update` (same id, revision + 1, the prior revision stays in memory's log). Nothing is read through an alias map, so recall code is unchanged.

**The rule.** Similarity is pg_trgm's: lowercase, split into runs of letters and digits, pad each word with two blanks before and one after, take every 3-character window, Jaccard of the two sets. A pair is a candidate only when every guard holds:

1. Neither handle is `user`, the speaker's fixed handle. It is never retagged, and no handle is retagged into it.
2. No live memory carries both. Two handles on one memory are two things; the pair is recorded as `distinct` and never asked about.
3. Tier `spelling`: the same characters once whitespace, `-` and `_` are dropped, case aside, the rule memory's extraction snaps handles with (`build cache` / `build-cache`, `vs code` / `vscode`). It merges with no model call. Any other character counts: `c++` / `c#` and `c` / `c++` are `near` pairs for the judge, and `java 1.8` / `java 18` never pair (other words and digit runs).
4. Otherwise tier `near` needs the same number of words, the same digit runs (`us-east-1` / `us-east-2`, `pg15` / `pg16` and `python` / `python3` never pair), each word pair equal or at 0.6 or more, and the whole handles at 0.6 or more (`dentist` / `dentista` 0.70, `postgres` / `postgresql` 0.67). `payments-api` / `ledger-api` fails twice: `payments` against `ledger` is 0.00, the whole handles 0.20.
5. A handle with two or more candidate partners merges with none of them; its pairs are listed as ambiguous.

The canonical handle is the one on more live memories, then the one whose first memory is older, then the smaller string.

**The judge.** No similarity floor tells `dentist` / `dentista` from two people: `gabriel` / `gabriela` scores the same 0.70. So a `near` pair merges only after a `router::complete` judge says both handles name the very same person, project, place or tool (another spelling, a plural, another language's word for it); two people, two services, a product and its part are different, and when unsure, different. One call per bank per pass, only when the bank has a near pair with no recorded verdict, for at most 20 pairs, each with both handles and up to two memory texts each; the rest wait for the next pass. The reply is strict JSON, `{"pairs":[{"n":1,"same":true,"reason":"..."}]}`, and its output budget holds 20 verdicts. A reply cut short, that does not parse, or that answers a pair twice or numbers one outside the batch halves the batch, one call per half, as the semantic tier does. A pair that still fails alone is recorded `distinct` (`kept apart: no judge reply about this pair parsed`) once another reply of the pass parsed, so it never holds up the others; when no reply parsed, nothing is recorded and the pairs are asked again on the next pass. Every call counts in `llm_calls`, tokens and cost and against `max_llm_calls_per_run`. With no router, or the cap reached, near pairs stay in the report as `waits for the judge`, with a note.

**The ledger.** Every decided pair is kept in the state worker (scope `memory_consolidate`, key `aliases:{bank}`): `{ from, to, tier, verdict, reason, ms, ids }`, `verdict` `same` or `distinct`, `ids` the memories the merge retagged. A `same` entry retags later memories that carry the alias with no judge call, and a `distinct` entry is never asked again. The entry is written before the retags; a ledger that cannot be read or saved skips the tier with a note, so it never judges or writes blind.

**Writes.** Each retag is `memory::update { bank, id, entities, expected_memory, reason }`: `entities` is the memory's list with the alias replaced, order kept, repeats dropped; `expected_memory` is the record as this pass read it, so memory refuses one edited since; `reason` (`alias: dentista -> dentist (<why>)`) rides the `memory::on-item-changed` event. Pinned memories are never retagged and are listed in `skipped_pinned`. A memory whose retag would carry more than eight handles is left alone with a note: an older memory worker keeps only the first eight, which would drop `user`. Each retag spends one unit of `max_supersedes_per_run`; the rest counts in `capped` and waits for the next pass. A memory that absorbed a duplicate earlier in the same pass is retagged on the next pass. A failed retag is a bank error, so the pass stays due. A retag bumps `updated_at`, so the semantic and verify tiers look at that memory again. An older memory worker ignores `expected_memory` and `reason` and retags all the same.

**Report.** Each bank lists `aliases`: `{ from, to, tier, similarity, applied, ids, skipped_pinned, reason }` per pair merged, judged distinct this pass, waiting for the judge, or ambiguous. A dry run lists what it would merge, `applied: false`, and never calls the judge or writes.

**Undo.** `memory-consolidate::alias-undo { bank, from }` turns the pair's entry `distinct` first, so no pass merges it again, then puts `from` back on each recorded memory that is live, unpinned, still carries `to`, does not already carry `from` (on a memory with both, the merge or a person may have put `to` there, so undo drops neither; it is listed in `skipped` for a hand edit) and holds at most eight handles, with the reason `alias merge undone: <to> -> <from>`. A failed write leaves the rest for a second call. No pass merges `user`, so an entry naming it (written by hand) is refused, and `user` is never taken off or put on a memory.

Not covered: accents (`júlia` / `julia` is 0.33), a name against a full name (`rafael` / `rafael souza`, different word counts), and aliases across two banks.

## The LLM tier (optional)

`llm_assist_enabled` (off by default) adds one `router::complete` judge call per bank after the deterministic pass:

- **Reorder groups** — word-order matches the deterministic pass surfaces report-only ("The deploy runs on Fridays" vs "On Fridays the deploy runs") are merged only when the judge confirms the meaning is identical; role swaps ("Alice manages Bob") stay untouched.
- **Rule promotion** — memories re-observed `promote_corroboration_threshold`+ times are offered as candidates, except a note an agent saved from a turn (confidence `stated` with a `source.turn_id`, pinned or not) and a line the rule's cap moved back (`demoted-rule`); ones the judge classifies as standing instructions land as one-line entries in the bank's auto-managed `learned` rule, fingerprint-deduped, append-only. The judge sees the `learned` lines already in force: a candidate one of them already states is never promoted, and the judge lists its id as `covered`. The ids of promoted and covered memories are kept in the state worker (scope `memory_consolidate`, key `promoted:{bank}`), so a later pass does not offer them again. The write re-reads `learned` right before it and sends that read as `expected`; when the rule changed in between, it merges again, up to three tries, then waits for the next pass with a note. Hand-authored rules are never touched, and the judge can only act on candidates it was offered — it cannot invent targets.

Fail-soft: an unavailable router just skips the tier (named in the report's `notes`, so it never blocks the checkpoint), and a malformed reply is treated as "no merges, no promotions". Either way, the deterministic pass has already completed. Each bank's report counts the tiers' model calls in `llm_calls`, the router's reported usage in `llm_input_tokens` and `llm_output_tokens`, and its cost in `llm_cost_usd` when the router priced the call; past `max_llm_calls_per_run` the tier is `deferred` with a note.

## The semantic tier (on by default)

`semantic_enabled` (on by default; `false` stops this tier's model calls only, the alias judge still calls the model: see above for a pass with none) runs one more tier, last, on a fresh fetch. It costs one `router::complete` call per 8 new memories, capped at `semantic_max_per_run` per bank per pass. Without a router or an embed provider, it stops with a note and never blocks the pass. While memories still lack a vector, it waits with a note (see **Vectors still landing** below). Its calls count in the bank's `llm_calls`, `llm_input_tokens`, `llm_output_tokens` and `llm_cost_usd`, and it is `deferred` with a note once the pass reaches `max_llm_calls_per_run`. A `router::complete` judge looks at each memory written since the last pass, next to its nearest neighbours, and decides:

- **merge**: both state the same fact (paraphrase, another language). The duplicate retires and the survivor absorbs its observations. The survivor must keep every detail; when both are equally complete, the older one survives. A merge the judge gives to an agent's note over a memory that is not one is swapped, so the user's line survives and the note retires into it (the number and operator checks then apply to the swapped pair), and an agent's note merged into another memory adds nothing to it.
- **supersede**: a state change ("moved to", "switched to", a rescheduled date). The outdated record retires in favour of the current one, which is the one stated last.
- **nothing**: distinct facts on one topic (two allergies, two projects). This is the normal answer.

Every change goes through `memory::supersede` and carries the judge's one-sentence `reason` on memory's `memory::on-item-changed` events. How it works:

- **Candidates.** Neighbours come from the raw lexical and raw semantic `memory::recall` arms, interleaved, because a fused rank buries the near-duplicate.
- **Targets.** The judge may only pair a memory with one of its *own* candidates, through short per-batch aliases.
- **Validation.** Before anything is written, every action is checked: pinned records are never retired; an unpinned agent's note never retires a memory that is not one (a merge is swapped, a supersede refused); direction follows `last stated`; a merge may not drop a number the survivor lacks or change an operator or sign; the reason is required; the per-run cap applies.
- **Freshness.** Each record is re-read right before the write, so a record edited since the judge read it drops its action.
- **Failure handling.**
  - Transport, auth or quota errors stop the tier for that bank with a note, and the cursor stays put.
  - An unparseable or truncated reply bisects the batch down to one memory. A memory that still fails alone is listed in `semantic_failed` and skipped once another reply of the pass parses. Until one does, the cursor stays before it and the tier goes on to the next batch; when that batch has failed replies too and none parsed, the tier stops with a note naming the first such memory and the cursor stays put.
  - **Vectors still landing.** Each semantic recall reports `vector_coverage {covered, live}`: how many live memories of the bank have a vector for the current embedding model. A memory without one is missing from the semantic arm, so its twin would pass as "no action". While `covered < live`, the tier defers before any judge call: the cursor stays put and the bank gets the note `semantic tier deferred: N of M memories have no vector for the current embedding model yet; cursor held`. This happens during a re-embed after a model change, an embed outage backlog, or a save seconds before the pass. The next scheduled pass tries again. The first pass at least 30 minutes after the first deferral judges with what exists and notes `semantic tier judged without N of M memories whose vectors have not landed`. A bank with no vector at all for the current model keeps deferring. Any pass that judges clears the wait.
- **Cursor.** Progress is a per-bank cursor in the state worker, keyed by `(updated_at, id)`, so edited or resurrected records are judged again. It advances only over batches that were fully judged and fully applied. While the tier waits for vectors, the same value also holds `waiting_since`, the time of the first deferral in ms.
- **Dry runs** skip this tier with an `unevaluated` note.

Config: `semantic_batch` (8 memories per LLM call), `semantic_candidates` (4 per arm), `semantic_max_per_run` (48 per bank per pass). The model comes from `llm_model`. `router::complete` has no temperature control, so the provider default applies.

**Judge eval.** `tests/judge_golden.rs` runs the real prompt and validator over 18 cases from memory's recall golden set: 8 supersedes, 4 merges and 6 keeps. It reports accuracy and **harmful** actions, which must be 0. With `claude-sonnet-5-5` the result was 18/18 with 0 harmful. Without an engine, `tests/offline_safety.rs` replays the golden merge pairs as supplied decisions through the validator on every `cargo test`. `UPDATE_CONSOLIDATION=1` records the decisions into memory's `tests/recall_quality/consolidation.json`, which memory's recall eval replays in its `hybrid-consolidated` mode: traps go from 2 to 0 and duplicate pollution from 23 to 0.

```bash
III_NAMESPACE=<ns> cargo test --test judge_golden -- --ignored --nocapture
```

## The verify tier (opt-in, off by default)

`verify_enabled` (off by default) runs one more tier per bank, after the semantic tier. For each note it reads the conversation the note came from and asks a `router::complete` judge whether that conversation contradicts the note. A contradicted note is replaced by a corrected one. The old note retires with a `superseded_by` pointer and stays visible with `include_superseded: true`.

- **Transcript.** The tier never calls session-manager. It asks memory for the note's source through `memory::transcript`, which returns the user, assistant and notice lines of that span, redacted, a long line as its head and tail. An extracted note's span runs from the first to the last entry of its extraction run. A note an agent saved from a turn is read as that turn, plus the user message right before it.
- **Order.** Priority notes go first: corrections this tier wrote, `inferred` notes, and `stated` notes saved from a turn. Plain extracted notes follow, with what is left of `verify_max_per_run`. Pinned notes are never checked.
- **Cursor.** Each bank keeps two positions in the state worker (scope `memory_consolidate`, key `verify_cursor:{bank}`): one for priority notes, one for the rest, each keyed by `(updated_at, id)`. It is saved after each note that cost a call, so a crash never pays for a judge call twice. A note edited since (a tag or entity edit included) is checked again. A cursor that cannot be read or decoded stops the tier with a note.
- **Judge reply.** Strict JSON, one of two shapes. Anything else fails to parse.

  ```json
  {"verdict":"supported"}
  {"verdict":"contradicted","quote":"<verbatim transcript text>","corrected":"<the note rewritten>"}
  ```

  The judge is told that silence means `supported`, and that the corrected text changes only what the transcript contradicts.
- **Guards.** A correction is written only when all three hold. Otherwise nothing is written and the note counts in `verify_rejected`.
  - **Text.** The corrected text is not empty, differs from the note after normalization, is at most 200 chars longer than the note, and holds no numeral other than ASCII digits.
  - **Quote.** The normalized quote is at least 8 chars and appears as whole tokens in one user or assistant line. Notice lines and the `role [date]:` prefix never count.
  - **Numbers.** Every number in the corrected text appears in the message text of a user or assistant line, or in the note itself. A line's date does not count. The note counts because extraction writes resolved dates ("on 2026-10-15" for "tomorrow") that no message holds; a correction may keep them, but the judge cannot add a number of its own.

  Normalization is the one duplicates use: lowercase; every char that is not a letter, digit or operator becomes a space, except `-`, `.`, `,`, `:` or `/` between two letters or digits; whitespace collapsed. A number is a whole normalized token with at least one ASCII digit, so "1,500", "1.5", "10:30", "2026-10-15" and "-5" are each one number and none of them vouches for a fragment ("500") or a flipped sign. Number words ("three") are not numbers. The guards are English only.
- **The write.** `memory::correct` takes `expected_memory`, the note as this pass read it, and refuses a note edited since. It appends the new record and the old record's tombstone in one write. The new record keeps the old note's confidence, tags, entities, source and statement date. Its corroboration starts at 0 and it is not pinned. When the corrected text already exists as a live note, the old note retires into it; when it exists only as a retired note, the correction is skipped, so a note never flips back.
- **Stored reason.** `source.correction_reason` is built in code: `the source conversation says: "<quote>"`, with the normalized quote cut to 200 chars. It also rides the `memory::on-item-changed` events. The quote stays on the record after its session is deleted.
- **Skipped and counted.** A skip counts in `verify_skipped` and moves the cursor past the note. A skip is final: the note is not checked again until it is rewritten. These skips cost no judge call:
  - records written before spans were kept, and notes with no session or span;
  - reinforced notes (`corroboration` above 0): later evidence the span does not cover;
  - edited notes, whose text no longer matches their id;
  - a missing session, entry or turn. An extracted span is read along its own branch, so a note from a rewound branch is still checked; `off_path` means the span's first entry is not on its last entry's path. An agent-saved note is walked along the session's current path and is skipped as `turn_not_found` when its turn is not on it;
  - a source over 24,000 chars, a span over 1,000 user and assistant messages, a turn walk over 3,000 user and assistant messages, or a read that could not finish within 10 s (a read is not started without its full 5 s left).

  When `memory::correct` refuses a correction that passed the guards (`changed`, `pinned`, `edited`, `no_change`, `text_exists`, `invalid_text`, `flagged`), the note also counts in `verify_skipped` under that reason and the cursor moves past it, but its judge call was spent and counts in `llm_calls`.

  Skips end as one note per bank, for example `verify: skipped 5 (no recorded span 3, session_not_found 2)`. Nothing goes into `errors`.
- **Caps.** `verify_max_per_run` (20) counts the notes whose transcript was requested, per bank per pass; skips with no call are free. Judge calls count in the bank's `llm_calls`, tokens and cost, and against `max_llm_calls_per_run`. The tier runs last, so when earlier tiers use the cap it is `deferred` with a note. A correction uses the `max_supersedes_per_run` budget and counts in `corrected`, not `superseded`. The model comes from `llm_model`.
- **Report.** Each bank counts `verified` (supported), `corrected`, `verify_skipped` and `verify_rejected`, each left out at zero. The report carries no per-note list and no quotes.
- **Failure handling.** A transport error, session-manager down, or a failed cursor save stops the tier for that bank with a note; the cursor stays before the note. A provider error other than a context overflow counts as transport, so a provider that reports a content refusal as an error holds the tier on that note. A context overflow counts in `verify_rejected`. An unparseable reply holds the cursor while no reply of the pass has parsed, and two of them with none parsed stop the tier.
- **Older memory worker.** A memory worker without `memory::transcript` stops the tier before any model call, with one note: `verify tier skipped: memory worker lacks memory::transcript — upgrade memory`. Release memory first.
- **Dry runs** skip this tier.

Both memory functions are internal, and memory refuses them, like `memory::supersede` and `memory::rule::set`, from inside an agent turn: only this worker calls them, from a pass that carries no turn baggage. A `memory-consolidate::run` started from inside a turn carries that turn's baggage into every memory call it makes, so memory refuses its writes and `memory::transcript` with `forbidden`. Each refusal is a bank error (the verify tier stops there, its cursor where it was), so that pass is not checkpointed and the scheduled pass still runs when due.

## What counts as a duplicate

v1 is deterministic and deliberately conservative: two live memories merge only when their normalized text matches (case, punctuation, and whitespace insensitive). Equal token sets (word-order shuffles) are only reported, and merge only when the LLM tier's judge confirms them. One differing word is NOT a duplicate — "always publishes" and "never publishes" must never merge. Survivor choice is stable: pinned first, then a memory that is not an agent's note (confidence `stated` with a `source.turn_id`), then highest corroboration, then the oldest record (it carries the original provenance). A pinned agent's note counts as the user's: the pin is the user's own act. Semantic near-duplicate merging is a later, LLM-assisted tier.

Normalization keeps operators and signs (`<`, `<=`, `!=`, `+`, `#`, `-5`) and any `-`, `.`, `,`, `:` or `/` between two letters or digits, so "timeout > 5s" and "timeout < 5s", "C++" and "C#", or "0.05" and "005" never merge.

## Boundaries

- Never touches memory's files; every mutation goes through `memory::update` for an alias retag, `memory::supersede` (with `reinforce` on merges, so the survivor absorbs the duplicate's observations in the same call; never from an agent's note, which is not the user saying it again, nor from a `demoted-rule` line, so its count cannot carry a twin back into the rule), or `memory::correct` for the verify tier — the same append-only, last-wins contract as everything else. Every change carries a `reason` on its `memory::on-item-changed` events.
- Pinned memories are never superseded, even inside a duplicate group, and never retagged.
- Every `memory::supersede` carries `expected_memory` and `expected_superseded_by`, both records as this pass read them, so a memory worker that knows these guards refuses a record edited since; the pass reports the error and stays due. An older memory worker ignores the fields.
- A memory worker whose `memory::recall` does not report `vector_coverage` is judged as before, with one note per bank: `memory worker did not report vector coverage; semantic tier judged without knowing it; upgrade memory`.
- Every `memory::rule::set` of `learned` carries `expected`, the rule as this pass last read it, so the memory worker refuses it with `rule_changed` when the rule changed since. An older memory worker ignores `expected` and writes unguarded.
- Not extraction (the memory worker captures), not recall, not rule management. One job: keep banks clean.

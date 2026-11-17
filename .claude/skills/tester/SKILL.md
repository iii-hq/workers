---
name: tester
description: QA pass over the ADE and its worker pages as a hard-to-please tester — real flows end to end, several chats and panes at once, what actually goes over the wire, reloads, second tabs, slow and failing workers — then a repro, a fix, a regression test and a before/after run for every bug. Use when asked to "test everything", "be my tester", "check UX and functions", "find weird bugs", before pushing a PR, or after a bug report like "card answers go to the wrong chat".
---

# Tester

A screen that looks right can still do the wrong thing. On 2026-11-16 a
question card's answer landed in a *different* chat whenever two chats were
open: every single-chat test passed, and the UI showed nothing wrong until
the other agent replied "I didn't ask that". This skill hunts for that kind
of bug: the right control doing the wrong thing, or the right thing in the
wrong place.

For visual states (hover, focus, themes, phone px), run `ui-sweep` as well.
This skill is about **behaviour**.

## 0. Ground rules on the person's stack

The usual stack holds the person's real chats and agents, so be careful:

- **Never let a test message reach an agent.** Before `goto`, capture and
  drop sends with `captureCalls(page, ['harness::send', ...])` from
  `scripts/lib.mjs`. Add any other mutating function the flow could call:
  `harness::ask::settle`, approvals, `kanban::*` writes, `compose::*`.
  Afterwards, check that nothing landed:
  `grep -c '<marker text>' <stack>/data/session-manager/<session>.jsonl`.
- **The workspace layout is shared state.** Panes and workspace tabs are
  saved on the server, so a stray split shows up in the person's own
  browser. Take `snapshotLayout` first and finish with `restoreLayout` in a
  `finally`. Open chats with their `open <title>` button, never by clicking
  the title text: that can hit the row's hover pin button and pin the chat.
- Never click Create, Save, Publish, Delete, Merge or Push on real data. If
  a flow needs one, use a throwaway stack (`.context/ade-stack/up.sh`, see
  `ui-sweep` §0), or stop at the dialog and screenshot it.
- Serve this branch's worker UIs through `III_WORKER_UI_OVERRIDES`
  (`ui-sweep` §0). A rebuilt `<worker>/ui` that is missing from the
  override list still shows the old bundle.

Setup, from the repo root:

```sh
(cd harness/ui && pnpm build)   # and every other worker UI you changed
rm -rf ade/web/node_modules/.cache/tester && cp -R .claude/skills/tester/scripts ade/web/node_modules/.cache/tester
cd ade/web && node node_modules/.cache/tester/chat-routing.mjs   # TESTER_URL=… for another port
```

Screenshots go to `.context/tester/`.

## 1. Find the surfaces

Don't test from memory. List everything that acts on something else:

```sh
grep -rn "chat?\?\.compose\|chat\.compose\|startChat\|openDraft\|sendDraftWhenReady(" \
  --include='*.ts' --include='*.tsx' . | grep -v node_modules | grep -v '\.test\.'
grep -rn "iii\.call(\|\.call('" --include='*.tsx' <worker>/ui/src | grep -v test
```

Sort each hit by **where it lives** and **what it targets**:

| Lives in | Must target | Bug when it uses |
| --- | --- | --- |
| A card in one chat's feed (ask card, function-call renderer) | that chat's session | the chat used last (`compose` without `sessionId`) |
| A composer control or action | the composer it sits in | the chat used last |
| A page beside chats (IDE ⌘L, browser pick, kanban, sentinel) | the chat the person means; ask when several are open (`ifUnambiguous`) | a silent guess |
| A session chip in a chat header | that header's session | global state |

Known compose callers to re-check whenever one of these changes:
`harness/ui` ask card (fixed: `sessionId`), `voice/ui` chip, composer
action and mic, `ide/ui` EditorPane/quick-edit/CommitBox, `browser/ui`
SessionView/SavedSets/Downloads, `kanban/ui` ticket, `sentinel/ui` detail,
`onboarding/ui` use-tour, `eval/ui` monitor detail, and `ade/ui` FunctionsPage.

## 2. The scenarios

Run every flow you touched through each scenario, not only the happy path.

**Many at once.** These are where the weird bugs hide.
- Two chats side by side (`splitRight(page, 'chat')`). Focus chat B's
  composer, then act in chat A: a card answer, an approval, dictation,
  attach, a slash command. Check which session the captured call names.
- The same chat open in two panes, and in two browser tabs: answer in one,
  then check that the other closes its card and shows no second Send.
- A chat next to the IDE or browser: ⌘L, Copy Reference, picks. With two
  chats open, it must ask or say "ambiguous", never guess.
- A sub-agent chat next to its parent: who receives what.

**Timing.**
- Act while the agent is streaming: is it queued, held, or dropped? Is it
  shown as queued in the right chat's strip?
- Act while the composer holds a draft: the draft must survive, and the
  action goes on its own (the held-send path).
- Double-click Send and press Enter twice fast: exactly one call.
- Switch chats in the middle of a send, and close the pane in the middle of
  a send.
- Use a slow worker: `page.route` or `routeWebSocket` can delay the reply by
  6 s, past the 5 s `CALL_TIMEOUT_MS`.

**Memory.**
- Reload in the middle of every flow: is the card still open or settled
  correctly? Did the draft and pane memory come back (`state-persistence`
  probes)?
- A second tab, then visibility change: a settled card closes.
- Did the sidebar status dot (working, awaiting answer, error) match the
  harness after each step? Reload and compare.

**Failure.**
- Stop the worker (`compose::stop` on a throwaway stack, or make
  `routeWebSocket` answer with an error): does the page say so, keep the
  input, and recover when the worker returns?
- The function is missing (an older worker): the feature-detect path.
- Empty data, one item, 500 items, a 2,000-character title, emoji and RTL.

**Keyboard and focus.** Do the whole flow with no mouse. Escape closes the
top layer only. Focus returns to the trigger. ⌘K, ⌘L and ⌃] work inside
panes. Check the order focus moves through panes.

**Every page.** Do one real action on each worker page (state, cron,
sentinel, kanban, database, browser, ide, iii-directory, voice, traces,
workers, functions, observers). `watchProblems` must come back empty: no
`pageerror`, no `console.error`, no 5xx, no failed request that the UI
doesn't explain.

## 3. Per bug: repro, fix, prove

1. **Repro on the wire, not just the screen.** Capture the payload (the
   `session_id`, the `function_id`). A UI-only check passes while the call
   goes elsewhere.
2. **Show "before".** Run the check against the old behaviour so it fails:
   `patchBundle(page, '<scope>', js => js.replace(...))` takes the fix out
   of the served bundle, or check out the parent commit. A check that never
   failed proves nothing.
3. **Find the root cause** (`systematic-debugging`). Fix it in the shared
   layer (`lib/composer-insert`, `ui-loader` host API, the console-ui
   types) so every caller benefits, then fix the callers.
4. **Write a unit test that fails without the fix.** The `ui-loader.test.tsx`
   `host.chat.compose` tests are the pattern.
5. **Show "after"** with the same script. Save before/after screenshots.
6. Typecheck (`pnpm exec tsc -b --noEmit` in `ade/web`, `tsc --noEmit` in
   the worker UI), `pnpm build` the worker UI (strict lint), and run vitest.
   Before calling a failure pre-existing, confirm it touches none of your
   files.
7. Commit locally. Don't push until the person says so.

Add a script to `scripts/` for each new class of bug, the way
`chat-routing.mjs` was added, so the next pass runs it for free.

## 4. Report

One table: severity, flow, scenario, expected, actual (with the captured
payload), cause (file:line), fix commit, test, before/after paths. Then:
- what was **not** covered, and why (needs a real send, worker not in this
  repo, no data);
- side effects on the person's stack (layout restored? a status changed?);
- suspects you saw but did not fix.

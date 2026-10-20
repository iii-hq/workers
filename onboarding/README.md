# onboarding

The guided console tour. The page and its tab are called **learn-iii**; a
"tour" is one unit of content inside it, so a second tour can be added
without renaming anything.

Three parts:

| Part | Lives in | Owns |
| --- | --- | --- |
| Content | `src/tours.mjs` | which tours exist, their steps, and the console element each step points at |
| Progress | the `state` worker, scope `learn-iii` | how far each operator got, so a reload resumes mid-tour |
| Rendering | `ui/page.tsx` (injected into the console) | the tour card, the spotlight box, and the page an older console falls back to |

## How it shows

On a console with the `overlays` slot the tour is a card floating in a corner
of the window (`ui/src/overlay.tsx`): one step at a time, Back and Next across
the steps already reached, a bar with a segment per step. It is a non-modal
dialog that never takes focus, and no pane opens for it, so following the tour
leaves the workspace being taught as the operator arranged it. The card moves
to another corner only when the spotlight would land under it, folds to a
pill, and closes until it is asked for again; which of those it is lives in
this browser's storage. Closing only stops the drawing: the tour, and the
triggers its steps wait on, run for as long as the worker's script is
loaded, so a step the agent finishes meanwhile is closed when the card comes
back. A step that waits on a `state` key also reads the key once when it is
bound, so one finished while no console was open closes on the next load
(Restart deletes those keys first). When the step being read closes, the
reader moves on; a step closing while they read another leaves them where
they are. A workspace saved while the tour was a pane has that
pane closed on load, and the card comes up in its place.

On a console without the slot the same tour renders as a page in a pane
(`ui/src/page.tsx`). Both run on `ui/src/use-tour.ts`.

`Open learn-iii` (the palette row, `learn-iii.open`) shows it either way.
The console runs that command by itself once the operator has had the ADE
for five minutes: the ADE records its first open (`console::onboarding::get`,
`setup_ended_at`) and the console's tour gate counts from there, across
reloads and restarts, waits for the setup wizard to close, and opens the tour
once (`tour_opened_at`). The wizard's Ready step does not start it.

The first step's button starts a new chat under the Onboarding agent profile
(`ade-worker-builder`, which the harness template ships) and sends "hello"
there, the way the wizard's example prompts start a chat
(`host.chat.startChat`); every later prompt goes to the chat the operator is
in. A step about a console screen (Traces) opens it beside the chat with its
own button, and moves on once the screen is there.

## Functions

- `learn-iii::tours::list` — every tour in curriculum order.
- `learn-iii::tours::get` — one tour with all of its steps.
- `learn-iii::progress::get` — the status of every step per tour, with the trigger evidence that closed it, plus the next tour to offer.
- `learn-iii::steps::complete` — mark one step complete. Pass `fired` when a trigger closed it; its type and payload are kept as evidence. The first time a step closes, it is also announced on the `onboarding:steps:complete` topic (see Events).
- `learn-iii::steps::reset` — forget one tour and start it again.
- `learn-iii::subscribe` — add an email address to the iii product-update list (the last step's signup box; the POST happens here, never in the browser).

Progress is written with `state::update` and its ordered atomic ops, never
`state::get` then `state::set`: two invocations can interleave around an
await, and a read-then-replace write would drop the earlier one. `merge`
walks and creates the path it names, so one step lands under its own tour.
A reset nulls its tour (the op set reaches top-level keys only for `remove`)
and the read drops nulls on the way out.

## Events

Each closed step is published to the `onboarding:steps:complete` topic through the
`queue` worker, so anything that follows a tour subscribes instead of reading
this worker's state. The payload:

```json
{
  "tour_id": "learn-iii",
  "tour_title": "Find your way around",
  "step_number": 3,
  "step_id": "coder",
  "step_title": "CODER"
}
```

The step number is the 1-based position in the tour, read from the ordered
step list. A step announces itself once per subject: a reload, or a second
report from the agent, writes the same progress again but publishes nothing.

Delivery is durable (`iii::durable::publish`, and subscribers bind the
`durable:subscriber` trigger type): the message waits in the queue and is
retried until a subscriber takes it, instead of being dropped when nothing is
listening at that instant. The publish itself is best effort — a project
without the `queue` worker logs the failure and still closes the step.

## Conditions

A step can wait on a real engine trigger instead of a button. The page binds
it the way the console's own live pages do — a per-tab function id, then
`registerTrigger` pointed at it — so what the operator sees is the payload the
engine delivered, not a simulation.

The condition is its own row under its step, and stays there once it fires:
the trigger type, its state, and the time it fired. Expanding it shows the
binding config and the payload, the way the harness shows a function call.
The row opens when the operator opens it: a trigger that fires does not throw
its payload over whatever is being read.

The tour ships two kinds. The first step waits on `harness::turn-completed`,
so it closes when the agent answers. The four steps whose work the AGENT does
— Composability, Discoverability, Extensibility, Reactivity — each wait on a
`state` trigger bound to one key, `step_<id>_completed`, in the `learn-iii`
scope; their prompts end by telling the agent to write it as it ends its turn.
The end of the turn is the boundary, not the agent's own sense of being
finished — asked for the latter it reports while it is still working. The
trigger matches the key, not the value: whatever the agent writes there, the
write is the report.

That is what keeps the open step and the step being worked on the same step. A
long build used to read as finished the moment its prompt was sent, because
the button closed the step. The page binds every reachable condition when the
step is displayed, before any button is clicked, so the wait is already
running when the prompt goes out.

## Anchors

A step's `anchors` are selectors for the console element it talks about, best
first. The first is always a class the console carries **for this tour**: `learn-iii-menu-bar`, `learn-iii-tabs`, `learn-iii-palette`,
`learn-iii-conversations`, `learn-iii-composer`, `learn-iii-traces`,
`learn-iii-new-tab`, `learn-iii-workspaces` (the tabs and the `+`
together). Each
one sits in `ade/web/src` beside a short comment that says it is a tour
anchor. `pnpm test` fails if a step points at a class the console no longer
has. The selectors after the first are the console's own stable hooks, so the
box still lands on a console build older than the anchor classes.

The spotlight is one fixed box in `document.body` that follows the anchor's
rectangle, animated with CSS transitions on the console's motion tokens (see
`ui/src/spotlight.ts`). No animation library: it is one moving rectangle. It
carries a breathing halo and a label naming the step (`6 · Observability`),
so a lit element reads at a glance without the card. The label takes the
first side — above, beside, under — that fits and covers no control; when
none does it stays off, since the card names the step anyway.

## Styling

The page is styled with the console's OWN CSS — its utility classes and the
`uiClasses` recipes (`card`, `listItem`, `chip`) — so it inherits the house
spacing, edges and hover states, and follows the theme. `ui/styles.css` holds
only what those cannot express: the spotlight box, the status dot, the
progress bar, the caret, and the payload block. The markup stays hand-rolled
rather than built from shared components, so the page renders on a console
build older than a component it would otherwise import.

## The steps

The tour says that this is an engine with workers, not a chat app with
plugins, and the steps together cover CODER: **Composable** (the agent adds a
kanban board, namespaced `todo-local` with `KANBAN_NAMESPACE`, so it sits apart
from any board the operator runs), **Observable** (the traces
step, anchored on the console's traces screen), **Discoverable** (one engine,
one palette), **Extensible** (the agent builds a TODO worker on that board: a
sidebar-like page beside it, with a due date, a due time, a priority, a repeat
and an alarm per item, shown as pills on the cards; Done moves the ticket on
the board at once; alarms run through the `cron` worker), **Reactive** (three
items, and a trigger that fires when the operator checks one off). The recipe
for the two build steps is `skills/SKILL.md`, which lands in the project as
`skills/onboarding/SKILL.md`; the prompts point the agent at it. The last step, "Stay in
touch", takes an email address for the roadmap and product updates, with icon
links to Discord, GitHub, X and LinkedIn, and a link to the docs. After it,
"Clean up" asks the agent to remove this worker, and the card goes with it —
the same composition move as adding one, run backwards. It carries no
condition: the worker that would report the step done is the worker being
removed.

The Observability step opens Traces beside the chat with its **Open traces**
button (`console::workspace::open`, `relative_to: chat`). A fresh workspace
already has it there, and the step frames it; while it is gone, the step's
`to_open` also points at where it opens by hand — the new-tab `+`, then the
`traces` row of the new tab's picker (on a phone, its search button) — and
the step moves on by itself once it is there. Open only in a tab that is not
showing, the step says so and lights that tab. Traces then opens on the
latest turn of the running chat (else the active one) by itself.

## Build

```sh
pnpm install          # worker deps (iii-sdk)
pnpm build            # ui/dist assets, then dist/bundle
pnpm test
```

`pnpm build` (= `build:bundle`, what the release runs) first runs `build:ui`:
from the repo root it installs `ui`'s slice of the shared console-UI workspace
and builds `ui/dist` with the same builder every worker UI uses. It then
writes the publishable shape: `dist/bundle/index.mjs` with the worker and its
dependencies inlined, and the two page assets beside it.
`learn-iii::ui-content` reads whichever layout has the page — beside the
bundle, or `ui/dist` in a checkout.

`III_LEARN_III_UI_WATCH=1` re-registers a changed asset, which hot-swaps the
page in every open console tab. `III_LEARN_III_UI_DIR` overrides where the
assets are read from.

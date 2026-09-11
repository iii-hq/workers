# onboarding

The guided console tour. The page and its tab are called **onboarding**; a
"tour" is one unit of content inside it, so a second tour can be added
without renaming anything.

Three parts:

| Part | Lives in | Owns |
| --- | --- | --- |
| Content | `src/tours.mjs` | which tours exist, their steps, and the console element each step points at |
| Progress | the `state` worker, scope `onboarding` | how far each operator got, so a reload resumes mid-tour |
| Rendering | `ui/page.tsx` (injected into the console) | the step list and the spotlight box |

## Functions

- `onboarding::tours::list` — every tour in curriculum order.
- `onboarding::tours::get` — one tour with all of its steps.
- `onboarding::progress::get` — the status of every step per tour, with the trigger evidence that closed it, plus the next tour to offer.
- `onboarding::steps::complete` — mark one step complete. Pass `fired` when a trigger closed it; its type and payload are kept as evidence.
- `onboarding::steps::reset` — forget one tour and start it again.
- `onboarding::subscribe` — add an email address to the iii product-update list (the last step's signup box; the POST happens here, never in the browser).

Progress is written with `state::update` and its ordered atomic ops, never
`state::get` then `state::set`: two invocations can interleave around an
await, and a read-then-replace write would drop the earlier one. `merge`
walks and creates the path it names, so one step lands under its own tour.
A reset nulls its tour (the op set reaches top-level keys only for `remove`)
and the read drops nulls on the way out.

## Conditions

A step can wait on a real engine trigger instead of a button. The page binds
it the way the console's own live pages do — a per-tab function id, then
`registerTrigger` pointed at it — so what the operator sees is the payload the
engine delivered, not a simulation.

The condition is its own row under its step, and stays there once it fires:
the trigger type, its state, and the time it fired. Expanding it shows the
binding config and the payload, the way the harness shows a function call. A
trigger that fires opens its row, so the payload arrives in view.

The tour ships two: `harness::turn-completed` (send a message) and `state`
scoped to `tour-scratch` (write a value and watch the trigger fire). The
second also carries a `prompt` — a sample message, with a copy control, that
makes the same trigger fire through the agent instead of the shell.

## Anchors

A step's `anchors` are selectors for the console element it talks about, best
first. The first is always a class the console carries **for this tour**: `onboarding-menu-bar`, `onboarding-tabs`, `onboarding-palette`,
`onboarding-conversations`, `onboarding-composer`, `onboarding-traces`. Each
one sits in `console/web/src` beside a short comment that says it is a tour
anchor. `pnpm test` fails if a step points at a class the console no longer
has. The selectors after the first are the console's own stable hooks, so the
box still lands on a console build older than the anchor classes.

The spotlight is one fixed box in `document.body` that follows the anchor's
rectangle, animated with CSS transitions on the console's motion tokens (see
`ui/src/spotlight.ts`). No animation library: it is one moving rectangle.

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
plugins, and the steps together cover CODER: **Composable** (the harness is a
worker calling functions other workers register), **Observable** (the traces
step, anchored on the console's traces screen), **Discoverable** (one engine,
one palette), **Extensible** (this page is a worker's page; install more from
the package repo), **Reactive** (the trigger step). The last step, "Stay in
touch", takes an email address for the roadmap and product updates, with icon
links to Discord, GitHub, X and LinkedIn, and a link to the docs.

## Build

```sh
pnpm install          # worker deps (iii-sdk)
pnpm --dir ui install # UI deps, from the shared console-UI workspace
pnpm build            # ui/dist assets, then dist/bundle
pnpm test
```

`pnpm build:assets` writes `ui/dist/{page.js,styles.css}` with esbuild alone —
no pnpm workspace and no `tsc` — which is what `scripts.install` runs inside
the worker's VM. `pnpm build:bundle` writes the publishable shape:
`dist/bundle/index.mjs` with the worker and its dependencies inlined, and the
two page assets beside it. `onboarding::ui-content` reads whichever layout has
the page — beside the bundle, or `ui/dist` in a checkout.

`III_ONBOARDING_UI_WATCH=1` re-registers a changed asset, which hot-swaps the
page in every open console tab. `III_ONBOARDING_UI_DIR` overrides where the
assets are read from.

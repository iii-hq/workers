# onboarding

The guided console tour. Three parts:

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

## Conditions

A step can wait on a real engine trigger instead of a button. The page binds
it the way the console's own live pages do — a per-tab function id, then
`registerTrigger` pointed at it — so what the operator sees is the payload the
engine delivered, not a simulation. The card shows the trigger type, its
binding config, and the payload, the way the harness shows a function call.

The tour ships two: `harness::turn-completed` (send a message) and `state`
scoped to `tour-scratch` (write a value and watch the trigger fire).

## Anchors

A step's `anchors` are selectors for the console element it talks about, best
first. The first is always a class the console carries **for this tour**: `onboarding-menu-bar`, `onboarding-tabs`, `onboarding-palette`,
`onboarding-conversations`, `onboarding-composer`, `onboarding-settings`. Each
one sits in `console/web/src` beside a short comment that says it is a tour
anchor. `pnpm test` fails if a step points at a class the console no longer
has. The selectors after the first are the console's own stable hooks, so the
box still lands on a console build older than the anchor classes.

The spotlight is one fixed box in `document.body` that follows the anchor's
rectangle, animated with CSS transitions on the console's motion tokens (see
`ui/src/spotlight.ts`). No animation library: it is one moving rectangle.

## Build

```sh
pnpm install          # worker deps (iii-sdk)
pnpm --dir ui install # UI deps, from the shared console-UI workspace
pnpm build            # writes ui/dist/{page.js,styles.css}
pnpm test
```

`III_ONBOARDING_UI_WATCH=1` re-registers a changed asset, which hot-swaps the
page in every open console tab. `III_ONBOARDING_UI_DIR` overrides where the
assets are read from.

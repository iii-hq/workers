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
- `onboarding::progress::get` — furthest step per tour, plus the next tour to offer.
- `onboarding::progress::set` — record the step an operator is on.

## Anchors

A step's `anchor` is a CSS selector for a class the console carries **for this
tour**: `onboarding-menu-bar`, `onboarding-tabs`, `onboarding-palette`,
`onboarding-conversations`, `onboarding-composer`, `onboarding-settings`. Each
one sits in `console/web/src` beside a short comment that says it is a tour
anchor. `pnpm test` fails if a step points at a class the console no longer
has.

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

---
title: react
description: Modern React for application code — the render/state/effect mental model, state placement, effect discipline, composition and Suspense, transitions, and measured performance work.
type: how-to
---

# React

Docs <https://react.dev/learn> · reference <https://react.dev/reference/react>. Read the
project's installed version's docs when the two disagree — the API surface has moved
(React 19 in particular).

## The mental model, in three sentences

1. **Render is pure and repeatable.** Given the same props and state it produces the same JSX;
   it must not call functions, mutate things, or start network work.
2. **State is a snapshot per render.** Setting it schedules a new render; the variable you
   just read is the *old* value for the rest of this one.
3. **Effects synchronize with external systems** — sockets, DOM APIs, subscriptions, timers.
   They are not the place for derived data, event responses, or initialization.

## Rules that catch most bugs

- Hooks at the top level of a component or hook, unconditional. No hooks in loops, conditions,
  or after an early `return`.
- **Keys are identity.** `key={index}` on a list that reorders or filters corrupts state.
  Use a stable id from the data.
- **StrictMode double-invokes render and remounts effects in dev.** That is a feature: it
  exposes missing cleanup and non-idempotent effects. Fix the code; do not remove StrictMode.
- **Never mutate props or state.** Copy, then set.
- **Do not lie about dependencies.** A stale-closure bug is a missing dep or a non-memoized
  function. Fix the dependency, do not silence the lint rule.

## State placement

- **Compute what you can; do not store it.** If it is derivable from props or state, derive it
  during render. Mirroring creates a second source of truth that drifts.
- **Colocate, then lift.** Keep state in the closest component that needs it; lift only when
  two siblings must agree, and lift to their **closest common parent**, not to the top of
  the app.
- **One state variable per concern.** `useReducer` when several fields change together or the
  next value depends on the previous (`setCount(c => c + 1)` — batching means the object form
  is often wrong in a burst of events).
- **Reset state by changing `key`, not by an effect.** `<Form key={userId} />` is the correct
  "reset on entity change" and also resets every child.
- **Server data is not React state.** Cache it (TanStack Query or equivalent); keeping a copy
  in `useState` guarantees stale UI and double-loading.
- **URL is state.** Filters, tabs, pagination, and selection belong in search params so they
  survive reload, back/forward, and sharing.

## Effects

```tsx
useEffect(() => {
  const ac = new AbortController()
  let active = true
  void (async () => {
    const data = await fetchThing(id, { signal: ac.signal })
    if (active) setData(data)
  })()
  return () => { active = false; ac.abort() }
}, [id])
```

- **Every effect that starts something must clean it up.** Timers, listeners, subscriptions,
  sockets, in-flight requests.
- **Guard against races** (`active` flag, `AbortController`) whenever the dep can change
  mid-flight — otherwise a slow response overwrites a fast one.
- `useLayoutEffect` only when you must read or write layout before paint (measuring, scroll
  restoration). It blocks paint; prefer `useEffect`.
- **Effects are the wrong tool for:** responding to a click, deriving state, fetching as a
  general pattern when a data layer exists, or reacting to a prop you could compute from.

## Refs, context, and escape hatches

- Refs hold values that do not affect rendering (timers, DOM nodes, previous values). Do not
  read or write them during render — only in effects and handlers.
- `useImperativeHandle` when a parent genuinely needs a handle on a child's behavior.
- **Context is for low-frequency values** (theme, current user, the iii client). Split it: a
  context whose value object is rebuilt every render re-renders every consumer. Pair a reducer
  with a context and expose `state` and `dispatch` in two separate contexts.
- `useSyncExternalStore` for subscribing to a store outside React (including the iii client's
  connection state) — it is the correct primitive, and hand-rolled subscription effects get it
  wrong under concurrent rendering.

## Composition

- **`children` as a prop** breaks prop-drilling without context.
- **Slots** (`<Card header={...} footer={...} />`) beat boolean flags that switch internal
  branches into a second component.
- **Compound components** (`<Tabs>`, `<Tabs.List>`, `<Tabs.Panel>`) share state through
  context; the parent owns the invariant.
- **A component with 6+ boolean props is 2^6 configurations nobody tests.** Split on the axis
  that actually varies.
- **Hoist state to the leaf that needs it**, and keep lists virtualized when they exceed a few
  hundred rows.

## Suspense, transitions, errors

- `<Suspense fallback>` + `React.lazy` for route-level code splitting; the fallback is a real
  UI state, not a spinner on a blank screen.
- `useTransition` / `startTransition` for updates that may be expensive (tab switches,
  filtered lists) so typing stays responsive. `useDeferredValue` when a *value* is the slow
  part (a query fed to a heavy list).
- **Error boundaries are still the mechanism** for render errors — a class component or
  `react-error-boundary`. An async rejection is not caught by a boundary unless you rethrow it
  in render or route it through the data layer.
- **`use()`** (React 19) unwraps a promise or context in render, but only inside a Suspense
  boundary.

React 19 notes: refs are props (`<input ref={r} />`, no `forwardRef`), `<Context>` is itself
usable as the provider, `useActionState` / `useOptimistic` / `<form action>` cover form
pending and optimistic UI, and document metadata (`<title>`, `<meta>`) can render inline.
Check the installed version before using any of these.

## Performance: measure, do not decorate

Order of leverage:

1. **Do less work.** Ship fewer/smaller deps, split routes, virtualize long lists, paginate.
2. **Stop unnecessary re-renders** — but only after profiling. React DevTools' Profiler shows
   which component re-rendered and why.
3. **Then memoize.** `memo`, `useMemo`, `useCallback` cost memory and comparison time; they are
   net-negative on cheap components with stable inputs and net-positive on expensive subtrees
   fed by unstable objects.
4. **Never memoize to fix a bug.** A correct dependency array is the fix; `useMemo` is only an
   optimization.

- Keep the tree shallow; a context value changing high up re-renders the world.
- Split a hot component so the state change lands low in the tree.
- Long lists: virtualize (TanStack Virtual, `react-window`) — rendering 5000 rows is the cost,
  not the state.
- Avoid layout thrash: batch DOM reads and writes; never read `getBoundingClientRect()` in the
  same frame as a write loop.

## Accessibility is part of the component

Semantic elements before ARIA, a real `<button>` before a clickable `<div>`, labels bound to
inputs, focus moved deliberately on route and dialog changes, and no interaction that only
works with a mouse. It is far cheaper to build in than to retrofit.

## Anti-patterns to refuse

- Deriving state in an effect and storing it (`useEffect(() => setX(a + b), [a, b])`).
- Syncing props to state with an effect.
- One giant context holding a mutable object.
- Fetching in an effect when a data layer exists.
- `useMemo` everywhere as a habit.
- Index keys on sortable/filterable lists.
- Suppressing the exhaustive-deps lint rule instead of fixing the dependency.
- Copying a library's internal markup instead of the library.

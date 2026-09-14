---
name: frontend
description: >-
  The Frontend Engineer's stack for a browser application on iii: the
  iii-browser-sdk client, React, Vite, TanStack Router and Query, and the
  accessibility, performance and testing bars every screen is held to.
---

# frontend

These documents describe a standalone browser application (Vite + React,
routed with TanStack Router, server state in TanStack Query, live engine data
over `iii-browser-sdk`). They are not about UI injected into the ADE console;
that is the `ade-worker-design` skill and the iii ADE Worker Designer's work.

- `iii-browser-sdk` — make the app an iii worker: `registerWorker`,
  `registerFunction`, `trigger`, `registerTrigger`, state and channels. The
  installed `.d.mts` is the contract.
- `react` — the render, state and effect model; state placement; composition,
  Suspense and transitions.
- `vite` — config, modes and env, dev proxy, pre-bundling, code splitting,
  production hygiene.
- `tanstack-router` — file-based routes, loaders, validated search params,
  pending and error states.
- `tanstack-query` — query keys, staleness, mutations and optimistic updates,
  invalidation, pagination.
- `web-accessibility` — WCAG 2.2 AA: semantics, keyboard and focus, ARIA only
  when needed, forms and live regions.
- `web-performance` — Core Web Vitals, budgets, loading strategy, INP, layout
  stability, caching.
- `frontend-testing` — Vitest and Testing Library for behaviour, MSW at the
  network boundary, Playwright for the critical journeys.

Fetch one with `directory::skills::get { "id": "kanban/frontend/<name>" }`.

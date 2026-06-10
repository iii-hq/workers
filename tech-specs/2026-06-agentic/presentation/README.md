# agentic workers — architecture overview

An interactive, infographic-style presentation of the
[2026-06 agentic spec](../README.md) for technical stakeholders: the five
workers, how they communicate, sequence walkthroughs, sub-agent spawning,
durable execution, governance, and onboarding — with the technical depth
tucked into expandable datasheets.

Built with Vite + React 19 + Tailwind v4, styled with the
[iii Schematic design system](../../../console/DESIGN.md).

## Run it

```bash
pnpm install
pnpm dev        # local presentation at http://localhost:5173
```

## Ship it

```bash
pnpm build      # static site in dist/ — host anywhere, no backend
pnpm preview    # sanity-check the production build locally
```

## Structure

| Route | Content |
|---|---|
| `#/` | the main scroll story: hero → system map → anatomy of a turn → the white box → reactive surface → durable execution → sub-agents → governance → use cases → onboarding → why it holds |
| `#/use-cases/telegram` | telegram bot walkthrough (sequence diagram, bindings, kv schema) |
| `#/use-cases/console` | console chat walkthrough (reactive bindings, approvals, traces) |
| `#/use-cases/loops` | agentic loops: bind any event to a goal, typed results, chains |

Every diagram is hand-built SVG driven by small data arrays in
`src/sections/` and `src/components/diagrams/` — no chart library. Worker
datasheets live in `src/data/workers.ts`, condensed from the spec documents.

The light/dark toggle persists to `localStorage` (`iii-theme`), matching the
console's convention.

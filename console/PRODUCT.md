# Product

## Register

product

## Users

Two audiences share the console equally:

- **Engine developers** building and testing iii workers, agent flows, and function registrations on a local or staging engine. They need to invoke `@` functions, switch models, approve tool calls, and confirm behavior before shipping.
- **Operators and SREs** debugging live or near-live sessions: tracing spans, following critical paths, filtering by service or session, and correlating chat activity with OpenTelemetry output.

Both use the same binary on one port. Context is usually a desk with a large monitor, long sessions, and the engine running on localhost or a reachable cluster. The UI is a work surface, not a marketing moment.

## Product Purpose

Console is the local control panel for the [iii](https://github.com/iii-hq) engine: agentic chat, trace exploration, worktree oversight for parallel agents, and provider configuration in a single embedded SPA served by one Rust binary. Surfaces backed by optional workers (like the worktree graph) appear only while their worker is connected, so the UI never advertises functions the engine does not have.

Success looks like an operator who can steer an agent, watch function calls resolve, jump from a conversation session into the trace explorer, see which agent owns which checkout and whether its branch landed, and configure AI providers without leaving the browser or fighting CORS. The product exists to make the engine legible: what ran, why it failed, and what to do next.

## Brand Personality

**Instrument panel.** Calm, precise, nothing decorative. Like reading a schematic or an engineering document, not a consumer chat app.

Voice is monospace, lowercase labels, explicit state (running / pending / error), and warm cream-and-ink warmth without softness. Confidence comes from clarity and density, not from gradients, mascots, or empty hero space. The iii Schematic design system (see DESIGN.md) encodes this: blueprint aesthetic, single vivid accent, zero border-radius by default.

Three words: **precise**, **legible**, **warm**.

## Anti-references

Do not drift toward:

- **Generic AI SaaS** — purple gradients, glass cards, hero metrics, Inter everywhere, rounded pill UI
- **Observability cliché** — dark navy + teal defaults, colored side-stripe alerts, Datadog-style chrome overload
- **ChatGPT clone** — centered bubble chat, soft shadows, consumer messaging patterns
- **Category reflex** — "AI tool" or "traces dashboard" should not predict the palette or layout; the schematic identity should be recognizable on its own

Avoid modals when inline or progressive disclosure works. Avoid decorative motion. Avoid nested card grids that repeat icon + heading + blurb.

## Design Principles

1. **Instrument over interface** — Every element on the schematic earns its place. Prefer density, labels, and state badges over decoration and empty padding.
2. **Dev and ops parity** — Building a worker and debugging a failed trace are equally first-class journeys; neither audience is secondary.
3. **Live truth** — Catalogs, traces, and chat stream from the engine. Static fallbacks exist only when the engine is unreachable, and they read as degraded, not default.
4. **Show the mechanism** — Sessions, spans, function calls, approvals, and context usage stay visible. Hide complexity in collapsible detail, not in opaque summaries.
5. **Keyboard-native power** — Numbered shortcuts, arrow navigation, focus rings, and live regions for streaming. Long-session operators should not reach for the mouse for routine actions.

## Accessibility & Inclusion

- Target **WCAG 2.1 AA** contrast for text and interactive controls in both light and dark themes.
- Full **keyboard operability** for configuration, provider dialogs, and list navigation; suppress shortcuts only inside text inputs and the chat composer.
- **Live regions** for streaming chat and status changes where appropriate.
- Respect **`prefers-reduced-motion`** for non-essential animation (thinking shimmer, transitions).
- Theme persisted with **pre-paint init** to avoid flash of wrong theme on load.

# Voice and tone for skill content

Skill content reaches both human readers (via the published README on iii.dev) and LLM agents (via `iii://`). The same voice rules apply to both — declarative, confident, technical.

## Always do

- State the recommendation directly. `Use X for Y` beats `you might want to consider X for Y`.
- Lead with the user-facing value over the implementation.
- Reference function ids and behaviour in prose where natural — these are context, not API surface.

## Never do

- Marketing language. The token list in `styles/Terminology/SlopMarketing.yml` is enforced by Vale at error level: `blazing fast`, `world-class`, `paradigm shift`, and the rest.
- Mystification claims (`SlopMagic`). Say what actually happens — which function, which trigger, which state scope.
- Capability boasts (`SlopEase`). Show the steps, the API, or the trade-offs instead of asserting `effortless` or `trivial`.
- Connection metaphors (`SlopConnection`). Prefer `register`, `invoke`, `subscribe`, `read`, `write` over `wire up`, `glue`, `bridge`, `weave`.
- Tutorial-speak in a how-to (`Diataxis.HowTo`). How-to docs solve a problem; they don't `teach`. Phrases like `in this guide you will learn` are flagged.
- Numbered `step` labels inside iii narrative content. Generic `step 1`, `step 2` is fine inside `tutorials/`, not in worker docs or skills.
- The bare term `telemetry`. Disambiguate: `OpenTelemetry` or `observability` for traces, metrics, and logs (the iii-observability worker), or `iii-telemetry` for anonymous-usage analytics.

The full slop and terminology lists are in `styles/Terminology/`. Vale enforces them on every rendered artifact.

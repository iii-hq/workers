# Voice and tone rules

Rules for the prose voice across iii docs pages.

## Scope

The "Manifesto-aligned voice" and "Recurring theme: compose from the registry" sections below
apply to the **main docs** — pages under `using-iii/`, `understanding-iii/`, `expanding-iii/`,
`tutorials/`, `how-to/`, `patterns/`, `sdk-reference/`, the changelog, and landing pages on
iii.dev.

They do **not** govern per-worker `README.md`, `skill.md`, or `skills/<leaf>.md` rendered by
`iii-skill-check` from a worker's `docs/` partials. Worker-specific docs follow a tighter,
technical voice that prioritizes precise mechanism over manifesto framing — see the
`iii-skill-authoring` skill bundle (in `iii-hq/workers`) for the voice rules that apply there.

The "What to avoid" rules at the bottom apply **everywhere** — Vale enforces the slop and
terminology lists on every markdown artifact regardless of surface.

## Manifesto-aligned voice

iii's docs voice should match the website's hero framing — declarative, confident,
paradigm-shift focused. Avoid promotional or tutorial-speak. State things directly.

The website (`iii-temp/website/index.html`, `iii-temp/website/manifesto.html`) is the canonical
voice reference. Examples of the target voice:

- "Software engineering is an exercise in assembling categories of services."
- "iii fundamentally eliminates this complexity."
- "worker. trigger. function."
- Analogies to paradigm shifts: Unix (everything-is-a-file), React (everything-is-a-component),
  iii (everything-is-a-worker).

## Recurring theme: compose from the registry

Keep driving home — across landing pages, explanations, tutorials, and how-tos — that robust,
interesting systems get built by **combining existing workers from the registry** rather than
writing everything from scratch. Hint at it whenever a section introduces a new primitive,
pattern, or use case: name a registry worker that already does part of the job, or gesture at the
"assemble categories of services" framing from `voice.md`.

Don't make it a slogan or repeat it verbatim. Vary the phrasing, keep it short, and let the
examples carry the weight.

## What to avoid

- Marketing fluff ("the best", "powerful", "revolutionary"). The voice is confident, not
  aggrandizing.
- Tutorial-speak ("Welcome! Let's get started!"). Be direct.
- Hedging ("you might want to consider"). State the recommendation.

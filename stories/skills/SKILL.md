---
name: stories
description: >-
  Component stories for any React project inside iii: list components and
  their states, build git lines (working tree, branch, commit, chat turn),
  compare lines into direct/indirect impact, render deterministic
  screenshots and DOM/React trees with arg overrides, and diff one state
  across two versions into a reviewer's verdict. Reach for it whenever UI
  components changed and someone must know what was affected and whether it
  still looks right.
---

# stories

The `stories` worker indexes CSF story files (`*.stories.tsx`) of every
project in a workspace and builds them with the project's own Vite. A
**component** is one story file (title, group, tags, source file); a
**state** is one named export of it (args, argTypes, inferred controls). A
**line** is a source tree: `worktree` (live), `prev` (the build before it),
a ref such as `HEAD` or `main`, a full sha, or `turn:<session>/<turn>/before|after`
when the ide worker keeps turn snapshots. Comparing two lines classifies each
component as `direct` (its story file or a direct import changed),
`indirect` (a deeper input changed), `new`, `removed` or `unchanged`, with
the files behind the change. `stories::screenshot`, `stories::tree` and
`stories::diff` render through the `browser` worker with time, randomness,
locale and motion frozen, so the same inputs give the same pixels.

## When to Use

- A pull request, a branch or a chat turn touched UI code and the review
  must say which components and states were affected, directly or through
  shared code.
- A change must be verified visually or structurally without a person
  opening a browser: per state, per prop value, against the previous
  version.
- An agent needs the rendered DOM, computed styles, layout boxes or the
  React tree of a component in a given state, with specific props.
- A console user wants the explorer or the compare view beside the chat.

## Boundaries

- Not a test runner: `play` functions, MDX docs and addons are ignored.
  Interaction tests stay in the project's own tooling.
- Renders need the `browser` worker and the console running at
  `console_url`; without them `components::*`, `compare` and `diff::file`
  still work.
- A line builds once and is cached by sha; building `main` in a large
  monorepo takes tens of seconds. Prefer `wait: true` on
  `stories::builds::create`, or poll `stories::builds::get`.
- Stories that reach application singletons carry an `error` field; read
  it instead of retrying.

## How-tos

- [component-review.md](component-review.md) — the review workflow: scope
  the impact, verify each affected state, write the report.

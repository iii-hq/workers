# stories

Component stories for any React project, rendered in the iii console. The
worker finds CSF story files (`*.stories.tsx` and friends) in one or more
workspaces, builds them with the **project's own Vite** (its plugins,
aliases, CSS pipeline and React), and keeps an index per git *line*: the
working tree, its previous build, any ref or commit, or a chat turn's
before/after tree. Two lines compare into a list of components that changed
directly (the story file or a direct import), indirectly (a deeper input),
were added or removed. Through the `browser` worker it renders deterministic
screenshots and DOM/React trees of any component state, with args and
globals overrides, and diffs two versions of a state into a verdict a
reviewer can act on. A console page shows the explorer (files as on disk,
git status, impact chips, live preview, props) and the compare view (line A
versus line B, side by side).

No Storybook runtime is needed. Existing `.stories.*` files and
`.storybook/preview.*` keep working; `storybook/test`'s `fn()` is shimmed and
records an actions log. `play` functions, MDX and addons are ignored.

## Install

```bash
iii trigger compose::add worker=stories
```

Requirements on the host: `git`, Node 22+ with `npm` on `PATH` (the compiler
is materialized under `data/stories/compiler` and installed there once), and
the `browser` worker for screenshots and trees. Each project must have its
dependencies installed (`react` and `react-dom` 18+); a project with its own
`vite` is built with that version.

## Configuration (`stories`)

| Field | Default | Meaning |
|---|---|---|
| `data_path` | `data/stories` | Lines, content-addressed files, renders, history, the compiler. |
| `console_url` | `http://127.0.0.1:3113` | Console origin the browser worker navigates to for renders. |
| `watch` | `true` | Rebuild the working tree when a story input changes. |
| `keep_lines` | `12` | Built ref/turn lines kept per workspace before pruning. |
| `viewport` | `1024×768 @1` | Default render viewport. |
| `workspaces[]` | one at `.` | `{ name, path, projects?, stories?, ignore?, base? }` |

A workspace is a repository. Projects inside it are discovered from the story
files (the nearest `package.json`), so `client-app/` and `admin-app/` in one
repo each build with their own config and React. Shared packages resolve to
their real paths, so a change in `packages/ui` impacts stories in every
project that imports it. `base` is the ref changes are classified against
(default `HEAD`).

## Functions

| Function | What it does |
|---|---|
| `stories::workspaces` / `stories::lines` | Git state, projects, component count and built lines per workspace. |
| `stories::components::list` | Components of a line with their states; filters by project, group, tag, path, query and change kind against a base line. |
| `stories::components::get` | One component: states with args, argTypes and inferred controls, inputs with hops, change, related components, history. |
| `stories::tree::fs` | The explorer tree: folders and story files as on disk with git status and change kinds. |
| `stories::builds::create` / `stories::builds::get` | Build a line (`worktree`, a ref, a sha, `turn:<session>/<turn>/<side>`) and poll it. |
| `stories::compare` | Every component that differs between two lines, classified, with the changed files. |
| `stories::diff::file` | Unified diff of one input between two lines. |
| `stories::screenshot` | Deterministic PNG of one state (args, globals, viewport, clip), cached by content. |
| `stories::tree` | DOM tree with layout boxes, computed styles and owning component; React tree; accessibility outline; html; actions. |
| `stories::diff` | One state across two lines: tree changes, pixel regions mapped to elements, verdict `identical` / `invisible` / `visual` / `unexplained`. |
| `stories::history` | Versions of one component seen by working-tree builds. |

Triggers: `stories:changed` fires after a working-tree rebuild with the
classified changes (`{ workspace?, project?, kinds? }`); `stories:build`
fires on every build state transition.

The review workflow for agents lives in [`skills/`](skills/).

## Lines and data

```text
data/stories/<workspace>/
  lines/worktree/            the live working tree (rebuilt by the watcher)
  lines/worktree.prev/       the build before it
  lines/<sha>/               a commit or a turn tree, immutable
  files/<sha256>             build outputs and story inputs, shared by all lines
  renders/<hash>/            shot.png, tree.json, meta.json of one render
  history/<project>__<id>.jsonl
```

A ref builds in a detached `git worktree`; a turn tree comes from the ide
worker's private snapshot repository through `git archive`. `node_modules`
folders are linked from the live checkout. A component's version is the
hash of every input's content, so a component unchanged between two lines
shares the same render cache.

Story documents are served to the console through `stories::ui-file` behind
the console's `/ui-files/stories/<workspace>/<line>/<path>` route; the page
embeds them in a same-origin iframe.

## Development

```bash
# terminal 1: rebuild the console page on save
pnpm --dir stories/ui watch
# terminal 2: hot reload the page assets
cd stories && III_STORIES_UI_WATCH=1 cargo run
```

`III_STORIES_NODE` / `III_STORIES_NPM` override the binaries;
`III_STORIES_IDE_TURNS` points at the ide worker's turn store (default
`data/shell/turns`). Tests: `cargo test`; the pipeline test runs against a
real project when `III_STORIES_FIXTURE=<workspace dir>` and
`III_STORIES_COMPILER_DIR=<installed compiler dir>` are set.

## Limits

React 18+ only. Projects built by webpack or Next.js need a `vite.config`
with the mocks they rely on (`next/link`, `next/image`). Stories that reach
application singletons (routers, engine clients) render with errors and show
up with an `error` field. Pixel diffs compare renders from one machine; a
different font stack between hosts reads as `unexplained`.

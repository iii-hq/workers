# @iii-dev/console-ui

The compile-time surface of the console's injectable-UI runtime module —
types for `setup(host)`, the slot contracts, the extension engine client,
and the shared component library (`Button`, `Dialog`, `Tabs`, `Markdown`, …).

**There is no bundleable runtime here, by design.** At runtime the console's
import map resolves `@iii-dev/console-ui` to `/vendor/console-ui.js`, which
re-exports the running SPA's own React tree, engine client, and components
from `window.__III_CONSOLE__`. Every worker shares the console's single copy
— nothing from this package (or React) ships inside a worker's asset, which
is what keeps injected bundles tens of KiB. The `index.js` entry throws with
instructions if a build bundles it anyway.

## Using it in a worker UI

The package is linked through the repo's pnpm workspace — no publishing, no
copying types around:

```jsonc
// <worker>/ui/package.json
{ "dependencies": { "@iii-dev/console-ui": "workspace:*" } }
```

```tsx
import { Button, EmptyState, type Host } from '@iii-dev/console-ui'
```

and keep it external in the build (alongside the react specifiers):

```js
external: ['react', 'react-dom', 'react-dom/client',
           'react/jsx-runtime', '@iii-dev/console-ui']
```

Full authoring guide: `workers/docs/sops/injectable-console-ui.md`.

## Keeping it honest

The declarations are hand-modeled on the console's real components; two
guards in `console/web` fail the build/tests when they drift:

- `src/lib/console-ui-conformance.test.ts` — type-level check that every
  declared component export is satisfied by the real component, plus a
  runtime check that the curated `components` record matches
  `component-names.mjs` (the manifest the `/vendor/console-ui.js` shim
  generator consumes).
- `scripts/generate-vendor-shims.mjs` — evaluates the generated shim, so a
  bad export name fails the console build, never a browser tab.

Declared props are the *supported authoring surface*: the real components
may accept more (Radix pass-through), and those extras carry no
compatibility promise.

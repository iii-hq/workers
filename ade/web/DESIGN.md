# Design system

The canonical iii Schematic design system lives in
[`../skills/design-system.md`](../skills/design-system.md) — principles, tokens, typography, the
numbers table, shared components, UX patterns and do/don't. This file is only a pointer; do
not add tokens or rules here.

- Tokens and utilities: [`src/index.css`](src/index.css)
- Public CSS recipes (`uiClasses`): [`src/styles/ui-recipes.css`](src/styles/ui-recipes.css)
- Components: [`src/components/ui`](src/components/ui), exported to workers through
  [`packages/console-ui`](../../packages/console-ui)
- Workspace behaviour (panes, tabs, keyboard, palette): [`README.md`](README.md)

Tailwind CSS v4 is configured through `@tailwindcss/vite` in `vite.config.ts`
and `@import "tailwindcss"` in `src/index.css`. Add utilities directly in JSX;
shared colors, fonts, and motion values come from the CSS `@theme` tokens.

Setup is styled entirely with Tailwind utilities in its components, using
Inter, a 14px body/12px caption scale, and the neutral color palette. The
`dark` variant follows the application’s `data-theme` attribute. Provider marks reuse the worker SVG
assets through `ProviderMark`. Step transitions use Motion (`motion/react`),
with `MotionConfig` and `useReducedMotion` honoring the system preference.

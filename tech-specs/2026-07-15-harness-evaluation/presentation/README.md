# Harness evaluation presentation

Interactive presentation for the harness evaluation tech spec. The Markdown
files in the parent directory are canonical and are bundled into the `#/spec`
reading view.

```bash
pnpm install --ignore-workspace
pnpm typecheck
pnpm build
pnpm dev
```

The Vite build uses relative assets so `dist/` can be served at the dated spec
slug by the repository-level tech-spec build.

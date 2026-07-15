# Tech specs and presentations

`tech-specs/` contains the canonical Markdown design documents for this repo and
one Vercel project that hosts their optional interactive presentations.

## Layout

```text
tech-specs/
├── YYYY-MM-DD-<slug>/
│   ├── README.md              canonical overview and frontmatter
│   ├── <topic>.md             one file per domain topic
│   └── presentation/          optional Vite/React deck
├── _gallery/                  presentation index
├── build.mjs                  gallery + deck build
└── dist/                      generated output; never committed
```

The directory basename is the stable identity used by links, the presentation
gallery, and the deployed URL. New specs use day precision:
`YYYY-MM-DD-<slug>`. Existing month-only directories are legacy and are not
renamed because their published URLs are already stable.

## Add a tech spec

1. Create `tech-specs/YYYY-MM-DD-<slug>/`.
2. Write `README.md` as the index/overview and use one Markdown file per
   distinct domain concern.
3. Put this frontmatter at the top of the README:

```yaml
---
title: concise title
tagline: one sentence explaining the outcome.
date: YYYY-MM-DD
tags: [topic, architecture] # at most four
status: draft               # draft or live
featured: false
---
```

The directory is the slug; never add a `slug` frontmatter field. The frontmatter
date must match the directory prefix. Markdown is the source of truth. Generated
HTML and review notes do not belong beside the canonical topic files.

A multi-file spec should lead with the constraint that drives the design,
distinguish shipped contracts from proposed shapes, link exact source for
current interfaces, include an index, and keep genuine unresolved decisions in
one anonymous `Open questions` section.

## Add a presentation

Presentations are optional. In this repository they remain standalone packages
under `<spec>/presentation/` because the workers Vercel build discovers that
layout.

This is an intentional workers-specific divergence from the current `iii`
repository convention. `iii` keeps `tech-specs/` Markdown-only and mounts deck
content from `website/roadmap/<slug>/` in its Astro/CloudFront site. Workers has
no equivalent website tree; its existing `tech-specs/vercel.json`, gallery, and
multi-package build are the deployment contract. Do not infer the workers
layout when authoring a spec in `iii`, and do not move a workers deck without a
separate hosting migration.

A workers deck must:

- use the same slug as its parent spec directory;
- include a `#/spec` reading view that bundles the canonical Markdown;
- keep content in `src/content` or thin sections and avoid duplicating a second
  authoritative specification;
- use packaged dependencies rather than runtime CDN assets;
- set Vite `base: './'` so static assets work under the slug path;
- include its own lockfile and pass strict TypeScript/build checks.

Add the same slug and frontmatter metadata to
`_gallery/src/content/presentations.ts`. A mismatch between the directory,
gallery entry, or build output causes a 404.

## Build and preview

```bash
pnpm build                         # gallery + every presentation
node build.mjs --only=<slug>       # gallery + one presentation
pnpm preview                       # full built site at http://localhost:4173
```

The built layout is:

```text
dist/                 gallery
dist/<slug>/          presentation for that spec
```

## Deploy

Vercel uses `tech-specs/vercel.json` with this directory as the project root.
The build output is fully static. Every new presentation must be smoke-tested at
its final `/<slug>/` path; a successful Vercel build alone does not prove the
route exists.

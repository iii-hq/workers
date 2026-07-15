/**
 * presentations.ts — the ONE file you fill in.
 *
 * This is the manifest the gallery renders from, and the single source of truth
 * for which spec presentations this site lists. The `/presentation` skill
 * appends an entry here every time it generates a deck; you can also edit it by
 * hand to re-order, re-word, or feature a deck.
 *
 * THE CONTRACT: every entry's `slug` MUST equal the spec directory name that
 * holds the deck (the folder under tech-specs/ that contains `presentation/`).
 * `../build.mjs` builds each `<slug>/presentation/` and copies its output to
 * `dist/<slug>/`, and each card links to `<slug>/`. Same string everywhere, or
 * the card links to a 404.
 *
 * Everything here is pure data (no JSX) so it stays the readable answer to
 * "what does this site host". The gallery chrome and cards live in components/.
 */

export interface GalleryMeta {
  /** text next to the wordmark in the header, e.g. "iii / tech-specs" */
  wordmarkLabel: string
  /** small-caps eyebrow above the hero title */
  heroEyebrow: string
  /** the big hero line — what this collection is */
  heroTitle: string
  /** one or two sentences under the hero title */
  heroLead: string
  /** left attribution in the footer bar */
  attribution: string
  /** right "source of truth" line in the footer bar */
  source: string
}

export interface Presentation {
  /** url slug — MUST equal the spec directory name (build copies dist/<slug>/) */
  slug: string
  /** deck title, lowercase, e.g. "the developer experience overhaul" */
  title: string
  /** the single claim / one-line tagline shown on the card */
  tagline: string
  /** the spec this came from, e.g. "tech-specs/2026-06-devexp" */
  spec: string
  /** date label shown on the card, e.g. "2026-06" — also the sort key */
  date: string
  /** short topic tags, e.g. ["architecture", "migration"] (0–4 read best) */
  tags?: string[]
  /** 'live' (default) shows the deck; 'draft' muted + flagged, still links */
  status?: 'live' | 'draft'
  /** pin to the top of the grid regardless of date */
  featured?: boolean
}

export const GALLERY_META: GalleryMeta = {
  wordmarkLabel: 'iii / workers',
  heroEyebrow: 'tech-specs',
  heroTitle: 'iii worker specs, made interactive',
  heroLead:
    'one interactive deck per worker spec: the architecture as a navigable map, the protocol steppable, and the why argued like a product launch. build-in-public ready.',
  attribution: 'iii workers — tech-spec presentations',
  source: 'source of truth: workers/tech-specs',
}

/**
 * The decks this site hosts. The `/presentation` skill keeps this in sync.
 */
export const PRESENTATIONS: Presentation[] = [
  {
    slug: '2026-07-15-harness-evaluation',
    title: 'harness evaluation — conformance and agent quality',
    tagline:
      'deterministic contract checks and real-model workflow evaluation for the durable harness.',
    spec: 'tech-specs/2026-07-15-harness-evaluation',
    date: '2026-07-15',
    tags: ['agents', 'evaluation', 'harness'],
    status: 'draft',
  },
  {
    slug: '2026-06-agentic',
    title: 'agentic workers, an architecture overview',
    tagline:
      'five standalone iii workers that compose into a reactive, durable agent backend.',
    spec: 'tech-specs/2026-06-agentic',
    date: '2026-06',
    tags: ['agents', 'workers', 'architecture'],
    status: 'live',
    featured: true,
  },
  {
    slug: '2026-06-rbac-proxy-worker',
    title: 'rbac at the edge, not inside the engine',
    tagline:
      'role-based access control in front of the engine, on its own port, filtering all eight discovery functions to the caller.',
    spec: 'tech-specs/2026-06-rbac-proxy-worker',
    date: '2026-06',
    tags: ['security', 'rbac', 'workers'],
    status: 'live',
  },
  {
    slug: '2026-06-codegen',
    title: 'iii codegen — typed worker integrations',
    tagline:
      'one command generates the types and wrappers you need to call any worker, projected from the engine\'s live json-schema catalog.',
    spec: 'tech-specs/2026-06-codegen',
    date: '2026-06',
    tags: ['codegen', 'dx', 'workers'],
    status: 'live',
  },
]

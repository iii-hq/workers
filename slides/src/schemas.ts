import { ICON_NAMES } from './deck-icons.js'
import { BLOCK_TYPES, CHART_KINDS, LAYOUTS, REVEALS, TRANSITIONS, VARIANTS, VISUALS } from './model.js'

export const object = (
  properties: Record<string, unknown> = {},
  required: string[] = [],
  extra: Record<string, unknown> = {},
) => ({
  type: 'object' as const,
  properties,
  ...(required.length ? { required } : {}),
  ...extra,
})
export const string = { type: 'string' }
export const nullableString = { type: ['string', 'null'] }
export const integer = { type: 'integer' }
export const boolean = { type: 'boolean' }
export const array = (items: unknown, extra: Record<string, unknown> = {}) => ({
  type: 'array' as const,
  items,
  ...extra,
})

export const blockSchema = object(
  {
    id: nullableString,
    type: { type: 'string', enum: [...BLOCK_TYPES] },
    text: nullableString,
    items: array(string),
    src: nullableString,
    alt: nullableString,
    caption: nullableString,
    code: nullableString,
    language: nullableString,
    attribution: nullableString,
    value: nullableString,
    label: nullableString,
    entries: array(
      object({ title: string, text: nullableString, icon: { type: ['string', 'null'], enum: [...ICON_NAMES, null] } }, [
        'title',
      ]),
    ),
    numbered: boolean,
    kind: { type: ['string', 'null'], enum: [...CHART_KINDS, null] },
    series: array(object({ label: string, value: { type: 'number' } }, ['label', 'value'])),
    unit: nullableString,
    title: nullableString,
    column: { type: ['string', 'null'], enum: ['left', 'right', null] },
  },
  ['type'],
  {
    description:
      'One content block. heading/text/quote use text; bullets uses items; image uses src/alt/caption; code uses code/language; metric uses value/label; cards/steps/timeline use entries [{ title, text?, icon? }] (cards may be numbered; icon is one of the built-in icon names); chart uses kind (bar, line, donut), series [{ label, value }], unit, title. column places the block in a two-column layout.',
  },
)

export const slideSchema = object(
  {
    id: nullableString,
    layout: { type: 'string', enum: [...LAYOUTS] },
    variant: { type: ['string', 'null'], enum: [...VARIANTS, null] },
    visual: { type: ['string', 'null'], enum: [...VISUALS, null] },
    kicker: nullableString,
    title: nullableString,
    subtitle: nullableString,
    blocks: array(blockSchema),
    notes: nullableString,
    background: nullableString,
  },
  [],
  {
    description:
      'One slide. layout defaults to content; variant (accent, gradient, muted) changes the background treatment; visual adds a decorative motif (orbits, grid, waves, arcs, rings); kicker is a short eyebrow label above the title; blocks render in order; notes are speaker notes; background is a hex color or image URL.',
  },
)

export const themeOverridesSchema = object(
  {
    accent: nullableString,
    background: nullableString,
    ink: nullableString,
    font_heading: nullableString,
    font_body: nullableString,
    footer: nullableString,
  },
  [],
  { description: 'Per-deck overrides of the theme: hex colors, font family names, footer text.' },
)

export const motionSchema = object(
  {
    transition: { type: ['string', 'null'], enum: [...TRANSITIONS, null] },
    reveal: { type: ['string', 'null'], enum: [...REVEALS, null] },
  },
  [],
  {
    description:
      'Presentation motion: transition between slides (fade, slide, zoom, none) and how elements reveal (stagger on entry, step through with keys, none).',
  },
)

export const deckSchema = object(
  {
    id: string,
    title: string,
    subtitle: nullableString,
    author: nullableString,
    theme: string,
    theme_overrides: themeOverridesSchema,
    motion: motionSchema,
    slides: array(slideSchema),
    revision: integer,
    created_at_ms: integer,
    updated_at_ms: integer,
  },
  ['id', 'title', 'theme', 'slides', 'revision', 'created_at_ms', 'updated_at_ms'],
)

export const deckSummarySchema = object(
  {
    id: string,
    title: string,
    subtitle: nullableString,
    theme: string,
    slide_count: integer,
    revision: integer,
    updated_at_ms: integer,
  },
  ['id', 'title', 'theme', 'slide_count', 'revision', 'updated_at_ms'],
)

export const deckResponse = object({ deck: deckSchema }, ['deck'])

export const themeSchema = object(
  {
    id: string,
    name: string,
    description: string,
    dark: boolean,
    colors: object(
      { background: string, surface: string, ink: string, muted: string, accent: string, accent_ink: string },
      ['background', 'surface', 'ink', 'muted', 'accent', 'accent_ink'],
    ),
    fonts: object({ heading: string, body: string, mono: string }, ['heading', 'body', 'mono']),
  },
  ['id', 'name', 'description', 'dark', 'colors', 'fonts'],
)

export const changedEventSchema = object(
  {
    kind: { type: 'string', enum: ['created', 'updated', 'deleted'] },
    deck_id: string,
    revision: integer,
    updated_at_ms: integer,
  },
  ['kind', 'deck_id', 'revision', 'updated_at_ms'],
)

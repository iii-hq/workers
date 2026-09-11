import { BLOCK_TYPES, LAYOUTS } from './model.js'

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
    column: { type: ['string', 'null'], enum: ['left', 'right', null] },
  },
  ['type'],
  {
    description:
      'One content block. heading/text/quote use text; bullets uses items; image uses src/alt/caption; code uses code/language; metric uses value/label. column places the block in a two-column layout.',
  },
)

export const slideSchema = object(
  {
    id: nullableString,
    layout: { type: 'string', enum: [...LAYOUTS] },
    title: nullableString,
    subtitle: nullableString,
    blocks: array(blockSchema),
    notes: nullableString,
    background: nullableString,
  },
  [],
  {
    description:
      'One slide. layout defaults to content; blocks render in order; notes are speaker notes; background is a hex color or image URL.',
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

export const deckSchema = object(
  {
    id: string,
    title: string,
    subtitle: nullableString,
    author: nullableString,
    theme: string,
    theme_overrides: themeOverridesSchema,
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

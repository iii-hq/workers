import { randomUUID } from 'node:crypto'

export const LAYOUTS = ['title', 'section', 'content', 'two-column', 'split', 'statement', 'image', 'blank'] as const
export const TRANSITIONS = ['fade', 'slide', 'zoom', 'none'] as const
export type Transition = (typeof TRANSITIONS)[number]
export const REVEALS = ['stagger', 'step', 'none'] as const
export type Reveal = (typeof REVEALS)[number]

export interface Motion {
  transition?: Transition
  reveal?: Reveal
}
export type Layout = (typeof LAYOUTS)[number]

export const BLOCK_TYPES = [
  'heading',
  'text',
  'bullets',
  'image',
  'code',
  'quote',
  'metric',
  'cards',
  'steps',
  'timeline',
] as const
export const VARIANTS = ['default', 'accent', 'gradient', 'muted'] as const
export type Variant = (typeof VARIANTS)[number]

export interface Entry {
  title: string
  text?: string
}
export type BlockType = (typeof BLOCK_TYPES)[number]

export type Column = 'left' | 'right'

export type Block =
  | { id: string; type: 'heading'; text: string; column?: Column }
  | { id: string; type: 'text'; text: string; column?: Column }
  | { id: string; type: 'bullets'; items: string[]; column?: Column }
  | { id: string; type: 'image'; src: string; alt?: string; caption?: string; column?: Column }
  | { id: string; type: 'code'; code: string; language?: string; column?: Column }
  | { id: string; type: 'quote'; text: string; attribution?: string; column?: Column }
  | { id: string; type: 'metric'; value: string; label: string; column?: Column }
  | { id: string; type: 'cards'; entries: Entry[]; numbered?: boolean; column?: Column }
  | { id: string; type: 'steps'; entries: Entry[]; column?: Column }
  | { id: string; type: 'timeline'; entries: Entry[]; column?: Column }

export interface Slide {
  id: string
  layout: Layout
  variant?: Variant
  kicker?: string
  title?: string
  subtitle?: string
  blocks: Block[]
  notes?: string
  background?: string
}

export interface ThemeOverrides {
  accent?: string
  background?: string
  ink?: string
  font_heading?: string
  font_body?: string
  footer?: string
}

export interface Deck {
  id: string
  title: string
  subtitle?: string
  author?: string
  theme: string
  theme_overrides?: ThemeOverrides
  motion?: Motion
  slides: Slide[]
  revision: number
  created_at_ms: number
  updated_at_ms: number
}

export interface DeckSummary {
  id: string
  title: string
  subtitle?: string
  theme: string
  slide_count: number
  revision: number
  updated_at_ms: number
}

export const MAX_SLIDES = 200
export const MAX_BLOCKS = 24
export const ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/

export function newId(prefix: string): string {
  return `${prefix}-${randomUUID().slice(0, 8)}`
}

export function text(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined
  const trimmed = value.trim()
  return trimmed || undefined
}

function record(value: unknown, what: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error(`INVALID_${what}: expected an object`)
  return value as Record<string, unknown>
}

function stringList(value: unknown): string[] {
  if (typeof value === 'string') {
    return value
      .split('\n')
      .map((line) => line.replace(/^\s*[-*\u2022]\s*/, '').trim())
      .filter(Boolean)
  }
  if (!Array.isArray(value)) return []
  return value.map((item) => (typeof item === 'string' ? item.trim() : String(item ?? '').trim())).filter(Boolean)
}

function column(value: unknown): Column | undefined {
  return value === 'left' || value === 'right' ? value : undefined
}

export const MAX_ENTRIES = 12

export function entryList(value: unknown): Entry[] {
  const raw = Array.isArray(value) ? value : typeof value === 'string' ? value.split('\n') : []
  const entries: Entry[] = []
  for (const item of raw) {
    if (typeof item === 'string') {
      const [title, ...rest] = item.split(/\s\|\s|\s\u2014\s|:\s/)
      const cleanTitle = title.replace(/^\s*[-*\u2022]\s*/, '').trim()
      if (!cleanTitle) continue
      entries.push({ title: cleanTitle, ...(rest.join(' ').trim() ? { text: rest.join(' ').trim() } : {}) })
    } else if (item && typeof item === 'object') {
      const entry = item as Record<string, unknown>
      const title = text(entry.title) ?? text(entry.label) ?? text(entry.name) ?? text(entry.heading)
      const body = text(entry.text) ?? text(entry.description) ?? text(entry.body) ?? text(entry.detail)
      if (!title && !body) continue
      entries.push({ title: title ?? (body as string), ...(title && body ? { text: body } : {}) })
    }
  }
  return entries.slice(0, MAX_ENTRIES)
}

export function normalizeBlock(input: unknown, index = 0): Block {
  const raw = record(input, 'BLOCK')
  const id = text(raw.id) ?? newId('block')
  const col = column(raw.column)
  const withColumn = <T extends Block>(block: T): T => (col ? { ...block, column: col } : block)
  const type = raw.type
  switch (type) {
    case 'heading':
      return withColumn({ id, type, text: text(raw.text) ?? '' })
    case 'text':
      return withColumn({ id, type, text: text(raw.text) ?? text(raw.body) ?? '' })
    case 'bullets':
      return withColumn({ id, type, items: stringList(raw.items ?? raw.text) })
    case 'image': {
      const src = text(raw.src) ?? text(raw.url)
      if (!src) throw new Error(`INVALID_BLOCK: blocks[${index}] image needs a src`)
      return withColumn({
        id,
        type,
        src,
        ...(text(raw.alt) ? { alt: text(raw.alt) } : {}),
        ...(text(raw.caption) ? { caption: text(raw.caption) } : {}),
      })
    }
    case 'code':
      return withColumn({
        id,
        type,
        code: typeof raw.code === 'string' ? raw.code.replace(/\s+$/, '') : '',
        ...(text(raw.language) ? { language: text(raw.language) } : {}),
      })
    case 'quote':
      return withColumn({
        id,
        type,
        text: text(raw.text) ?? '',
        ...(text(raw.attribution) ? { attribution: text(raw.attribution) } : {}),
      })
    case 'metric':
      return withColumn({ id, type, value: text(raw.value) ?? '', label: text(raw.label) ?? '' })
    case 'cards':
      return withColumn({
        id,
        type,
        entries: entryList(raw.entries ?? raw.items),
        ...(raw.numbered === true ? { numbered: true } : {}),
      })
    case 'steps':
    case 'timeline':
      return withColumn({ id, type, entries: entryList(raw.entries ?? raw.items) })
    default:
      throw new Error(`INVALID_BLOCK: blocks[${index}] has unknown type ${String(type)}`)
  }
}

export function normalizeSlide(input: unknown, index = 0): Slide {
  const raw = record(input, 'SLIDE')
  const layout = (LAYOUTS as readonly string[]).includes(String(raw.layout)) ? (raw.layout as Layout) : 'content'
  const rawBlocks = Array.isArray(raw.blocks) ? raw.blocks : []
  if (rawBlocks.length > MAX_BLOCKS)
    throw new Error(`INVALID_SLIDE: slides[${index}] has more than ${MAX_BLOCKS} blocks`)
  const blocks = rawBlocks.map((block, blockIndex) => normalizeBlock(block, blockIndex))
  if (Array.isArray(raw.bullets) && raw.bullets.length) {
    blocks.push({ id: newId('block'), type: 'bullets', items: stringList(raw.bullets) })
  }
  if (text(raw.body)) blocks.push({ id: newId('block'), type: 'text', text: text(raw.body) as string })
  const variant =
    (VARIANTS as readonly string[]).includes(String(raw.variant)) && raw.variant !== 'default'
      ? (raw.variant as Variant)
      : undefined
  return {
    id: text(raw.id) ?? newId('slide'),
    layout,
    ...(variant ? { variant } : {}),
    ...(text(raw.kicker) ? { kicker: text(raw.kicker) } : {}),
    ...(text(raw.title) ? { title: text(raw.title) } : {}),
    ...(text(raw.subtitle) ? { subtitle: text(raw.subtitle) } : {}),
    blocks,
    ...(text(raw.notes) ? { notes: text(raw.notes) } : {}),
    ...(text(raw.background) ? { background: text(raw.background) } : {}),
  }
}

export function normalizeSlides(input: unknown): Slide[] {
  if (!Array.isArray(input)) return []
  if (input.length > MAX_SLIDES) throw new Error(`INVALID_DECK: a deck supports at most ${MAX_SLIDES} slides`)
  const slides = input.map((slide, index) => normalizeSlide(slide, index))
  const ids = new Set<string>()
  for (const slide of slides) {
    if (ids.has(slide.id)) slide.id = newId('slide')
    ids.add(slide.id)
  }
  return slides
}

export function normalizeOverrides(input: unknown): ThemeOverrides | undefined {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return undefined
  const raw = input as Record<string, unknown>
  const out: ThemeOverrides = {}
  for (const key of ['accent', 'background', 'ink', 'font_heading', 'font_body', 'footer'] as const) {
    const value = text(raw[key])
    if (value) out[key] = value
  }
  return Object.keys(out).length ? out : undefined
}

export function normalizeMotion(input: unknown): Motion | undefined {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return undefined
  const raw = input as Record<string, unknown>
  const out: Motion = {}
  if ((TRANSITIONS as readonly string[]).includes(String(raw.transition))) out.transition = raw.transition as Transition
  if ((REVEALS as readonly string[]).includes(String(raw.reveal))) out.reveal = raw.reveal as Reveal
  return Object.keys(out).length ? out : undefined
}

export function summarize(deck: Deck): DeckSummary {
  return {
    id: deck.id,
    title: deck.title,
    ...(deck.subtitle ? { subtitle: deck.subtitle } : {}),
    theme: deck.theme,
    slide_count: deck.slides.length,
    revision: deck.revision,
    updated_at_ms: deck.updated_at_ms,
  }
}

export function isDeck(value: unknown): value is Deck {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const deck = value as Partial<Deck>
  return typeof deck.id === 'string' && typeof deck.title === 'string' && Array.isArray(deck.slides)
}

export function slideIndex(deck: Deck, slideId: string): number {
  const index = deck.slides.findIndex((slide) => slide.id === slideId)
  if (index < 0) throw new Error(`SLIDE_NOT_FOUND: ${slideId}`)
  return index
}

export function insertSlide(deck: Deck, slide: Slide, index?: number): Slide[] {
  if (deck.slides.length >= MAX_SLIDES) throw new Error(`INVALID_DECK: a deck supports at most ${MAX_SLIDES} slides`)
  const at = index === undefined ? deck.slides.length : Math.max(0, Math.min(deck.slides.length, index))
  const slides = [...deck.slides]
  slides.splice(at, 0, slide)
  return slides
}

export function reorderSlides(deck: Deck, slideIds: string[]): Slide[] {
  const byId = new Map(deck.slides.map((slide) => [slide.id, slide]))
  const seen = new Set<string>()
  const ordered: Slide[] = []
  for (const id of slideIds) {
    const slide = byId.get(id)
    if (!slide) throw new Error(`SLIDE_NOT_FOUND: ${id}`)
    if (seen.has(id)) throw new Error(`INVALID_ORDER: ${id} listed twice`)
    seen.add(id)
    ordered.push(slide)
  }
  for (const slide of deck.slides) if (!seen.has(slide.id)) ordered.push(slide)
  return ordered
}

export function mergeSlide(current: Slide, patch: unknown): Slide {
  const raw = record(patch, 'SLIDE')
  const next = normalizeSlide({ ...current, ...raw, id: current.id })
  if (raw.title === null || raw.title === '') delete next.title
  if (raw.kicker === null || raw.kicker === '') delete next.kicker
  if (raw.variant === null || raw.variant === '' || raw.variant === 'default') delete next.variant
  if (raw.subtitle === null || raw.subtitle === '') delete next.subtitle
  if (raw.notes === null || raw.notes === '') delete next.notes
  if (raw.background === null || raw.background === '') delete next.background
  if (raw.blocks === undefined) next.blocks = current.blocks
  return next
}

const DIRECTIVE = /^<!--\s*([a-z_]+)\s*(?::\s*([\s\S]*?))?\s*-->$/i

export function slidesFromMarkdown(markdown: string): { title?: string; subtitle?: string; slides: Slide[] } {
  const sections = markdown
    .replace(/\r\n/g, '\n')
    .split(/\n\s*---\s*\n/)
    .map((section) => section.trim())
    .filter(Boolean)
  const slides: Slide[] = []
  let deckTitle: string | undefined
  let deckSubtitle: string | undefined
  for (const [sectionIndex, section] of sections.entries()) {
    const slide: Slide = { id: newId('slide'), layout: 'content', blocks: [] }
    let layoutSet = false
    let pendingList: 'cards' | 'steps' | 'timeline' | null = null
    const lines = section.split('\n')
    let index = 0
    const paragraph: string[] = []
    const flushParagraph = () => {
      if (!paragraph.length) return
      slide.blocks.push({ id: newId('block'), type: 'text', text: paragraph.join(' ') })
      paragraph.length = 0
    }
    while (index < lines.length) {
      const line = lines[index]
      const trimmed = line.trim()
      const directive = trimmed.match(DIRECTIVE)
      if (directive) {
        flushParagraph()
        const [, key, rawValue] = directive
        const value = rawValue ?? ''
        if (key === 'layout' && (LAYOUTS as readonly string[]).includes(value)) {
          slide.layout = value as Layout
          layoutSet = true
        } else if (key === 'notes') slide.notes = value.trim()
        else if (key === 'background') slide.background = value.trim()
        else if (key === 'kicker') slide.kicker = value.trim()
        else if (key === 'variant' && (VARIANTS as readonly string[]).includes(value.trim()))
          slide.variant = value.trim() as Variant
        else if ((key === 'cards' || key === 'steps' || key === 'timeline') && !value.trim()) pendingList = key
        index += 1
        continue
      }
      if (trimmed.startsWith('```')) {
        flushParagraph()
        const language = trimmed.slice(3).trim()
        const code: string[] = []
        index += 1
        while (index < lines.length && !lines[index].trim().startsWith('```')) {
          code.push(lines[index])
          index += 1
        }
        slide.blocks.push({
          id: newId('block'),
          type: 'code',
          code: code.join('\n'),
          ...(language ? { language } : {}),
        })
        index += 1
        continue
      }
      if (!trimmed) {
        flushParagraph()
        index += 1
        continue
      }
      const heading = trimmed.match(/^(#{1,3})\s+(.*)$/)
      if (heading) {
        flushParagraph()
        const level = heading[1].length
        const value = heading[2].trim()
        if (level === 1 && sectionIndex === 0 && !slide.title) {
          deckTitle = value
          slide.title = value
          if (!layoutSet) slide.layout = 'title'
        } else if (!slide.title) slide.title = value
        else if (level >= 3) slide.blocks.push({ id: newId('block'), type: 'heading', text: value })
        else slide.subtitle = value
        index += 1
        continue
      }
      const image = trimmed.match(/^!\[([^\]]*)\]\(([^)]+)\)$/)
      if (image) {
        flushParagraph()
        slide.blocks.push({
          id: newId('block'),
          type: 'image',
          src: image[2].trim(),
          ...(image[1].trim() ? { alt: image[1].trim() } : {}),
        })
        if (!layoutSet && slide.blocks.length === 1) slide.layout = 'image'
        index += 1
        continue
      }
      if (/^[-*\u2022]\s+/.test(trimmed)) {
        flushParagraph()
        const items: string[] = []
        while (index < lines.length && /^\s*[-*\u2022]\s+/.test(lines[index])) {
          items.push(lines[index].replace(/^\s*[-*\u2022]\s+/, '').trim())
          index += 1
        }
        if (pendingList) {
          slide.blocks.push({ id: newId('block'), type: pendingList, entries: entryList(items) })
          pendingList = null
        } else slide.blocks.push({ id: newId('block'), type: 'bullets', items })
        continue
      }
      if (trimmed.startsWith('>')) {
        flushParagraph()
        const quoted: string[] = []
        while (index < lines.length && lines[index].trim().startsWith('>')) {
          quoted.push(lines[index].trim().replace(/^>\s?/, ''))
          index += 1
        }
        const last = quoted[quoted.length - 1] ?? ''
        const attribution = last.match(/^[-\u2014]\s*(.+)$/)
        if (attribution) quoted.pop()
        slide.blocks.push({
          id: newId('block'),
          type: 'quote',
          text: quoted.join(' ').trim(),
          ...(attribution ? { attribution: attribution[1].trim() } : {}),
        })
        if (!layoutSet && slide.blocks.length === 1 && !slide.title) slide.layout = 'statement'
        continue
      }
      const notes = trimmed.match(/^notes?:\s*(.+)$/i)
      if (notes) {
        flushParagraph()
        slide.notes = notes[1].trim()
        index += 1
        continue
      }
      paragraph.push(trimmed)
      index += 1
    }
    flushParagraph()
    if (slide.layout === 'title' && !deckSubtitle) {
      const first = slide.blocks.find((block) => block.type === 'text')
      if (first && first.type === 'text') {
        deckSubtitle = first.text
        slide.subtitle = first.text
        slide.blocks = slide.blocks.filter((block) => block !== first)
      }
    }
    if (!layoutSet && slide.title && slide.blocks.length === 0 && slide.layout === 'content') slide.layout = 'section'
    slides.push(slide)
  }
  return { ...(deckTitle ? { title: deckTitle } : {}), ...(deckSubtitle ? { subtitle: deckSubtitle } : {}), slides }
}

export function deckToMarkdown(deck: Deck): string {
  return deck.slides
    .map((slide) => {
      const lines: string[] = []
      if (slide.layout !== 'content') lines.push(`<!-- layout: ${slide.layout} -->`)
      if (slide.variant) lines.push(`<!-- variant: ${slide.variant} -->`)
      if (slide.kicker) lines.push(`<!-- kicker: ${slide.kicker} -->`)
      if (slide.title) lines.push(`${slide.layout === 'title' ? '#' : '##'} ${slide.title}`)
      if (slide.subtitle) lines.push(slide.layout === 'title' ? slide.subtitle : `## ${slide.subtitle}`)
      for (const block of slide.blocks) {
        lines.push('')
        if (block.type === 'heading') lines.push(`### ${block.text}`)
        else if (block.type === 'text') lines.push(block.text)
        else if (block.type === 'bullets') lines.push(...block.items.map((item) => `- ${item}`))
        else if (block.type === 'image') lines.push(`![${block.alt ?? ''}](${block.src})`)
        else if (block.type === 'code') lines.push(`\`\`\`${block.language ?? ''}`, block.code, '```')
        else if (block.type === 'quote') {
          lines.push(`> ${block.text}`)
          if (block.attribution) lines.push(`> \u2014 ${block.attribution}`)
        } else if (block.type === 'metric') lines.push(`### ${block.value}`, block.label)
        else if (block.type === 'cards' || block.type === 'steps' || block.type === 'timeline') {
          lines.push(
            `<!-- ${block.type} -->`,
            ...block.entries.map((entry) => `- ${entry.title}${entry.text ? ` | ${entry.text}` : ''}`),
          )
        }
      }
      if (slide.notes) lines.push('', `<!-- notes: ${slide.notes} -->`)
      return lines.join('\n')
    })
    .join('\n\n---\n\n')
}

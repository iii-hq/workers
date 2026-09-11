export const LAYOUTS = ['title', 'section', 'content', 'two-column', 'statement', 'image', 'blank'] as const
export type Layout = (typeof LAYOUTS)[number]
export const BLOCK_TYPES = ['heading', 'text', 'bullets', 'image', 'code', 'quote', 'metric'] as const
export type BlockType = (typeof BLOCK_TYPES)[number]
export type Column = 'left' | 'right'

export type Block = {
  id: string
  type: BlockType
  text?: string
  items?: string[]
  src?: string
  alt?: string
  caption?: string
  code?: string
  language?: string
  attribution?: string
  value?: string
  label?: string
  column?: Column
}

export type Slide = {
  id: string
  layout: Layout
  title?: string
  subtitle?: string
  blocks: Block[]
  notes?: string
  background?: string
}

export type ThemeOverrides = {
  accent?: string
  background?: string
  ink?: string
  font_heading?: string
  font_body?: string
  footer?: string
}

export type Deck = {
  id: string
  title: string
  subtitle?: string
  author?: string
  theme: string
  theme_overrides?: ThemeOverrides
  slides: Slide[]
  revision: number
  created_at_ms: number
  updated_at_ms: number
}

export type DeckSummary = {
  id: string
  title: string
  subtitle?: string
  theme: string
  slide_count: number
  revision: number
  updated_at_ms: number
}

export type Theme = {
  id: string
  name: string
  description: string
  dark: boolean
  colors: { background: string; surface: string; ink: string; muted: string; accent: string; accent_ink: string }
  fonts: { heading: string; body: string; mono: string }
}

export type ChangedEvent = {
  kind: 'created' | 'updated' | 'deleted'
  deck_id: string
  revision: number
  updated_at_ms: number
}

export type ExportFormat = 'html' | 'pdf' | 'pptx'

export const LAYOUT_LABEL: Record<Layout, string> = {
  title: 'Title',
  section: 'Section',
  content: 'Content',
  'two-column': 'Two columns',
  statement: 'Statement',
  image: 'Full image',
  blank: 'Blank',
}

export const BLOCK_LABEL: Record<BlockType, string> = {
  heading: 'Heading',
  text: 'Text',
  bullets: 'Bullets',
  image: 'Image',
  code: 'Code',
  quote: 'Quote',
  metric: 'Metric',
}

export function newId(prefix: string): string {
  return `${prefix}-${globalThis.crypto.randomUUID().slice(0, 8)}`
}

export function describeError(cause: unknown): string {
  if (cause instanceof Error) return cause.message.replace(/^handler error:\s*/i, '')
  if (cause && typeof cause === 'object') {
    const message = (cause as { message?: unknown }).message
    if (typeof message === 'string') return message.replace(/^handler error:\s*/i, '')
    try {
      return JSON.stringify(cause)
    } catch {
      return String(cause)
    }
  }
  return String(cause)
}

export function relativeTime(timestamp: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000))
  if (seconds < 10) return 'now'
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  return `${Math.floor(hours / 24)}d ago`
}

export function defaultBlock(type: BlockType): Block {
  const id = newId('block')
  switch (type) {
    case 'heading':
      return { id, type, text: 'Heading' }
    case 'text':
      return { id, type, text: 'Say one clear thing here.' }
    case 'bullets':
      return { id, type, items: ['First point', 'Second point', 'Third point'] }
    case 'image':
      return { id, type, src: 'https://', alt: '' }
    case 'code':
      return { id, type, code: 'const answer = 42', language: 'ts' }
    case 'quote':
      return { id, type, text: 'A memorable sentence.', attribution: '' }
    case 'metric':
      return { id, type, value: '42%', label: 'What this number means' }
  }
}

export function defaultSlide(layout: Layout = 'content'): Slide {
  const id = newId('slide')
  if (layout === 'title')
    return { id, layout, title: 'Deck title', subtitle: 'One line on what this is about', blocks: [] }
  if (layout === 'section') return { id, layout, title: 'Section', blocks: [] }
  if (layout === 'statement') return { id, layout, title: 'One bold sentence that carries the slide.', blocks: [] }
  if (layout === 'two-column')
    return {
      id,
      layout,
      title: 'Two things side by side',
      blocks: [
        { ...defaultBlock('metric'), column: 'left' },
        { ...defaultBlock('bullets'), column: 'right' },
      ],
    }
  if (layout === 'image') return { id, layout, title: 'Caption for the image', blocks: [defaultBlock('image')] }
  if (layout === 'blank') return { id, layout, blocks: [] }
  return { id, layout, title: 'The takeaway, as a sentence', blocks: [defaultBlock('bullets')] }
}

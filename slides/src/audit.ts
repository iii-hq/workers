import type { SlideMeasure } from './capture.js'
import type { Block, Deck, Slide } from './model.js'

export const MIN_FONT_PX = 18
export const MAX_WORDS_PER_SLIDE = 120
export const MAX_BULLETS = 6
export const MAX_EMPTY_SPACE = 0.72

export interface Finding {
  code: string
  severity: 'error' | 'warning' | 'info'
  message: string
  block_id?: string
}

export interface SlideAudit {
  slide_id: string
  index: number
  title?: string
  layout: string
  word_count: number
  block_count: number
  measured: boolean
  overflow: boolean
  minimum_font_size: number | null
  empty_space_ratio: number | null
  collisions: { a: string; b: string }[]
  clipped_labels: string[]
  repeated_words: { word: string; count: number }[]
  findings: Finding[]
}

export interface DeckAudit {
  deck_id: string
  revision: number
  measured: boolean
  measurement_error?: string
  slides: SlideAudit[]
  error_count: number
  warning_count: number
}

export function blockText(block: Block): string[] {
  switch (block.type) {
    case 'heading':
    case 'text':
    case 'quote':
      return [block.text, ...(block.type === 'quote' && block.attribution ? [block.attribution] : [])]
    case 'bullets':
      return block.items
    case 'image':
      return [block.alt ?? '', block.caption ?? '']
    case 'code':
      return []
    case 'metric':
      return [block.value, block.label]
    case 'cards':
    case 'steps':
    case 'timeline':
      return block.entries.flatMap((entry) => [entry.title, entry.text ?? ''])
    case 'chart':
      return [block.title ?? '', ...block.series.map((point) => point.label)]
    case 'table':
      return [...block.columns, ...block.rows.flat()]
    case 'diagram':
      return [block.title ?? '', ...block.nodes.flatMap((node) => [node.label, node.text ?? ''])]
  }
}

const STOP_WORDS = new Set([
  'the',
  'and',
  'that',
  'with',
  'from',
  'this',
  'into',
  'your',
  'have',
  'more',
  'than',
  'when',
  'what',
  'will',
  'each',
  'over',
  'them',
  'they',
  'their',
  'about',
  'where',
  'which',
  'while',
  'these',
  'those',
  'there',
  'through',
])

export function repeatedWords(content: string): { word: string; count: number }[] {
  const counts = new Map<string, number>()
  for (const word of content.toLowerCase().match(/[a-z][a-z'-]{3,}/g) ?? []) {
    if (STOP_WORDS.has(word)) continue
    counts.set(word, (counts.get(word) ?? 0) + 1)
  }
  return [...counts.entries()]
    .filter(([, count]) => count >= 3)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8)
    .map(([word, count]) => ({ word, count }))
}

export function auditSlideContent(slide: Slide, index: number): SlideAudit {
  const findings: Finding[] = []
  const parts = [slide.kicker ?? '', slide.title ?? '', slide.subtitle ?? '', ...slide.blocks.flatMap(blockText)]
  const content = parts.filter(Boolean).join(' ')
  const word_count = content.split(/\s+/).filter(Boolean).length
  const repeated = repeatedWords(content)
  if (!slide.notes?.trim() && slide.layout !== 'section' && slide.layout !== 'blank')
    findings.push({ code: 'missing_notes', severity: 'info', message: 'No speaker notes.' })
  if (word_count > MAX_WORDS_PER_SLIDE)
    findings.push({
      code: 'too_many_words',
      severity: 'warning',
      message: `${word_count} words on one slide; aim for under ${MAX_WORDS_PER_SLIDE}.`,
    })
  if (!slide.title && !slide.blocks.length && slide.layout !== 'blank')
    findings.push({ code: 'empty_slide', severity: 'error', message: 'The slide has no title and no blocks.' })
  for (const block of slide.blocks) {
    if (block.type === 'bullets' && block.items.length > MAX_BULLETS)
      findings.push({
        code: 'too_many_bullets',
        severity: 'warning',
        message: `${block.items.length} bullets; split the list or use cards.`,
        block_id: block.id,
      })
    if (block.type === 'image' && !block.alt)
      findings.push({ code: 'missing_alt', severity: 'info', message: 'Image without alt text.', block_id: block.id })
    if (block.type === 'chart' && block.series.length < 2)
      findings.push({
        code: 'thin_chart',
        severity: 'warning',
        message: 'A chart with fewer than two points reads better as a metric.',
        block_id: block.id,
      })
    if (block.type === 'diagram' && block.nodes.length < 2)
      findings.push({
        code: 'thin_diagram',
        severity: 'warning',
        message: 'A diagram needs at least two nodes.',
        block_id: block.id,
      })
    if (block.type === 'table' && block.rows.some((row) => row.length !== block.columns.length))
      findings.push({
        code: 'ragged_table',
        severity: 'error',
        message: 'Some table rows do not match the column count.',
        block_id: block.id,
      })
  }
  if (repeated.length)
    findings.push({
      code: 'repeated_words',
      severity: 'info',
      message: `Repeated: ${repeated.map((entry) => `${entry.word} (${entry.count})`).join(', ')}.`,
    })
  return {
    slide_id: slide.id,
    index,
    ...(slide.title ? { title: slide.title } : {}),
    layout: slide.layout,
    word_count,
    block_count: slide.blocks.length,
    measured: false,
    overflow: false,
    minimum_font_size: null,
    empty_space_ratio: null,
    collisions: [],
    clipped_labels: [],
    repeated_words: repeated,
    findings,
  }
}

export function applyMeasure(audit: SlideAudit, measure: SlideMeasure): SlideAudit {
  const findings = [...audit.findings]
  if (measure.overflow) {
    const blocks = measure.blocks.filter((block) => block.overflow)
    findings.push({
      code: 'overflow',
      severity: 'error',
      message: `Content overflows the slide by ${measure.overflow_px}px after auto-fit${blocks.length ? ` (${blocks.map((block) => block.block_id).join(', ')})` : ''}.`,
      ...(blocks[0] ? { block_id: blocks[0].block_id } : {}),
    })
  }
  if (measure.fit < 1)
    findings.push({
      code: 'auto_fit',
      severity: measure.fit <= 0.76 ? 'warning' : 'info',
      message: `Auto-fit shrank the body to ${Math.round(measure.fit * 100)}%.`,
    })
  if (measure.minimum_font_size > 0 && measure.minimum_font_size < MIN_FONT_PX)
    findings.push({
      code: 'small_text',
      severity: 'warning',
      message: `Smallest text is ${measure.minimum_font_size}px; keep body text at or above ${MIN_FONT_PX}px.`,
    })
  for (const pair of measure.collisions)
    findings.push({
      code: 'collision',
      severity: 'error',
      message: `Blocks ${pair.a} and ${pair.b} overlap.`,
      block_id: pair.a,
    })
  if (measure.clipped_labels.length)
    findings.push({
      code: 'clipped_labels',
      severity: 'warning',
      message: `Clipped: ${measure.clipped_labels.join(' | ')}.`,
    })
  if (measure.empty_space_ratio > MAX_EMPTY_SPACE && audit.layout !== 'section' && audit.layout !== 'title')
    findings.push({
      code: 'sparse_slide',
      severity: 'info',
      message: `${Math.round(measure.empty_space_ratio * 100)}% of the body is empty; consider a statement layout or a visual.`,
    })
  return {
    ...audit,
    measured: true,
    overflow: measure.overflow,
    minimum_font_size: measure.minimum_font_size || null,
    empty_space_ratio: measure.empty_space_ratio,
    collisions: measure.collisions,
    clipped_labels: measure.clipped_labels,
    findings,
  }
}

export function auditDeck(deck: Deck, measures?: SlideMeasure[], measurementError?: string): DeckAudit {
  const byIndex = new Map((measures ?? []).map((measure) => [measure.index, measure]))
  const slides = deck.slides.map((slide, index) => {
    const audit = auditSlideContent(slide, index)
    const measure = byIndex.get(index)
    return measure ? applyMeasure(audit, measure) : audit
  })
  const all = slides.flatMap((slide) => slide.findings)
  return {
    deck_id: deck.id,
    revision: deck.revision,
    measured: Boolean(measures?.length),
    ...(measurementError ? { measurement_error: measurementError } : {}),
    slides,
    error_count: all.filter((finding) => finding.severity === 'error').length,
    warning_count: all.filter((finding) => finding.severity === 'warning').length,
  }
}

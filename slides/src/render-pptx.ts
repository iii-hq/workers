import pptxgen from 'pptxgenjs'
import type { Block, Deck, Slide, ThemeOverrides } from './model.js'
import { type ResolvedTheme, resolveTheme } from './themes.js'

const W = 10
const H = 5.625
const MX = 0.6
const MT = 0.45
const FOOTER_Y = H - 0.38
const FETCH_TIMEOUT_MS = 8_000

type PptxGenJSClass = typeof pptxgen extends { default: infer D } ? D : typeof pptxgen
const PptxGenJS = ((pptxgen as unknown as { default?: unknown }).default ?? pptxgen) as PptxGenJSClass
type Pptx = InstanceType<PptxGenJSClass>
type PptxSlide = ReturnType<Pptx['addSlide']>

function hex(value: string): string {
  const clean = value.trim().replace('#', '')
  return clean.length === 3
    ? clean
        .split('')
        .map((c) => c + c)
        .join('')
        .toUpperCase()
    : clean.toUpperCase()
}

function plain(text: string): string {
  return text.replace(/\*\*(.+?)\*\*/g, '$1').replace(/`([^`]+)`/g, '$1')
}

async function imageData(src: string, cache: Map<string, string | null>): Promise<string | null> {
  if (cache.has(src)) return cache.get(src) ?? null
  let data: string | null = null
  try {
    const inline = src.match(/^data:(image\/(png|jpeg|jpg|gif));base64,(.+)$/i)
    if (inline) data = `${inline[1].toLowerCase()};base64,${inline[3]}`
    else if (/^https?:\/\//i.test(src)) {
      const response = await fetch(src, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) })
      const type = (response.headers.get('content-type') ?? '').toLowerCase()
      if (response.ok && /image\/(png|jpeg|jpg|gif)/.test(type)) {
        const mime = type.split(';')[0].trim()
        data = `${mime};base64,${Buffer.from(await response.arrayBuffer()).toString('base64')}`
      }
    }
  } catch {
    data = null
  }
  cache.set(src, data)
  return data
}

interface Box {
  x: number
  y: number
  w: number
  bottom: number
}

interface Painter {
  pptx: Pptx
  slide: PptxSlide
  theme: ResolvedTheme
  images: Map<string, string | null>
}

function estimateLines(text: string, fontSize: number, widthIn: number): number {
  const charsPerLine = Math.max(8, Math.floor((widthIn * 72) / (fontSize * 0.5)))
  return text.split('\n').reduce((sum, line) => sum + Math.max(1, Math.ceil(line.length / charsPerLine)), 0)
}

function textHeight(text: string, fontSize: number, widthIn: number, lineHeight = 1.3): number {
  return (estimateLines(text, fontSize, widthIn) * fontSize * lineHeight) / 72 + 0.08
}

async function drawBlock(p: Painter, block: Block, box: Box, scale = 1): Promise<number> {
  const { slide, theme } = p
  const c = theme.colors
  const ink = hex(c.ink)
  const muted = hex(c.muted)
  const accent = hex(c.accent)
  const available = box.bottom - box.y
  if (available <= 0.2) return box.y
  switch (block.type) {
    case 'heading': {
      const size = 20 * scale
      const h = Math.min(available, textHeight(plain(block.text), size, box.w, 1.2))
      slide.addText(plain(block.text), {
        x: box.x,
        y: box.y,
        w: box.w,
        h,
        fontSize: size,
        fontFace: theme.fonts.heading,
        color: ink,
        bold: true,
        valign: 'top',
        margin: 0,
      })
      return box.y + h + 0.05
    }
    case 'text': {
      const size = 15 * scale
      const h = Math.min(available, textHeight(plain(block.text), size, box.w))
      slide.addText(plain(block.text), {
        x: box.x,
        y: box.y,
        w: box.w,
        h,
        fontSize: size,
        fontFace: theme.fonts.body,
        color: ink,
        valign: 'top',
        margin: 0,
      })
      return box.y + h + 0.08
    }
    case 'bullets': {
      const size = 15 * scale
      const text = block.items.map(plain).join('\n')
      const h = Math.min(available, textHeight(text, size, box.w - 0.3, 1.45))
      slide.addText(
        block.items.map((item) => ({ text: plain(item), options: { bullet: { code: '25C6' }, breakLine: true } })),
        {
          x: box.x,
          y: box.y,
          w: box.w,
          h,
          fontSize: size,
          fontFace: theme.fonts.body,
          color: ink,
          valign: 'top',
          paraSpaceAfter: 6,
          margin: 0,
        },
      )
      return box.y + h + 0.08
    }
    case 'image': {
      const data = await imageData(block.src, p.images)
      const captionH = block.caption ? 0.3 : 0
      const h = Math.max(0.5, available - captionH - 0.1)
      if (data) slide.addImage({ data, x: box.x, y: box.y, w: box.w, h, sizing: { type: 'contain', w: box.w, h } })
      else {
        slide.addShape(p.pptx.ShapeType.rect, {
          x: box.x,
          y: box.y,
          w: box.w,
          h: Math.min(h, 1.6),
          line: { color: muted, width: 1, dashType: 'dash' },
        })
        slide.addText('Image unavailable', {
          x: box.x,
          y: box.y,
          w: box.w,
          h: Math.min(h, 1.6),
          fontSize: 12,
          fontFace: theme.fonts.body,
          color: muted,
          align: 'center',
          valign: 'middle',
        })
      }
      let y = box.y + (data ? h : Math.min(h, 1.6)) + 0.05
      if (block.caption) {
        slide.addText(plain(block.caption), {
          x: box.x,
          y,
          w: box.w,
          h: captionH,
          fontSize: 11,
          fontFace: theme.fonts.body,
          color: muted,
          margin: 0,
        })
        y += captionH
      }
      return y + 0.05
    }
    case 'code': {
      const size = 11 * scale
      const h = Math.min(available, textHeight(block.code, size, box.w - 0.4, 1.4) + 0.3)
      slide.addShape(p.pptx.ShapeType.roundRect, {
        x: box.x,
        y: box.y,
        w: box.w,
        h,
        fill: { color: hex(c.surface) },
        line: { color: accent, width: 0.5 },
        rectRadius: 0.08,
      })
      slide.addText(block.code, {
        x: box.x + 0.15,
        y: box.y + 0.12,
        w: box.w - 0.3,
        h: h - 0.24,
        fontSize: size,
        fontFace: theme.fonts.mono,
        color: ink,
        valign: 'top',
        margin: 0,
      })
      return box.y + h + 0.1
    }
    case 'quote': {
      const size = 20 * scale
      const attributionH = block.attribution ? 0.32 : 0
      const h = Math.min(available, textHeight(plain(block.text), size, box.w - 0.4, 1.3) + attributionH)
      slide.addShape(p.pptx.ShapeType.rect, {
        x: box.x,
        y: box.y,
        w: 0.07,
        h,
        fill: { color: accent },
        line: { color: accent, width: 0 },
      })
      slide.addText(plain(block.text), {
        x: box.x + 0.3,
        y: box.y,
        w: box.w - 0.3,
        h: h - attributionH,
        fontSize: size,
        fontFace: theme.fonts.heading,
        color: ink,
        italic: true,
        valign: 'top',
        margin: 0,
      })
      if (block.attribution) {
        slide.addText(plain(block.attribution), {
          x: box.x + 0.3,
          y: box.y + h - attributionH,
          w: box.w - 0.3,
          h: attributionH,
          fontSize: 12,
          fontFace: theme.fonts.body,
          color: muted,
          margin: 0,
        })
      }
      return box.y + h + 0.1
    }
    case 'metric': {
      const h = Math.min(available, 1.35)
      const w = Math.min(box.w, Math.max(2.2, block.value.length * 0.42 + 0.6))
      slide.addShape(p.pptx.ShapeType.roundRect, {
        x: box.x,
        y: box.y,
        w,
        h,
        fill: { color: hex(c.surface) },
        line: { color: accent, width: 0.5 },
        rectRadius: 0.08,
      })
      slide.addText(plain(block.value), {
        x: box.x + 0.2,
        y: box.y + 0.1,
        w: w - 0.4,
        h: h * 0.62,
        fontSize: 40 * scale,
        fontFace: theme.fonts.heading,
        color: accent,
        bold: true,
        valign: 'middle',
        margin: 0,
        fit: 'shrink',
      })
      slide.addText(plain(block.label), {
        x: box.x + 0.2,
        y: box.y + h * 0.68,
        w: w - 0.4,
        h: h * 0.28,
        fontSize: 12,
        fontFace: theme.fonts.body,
        color: muted,
        valign: 'top',
        margin: 0,
      })
      return box.y + h + 0.12
    }
    default:
      return box.y
  }
}

async function drawBlocks(p: Painter, blocks: Block[], box: Box, scale = 1): Promise<number> {
  let y = box.y
  for (const block of blocks) {
    if (y >= box.bottom) break
    y = await drawBlock(p, block, { ...box, y }, scale)
  }
  return y
}

function splitColumns(blocks: Block[]): [Block[], Block[]] {
  const left: Block[] = []
  const right: Block[] = []
  for (const [index, block] of blocks.entries()) {
    ;((block.column ?? (index % 2 === 0 ? 'left' : 'right')) === 'left' ? left : right).push(block)
  }
  return [left, right]
}

async function drawSlide(p: Painter, slide: Slide, index: number, total: number): Promise<void> {
  const { slide: s, theme } = p
  const c = theme.colors
  const ink = hex(c.ink)
  const muted = hex(c.muted)
  const accent = hex(c.accent)
  const background =
    slide.background && /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(slide.background) ? slide.background : c.background
  s.background = { color: hex(background) }
  const w = W - MX * 2
  const full: Box = { x: MX, y: MT, w, bottom: FOOTER_Y - 0.15 }
  const heading = theme.fonts.heading
  const body = theme.fonts.body

  if (slide.layout === 'title') {
    s.addShape(p.pptx.ShapeType.rect, {
      x: MX,
      y: 1.55,
      w: 0.9,
      h: 0.09,
      fill: { color: accent },
      line: { color: accent, width: 0 },
    })
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y: 1.75,
      w,
      h: 1.6,
      fontSize: 44,
      fontFace: heading,
      color: ink,
      bold: true,
      valign: 'top',
      margin: 0,
      fit: 'shrink',
    })
    if (slide.subtitle)
      s.addText(plain(slide.subtitle), {
        x: MX,
        y: 3.4,
        w,
        h: 0.8,
        fontSize: 20,
        fontFace: body,
        color: muted,
        valign: 'top',
        margin: 0,
      })
    await drawBlocks(p, slide.blocks, { ...full, y: 4.25 })
  } else if (slide.layout === 'section') {
    s.addText(String(index + 1).padStart(2, '0'), {
      x: MX,
      y: 1.0,
      w: 3,
      h: 1.3,
      fontSize: 72,
      fontFace: heading,
      color: accent,
      bold: true,
      margin: 0,
    })
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y: 2.35,
      w,
      h: 1.1,
      fontSize: 38,
      fontFace: heading,
      color: ink,
      bold: true,
      valign: 'top',
      margin: 0,
      fit: 'shrink',
    })
    if (slide.subtitle)
      s.addText(plain(slide.subtitle), {
        x: MX,
        y: 3.5,
        w,
        h: 0.6,
        fontSize: 18,
        fontFace: body,
        color: muted,
        margin: 0,
      })
    await drawBlocks(p, slide.blocks, { ...full, y: 4.15 })
  } else if (slide.layout === 'statement') {
    const quote = slide.blocks.find((block) => block.type === 'quote')
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y: 1.2,
      w,
      h: 1.5,
      fontSize: 36,
      fontFace: heading,
      color: ink,
      bold: true,
      align: 'center',
      valign: 'middle',
      margin: 0,
      fit: 'shrink',
    })
    if (quote && quote.type === 'quote') {
      s.addText(plain(quote.text), {
        x: MX + 0.5,
        y: 2.75,
        w: w - 1,
        h: 1.3,
        fontSize: 24,
        fontFace: heading,
        color: ink,
        italic: true,
        align: 'center',
        valign: 'top',
        margin: 0,
      })
      if (quote.attribution)
        s.addText(`- ${plain(quote.attribution)}`, {
          x: MX,
          y: 4.1,
          w,
          h: 0.4,
          fontSize: 14,
          fontFace: body,
          color: muted,
          align: 'center',
          margin: 0,
        })
    } else if (slide.subtitle)
      s.addText(plain(slide.subtitle), {
        x: MX,
        y: 2.8,
        w,
        h: 0.8,
        fontSize: 18,
        fontFace: body,
        color: muted,
        align: 'center',
        margin: 0,
      })
  } else if (slide.layout === 'image') {
    const image = slide.blocks.find((block) => block.type === 'image')
    const data = image && image.type === 'image' ? await imageData(image.src, p.images) : null
    if (data) {
      s.addImage({ data, x: 0, y: 0, w: W, h: H, sizing: { type: 'cover', w: W, h: H } })
      s.addShape(p.pptx.ShapeType.rect, {
        x: 0,
        y: H - 1.7,
        w: W,
        h: 1.7,
        fill: { color: '000000', transparency: 45 },
        line: { color: '000000', width: 0 },
      })
    }
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y: H - 1.5,
      w,
      h: 0.7,
      fontSize: 28,
      fontFace: heading,
      color: data ? 'FFFFFF' : ink,
      bold: true,
      valign: 'bottom',
      margin: 0,
      fit: 'shrink',
    })
    if (slide.subtitle)
      s.addText(plain(slide.subtitle), {
        x: MX,
        y: H - 0.8,
        w,
        h: 0.4,
        fontSize: 15,
        fontFace: body,
        color: data ? 'E6E6E6' : muted,
        margin: 0,
      })
    if (!data)
      await drawBlocks(
        p,
        slide.blocks.filter((block) => block !== image),
        { ...full, y: MT, bottom: H - 1.6 },
      )
  } else {
    let y = MT
    if (slide.title) {
      const h = Math.min(1.1, textHeight(plain(slide.title), 28, w, 1.15))
      s.addText(plain(slide.title), {
        x: MX,
        y,
        w,
        h,
        fontSize: 28,
        fontFace: heading,
        color: ink,
        bold: true,
        valign: 'top',
        margin: 0,
        fit: 'shrink',
      })
      y += h + 0.02
    }
    if (slide.subtitle) {
      const h = Math.min(0.6, textHeight(plain(slide.subtitle), 15, w))
      s.addText(plain(slide.subtitle), {
        x: MX,
        y,
        w,
        h,
        fontSize: 15,
        fontFace: body,
        color: muted,
        valign: 'top',
        margin: 0,
      })
      y += h
    }
    if (slide.title || slide.subtitle) {
      s.addShape(p.pptx.ShapeType.rect, {
        x: MX,
        y: y + 0.05,
        w: 0.6,
        h: 0.04,
        fill: { color: accent },
        line: { color: accent, width: 0 },
      })
      y += 0.3
    }
    const twoColumns = slide.layout === 'two-column' || slide.blocks.some((block) => block.column)
    if (twoColumns) {
      const [left, right] = splitColumns(slide.blocks)
      const cw = (w - 0.5) / 2
      await drawBlocks(p, left, { ...full, y, w: cw }, 0.9)
      await drawBlocks(p, right, { ...full, y, x: MX + cw + 0.5, w: cw }, 0.9)
    } else await drawBlocks(p, slide.blocks, { ...full, y })
  }

  if (theme.footer)
    s.addText(plain(theme.footer), {
      x: MX,
      y: FOOTER_Y,
      w: w / 2,
      h: 0.3,
      fontSize: 9,
      fontFace: body,
      color: muted,
      margin: 0,
    })
  s.addText(`${index + 1} / ${total}`, {
    x: W - MX - 2,
    y: FOOTER_Y,
    w: 2,
    h: 0.3,
    fontSize: 9,
    fontFace: body,
    color: muted,
    align: 'right',
    margin: 0,
  })
  if (slide.notes) s.addNotes(slide.notes)
}

export async function renderDeckPptx(deck: Deck, brand?: ThemeOverrides): Promise<Buffer> {
  const theme = resolveTheme(deck, brand)
  const pptx = new PptxGenJS()
  pptx.layout = 'LAYOUT_16x9'
  pptx.title = deck.title
  if (deck.subtitle) pptx.subject = deck.subtitle
  if (deck.author) pptx.author = deck.author
  pptx.company = 'iii'
  const images = new Map<string, string | null>()
  const slides = deck.slides.length
    ? deck.slides
    : [{ id: 'empty', layout: 'title' as const, title: deck.title, blocks: [] }]
  for (const [index, slide] of slides.entries()) {
    await drawSlide({ pptx, slide: pptx.addSlide(), theme, images }, slide, index, slides.length)
  }
  const output = await pptx.write({ outputType: 'nodebuffer' })
  return Buffer.isBuffer(output) ? output : Buffer.from(output as ArrayBuffer)
}

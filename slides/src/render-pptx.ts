import pptxgen from 'pptxgenjs'
import type { Block, Deck, Entry, Slide, ThemeOverrides } from './model.js'
import { gridColumns } from './render-html.js'
import { paletteFor } from './render-pdf.js'
import { mix, type ResolvedTheme, resolveTheme } from './themes.js'

const W = 10
const H = 5.625
const MX = 0.6
const MT = 0.42
const FOOTER_Y = H - 0.36
const FETCH_TIMEOUT_MS = 8_000

type PptxGenJSClass = typeof pptxgen extends { default: infer D } ? D : typeof pptxgen
const PptxGenJS = ((pptxgen as unknown as { default?: unknown }).default ?? pptxgen) as PptxGenJSClass
type Pptx = InstanceType<PptxGenJSClass>
type PptxSlide = ReturnType<Pptx['addSlide']>
type Palette = ReturnType<typeof paletteFor>

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
  palette: Palette
  images: Map<string, string | null>
}

function estimateLines(text: string, fontSize: number, widthIn: number): number {
  const charsPerLine = Math.max(6, Math.floor((widthIn * 72) / (fontSize * 0.52)))
  return text.split('\n').reduce((sum, line) => sum + Math.max(1, Math.ceil(line.length / charsPerLine)), 0)
}

function textHeight(text: string, fontSize: number, widthIn: number, lineHeight = 1.3): number {
  return (estimateLines(text, fontSize, widthIn) * fontSize * lineHeight) / 72 + 0.08
}

function panel(p: Painter, x: number, y: number, w: number, h: number) {
  p.slide.addShape(p.pptx.ShapeType.roundRect, {
    x,
    y,
    w,
    h,
    fill: { color: hex(p.palette.surface) },
    line: { color: hex(mix(p.palette.surface, p.palette.accent, 0.35)), width: 0.5 },
    rectRadius: 0.08,
  })
}

function entryText(p: Painter, entry: Entry, box: Box, titleSize: number, textSize: number, h: number) {
  const { slide, theme, palette } = p
  const titleH = Math.min(h * 0.55, textHeight(plain(entry.title), titleSize, box.w, 1.2))
  slide.addText(plain(entry.title), {
    x: box.x,
    y: box.y,
    w: box.w,
    h: titleH,
    fontSize: titleSize,
    fontFace: theme.fonts.heading,
    color: hex(palette.ink),
    bold: true,
    valign: 'top',
    margin: 0,
    fit: 'shrink',
  })
  if (entry.text) {
    const textH = Math.max(0.2, h - titleH)
    slide.addText(plain(entry.text), {
      x: box.x,
      y: box.y + titleH,
      w: box.w,
      h: textH,
      fontSize: textSize,
      fontFace: theme.fonts.body,
      color: hex(palette.muted),
      valign: 'top',
      margin: 0,
      fit: 'shrink',
    })
  }
}

async function drawBlock(p: Painter, block: Block, box: Box, scale = 1): Promise<number> {
  const { slide, theme, palette } = p
  const ink = hex(palette.ink)
  const muted = hex(palette.muted)
  const accent = hex(palette.accent)
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
      const size = (block.items.length > 5 ? 13 : 15) * scale
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
      panel(p, box.x, box.y, box.w, h)
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
      if (block.attribution)
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
      return box.y + h + 0.1
    }
    case 'metric': {
      const h = Math.min(available, 1.35)
      const w = Math.min(box.w, Math.max(2.2, block.value.length * 0.42 + 0.6))
      panel(p, box.x, box.y, w, h)
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
    case 'cards': {
      const count = block.entries.length
      if (!count) return box.y
      const cols = Math.min(gridColumns(count), count)
      const rows = Math.ceil(count / cols)
      const gap = 0.14
      const cw = (box.w - gap * (cols - 1)) / cols
      const ch = Math.max(0.55, Math.min(1.7, (available - gap * (rows - 1)) / rows))
      const dense = count > 6 || ch < 0.9
      for (const [index, entry] of block.entries.entries()) {
        const x = box.x + (index % cols) * (cw + gap)
        const y = box.y + Math.floor(index / cols) * (ch + gap)
        panel(p, x, y, cw, ch)
        let inner = y + 0.12
        if (block.numbered) {
          slide.addText(String(index + 1).padStart(2, '0'), {
            x: x + 0.15,
            y: inner,
            w: cw - 0.3,
            h: 0.2,
            fontSize: 8,
            fontFace: theme.fonts.mono,
            color: accent,
            margin: 0,
          })
          inner += 0.2
        }
        entryText(
          p,
          entry,
          { x: x + 0.15, y: inner, w: cw - 0.3, bottom: y + ch },
          (dense ? 11 : 14) * scale,
          (dense ? 9 : 11) * scale,
          y + ch - inner - 0.1,
        )
      }
      return box.y + rows * ch + (rows - 1) * gap + 0.12
    }
    case 'steps': {
      const count = block.entries.length
      if (!count) return box.y
      const perRow = Math.min(count, count > 4 ? Math.ceil(count / 2) : count)
      const rows = Math.ceil(count / perRow)
      const gap = 0.3
      const sw = (box.w - gap * (perRow - 1)) / perRow
      const sh = Math.max(0.7, Math.min(1.9, (available - 0.14 * (rows - 1)) / rows))
      for (const [index, entry] of block.entries.entries()) {
        const col = index % perRow
        const x = box.x + col * (sw + gap)
        const y = box.y + Math.floor(index / perRow) * (sh + 0.14)
        panel(p, x, y, sw, sh)
        slide.addShape(p.pptx.ShapeType.ellipse, {
          x: x + 0.15,
          y: y + 0.14,
          w: 0.34,
          h: 0.34,
          fill: { color: hex(mix(palette.surface, palette.accent, 0.25)) },
          line: { color: accent, width: 0 },
        })
        slide.addText(String(index + 1), {
          x: x + 0.15,
          y: y + 0.14,
          w: 0.34,
          h: 0.34,
          fontSize: 10,
          fontFace: theme.fonts.mono,
          color: accent,
          bold: true,
          align: 'center',
          valign: 'middle',
          margin: 0,
        })
        entryText(
          p,
          entry,
          { x: x + 0.15, y: y + 0.58, w: sw - 0.3, bottom: y + sh },
          (perRow > 4 ? 11 : 14) * scale,
          (perRow > 4 ? 9 : 11) * scale,
          sh - 0.7,
        )
        if (col < perRow - 1 && index < count - 1) {
          slide.addText('\u2192', {
            x: x + sw,
            y: y + sh / 2 - 0.15,
            w: gap,
            h: 0.3,
            fontSize: 14,
            fontFace: theme.fonts.heading,
            color: accent,
            align: 'center',
            valign: 'middle',
            margin: 0,
          })
        }
      }
      return box.y + rows * sh + (rows - 1) * 0.14 + 0.12
    }
    case 'timeline': {
      const count = block.entries.length
      if (!count) return box.y
      const gap = 0.2
      const w = (box.w - gap * (count - 1)) / count
      const lineY = box.y + 0.12
      slide.addShape(p.pptx.ShapeType.rect, {
        x: box.x + 0.1,
        y: lineY,
        w: box.w - 0.2,
        h: 0.03,
        fill: { color: hex(mix(palette.surface, palette.accent, 0.4)) },
        line: { color: accent, width: 0 },
      })
      const h = Math.max(0.6, Math.min(available - 0.4, 1.8))
      for (const [index, entry] of block.entries.entries()) {
        const x = box.x + index * (w + gap)
        slide.addShape(p.pptx.ShapeType.ellipse, {
          x: x + 0.02,
          y: lineY - 0.08,
          w: 0.2,
          h: 0.2,
          fill: { color: accent },
          line: { color: accent, width: 0 },
        })
        entryText(
          p,
          entry,
          { x, y: lineY + 0.24, w, bottom: lineY + 0.24 + h },
          (count > 4 ? 11 : 14) * scale,
          (count > 4 ? 9 : 11) * scale,
          h,
        )
      }
      return lineY + 0.24 + h + 0.1
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

function drawBackground(p: Painter, slide: Slide) {
  const { slide: s, palette, theme } = p
  if (slide.variant === 'gradient') {
    const end = mix(theme.colors.background, theme.colors.accent, theme.dark ? 0.35 : 0.18)
    s.background = { color: hex(palette.background) }
    s.addShape(p.pptx.ShapeType.rect, {
      x: 0,
      y: 0,
      w: W,
      h: H,
      fill: { type: 'solid', color: hex(palette.background) },
      line: { color: hex(palette.background), width: 0 },
    })
    const bands = 24
    for (let i = 0; i < bands; i += 1) {
      s.addShape(p.pptx.ShapeType.rect, {
        x: (W / bands) * i,
        y: 0,
        w: W / bands + 0.02,
        h: H,
        fill: { color: hex(mix(palette.background, end, i / (bands - 1))) },
        line: { color: hex(mix(palette.background, end, i / (bands - 1))), width: 0 },
      })
    }
  } else s.background = { color: hex(palette.background) }
  const glow =
    slide.variant === 'accent'
      ? mix(palette.background, '#ffffff', 0.18)
      : mix(palette.background, palette.accent, theme.dark ? 0.22 : 0.14)
  const big = slide.layout === 'title'
  const r = big ? 4.4 : 3
  s.addShape(p.pptx.ShapeType.ellipse, {
    x: W - r / 2 - (big ? 0.6 : 0.2),
    y: -r / 2 - (big ? 0.4 : 0.9),
    w: r,
    h: r,
    fill: { color: hex(glow), transparency: 45 },
    line: { color: hex(glow), width: 0 },
  })
  s.addShape(p.pptx.ShapeType.ellipse, {
    x: -1.2,
    y: H - 1.4,
    w: big ? 3 : 2.2,
    h: big ? 3 : 2.2,
    fill: { color: hex(mix(palette.background, palette.accent, 0.1)), transparency: 40 },
    line: { color: hex(palette.background), width: 0 },
  })
}

function drawKicker(
  p: Painter,
  slide: Slide,
  x: number,
  y: number,
  w: number,
  align: 'left' | 'center' = 'left',
): number {
  if (!slide.kicker) return y
  const { slide: s, theme, palette } = p
  const label = plain(slide.kicker).toUpperCase()
  if (align === 'left')
    s.addShape(p.pptx.ShapeType.rect, {
      x,
      y: y + 0.11,
      w: 0.22,
      h: 0.03,
      fill: { color: hex(palette.accent) },
      line: { color: hex(palette.accent), width: 0 },
    })
  s.addText(label, {
    x: align === 'left' ? x + 0.3 : x,
    y,
    w: align === 'left' ? w - 0.3 : w,
    h: 0.26,
    fontSize: 9,
    fontFace: theme.fonts.heading,
    color: hex(palette.accent),
    bold: true,
    charSpacing: 3,
    align,
    valign: 'middle',
    margin: 0,
  })
  return y + 0.34
}

async function drawSlide(
  p: Painter,
  slide: Slide,
  index: number,
  total: number,
  deck: Pick<Deck, 'author'>,
): Promise<void> {
  const { slide: s, theme, palette } = p
  const ink = hex(palette.ink)
  const muted = hex(palette.muted)
  const accent = hex(palette.accent)
  drawBackground(p, slide)
  const w = W - MX * 2
  const full: Box = { x: MX, y: MT, w, bottom: FOOTER_Y - 0.15 }
  const heading = theme.fonts.heading
  const body = theme.fonts.body

  if (slide.layout === 'title') {
    let y = 1.35
    y = drawKicker(p, slide, MX, y, w)
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y,
      w: w - 0.5,
      h: 1.75,
      fontSize: 46,
      fontFace: heading,
      color: ink,
      bold: true,
      valign: 'top',
      margin: 0,
      fit: 'shrink',
    })
    y += 1.8
    if (slide.subtitle) {
      s.addText(plain(slide.subtitle), {
        x: MX,
        y,
        w: w - 1,
        h: 0.7,
        fontSize: 19,
        fontFace: body,
        color: muted,
        valign: 'top',
        margin: 0,
        fit: 'shrink',
      })
      y += 0.75
    }
    s.addShape(p.pptx.ShapeType.rect, {
      x: MX,
      y: y + 0.08,
      w: 0.9,
      h: 0.08,
      fill: { color: accent },
      line: { color: accent, width: 0 },
    })
    if (deck.author)
      s.addText(plain(deck.author), {
        x: MX + 1.1,
        y: y - 0.04,
        w: 5,
        h: 0.32,
        fontSize: 13,
        fontFace: body,
        color: muted,
        valign: 'middle',
        margin: 0,
      })
    await drawBlocks(p, slide.blocks, { ...full, y: y + 0.45 })
  } else if (slide.layout === 'section') {
    s.addText(String(index + 1).padStart(2, '0'), {
      x: MX - 0.05,
      y: 0.75,
      w: 4,
      h: 1.6,
      fontSize: 88,
      fontFace: heading,
      color: accent,
      bold: true,
      margin: 0,
    })
    let y = drawKicker(p, slide, MX, 2.35, w)
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y,
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
    y += 1.15
    if (slide.subtitle)
      s.addText(plain(slide.subtitle), {
        x: MX,
        y,
        w: w - 1,
        h: 0.6,
        fontSize: 17,
        fontFace: body,
        color: muted,
        margin: 0,
        fit: 'shrink',
      })
    await drawBlocks(p, slide.blocks, { ...full, y: y + 0.7 })
  } else if (slide.layout === 'statement') {
    const quote = slide.blocks.find((block) => block.type === 'quote')
    let y = 1.0
    y = drawKicker(p, slide, MX, y, w, 'center')
    s.addText(plain(slide.title ?? ''), {
      x: MX + 0.3,
      y,
      w: w - 0.6,
      h: 1.7,
      fontSize: 36,
      fontFace: heading,
      color: ink,
      bold: true,
      align: 'center',
      valign: 'middle',
      margin: 0,
      fit: 'shrink',
    })
    y += 1.75
    if (quote && quote.type === 'quote') {
      s.addText(plain(quote.text), {
        x: MX + 0.5,
        y,
        w: w - 1,
        h: 1.2,
        fontSize: 24,
        fontFace: heading,
        color: ink,
        italic: true,
        align: 'center',
        valign: 'top',
        margin: 0,
        fit: 'shrink',
      })
      if (quote.attribution)
        s.addText(`- ${plain(quote.attribution)}`, {
          x: MX,
          y: y + 1.25,
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
        x: MX + 0.5,
        y,
        w: w - 1,
        h: 0.8,
        fontSize: 18,
        fontFace: body,
        color: muted,
        align: 'center',
        margin: 0,
        fit: 'shrink',
      })
  } else if (slide.layout === 'image') {
    const image = slide.blocks.find((block) => block.type === 'image')
    const data = image && image.type === 'image' ? await imageData(image.src, p.images) : null
    if (data) {
      s.addImage({ data, x: 0, y: 0, w: W, h: H, sizing: { type: 'cover', w: W, h: H } })
      s.addShape(p.pptx.ShapeType.rect, {
        x: 0,
        y: H - 1.8,
        w: W,
        h: 1.8,
        fill: { color: '000000', transparency: 40 },
        line: { color: '000000', width: 0 },
      })
    }
    s.addText(plain(slide.title ?? ''), {
      x: MX,
      y: H - 1.55,
      w,
      h: 0.75,
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
        { ...full, y: MT, bottom: H - 1.7 },
      )
  } else {
    let y = drawKicker(p, slide, MX, MT, w)
    if (slide.title) {
      const h = Math.min(1.0, textHeight(plain(slide.title), 27, w, 1.15))
      s.addText(plain(slide.title), {
        x: MX,
        y,
        w,
        h,
        fontSize: 27,
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
      const h = Math.min(0.55, textHeight(plain(slide.subtitle), 14, w))
      s.addText(plain(slide.subtitle), {
        x: MX,
        y,
        w,
        h,
        fontSize: 14,
        fontFace: body,
        color: muted,
        valign: 'top',
        margin: 0,
        fit: 'shrink',
      })
      y += h
    }
    if (slide.title || slide.subtitle) y += 0.18
    const twoColumns = slide.layout === 'two-column' || slide.blocks.some((block) => block.column)
    if (twoColumns) {
      const [left, right] = splitColumns(slide.blocks)
      const cw = (w - 0.45) / 2
      await drawBlocks(p, left, { ...full, y, w: cw }, 0.9)
      await drawBlocks(p, right, { ...full, y, x: MX + cw + 0.45, w: cw }, 0.9)
    } else await drawBlocks(p, slide.blocks, { ...full, y })
  }

  if (theme.footer)
    s.addText(plain(theme.footer).toUpperCase(), {
      x: MX,
      y: FOOTER_Y,
      w: w / 2,
      h: 0.28,
      fontSize: 8,
      fontFace: body,
      color: muted,
      charSpacing: 2,
      margin: 0,
    })
  s.addText(`${index + 1} / ${total}`, {
    x: W - MX - 2,
    y: FOOTER_Y,
    w: 2,
    h: 0.28,
    fontSize: 8,
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
    await drawSlide(
      { pptx, slide: pptx.addSlide(), theme, palette: paletteFor(theme, slide), images },
      slide,
      index,
      slides.length,
      deck,
    )
  }
  const output = await pptx.write({ outputType: 'nodebuffer' })
  return Buffer.isBuffer(output) ? output : Buffer.from(output as ArrayBuffer)
}

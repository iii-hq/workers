import { PDFDocument, type PDFFont, type PDFImage, type PDFPage, rgb, StandardFonts } from 'pdf-lib'
import type { Block, Deck, Entry, Slide, ThemeOverrides } from './model.js'
import { gridColumns } from './render-html.js'
import { hexToRgb, mix, type ResolvedTheme, resolveTheme } from './themes.js'

export const PAGE_WIDTH = 960
export const PAGE_HEIGHT = 540
const MARGIN_X = 64
const MARGIN_TOP = 52
const FOOTER_Y = 24
const FETCH_TIMEOUT_MS = 8_000

type Fonts = { heading: PDFFont; body: PDFFont; italic: PDFFont; mono: PDFFont }
type Color = ReturnType<typeof rgb>

interface Palette {
  background: string
  surface: string
  ink: string
  muted: string
  accent: string
}

function color(hex: string): Color {
  const [r, g, b] = hexToRgb(hex)
  return rgb(r / 255, g / 255, b / 255)
}

const REPLACEMENTS: Record<string, string> = {
  '\u2018': "'",
  '\u2019': "'",
  '\u201c': '"',
  '\u201d': '"',
  '\u2013': '-',
  '\u2014': '-',
  '\u2026': '...',
  '\u2022': '-',
  '\u2192': '->',
  '\u00a0': ' ',
}

export function sanitize(text: string): string {
  let out = ''
  for (const char of text.replace(/\*\*|`/g, '')) {
    const code = char.codePointAt(0) ?? 0
    if (REPLACEMENTS[char]) out += REPLACEMENTS[char]
    else if ((code >= 0x20 && code <= 0x7e) || (code >= 0xa1 && code <= 0xff) || char === '\n') out += char
  }
  return out
}

export function wrap(font: PDFFont, size: number, maxWidth: number, text: string): string[] {
  const lines: string[] = []
  for (const paragraph of sanitize(text).split('\n')) {
    const words = paragraph.split(/\s+/).filter(Boolean)
    if (!words.length) {
      lines.push('')
      continue
    }
    let line = ''
    for (const word of words) {
      const candidate = line ? `${line} ${word}` : word
      if (font.widthOfTextAtSize(candidate, size) <= maxWidth) line = candidate
      else {
        if (line) lines.push(line)
        line = word
        while (font.widthOfTextAtSize(line, size) > maxWidth && line.length > 1) {
          let cut = line.length - 1
          while (cut > 1 && font.widthOfTextAtSize(line.slice(0, cut), size) > maxWidth) cut -= 1
          lines.push(line.slice(0, cut))
          line = line.slice(cut)
        }
      }
    }
    if (line) lines.push(line)
  }
  return lines
}

interface Cursor {
  x: number
  y: number
  width: number
  bottom: number
}

function drawLines(
  page: PDFPage,
  lines: string[],
  cursor: Cursor,
  font: PDFFont,
  size: number,
  fill: Color,
  lineHeight = 1.35,
  align: 'left' | 'center' = 'left',
): number {
  let y = cursor.y
  for (const line of lines) {
    if (y - size < cursor.bottom) break
    const width = font.widthOfTextAtSize(line, size)
    const x = align === 'center' ? cursor.x + (cursor.width - width) / 2 : cursor.x
    page.drawText(line, { x, y: y - size, size, font, color: fill })
    y -= size * lineHeight
  }
  return y
}

function fitSize(
  font: PDFFont,
  text: string,
  maxWidth: number,
  maxSize: number,
  minSize: number,
  maxLines: number,
): number {
  let size = maxSize
  while (size > minSize && wrap(font, size, maxWidth, text).length > maxLines) size -= 2
  return size
}

async function loadImage(doc: PDFDocument, src: string, cache: Map<string, PDFImage | null>): Promise<PDFImage | null> {
  if (cache.has(src)) return cache.get(src) ?? null
  let image: PDFImage | null = null
  try {
    let bytes: Uint8Array | null = null
    let type = ''
    const data = src.match(/^data:(image\/(png|jpeg|jpg));base64,(.+)$/i)
    if (data) {
      type = data[1].toLowerCase()
      bytes = new Uint8Array(Buffer.from(data[3], 'base64'))
    } else if (/^https?:\/\//i.test(src)) {
      const response = await fetch(src, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) })
      if (response.ok) {
        type = (response.headers.get('content-type') ?? '').toLowerCase()
        bytes = new Uint8Array(await response.arrayBuffer())
      }
    }
    if (bytes) {
      const isPng = type.includes('png') || (bytes[0] === 0x89 && bytes[1] === 0x50)
      const isJpg = type.includes('jpeg') || type.includes('jpg') || (bytes[0] === 0xff && bytes[1] === 0xd8)
      if (isPng) image = await doc.embedPng(bytes)
      else if (isJpg) image = await doc.embedJpg(bytes)
    }
  } catch {
    image = null
  }
  cache.set(src, image)
  return image
}

interface Painter {
  doc: PDFDocument
  page: PDFPage
  theme: ResolvedTheme
  palette: Palette
  fonts: Fonts
  images: Map<string, PDFImage | null>
}

function panel(p: Painter, x: number, y: number, width: number, height: number) {
  p.page.drawRectangle({
    x,
    y,
    width,
    height,
    color: color(p.palette.surface),
    borderColor: color(mix(p.palette.surface, p.palette.accent, 0.35)),
    borderWidth: 0.6,
  })
}

function drawEntryText(p: Painter, entry: Entry, cursor: Cursor, titleSize: number, textSize: number): number {
  const ink = color(p.palette.ink)
  const muted = color(p.palette.muted)
  let y = drawLines(
    p.page,
    wrap(p.fonts.heading, titleSize, cursor.width, entry.title),
    cursor,
    p.fonts.heading,
    titleSize,
    ink,
    1.2,
  )
  if (entry.text)
    y = drawLines(
      p.page,
      wrap(p.fonts.body, textSize, cursor.width, entry.text),
      { ...cursor, y: y - 2 },
      p.fonts.body,
      textSize,
      muted,
      1.3,
    )
  return y
}

async function drawBlock(p: Painter, block: Block, cursor: Cursor, scale = 1): Promise<number> {
  const { page, palette, fonts } = p
  const ink = color(palette.ink)
  const muted = color(palette.muted)
  const accent = color(palette.accent)
  switch (block.type) {
    case 'heading':
      return (
        drawLines(
          page,
          wrap(fonts.heading, 22 * scale, cursor.width, block.text),
          cursor,
          fonts.heading,
          22 * scale,
          ink,
        ) - 6
      )
    case 'text':
      return (
        drawLines(page, wrap(fonts.body, 17 * scale, cursor.width, block.text), cursor, fonts.body, 17 * scale, ink) - 6
      )
    case 'bullets': {
      let y = cursor.y
      const size = (block.items.length > 5 ? 15 : 17) * scale
      for (const item of block.items) {
        if (y - size < cursor.bottom) break
        page.drawRectangle({ x: cursor.x + 2, y: y - size + 4, width: 7, height: 7, color: accent })
        const lines = wrap(fonts.body, size, cursor.width - 22, item)
        y = drawLines(page, lines, { ...cursor, x: cursor.x + 22, y }, fonts.body, size, ink) - 4
      }
      return y - 4
    }
    case 'image': {
      const image = await loadImage(p.doc, block.src, p.images)
      const available = cursor.y - cursor.bottom - (block.caption ? 24 : 0)
      if (available < 40) return cursor.y
      if (!image) {
        const h = Math.min(available, 160)
        page.drawRectangle({
          x: cursor.x,
          y: cursor.y - h,
          width: cursor.width,
          height: h,
          borderColor: muted,
          borderWidth: 1,
          borderDashArray: [6, 4],
        })
        page.drawText('Image unavailable', {
          x: cursor.x + 16,
          y: cursor.y - h / 2 - 6,
          size: 14,
          font: fonts.body,
          color: muted,
        })
        return cursor.y - h - 12
      }
      const fit = image.scaleToFit(cursor.width, available)
      page.drawImage(image, { x: cursor.x, y: cursor.y - fit.height, width: fit.width, height: fit.height })
      let y = cursor.y - fit.height - 8
      if (block.caption)
        y = drawLines(page, wrap(fonts.body, 12, cursor.width, block.caption), { ...cursor, y }, fonts.body, 12, muted)
      return y - 8
    }
    case 'code': {
      const size = 12 * scale
      const lines = sanitize(block.code)
        .split('\n')
        .flatMap((line) => wrap(fonts.mono, size, cursor.width - 32, line || ' '))
      const maxLines = Math.max(1, Math.floor((cursor.y - cursor.bottom - 24) / (size * 1.4)))
      const shown = lines.slice(0, maxLines)
      const height = shown.length * size * 1.4 + 24
      panel(p, cursor.x, cursor.y - height, cursor.width, height)
      drawLines(page, shown, { ...cursor, x: cursor.x + 16, y: cursor.y - 12 }, fonts.mono, size, ink, 1.4)
      return cursor.y - height - 10
    }
    case 'quote': {
      const size = 22 * scale
      const lines = wrap(fonts.italic, size, cursor.width - 28, block.text)
      const height = lines.length * size * 1.3 + (block.attribution ? 22 : 0)
      page.drawRectangle({ x: cursor.x, y: cursor.y - height, width: 5, height, color: accent })
      let y = drawLines(page, lines, { ...cursor, x: cursor.x + 24 }, fonts.italic, size, ink, 1.3)
      if (block.attribution)
        y = drawLines(
          page,
          [sanitize(block.attribution)],
          { ...cursor, x: cursor.x + 24, y: y - 2 },
          fonts.body,
          13,
          muted,
        )
      return y - 8
    }
    case 'metric': {
      const size = 48 * scale
      const value = sanitize(block.value)
      const width = Math.min(
        cursor.width,
        Math.max(
          170,
          fonts.heading.widthOfTextAtSize(value, size) + 40,
          fonts.body.widthOfTextAtSize(sanitize(block.label), 13) + 40,
        ),
      )
      const height = size + 44
      panel(p, cursor.x, cursor.y - height, width, height)
      page.drawText(value, { x: cursor.x + 20, y: cursor.y - size - 8, size, font: fonts.heading, color: accent })
      page.drawText(sanitize(block.label), {
        x: cursor.x + 20,
        y: cursor.y - height + 12,
        size: 13,
        font: fonts.body,
        color: muted,
      })
      return cursor.y - height - 12
    }
    case 'cards': {
      const count = block.entries.length
      if (!count) return cursor.y
      const cols = Math.min(gridColumns(count), count)
      const rows = Math.ceil(count / cols)
      const gap = 12
      const cardWidth = (cursor.width - gap * (cols - 1)) / cols
      const available = cursor.y - cursor.bottom
      const cardHeight = Math.max(48, Math.min(150, (available - gap * (rows - 1)) / rows))
      const dense = count > 6 || cardHeight < 80
      const titleSize = (dense ? 12 : 15) * scale
      const textSize = (dense ? 10 : 12) * scale
      for (const [index, entry] of block.entries.entries()) {
        const col = index % cols
        const row = Math.floor(index / cols)
        const x = cursor.x + col * (cardWidth + gap)
        const top = cursor.y - row * (cardHeight + gap)
        panel(p, x, top - cardHeight, cardWidth, cardHeight)
        let y = top - 12
        if (block.numbered) {
          page.drawText(String(index + 1).padStart(2, '0'), {
            x: x + 14,
            y: y - 9,
            size: 9,
            font: fonts.mono,
            color: accent,
          })
          y -= 16
        }
        drawEntryText(
          p,
          entry,
          { x: x + 14, y, width: cardWidth - 28, bottom: top - cardHeight + 8 },
          titleSize,
          textSize,
        )
      }
      return cursor.y - rows * cardHeight - (rows - 1) * gap - 12
    }
    case 'steps': {
      const count = block.entries.length
      if (!count) return cursor.y
      const perRow = Math.min(count, count > 4 ? Math.ceil(count / 2) : count)
      const rows = Math.ceil(count / perRow)
      const gap = 22
      const stepWidth = (cursor.width - gap * (perRow - 1)) / perRow
      const available = cursor.y - cursor.bottom
      const stepHeight = Math.max(60, Math.min(170, (available - 12 * (rows - 1)) / rows))
      const titleSize = (perRow > 4 ? 12 : 15) * scale
      const textSize = (perRow > 4 ? 10 : 12) * scale
      for (const [index, entry] of block.entries.entries()) {
        const col = index % perRow
        const row = Math.floor(index / perRow)
        const x = cursor.x + col * (stepWidth + gap)
        const top = cursor.y - row * (stepHeight + 12)
        panel(p, x, top - stepHeight, stepWidth, stepHeight)
        page.drawCircle({ x: x + 26, y: top - 24, size: 12, color: color(mix(palette.surface, palette.accent, 0.25)) })
        page.drawText(String(index + 1), {
          x: x + 26 - fonts.mono.widthOfTextAtSize(String(index + 1), 11) / 2,
          y: top - 28,
          size: 11,
          font: fonts.mono,
          color: accent,
        })
        drawEntryText(
          p,
          entry,
          { x: x + 14, y: top - 46, width: stepWidth - 28, bottom: top - stepHeight + 8 },
          titleSize,
          textSize,
        )
        if (col < perRow - 1 && index < count - 1) {
          page.drawText('>', {
            x: x + stepWidth + gap / 2 - 4,
            y: top - stepHeight / 2 - 6,
            size: 16,
            font: fonts.heading,
            color: accent,
          })
        }
      }
      return cursor.y - rows * stepHeight - (rows - 1) * 12 - 12
    }
    case 'timeline': {
      const count = block.entries.length
      if (!count) return cursor.y
      const gap = 16
      const width = (cursor.width - gap * (count - 1)) / count
      const lineY = cursor.y - 10
      page.drawRectangle({
        x: cursor.x + 8,
        y: lineY - 1,
        width: cursor.width - 16,
        height: 2,
        color: color(mix(palette.surface, palette.accent, 0.4)),
      })
      let lowest = lineY
      for (const [index, entry] of block.entries.entries()) {
        const x = cursor.x + index * (width + gap)
        page.drawCircle({ x: x + 8, y: lineY, size: 7, color: accent })
        const y = drawEntryText(
          p,
          entry,
          { x, y: lineY - 18, width, bottom: cursor.bottom },
          (count > 4 ? 12 : 15) * scale,
          (count > 4 ? 10 : 12) * scale,
        )
        lowest = Math.min(lowest, y)
      }
      return lowest - 10
    }
    default:
      return cursor.y
  }
}

async function drawBlocks(p: Painter, blocks: Block[], cursor: Cursor, scale = 1): Promise<number> {
  let y = cursor.y
  for (const block of blocks) {
    if (y <= cursor.bottom) break
    y = await drawBlock(p, block, { ...cursor, y }, scale)
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

export function paletteFor(theme: ResolvedTheme, slide: Slide): Palette {
  const c = theme.colors
  const background =
    slide.background && /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(slide.background) ? slide.background : c.background
  if (slide.variant === 'accent') {
    return {
      background: c.accent,
      surface: mix(c.accent, c.accent_ink, 0.14),
      ink: c.accent_ink,
      muted: mix(c.accent, c.accent_ink, 0.75),
      accent: c.accent_ink,
    }
  }
  if (slide.variant === 'muted')
    return {
      background: c.surface,
      surface: mix(c.surface, c.background, 0.6),
      ink: c.ink,
      muted: c.muted,
      accent: c.accent,
    }
  return { background, surface: c.surface, ink: c.ink, muted: c.muted, accent: c.accent }
}

function drawBackground(p: Painter, slide: Slide) {
  const { page, palette, theme } = p
  page.drawRectangle({ x: 0, y: 0, width: PAGE_WIDTH, height: PAGE_HEIGHT, color: color(palette.background) })
  if (slide.variant === 'gradient') {
    const end = mix(theme.colors.background, theme.colors.accent, theme.dark ? 0.35 : 0.18)
    const bands = 32
    for (let i = 0; i < bands; i += 1) {
      const x = (PAGE_WIDTH / bands) * i
      page.drawRectangle({
        x,
        y: 0,
        width: PAGE_WIDTH / bands + 1,
        height: PAGE_HEIGHT,
        color: color(mix(palette.background, end, i / (bands - 1))),
      })
    }
  }
  const glow =
    slide.variant === 'accent'
      ? mix(palette.background, '#ffffff', 0.18)
      : mix(palette.background, palette.accent, theme.dark ? 0.22 : 0.14)
  const big = slide.layout === 'title'
  page.drawCircle({
    x: PAGE_WIDTH - (big ? 80 : 40),
    y: PAGE_HEIGHT + (big ? 40 : 90),
    size: big ? 330 : 220,
    color: color(glow),
    opacity: 0.55,
  })
  page.drawCircle({
    x: PAGE_WIDTH - (big ? 80 : 40),
    y: PAGE_HEIGHT + (big ? 40 : 90),
    size: big ? 230 : 150,
    color: color(glow),
    opacity: 0.5,
  })
  page.drawCircle({
    x: -40,
    y: -60,
    size: big ? 220 : 150,
    color: color(mix(palette.background, palette.accent, 0.1)),
    opacity: 0.6,
  })
}

function drawKicker(
  p: Painter,
  slide: Slide,
  x: number,
  y: number,
  align: 'left' | 'center' = 'left',
  width = 0,
): number {
  if (!slide.kicker) return y
  const size = 10
  const label = sanitize(slide.kicker).toUpperCase()
  const textWidth = p.fonts.heading.widthOfTextAtSize(label, size)
  const startX = align === 'center' ? x + (width - textWidth - 20) / 2 : x
  p.page.drawRectangle({ x: startX, y: y - size + 3, width: 14, height: 2, color: color(p.palette.accent) })
  p.page.drawText(label, { x: startX + 20, y: y - size, size, font: p.fonts.heading, color: color(p.palette.accent) })
  return y - size - 12
}

async function drawSlide(
  p: Painter,
  slide: Slide,
  index: number,
  total: number,
  deck: Pick<Deck, 'author'>,
): Promise<void> {
  const { page, palette, fonts } = p
  const ink = color(palette.ink)
  const muted = color(palette.muted)
  const accent = color(palette.accent)
  drawBackground(p, slide)
  const width = PAGE_WIDTH - MARGIN_X * 2
  const full: Cursor = { x: MARGIN_X, y: PAGE_HEIGHT - MARGIN_TOP, width, bottom: FOOTER_Y + 28 }

  if (slide.layout === 'title') {
    const titleSize = fitSize(fonts.heading, slide.title ?? '', width, 56, 34, 3)
    const titleLines = wrap(fonts.heading, titleSize, width, slide.title ?? '')
    const subtitleLines = slide.subtitle ? wrap(fonts.body, 20, width - 80, slide.subtitle) : []
    const blockHeight =
      (slide.kicker ? 24 : 0) + titleLines.length * titleSize * 1.05 + subtitleLines.length * 20 * 1.35 + 44
    let y = PAGE_HEIGHT / 2 + blockHeight / 2
    y = drawKicker(p, slide, MARGIN_X, y)
    y = drawLines(page, titleLines, { ...full, y }, fonts.heading, titleSize, ink, 1.05) - 4
    y = drawLines(page, subtitleLines, { ...full, y }, fonts.body, 20, muted) - 10
    page.drawRectangle({ x: MARGIN_X, y: y - 4, width: 64, height: 5, color: accent })
    if (deck.author)
      page.drawText(sanitize(deck.author), { x: MARGIN_X + 80, y: y - 8, size: 13, font: fonts.body, color: muted })
    await drawBlocks(p, slide.blocks, { ...full, y: y - 30 })
  } else if (slide.layout === 'section') {
    page.drawText(String(index + 1).padStart(2, '0'), {
      x: MARGIN_X - 4,
      y: PAGE_HEIGHT / 2 + 40,
      size: 120,
      font: fonts.heading,
      color: accent,
      opacity: 0.9,
    })
    let y = drawKicker(p, slide, MARGIN_X, PAGE_HEIGHT / 2 + 30)
    const titleSize = fitSize(fonts.heading, slide.title ?? '', width, 46, 30, 2)
    y =
      drawLines(
        page,
        wrap(fonts.heading, titleSize, width, slide.title ?? ''),
        { ...full, y },
        fonts.heading,
        titleSize,
        ink,
        1.1,
      ) - 6
    if (slide.subtitle)
      y = drawLines(page, wrap(fonts.body, 18, width - 120, slide.subtitle), { ...full, y }, fonts.body, 18, muted)
    await drawBlocks(p, slide.blocks, { ...full, y: y - 8 })
  } else if (slide.layout === 'statement') {
    const titleSize = fitSize(fonts.heading, slide.title ?? '', width - 60, 44, 26, 4)
    const lines = wrap(fonts.heading, titleSize, width - 60, slide.title ?? '')
    const quote = slide.blocks.find((block) => block.type === 'quote')
    const quoteLines = quote && quote.type === 'quote' ? wrap(fonts.italic, 28, width - 100, quote.text) : []
    const height =
      (slide.kicker ? 24 : 0) +
      lines.length * titleSize * 1.1 +
      quoteLines.length * 28 * 1.3 +
      (slide.subtitle ? 30 : 0)
    let y = PAGE_HEIGHT / 2 + height / 2
    y = drawKicker(p, slide, MARGIN_X, y, 'center', width)
    y = drawLines(page, lines, { ...full, y }, fonts.heading, titleSize, ink, 1.1, 'center')
    if (quoteLines.length) y = drawLines(page, quoteLines, { ...full, y: y - 6 }, fonts.italic, 28, ink, 1.3, 'center')
    if (quote && quote.type === 'quote' && quote.attribution)
      y = drawLines(page, [`- ${sanitize(quote.attribution)}`], { ...full, y }, fonts.body, 15, muted, 1.3, 'center')
    if (slide.subtitle)
      drawLines(
        page,
        wrap(fonts.body, 19, width - 120, slide.subtitle),
        { ...full, y: y - 4 },
        fonts.body,
        19,
        muted,
        1.3,
        'center',
      )
  } else if (slide.layout === 'image') {
    const image = slide.blocks.find((block) => block.type === 'image')
    const loaded = image && image.type === 'image' ? await loadImage(p.doc, image.src, p.images) : null
    if (loaded) {
      const scale = Math.max(PAGE_WIDTH / loaded.width, PAGE_HEIGHT / loaded.height)
      const w = loaded.width * scale
      const h = loaded.height * scale
      page.drawImage(loaded, { x: (PAGE_WIDTH - w) / 2, y: (PAGE_HEIGHT - h) / 2, width: w, height: h })
      page.drawRectangle({ x: 0, y: 0, width: PAGE_WIDTH, height: 180, color: rgb(0, 0, 0), opacity: 0.6 })
    }
    const overlayInk = loaded ? rgb(1, 1, 1) : ink
    let y = drawLines(
      page,
      wrap(fonts.heading, 32, width, slide.title ?? ''),
      { ...full, y: 150 },
      fonts.heading,
      32,
      overlayInk,
      1.1,
    )
    if (slide.subtitle)
      y = drawLines(
        page,
        wrap(fonts.body, 17, width, slide.subtitle),
        { ...full, y },
        fonts.body,
        17,
        loaded ? rgb(0.9, 0.9, 0.9) : muted,
      )
    if (!loaded)
      await drawBlocks(
        p,
        slide.blocks.filter((block) => block !== image),
        { ...full, y: y - 8 },
      )
  } else if (slide.layout === 'split') {
    const leftWidth = width * 0.42
    const rightX = MARGIN_X + leftWidth + 40
    const rightWidth = width - leftWidth - 40
    let y = drawKicker(p, slide, MARGIN_X, PAGE_HEIGHT / 2 + 90)
    const titleSize = fitSize(fonts.heading, slide.title ?? '', leftWidth, 34, 22, 4)
    y =
      drawLines(
        page,
        wrap(fonts.heading, titleSize, leftWidth, slide.title ?? ''),
        { ...full, y, width: leftWidth },
        fonts.heading,
        titleSize,
        ink,
        1.1,
      ) - 6
    if (slide.subtitle)
      drawLines(
        page,
        wrap(fonts.body, 16, leftWidth, slide.subtitle),
        { ...full, y, width: leftWidth },
        fonts.body,
        16,
        muted,
      )
    panel(p, rightX - 16, FOOTER_Y + 24, rightWidth + 32, PAGE_HEIGHT - MARGIN_TOP - FOOTER_Y - 24)
    await drawBlocks(
      p,
      slide.blocks,
      { x: rightX, y: PAGE_HEIGHT - MARGIN_TOP - 16, width: rightWidth, bottom: FOOTER_Y + 36 },
      0.9,
    )
  } else {
    let y = drawKicker(p, slide, MARGIN_X, full.y)
    if (slide.title) {
      const titleSize = fitSize(fonts.heading, slide.title, width, 32, 22, 2)
      y =
        drawLines(
          page,
          wrap(fonts.heading, titleSize, width, slide.title),
          { ...full, y },
          fonts.heading,
          titleSize,
          ink,
          1.12,
        ) - 2
    }
    if (slide.subtitle)
      y = drawLines(page, wrap(fonts.body, 16, width, slide.subtitle), { ...full, y }, fonts.body, 16, muted) - 2
    if (slide.title || slide.subtitle) y -= 14
    const twoColumns = slide.layout === 'two-column' || slide.blocks.some((block) => block.column)
    if (twoColumns) {
      const [left, right] = splitColumns(slide.blocks)
      const columnWidth = (width - 36) / 2
      await drawBlocks(p, left, { ...full, y, width: columnWidth }, 0.9)
      await drawBlocks(p, right, { ...full, y, x: MARGIN_X + columnWidth + 36, width: columnWidth }, 0.9)
    } else await drawBlocks(p, slide.blocks, { ...full, y })
  }

  if (p.theme.footer)
    page.drawText(sanitize(p.theme.footer).toUpperCase(), {
      x: MARGIN_X,
      y: FOOTER_Y,
      size: 8,
      font: fonts.body,
      color: muted,
    })
  const pageLabel = `${index + 1} / ${total}`
  page.drawText(pageLabel, {
    x: PAGE_WIDTH - MARGIN_X - fonts.body.widthOfTextAtSize(pageLabel, 8),
    y: FOOTER_Y,
    size: 8,
    font: fonts.body,
    color: muted,
  })
}

export async function renderDeckPdf(deck: Deck, brand?: ThemeOverrides): Promise<Uint8Array> {
  const theme = resolveTheme(deck, brand)
  const doc = await PDFDocument.create()
  doc.setTitle(deck.title)
  if (deck.author) doc.setAuthor(deck.author)
  doc.setProducer('iii slides')
  const fonts: Fonts = {
    heading: await doc.embedFont(StandardFonts.HelveticaBold),
    body: await doc.embedFont(StandardFonts.Helvetica),
    italic: await doc.embedFont(StandardFonts.HelveticaOblique),
    mono: await doc.embedFont(StandardFonts.Courier),
  }
  const images = new Map<string, PDFImage | null>()
  const slides = deck.slides.length
    ? deck.slides
    : [{ id: 'empty', layout: 'title' as const, title: deck.title, blocks: [] }]
  for (const [index, slide] of slides.entries()) {
    const page = doc.addPage([PAGE_WIDTH, PAGE_HEIGHT])
    await drawSlide(
      { doc, page, theme, palette: paletteFor(theme, slide), fonts, images },
      slide,
      index,
      slides.length,
      deck,
    )
  }
  return doc.save()
}

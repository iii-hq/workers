import { PDFDocument, type PDFFont, type PDFImage, type PDFPage, rgb, StandardFonts } from 'pdf-lib'
import type { Block, Deck, Slide, ThemeOverrides } from './model.js'
import { hexToRgb, type ResolvedTheme, resolveTheme } from './themes.js'

export const PAGE_WIDTH = 960
export const PAGE_HEIGHT = 540
const MARGIN_X = 64
const MARGIN_TOP = 56
const FOOTER_Y = 24
const FETCH_TIMEOUT_MS = 8_000

type Fonts = { heading: PDFFont; body: PDFFont; italic: PDFFont; mono: PDFFont }

function color(hex: string) {
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
  fill: ReturnType<typeof rgb>,
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
  fonts: Fonts
  images: Map<string, PDFImage | null>
}

async function drawBlock(p: Painter, block: Block, cursor: Cursor, scale = 1): Promise<number> {
  const { page, theme, fonts } = p
  const ink = color(theme.colors.ink)
  const muted = color(theme.colors.muted)
  const accent = color(theme.colors.accent)
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
      const size = 17 * scale
      for (const item of block.items) {
        if (y - size < cursor.bottom) break
        page.drawRectangle({ x: cursor.x + 2, y: y - size + 4, width: 7, height: 7, color: accent, rotate: undefined })
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
      page.drawRectangle({
        x: cursor.x,
        y: cursor.y - height,
        width: cursor.width,
        height,
        color: color(theme.colors.surface),
        borderColor: accent,
        borderWidth: 0.5,
        opacity: 1,
      })
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
      const width = Math.max(
        150,
        fonts.heading.widthOfTextAtSize(value, size) + 40,
        fonts.body.widthOfTextAtSize(sanitize(block.label), 13) + 40,
      )
      const height = size + 44
      page.drawRectangle({
        x: cursor.x,
        y: cursor.y - height,
        width: Math.min(width, cursor.width),
        height,
        color: color(theme.colors.surface),
        borderColor: accent,
        borderWidth: 0.5,
      })
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

async function drawSlide(p: Painter, slide: Slide, index: number, total: number): Promise<void> {
  const { page, theme, fonts } = p
  const ink = color(theme.colors.ink)
  const muted = color(theme.colors.muted)
  const accent = color(theme.colors.accent)
  const background =
    slide.background && /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(slide.background)
      ? slide.background
      : theme.colors.background
  page.drawRectangle({ x: 0, y: 0, width: PAGE_WIDTH, height: PAGE_HEIGHT, color: color(background) })
  const width = PAGE_WIDTH - MARGIN_X * 2
  const full: Cursor = { x: MARGIN_X, y: PAGE_HEIGHT - MARGIN_TOP, width, bottom: FOOTER_Y + 28 }

  if (slide.layout === 'title') {
    const titleLines = wrap(fonts.heading, 54, width, slide.title ?? '')
    const subtitleLines = slide.subtitle ? wrap(fonts.body, 22, width, slide.subtitle) : []
    const blockHeight = titleLines.length * 54 * 1.1 + subtitleLines.length * 22 * 1.35 + 40
    let y = PAGE_HEIGHT / 2 + blockHeight / 2
    page.drawRectangle({ x: MARGIN_X, y: y + 8, width: 72, height: 7, color: accent })
    y = drawLines(page, titleLines, { ...full, y: y - 8 }, fonts.heading, 54, ink, 1.1) - 6
    y = drawLines(page, subtitleLines, { ...full, y }, fonts.body, 22, muted)
    await drawBlocks(p, slide.blocks, { ...full, y: y - 10 })
  } else if (slide.layout === 'section') {
    page.drawText(String(index + 1).padStart(2, '0'), {
      x: MARGIN_X,
      y: PAGE_HEIGHT / 2 + 30,
      size: 96,
      font: fonts.heading,
      color: accent,
    })
    let y =
      drawLines(
        page,
        wrap(fonts.heading, 48, width, slide.title ?? ''),
        { ...full, y: PAGE_HEIGHT / 2 + 10 },
        fonts.heading,
        48,
        ink,
        1.1,
      ) - 6
    if (slide.subtitle)
      y = drawLines(page, wrap(fonts.body, 20, width, slide.subtitle), { ...full, y }, fonts.body, 20, muted)
    await drawBlocks(p, slide.blocks, { ...full, y: y - 8 })
  } else if (slide.layout === 'statement') {
    const size = 44
    const lines = wrap(fonts.heading, size, width, slide.title ?? '')
    const quote = slide.blocks.find((block) => block.type === 'quote')
    const quoteLines = quote && quote.type === 'quote' ? wrap(fonts.italic, 30, width, quote.text) : []
    const height = lines.length * size * 1.1 + quoteLines.length * 30 * 1.3 + (slide.subtitle ? 30 : 0)
    let y = PAGE_HEIGHT / 2 + height / 2
    y = drawLines(page, lines, { ...full, y }, fonts.heading, size, ink, 1.1, 'center')
    if (quoteLines.length) y = drawLines(page, quoteLines, { ...full, y: y - 6 }, fonts.italic, 30, ink, 1.3, 'center')
    if (quote && quote.type === 'quote' && quote.attribution)
      y = drawLines(page, [`- ${sanitize(quote.attribution)}`], { ...full, y }, fonts.body, 16, muted, 1.3, 'center')
    if (slide.subtitle)
      drawLines(
        page,
        wrap(fonts.body, 20, width, slide.subtitle),
        { ...full, y: y - 4 },
        fonts.body,
        20,
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
      page.drawRectangle({ x: 0, y: 0, width: PAGE_WIDTH, height: 170, color: rgb(0, 0, 0), opacity: 0.55 })
    }
    const overlayInk = loaded ? rgb(1, 1, 1) : ink
    let y = drawLines(
      page,
      wrap(fonts.heading, 34, width, slide.title ?? ''),
      { ...full, y: 150 },
      fonts.heading,
      34,
      overlayInk,
      1.1,
    )
    if (slide.subtitle)
      y = drawLines(
        page,
        wrap(fonts.body, 18, width, slide.subtitle),
        { ...full, y },
        fonts.body,
        18,
        loaded ? rgb(0.9, 0.9, 0.9) : muted,
      )
    if (!loaded)
      await drawBlocks(
        p,
        slide.blocks.filter((block) => block !== image),
        { ...full, y: y - 8 },
      )
  } else {
    let y = full.y
    if (slide.title)
      y = drawLines(page, wrap(fonts.heading, 34, width, slide.title), full, fonts.heading, 34, ink, 1.15) - 4
    if (slide.subtitle)
      y = drawLines(page, wrap(fonts.body, 18, width, slide.subtitle), { ...full, y }, fonts.body, 18, muted) - 4
    if (slide.title || slide.subtitle) {
      page.drawRectangle({ x: MARGIN_X, y: y - 2, width: 48, height: 3, color: accent })
      y -= 22
    }
    const twoColumns = slide.layout === 'two-column' || slide.blocks.some((block) => block.column)
    if (twoColumns) {
      const [left, right] = splitColumns(slide.blocks)
      const columnWidth = (width - 40) / 2
      await Promise.all([
        drawBlocks(p, left, { ...full, y, width: columnWidth }, 0.9),
        drawBlocks(p, right, { ...full, y, x: MARGIN_X + columnWidth + 40, width: columnWidth }, 0.9),
      ])
    } else await drawBlocks(p, slide.blocks, { ...full, y })
  }

  if (theme.footer)
    page.drawText(sanitize(theme.footer), { x: MARGIN_X, y: FOOTER_Y, size: 10, font: fonts.body, color: muted })
  const pageLabel = `${index + 1} / ${total}`
  page.drawText(pageLabel, {
    x: PAGE_WIDTH - MARGIN_X - fonts.body.widthOfTextAtSize(pageLabel, 10),
    y: FOOTER_Y,
    size: 10,
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
    await drawSlide({ doc, page, theme, fonts, images }, slide, index, slides.length)
  }
  return doc.save()
}

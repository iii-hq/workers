import { PDFDocument } from 'pdf-lib'
import { describe, expect, it } from 'vitest'
import { renderDeckHtml } from '../src/render-html.js'
import { renderDeckPdf, sanitize } from '../src/render-pdf.js'
import { renderDeckPptx } from '../src/render-pptx.js'
import { sampleDeck } from './support/sample.js'

describe('html renderer', () => {
  it('produces a self-contained document with every slide and escapes content', () => {
    const deck = sampleDeck()
    deck.slides[1].title = 'Revenue <script>alert(1)</script>'
    const html = renderDeckHtml(deck, { footer: 'ACME & co' })
    expect(html.startsWith('<!doctype html>')).toBe(true)
    expect(html.match(/<section class="slide/g)?.length).toBe(5)
    expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
    expect(html).not.toContain('<script>alert(1)')
    expect(html).toContain('ACME &amp; co')
    expect(html).toContain('<strong>mid-market</strong>')
    expect(html).toContain('class="metric-value"')
    expect(html).toContain('data-language="ts"')
    expect(html).toContain('class="notes"')
  })

  it('drops unsafe image sources', () => {
    const deck = sampleDeck()
    deck.slides[4].blocks[1] = { id: 'b', type: 'image', src: 'javascript:alert(1)' }
    const html = renderDeckHtml(deck)
    expect(html).not.toContain('javascript:alert')
    expect(html).toContain('Image unavailable')
  })
})

describe('pdf renderer', () => {
  it('writes one page per slide and survives missing images', async () => {
    const bytes = await renderDeckPdf(sampleDeck(), { footer: 'ACME' })
    expect(Buffer.from(bytes.slice(0, 5)).toString()).toBe('%PDF-')
    expect((await PDFDocument.load(bytes)).getPageCount()).toBe(5)
  }, 20_000)

  it('sanitizes text to the standard font encoding', () => {
    expect(sanitize('Smart \u201cquotes\u201d \u2014 and emoji \u{1F600}')).toBe('Smart "quotes" - and emoji ')
    expect(sanitize('**bold** and `code`')).toBe('bold and code')
  })
})

describe('pptx renderer', () => {
  it('writes a zip with one slide part per slide', async () => {
    const buffer = await renderDeckPptx(sampleDeck(), { footer: 'ACME' })
    expect(buffer.subarray(0, 2).toString()).toBe('PK')
    const listing = buffer.toString('latin1')
    for (let index = 1; index <= 5; index += 1) expect(listing).toContain(`ppt/slides/slide${index}.xml`)
    expect(listing).not.toContain('ppt/slides/slide6.xml')
    expect(listing).toContain('ppt/notesSlides/notesSlide1.xml')
  }, 20_000)
})

import { PDFDocument } from 'pdf-lib'
import { describe, expect, it } from 'vitest'
import { applyMeasure, auditDeck, auditSlideContent, blockText, repeatedWords } from '../src/audit.js'
import type { SlideMeasure } from '../src/capture.js'
import { renderDeckHtml } from '../src/render-html.js'
import { rasterPdf, rasterPptx } from '../src/render-raster.js'
import { sampleDeck } from './support/sample.js'

const PIXEL_JPEG =
  '/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA//9k='

describe('audit content checks', () => {
  it('extracts text from every block type', () => {
    const deck = sampleDeck()
    expect(blockText(deck.slides[1].blocks[0])).toEqual([
      'Enterprise ARR up 31%',
      'Churn down to 2.1%',
      'Two new regions live',
    ])
    expect(blockText({ id: 'x', type: 'metric', value: '9', label: 'nine' })).toEqual(['9', 'nine'])
    expect(blockText({ id: 'x', type: 'table', columns: ['a'], rows: [['1']] })).toEqual(['a', '1'])
    expect(blockText({ id: 'x', type: 'code', code: 'let x' })).toEqual([])
  })

  it('flags repeated words, missing notes, long slides and ragged tables', () => {
    expect(repeatedWords('pipeline pipeline pipeline the the the')).toEqual([{ word: 'pipeline', count: 3 }])
    const deck = sampleDeck()
    const audit = auditSlideContent(deck.slides[2], 2)
    expect(audit.findings.map((finding) => finding.code)).toContain('missing_notes')
    expect(audit.measured).toBe(false)
    const ragged = auditSlideContent(
      {
        id: 's',
        layout: 'content',
        title: 'Table',
        blocks: [
          { id: 't', type: 'table', columns: ['a', 'b'], rows: [['1']] },
          { id: 'b', type: 'bullets', items: ['1', '2', '3', '4', '5', '6', '7'] },
        ],
        notes: 'ok',
      },
      0,
    )
    const codes = ragged.findings.map((finding) => finding.code)
    expect(codes).toContain('ragged_table')
    expect(codes).toContain('too_many_bullets')
    expect(codes).not.toContain('missing_notes')
  })

  it('merges browser measurements into findings and totals', () => {
    const deck = sampleDeck()
    const measure: SlideMeasure = {
      index: 1,
      fit: 0.7,
      overflow: true,
      overflow_px: 44,
      minimum_font_size: 14,
      empty_space_ratio: 0.2,
      clipped_labels: ['Net revenue retention'],
      collisions: [{ a: 'block-1', b: 'block-2' }],
      blocks: [
        { block_id: 'block-1', type: 'bullets', x: 0, y: 0, width: 10, height: 10, overflow: true },
        { block_id: 'block-2', type: 'text', x: 0, y: 0, width: 10, height: 10, overflow: false },
      ],
    }
    const slide = applyMeasure(auditSlideContent(deck.slides[1], 1), measure)
    const codes = slide.findings.map((finding) => finding.code)
    expect(slide.measured).toBe(true)
    expect(codes).toEqual(expect.arrayContaining(['overflow', 'auto_fit', 'small_text', 'collision', 'clipped_labels']))
    expect(slide.findings.find((finding) => finding.code === 'overflow')?.block_id).toBe('block-1')
    const full = auditDeck(deck, [measure])
    expect(full.measured).toBe(true)
    expect(full.slides[1].measured).toBe(true)
    expect(full.slides[0].measured).toBe(false)
    expect(full.error_count).toBeGreaterThanOrEqual(2)
    const unmeasured = auditDeck(deck, undefined, 'CAPTURE_UNAVAILABLE: no browser')
    expect(unmeasured.measured).toBe(false)
    expect(unmeasured.measurement_error).toMatch(/CAPTURE_UNAVAILABLE/)
  })
})

describe('capture render mode', () => {
  it('disables motion and reveals every block', () => {
    const html = renderDeckHtml(sampleDeck(), undefined, { capture: true })
    expect(html).toContain('class="deck t-none r-none capture"')
    expect(html).toContain('.capture .rv{opacity:1!important')
  })
})

describe('raster exporters', () => {
  const images = (indexes: number[]) =>
    indexes.map((index) => ({ index, content_type: 'image/jpeg', width: 1, height: 1, data_base64: PIXEL_JPEG }))

  it('writes one 16:9 pdf page per captured slide', async () => {
    const bytes = await rasterPdf(sampleDeck(), images([0, 1, 2]))
    const doc = await PDFDocument.load(bytes)
    expect(doc.getPageCount()).toBe(3)
    expect(doc.getPage(0).getSize()).toEqual({ width: 1600, height: 900 })
    expect(doc.getTitle()).toBe('Quarterly review')
  })

  it('writes a pptx with one picture slide per capture and keeps notes', async () => {
    const buffer = await rasterPptx(sampleDeck(), images([0, 1]))
    expect(buffer.subarray(0, 2).toString()).toBe('PK')
    const listing = buffer.toString('latin1')
    expect(listing).toContain('ppt/slides/slide2.xml')
    expect(listing).not.toContain('ppt/slides/slide3.xml')
    expect(listing).toContain('ppt/media/image')
    expect(listing).toContain('ppt/notesSlides/notesSlide2.xml')
  }, 20_000)
})

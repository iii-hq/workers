import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  expandDocumentAttachments,
  isDocumentAttachment,
  MAX_DOCUMENT_BYTES,
  MAX_DOCUMENTS_PER_SEND,
  TO_MARKDOWN_FUNCTION_ID,
} from './documents'

function doc(name = 'report.docx', bytes = 'PK'): Attachment {
  return {
    id: name,
    name,
    size: bytes.length,
    type: '',
    file: new File([bytes], name),
  }
}

function converted(over: Record<string, unknown> = {}) {
  return {
    format: 'docx',
    family: 'prose',
    detected_from: 'content',
    body: {
      text: '# Quarterly Notes',
      chars: 17,
      total_chars: 17,
      truncated: false,
    },
    asset_count: 0,
    elapsed_ms: 4,
    ...over,
  }
}

describe('isDocumentAttachment', () => {
  it('recognises every office format by extension', () => {
    for (const name of [
      'a.docx',
      'a.doc',
      'a.pptx',
      'a.xlsx',
      'a.xlsb',
      'a.odt',
      'a.ods',
      'a.odp',
      'a.rtf',
      'a.epub',
      'a.csv',
    ]) {
      expect(isDocumentAttachment(doc(name)), name).toBe(true)
    }
  })

  /* The browser's MIME type for a CSV is `application/vnd.ms-excel`, and for a
     file dragged out of an archive it is often nothing at all — the name is the
     one thing that survives every route into the composer. */
  it('ignores the declared MIME type', () => {
    const csv: Attachment = {
      ...doc('rows.csv'),
      type: 'application/vnd.ms-excel',
    }
    expect(isDocumentAttachment(csv)).toBe(true)
  })

  it('leaves PDFs and images to their own paths', () => {
    expect(isDocumentAttachment(doc('report.pdf'))).toBe(false)
    expect(isDocumentAttachment(doc('shot.png'))).toBe(false)
  })
})

describe('expandDocumentAttachments', () => {
  it('does nothing when there is no document', async () => {
    const trigger = vi.fn()
    const result = await expandDocumentAttachments([doc('shot.png')], trigger)
    expect(result.blocks).toEqual([])
    expect(trigger).not.toHaveBeenCalled()
  })

  /* A CSV has no signature of its own. Without the name the worker cannot
     recognise it and refuses a file it reads perfectly well. */
  it('sends the file name alongside the bytes', async () => {
    const trigger = vi.fn().mockResolvedValue(converted({ format: 'csv' }))
    await expandDocumentAttachments([doc('rows.csv', 'a,b\n')], trigger)

    expect(trigger).toHaveBeenCalledWith(
      TO_MARKDOWN_FUNCTION_ID,
      expect.objectContaining({ file_name: 'rows.csv' }),
    )
  })

  it('wraps the markdown in an attached-file block', async () => {
    const trigger = vi.fn().mockResolvedValue(converted())
    const result = await expandDocumentAttachments([doc()], trigger)

    expect(result.blocks).toHaveLength(1)
    expect(result.blocks[0]).toContain('path="report.docx"')
    expect(result.blocks[0]).toContain('format="docx-markdown"')
    expect(result.blocks[0]).toContain('# Quarterly Notes')
    expect(result.failures).toEqual([])
    expect(result.read[0].label).toContain('docx')
  })

  /* Markdown renders an embedded image as alt text, so a deck of diagrams
     converts to almost nothing. The block has to say the pictures exist, or the
     model reports an empty document. */
  it('names the images the markdown could not carry', async () => {
    const trigger = vi.fn().mockResolvedValue(
      converted({
        format: 'pptx',
        asset_count: 12,
        body: { text: 'Roadmap', chars: 7, total_chars: 7, truncated: false },
      }),
    )
    const result = await expandDocumentAttachments([doc('deck.pptx')], trigger)

    expect(result.blocks[0]).toContain('embedded-images="12"')
    expect(result.blocks[0]).toContain('document::extract-assets')
    expect(result.read[0].label).toContain('12 images')
  })

  /* An empty conversion and a deck whose content is pictures look identical on
     the wire. The block is the only place that difference can be stated. */
  it('distinguishes an image-only document from an empty one', async () => {
    const pictures = vi.fn().mockResolvedValue(
      converted({
        asset_count: 3,
        body: { text: '', chars: 0, total_chars: 0, truncated: false },
      }),
    )
    const withPictures = await expandDocumentAttachments(
      [doc('deck.pptx')],
      pictures,
    )
    expect(withPictures.blocks[0]).toContain('NOT included in this message')
    expect(withPictures.blocks[0]).toContain('Do not call the document empty')

    const blank = vi.fn().mockResolvedValue(
      converted({
        asset_count: 0,
        body: { text: '', chars: 0, total_chars: 0, truncated: false },
      }),
    )
    const withNothing = await expandDocumentAttachments([doc()], blank)
    expect(withNothing.blocks[0]).toContain('no text and no images')
  })

  it('reports truncation with the way to get the rest', async () => {
    const trigger = vi.fn().mockResolvedValue(
      converted({
        body: {
          text: 'start',
          chars: 5,
          total_chars: 90_000,
          truncated: true,
        },
      }),
    )
    const result = await expandDocumentAttachments([doc()], trigger)

    expect(result.blocks[0]).toContain('truncated="true"')
    expect(result.blocks[0]).toContain('total-chars="90000"')
    expect(result.blocks[0]).toContain('max_chars 0')
    expect(result.read[0].label).toContain('90,000+ chars')
  })

  /* A failure has to reach the model as a block. Staying silent is the bug this
     path exists to prevent: the agent answers as though nothing was attached. */
  it('turns a worker failure into a block and a named failure', async () => {
    const trigger = vi
      .fn()
      .mockRejectedValue(new Error('function document::to-markdown not found'))
    const result = await expandDocumentAttachments([doc()], trigger)

    expect(result.blocks[0]).toContain('error=')
    expect(result.blocks[0]).toContain('iii worker add document')
    expect(result.failures).toHaveLength(1)
  })

  it('refuses a document over the composer ceiling without calling the worker', async () => {
    const trigger = vi.fn()
    const huge: Attachment = { ...doc(), size: MAX_DOCUMENT_BYTES + 1 }
    const result = await expandDocumentAttachments([huge], trigger)

    expect(trigger).not.toHaveBeenCalled()
    expect(result.failures[0].reason).toContain('limit')
  })

  it('reports the documents past the per-send ceiling instead of dropping them', async () => {
    const trigger = vi.fn().mockResolvedValue(converted())
    const many = Array.from({ length: MAX_DOCUMENTS_PER_SEND + 2 }, (_, i) =>
      doc(`report-${i}.docx`),
    )
    const result = await expandDocumentAttachments(many, trigger)

    expect(trigger).toHaveBeenCalledTimes(MAX_DOCUMENTS_PER_SEND)
    expect(result.blocks).toHaveLength(MAX_DOCUMENTS_PER_SEND + 2)
    expect(result.failures).toHaveLength(2)
    expect(result.failures[0].reason).toContain('per message')
  })

  /* A conversation reloaded from history keeps the chip, not the bytes. */
  it('skips an attachment with no file', async () => {
    const trigger = vi.fn()
    const { file, ...withoutBytes } = doc()
    void file
    const result = await expandDocumentAttachments([withoutBytes], trigger)

    expect(result.blocks).toEqual([])
    expect(trigger).not.toHaveBeenCalled()
  })
})

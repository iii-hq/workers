import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  expandDocumentAttachments,
  isDocumentAttachment,
  MAX_DOCUMENT_BYTES,
  MAX_MARKDOWN_CHARS,
  TO_MARKDOWN_FUNCTION_ID,
} from './documents'
import { failureBlock } from './shared'

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

/** `count` stored documents, with ids `a_0`, `a_1`, … */
function storedDocs(count: number): Attachment[] {
  return Array.from({ length: count }, (_, i) => ({
    ...doc(`report-${i}.docx`),
    attachmentId: `a_${i}`,
  }))
}

/** A worker holding documents of `total` characters, cut at `max_chars`. */
function workerOf(total: number) {
  return vi.fn(async (_id: string, payload: Record<string, unknown>) => {
    const chars = Math.min(total, payload.max_chars as number)
    return converted({
      body: {
        text: 'x'.repeat(chars),
        chars,
        total_chars: total,
        truncated: chars < total,
      },
    })
  })
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
    const call = vi.fn()
    const result = await expandDocumentAttachments([doc('shot.png')], call)
    expect(result.blocks).toEqual([])
    expect(call).not.toHaveBeenCalled()
  })

  /* A CSV has no signature of its own. Without the name the worker cannot
     recognise it and refuses a file it reads perfectly well. */
  it('sends the file name alongside the bytes', async () => {
    const call = vi.fn().mockResolvedValue(converted({ format: 'csv' }))
    await expandDocumentAttachments([doc('rows.csv', 'a,b\n')], call)

    expect(call).toHaveBeenCalledWith(
      TO_MARKDOWN_FUNCTION_ID,
      expect.objectContaining({ file_name: 'rows.csv' }),
    )
  })

  it('wraps the markdown in an attached-file block', async () => {
    const call = vi.fn().mockResolvedValue(converted())
    const result = await expandDocumentAttachments([doc()], call)

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
    const call = vi.fn().mockResolvedValue(
      converted({
        format: 'pptx',
        asset_count: 12,
        body: { text: 'Roadmap', chars: 7, total_chars: 7, truncated: false },
      }),
    )
    const result = await expandDocumentAttachments([doc('deck.pptx')], call)

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
    const call = vi.fn().mockResolvedValue(
      converted({
        body: {
          text: 'start',
          chars: 5,
          total_chars: 90_000,
          truncated: true,
        },
      }),
    )
    const result = await expandDocumentAttachments([doc()], call)

    expect(result.blocks[0]).toContain('truncated="true"')
    expect(result.blocks[0]).toContain('total-chars="90000"')
    expect(result.blocks[0]).toContain('max_chars 0')
    expect(result.read[0].label).toContain('90,000+ chars')
  })

  it('names the call that reads the rest of a stored document', async () => {
    const [stored] = storedDocs(1)
    const result = await expandDocumentAttachments([stored], workerOf(90_000))

    expect(result.blocks[0]).toContain(
      'This is the first 20000 of 90000 characters. Read the rest with session::read-attachment {"attachment_id":"a_0","offset":20000}.\n\n',
    )
    expect(result.blocks[0]).not.toContain('max_chars 0')
  })

  /* Five emoji are ten UTF-16 units; the read counts them as five. */
  it('counts the rest from the characters the worker counted', async () => {
    const call = vi.fn().mockResolvedValue(
      converted({
        body: {
          text: '\u{1F600}'.repeat(5),
          chars: 5,
          total_chars: 10,
          truncated: true,
        },
      }),
    )
    const result = await expandDocumentAttachments(storedDocs(1), call)

    expect(result.blocks[0]).toContain('{"attachment_id":"a_0","offset":5}')
  })

  /* A failure has to reach the model as a block. Staying silent is the bug this
     path exists to prevent: the agent answers as though nothing was attached. */
  it('turns a worker failure into a block and a named failure', async () => {
    const call = vi
      .fn()
      .mockRejectedValue(new Error('function document::to-markdown not found'))
    const result = await expandDocumentAttachments([doc()], call)

    expect(result.blocks[0]).toContain('error=')
    expect(result.blocks[0]).toContain('iii call compose::add worker=document')
    expect(result.failures).toHaveLength(1)
  })

  it('refuses a document over the composer ceiling without calling the worker', async () => {
    const call = vi.fn()
    const huge: Attachment = { ...doc(), size: MAX_DOCUMENT_BYTES + 1 }
    const result = await expandDocumentAttachments([huge], call)

    expect(call).not.toHaveBeenCalled()
    expect(result.failures[0].reason).toContain('limit')
  })

  it('puts the stored id on the stub of a document it could not read', async () => {
    const [huge, broken] = storedDocs(2)
    const result = await expandDocumentAttachments(
      [{ ...huge, size: MAX_DOCUMENT_BYTES + 1 }, broken],
      vi.fn().mockRejectedValue(new Error('malformed document')),
    )

    expect(result.blocks).toEqual([
      failureBlock(huge.name, result.failures[0].reason, 'a_0'),
      failureBlock(broken.name, 'malformed document', 'a_1'),
    ])
  })

  it('inlines every document while the message has room', async () => {
    const call = workerOf(4_500)
    const result = await expandDocumentAttachments(storedDocs(14), call)

    expect(call).toHaveBeenCalledTimes(14)
    expect(result.read).toHaveLength(14)
    expect(result.failures).toEqual([])
  })

  it('cuts at the character budget and stubs the rest instead of dropping them', async () => {
    const call = workerOf(15_000)
    const huge = { ...doc('huge.docx'), size: MAX_DOCUMENT_BYTES + 1 }
    const result = await expandDocumentAttachments(
      [...storedDocs(7), doc('never-stored.docx'), huge],
      call,
    )

    // Five take 75,000 characters, the sixth what is left of the 80,000.
    expect(call.mock.calls.map(([, payload]) => payload.max_chars)).toEqual([
      ...Array(5).fill(MAX_MARKDOWN_CHARS),
      5_000,
    ])
    expect(result.read).toHaveLength(6)
    expect(result.blocks[5]).toContain('{"attachment_id":"a_5","offset":5000}')
    expect(result.blocks.slice(6)).toEqual([
      failureBlock(
        'report-6.docx',
        'this message already holds 80,000 characters of documents; the agent can read this one by its id',
        'a_6',
      ),
      failureBlock(
        'never-stored.docx',
        'this message already holds 80,000 characters of documents',
      ),
      failureBlock(
        'huge.docx',
        'larger than the 64 MB limit for reading a document in the composer',
      ),
    ])
    expect(result.failures).toHaveLength(3)
  })

  it('spends the budget on what it inlines, not on what a document holds', async () => {
    const call = workerOf(90_000)
    const result = await expandDocumentAttachments(storedDocs(5), call)

    expect(call.mock.calls.map(([, payload]) => payload.max_chars)).toEqual(
      Array(4).fill(MAX_MARKDOWN_CHARS),
    )
    expect(result.read).toHaveLength(4)
    expect(result.failures).toHaveLength(1)
    expect(result.blocks[4]).toContain('id="a_4" error=')
  })

  it('asks a worker that is not running once for the whole message', async () => {
    const call = vi
      .fn()
      .mockRejectedValue(
        new Error(
          "remote error (NOT_FOUND): Function 'document::to-markdown' is not registered.",
        ),
      )
    const result = await expandDocumentAttachments(storedDocs(14), call)

    expect(call).toHaveBeenCalledTimes(1)
    const { reason } = result.failures[0]
    expect(reason).toContain('iii call compose::add worker=document')
    expect(result.failures).toHaveLength(14)
    expect(result.blocks).toEqual(
      storedDocs(14).map((d) => failureBlock(d.name, reason, d.attachmentId)),
    )
  })

  it('keeps what it read before the worker went away', async () => {
    const call = vi
      .fn()
      .mockResolvedValueOnce(converted())
      .mockRejectedValue(
        new Error("Function 'document::to-markdown' is not registered."),
      )
    const result = await expandDocumentAttachments(storedDocs(3), call)

    expect(call).toHaveBeenCalledTimes(2)
    expect(result.read).toHaveLength(1)
    expect(result.blocks).toHaveLength(3)
    expect(result.blocks[0]).toContain('# Quarterly Notes')
    expect(result.failures).toHaveLength(2)
  })

  /* A call waits five minutes, and the message waits for every document. */
  it('asks a worker that does not answer once for the whole message', async () => {
    const reason = 'Invocation timeout after 300000ms: document::to-markdown'
    const call = vi
      .fn()
      .mockRejectedValue(Object.assign(new Error(reason), { code: 'TIMEOUT' }))
    const result = await expandDocumentAttachments(storedDocs(14), call)

    expect(call).toHaveBeenCalledTimes(1)
    expect(result.blocks).toEqual(
      storedDocs(14).map((d) => failureBlock(d.name, reason, d.attachmentId)),
    )
  })

  it('keeps converting after one document fails on its own', async () => {
    const call = vi
      .fn()
      .mockRejectedValueOnce(new Error('malformed document'))
      .mockResolvedValue(converted())
    const result = await expandDocumentAttachments(storedDocs(3), call)

    expect(call).toHaveBeenCalledTimes(3)
    expect(result.read).toHaveLength(2)
    expect(result.failures).toHaveLength(1)
  })

  /* A conversation reloaded from history keeps the chip, not the bytes. */
  it('skips an attachment with no file', async () => {
    const call = vi.fn()
    const { file, ...withoutBytes } = doc()
    void file
    const result = await expandDocumentAttachments([withoutBytes], call)

    expect(result.blocks).toEqual([])
    expect(call).not.toHaveBeenCalled()
  })
})

import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  CLASSIFY_FUNCTION_ID,
  expandPdfAttachments,
  isPdfAttachment,
  MAX_PDFS_PER_SEND,
  TO_MARKDOWN_FUNCTION_ID,
} from './pdf-attachments'

function pdf(name = 'report.pdf', bytes = 'hello'): Attachment {
  return {
    id: name,
    name,
    size: bytes.length,
    type: 'application/pdf',
    file: new File([bytes], name, { type: 'application/pdf' }),
  }
}

function textFile(): Attachment {
  return {
    id: 'notes.txt',
    name: 'notes.txt',
    size: 4,
    type: 'text/plain',
    file: new File(['note'], 'notes.txt', { type: 'text/plain' }),
  }
}

type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

/**
 * A fake bus: each function id maps to the value it returns, or to an `Error`
 * it throws. The second element records call order, because "classify before
 * extracting, and skip extracting on a scan" is behaviour worth asserting.
 */
function trigger(
  responses: Record<string, unknown>,
): [TriggerFn & { mock: { calls: unknown[][] } }, string[]] {
  const calls: string[] = []
  const fn = vi.fn(
    async (functionId: string, _payload: Record<string, unknown>) => {
      calls.push(functionId)
      const value = responses[functionId]
      if (value instanceof Error) throw value
      return value
    },
  )
  return [fn as unknown as TriggerFn & { mock: { calls: unknown[][] } }, calls]
}

describe('isPdfAttachment', () => {
  it('matches by declared type', () => {
    expect(isPdfAttachment(pdf())).toBe(true)
  })

  it('matches by extension when the browser reports no type', () => {
    expect(
      isPdfAttachment({ ...pdf(), type: 'application/octet-stream' }),
    ).toBe(true)
    expect(isPdfAttachment({ ...pdf('REPORT.PDF'), type: '' })).toBe(true)
  })

  it('leaves other files alone', () => {
    expect(isPdfAttachment(textFile())).toBe(false)
  })
})

describe('expandPdfAttachments', () => {
  it('does nothing when there are no PDFs', async () => {
    const [fn] = trigger({})
    const result = await expandPdfAttachments([textFile()], fn)
    expect(result).toEqual({ blocks: [], failures: [] })
    expect(fn.mock.calls).toHaveLength(0)
  })

  it('classifies before extracting and inlines the markdown', async () => {
    const [fn, calls] = trigger({
      [CLASSIFY_FUNCTION_ID]: {
        document_type: 'text_based',
        page_count: 8,
        pages_needing_ocr: [],
      },
      [TO_MARKDOWN_FUNCTION_ID]: {
        body: { text: '# Prospectus', chars: 12, total_chars: 12 },
        page_count: 8,
      },
    })

    const { blocks, failures } = await expandPdfAttachments([pdf()], fn)

    expect(calls).toEqual([CLASSIFY_FUNCTION_ID, TO_MARKDOWN_FUNCTION_ID])
    expect(failures).toEqual([])
    expect(blocks).toHaveLength(1)
    expect(blocks[0]).toContain('<attached-file ')
    expect(blocks[0]).toContain('path="report.pdf"')
    expect(blocks[0]).toContain('pages="8"')
    expect(blocks[0]).toContain('# Prospectus')
    expect(blocks[0]).toContain('</attached-file>')
  })

  /**
   * A scan must not silently become an empty attachment: the model has to be
   * able to tell "I read it and it has no text" from "I was given nothing",
   * because only the first is worth reporting back to the person.
   */
  it('says a scan was read and found unreadable, and skips extraction', async () => {
    const [fn, calls] = trigger({
      [CLASSIFY_FUNCTION_ID]: {
        document_type: 'scanned',
        page_count: 3,
        pages_needing_ocr: [1, 2, 3],
        ocr_reasons: [{ page: 1, reasons: ['scanned'] }],
      },
    })

    const { blocks } = await expandPdfAttachments([pdf()], fn)

    expect(calls).toEqual([CLASSIFY_FUNCTION_ID])
    expect(blocks[0]).toContain('needs-ocr="true"')
    expect(blocks[0]).toContain('no extractable text')
    expect(blocks[0]).toContain('scanned')
    expect(blocks[0]).toContain('Do not claim the document is empty')
  })

  it('reports truncation with the size it withheld', async () => {
    const [fn] = trigger({
      [CLASSIFY_FUNCTION_ID]: { document_type: 'text_based', page_count: 400 },
      [TO_MARKDOWN_FUNCTION_ID]: {
        body: {
          text: 'start of a very long report',
          chars: 27,
          total_chars: 900_000,
          truncated: true,
        },
        page_count: 400,
      },
    })

    const { blocks } = await expandPdfAttachments([pdf()], fn)

    expect(blocks[0]).toContain('truncated="true"')
    expect(blocks[0]).toContain('total-chars="900000"')
    expect(blocks[0]).toContain('pages filter')
  })

  it('carries the per-page OCR verdict of a mixed document', async () => {
    const [fn] = trigger({
      [CLASSIFY_FUNCTION_ID]: {
        document_type: 'mixed',
        page_count: 10,
        pages_needing_ocr: [4, 5],
      },
      [TO_MARKDOWN_FUNCTION_ID]: {
        body: { text: 'readable pages', chars: 14, total_chars: 14 },
        page_count: 10,
      },
    })

    const { blocks } = await expandPdfAttachments([pdf()], fn)

    expect(blocks[0]).toContain('pages-needing-ocr="4,5"')
    expect(blocks[0]).toContain('Pages 4, 5 hold no readable text')
  })

  /** A missing worker is the failure a person can actually act on. */
  it('names the missing worker rather than surfacing a bus error', async () => {
    const [fn] = trigger({
      [CLASSIFY_FUNCTION_ID]: new Error(
        "remote error (NOT_FOUND): Function 'pdf::classify' is not registered.",
      ),
    })

    const { blocks, failures } = await expandPdfAttachments([pdf()], fn)

    expect(failures).toHaveLength(1)
    expect(failures[0].reason).toContain('iii worker add pdf')
    expect(blocks[0]).toContain('error=')
  })

  /** A failed read must still produce a block, so the send is never blocked. */
  it('turns a failure into a placeholder block instead of throwing', async () => {
    const [fn] = trigger({
      [CLASSIFY_FUNCTION_ID]: new Error('boom'),
    })

    const { blocks, failures } = await expandPdfAttachments([pdf()], fn)

    expect(blocks).toHaveLength(1)
    expect(blocks[0]).toContain('path="report.pdf"')
    expect(failures[0].reason).toBe('boom')
  })

  it('reports the documents it dropped over the per-send cap', async () => {
    const [fn] = trigger({
      [CLASSIFY_FUNCTION_ID]: { document_type: 'text_based', page_count: 1 },
      [TO_MARKDOWN_FUNCTION_ID]: {
        body: { text: 'x', chars: 1, total_chars: 1 },
        page_count: 1,
      },
    })

    const many = Array.from({ length: MAX_PDFS_PER_SEND + 2 }, (_, i) =>
      pdf(`doc-${i}.pdf`),
    )
    const { blocks, failures } = await expandPdfAttachments(many, fn)

    expect(blocks).toHaveLength(MAX_PDFS_PER_SEND + 2)
    expect(failures).toHaveLength(2)
    expect(failures[0].reason).toContain('per message')
  })

  /**
   * A conversation reloaded from history keeps the chip but not the bytes.
   * Re-reading is not this function's job, and it must not error.
   */
  it('skips an attachment whose file is gone', async () => {
    const [fn] = trigger({})
    const withoutFile: Attachment = {
      id: 'old',
      name: 'old.pdf',
      size: 10,
      type: 'application/pdf',
    }
    const result = await expandPdfAttachments([withoutFile], fn)
    expect(result).toEqual({ blocks: [], failures: [] })
    expect(fn.mock.calls).toHaveLength(0)
  })

  it('escapes quotes in a file name rather than breaking the header', async () => {
    const [fn] = trigger({
      [CLASSIFY_FUNCTION_ID]: { document_type: 'text_based', page_count: 1 },
      [TO_MARKDOWN_FUNCTION_ID]: {
        body: { text: 'x', chars: 1, total_chars: 1 },
        page_count: 1,
      },
    })

    const { blocks } = await expandPdfAttachments([pdf('a"b.pdf')], fn)
    expect(blocks[0]).toContain('path="a&quot;b.pdf"')
  })
})

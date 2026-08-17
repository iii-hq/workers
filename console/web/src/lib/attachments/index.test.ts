import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import { expandAttachments, hasExpandableAttachments } from './index'

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    trigger: async (functionId: string) => {
      if (functionId === 'pdf::classify') {
        return {
          document_type: 'text_based',
          page_count: 2,
          pages_needing_ocr: [],
          ocr_reasons: [],
          elapsed_ms: 3,
        }
      }
      if (functionId === 'pdf::to-markdown') {
        return {
          body: {
            text: 'the pdf text',
            chars: 12,
            total_chars: 12,
            truncated: false,
          },
          page_count: 2,
          elapsed_ms: 5,
        }
      }
      if (functionId === 'document::to-markdown') {
        return {
          format: 'docx',
          family: 'prose',
          detected_from: 'content',
          body: {
            text: 'the word text',
            chars: 13,
            total_chars: 13,
            truncated: false,
          },
          asset_count: 0,
          elapsed_ms: 4,
        }
      }
      throw new Error(`unexpected function ${functionId}`)
    },
  }),
}))

function attachment(name: string, type = '', content = 'bytes'): Attachment {
  return {
    id: name,
    name,
    size: content.length,
    type,
    file: new File([content], name, { type }),
  }
}

describe('hasExpandableAttachments', () => {
  /* A conversation reloaded from history carries chips, not bytes. Paying for
     an expansion pass over them would be work with nothing to show. */
  it('is false when nothing carries its bytes', () => {
    const { file, ...chipOnly } = attachment('report.docx')
    void file
    expect(hasExpandableAttachments([chipOnly])).toBe(false)
    expect(hasExpandableAttachments([attachment('report.docx')])).toBe(true)
  })
})

describe('expandAttachments', () => {
  it('routes each kind down its own path in one pass', async () => {
    const result = await expandAttachments([
      attachment('report.pdf', 'application/pdf'),
      attachment('quarterly.docx'),
      attachment('shot.png', 'image/png'),
      attachment('main.ts', '', 'export const x = 1'),
    ])

    expect(result.blocks.join('\n')).toContain('the pdf text')
    expect(result.blocks.join('\n')).toContain('the word text')
    expect(result.blocks.join('\n')).toContain('export const x = 1')
    expect(result.images).toHaveLength(1)
    expect(result.images[0].mime).toBe('image/png')
    expect(result.failures).toEqual([])
    // One relabelled chip per attachment that was actually read.
    expect(result.read).toHaveLength(4)
  })

  /* The whole point of the router: a file it cannot read still reaches the
     model as a block that says so, rather than vanishing. */
  it('names a file no path can read', async () => {
    const result = await expandAttachments([
      attachment('archive.zip', 'application/zip'),
    ])

    expect(result.blocks).toHaveLength(1)
    expect(result.blocks[0]).toContain('error=')
    expect(result.failures[0].reason).toContain('application/zip')
  })

  it('does no work when nothing carries its bytes', async () => {
    const { file, ...chipOnly } = attachment('report.docx')
    void file
    const result = await expandAttachments([chipOnly])

    expect(result).toEqual({ blocks: [], images: [], read: [], failures: [] })
  })
})

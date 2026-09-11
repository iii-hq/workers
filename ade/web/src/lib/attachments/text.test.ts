import { describe, expect, it } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  expandTextAttachments,
  isTextAttachment,
  MAX_TEXT_BYTES,
  MAX_TEXT_CHARS,
  MAX_TEXT_FILES_PER_SEND,
} from './text'

function file(name: string, content: string, type = ''): Attachment {
  return {
    id: name,
    name,
    size: content.length,
    type,
    file: new File([content], name, { type }),
  }
}

describe('isTextAttachment', () => {
  /* A browser calls a `.ts` file `video/mp2t` — the MPEG transport stream.
     Trusting the declared type would inline a video and refuse a TypeScript
     file. */
  it('trusts the extension over a wrong MIME type', () => {
    expect(isTextAttachment(file('main.ts', 'x', 'video/mp2t'))).toBe(true)
    expect(isTextAttachment(file('lib.rs', 'x', ''))).toBe(true)
    expect(isTextAttachment(file('notes.md', 'x', ''))).toBe(true)
  })

  it('still accepts anything the browser calls text', () => {
    expect(isTextAttachment(file('unknown.conf', 'x', 'text/plain'))).toBe(true)
    expect(isTextAttachment(file('data.json', 'x', 'application/json'))).toBe(
      true,
    )
  })

  it('leaves documents and images to their own paths', () => {
    expect(isTextAttachment(file('report.docx', 'x'))).toBe(false)
    expect(isTextAttachment(file('shot.png', 'x', 'image/png'))).toBe(false)
  })
})

describe('expandTextAttachments', () => {
  it('inlines the file in an attached-file block', async () => {
    const result = await expandTextAttachments([
      file('main.ts', 'export const x = 1\n'),
    ])

    expect(result.blocks).toHaveLength(1)
    expect(result.blocks[0]).toContain('path="main.ts"')
    expect(result.blocks[0]).toContain('export const x = 1')
    expect(result.read[0].label).toContain('chars')
  })

  it('truncates a long file and says by how much', async () => {
    const long = 'x'.repeat(MAX_TEXT_CHARS + 500)
    const result = await expandTextAttachments([file('big.log', long)])

    expect(result.blocks[0]).toContain('truncated="true"')
    expect(result.blocks[0]).toContain(`total-chars="${long.length}"`)
    expect(result.read[0].label).toContain('+')
  })

  it('refuses a file over the byte ceiling', async () => {
    const attachment = file('huge.log', 'x')
    const oversized: Attachment = { ...attachment, size: MAX_TEXT_BYTES + 1 }
    const result = await expandTextAttachments([oversized])

    expect(result.blocks[0]).toContain('error=')
    expect(result.failures[0].reason).toContain('limit')
  })

  it('reports the files past the per-send ceiling instead of dropping them', async () => {
    const many = Array.from({ length: MAX_TEXT_FILES_PER_SEND + 3 }, (_, i) =>
      file(`note-${i}.md`, 'hello'),
    )
    const result = await expandTextAttachments(many)

    expect(result.read).toHaveLength(MAX_TEXT_FILES_PER_SEND)
    expect(result.failures).toHaveLength(3)
  })
})

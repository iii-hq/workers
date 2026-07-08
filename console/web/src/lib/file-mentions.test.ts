import { describe, expect, it, vi } from 'vitest'
import {
  expandFileMentions,
  isAttachedFileBlock,
  MAX_MENTIONS_PER_SEND,
  parseAttachedFileHeader,
  parseFileMentions,
  READ_FILE_FUNCTION_ID,
} from './file-mentions'

describe('parseFileMentions', () => {
  it('extracts unique paths in first-appearance order', () => {
    const text =
      'see #file(src/a.rs) and #file(docs/x y.md) then #file(src/a.rs) again'
    expect(parseFileMentions(text)).toEqual(['src/a.rs', 'docs/x y.md'])
  })

  it('returns [] when there are no mentions', () => {
    expect(parseFileMentions('plain text @fn(engine::echo) # heading')).toEqual(
      [],
    )
  })

  it('caps at MAX_MENTIONS_PER_SEND unique paths', () => {
    const text = Array.from(
      { length: MAX_MENTIONS_PER_SEND + 5 },
      (_, i) => `#file(f${i}.txt)`,
    ).join(' ')
    expect(parseFileMentions(text)).toHaveLength(MAX_MENTIONS_PER_SEND)
  })
})

describe('expandFileMentions', () => {
  it('formats content blocks and chip attachments from a batch read', async () => {
    const trigger = vi.fn().mockResolvedValue({
      results: [
        {
          path: '/w/src/a.rs',
          success: true,
          content: 'fn main() {}',
          is_utf8: true,
          total_lines: 1,
          more_lines: false,
          size: 12,
        },
      ],
    })
    const out = await expandFileMentions('/w', ['src/a.rs'], trigger)
    expect(trigger).toHaveBeenCalledWith(READ_FILE_FUNCTION_ID, {
      paths: ['src/a.rs'],
      fs_scope: { root: '/w' },
    })
    expect(out.blocks).toEqual([
      '<attached-file path="src/a.rs" size="12" total-lines="1">\nfn main() {}\n</attached-file>',
    ])
    expect(out.attachments).toEqual([{ path: 'src/a.rs', size: 12 }])
    expect(out.failures).toEqual([])
  })

  it('marks truncated reads and keeps them as attachments', async () => {
    const trigger = vi.fn().mockResolvedValue({
      results: [
        {
          path: '/w/big.log',
          success: true,
          content: 'first lines',
          is_utf8: true,
          more_lines: true,
          size: 999_999,
        },
      ],
    })
    const out = await expandFileMentions('/w', ['big.log'], trigger)
    expect(out.blocks[0]).toContain('truncated="true"')
    expect(out.failures).toEqual([])
  })

  it('turns per-entry failures and binary files into placeholder blocks', async () => {
    const trigger = vi.fn().mockResolvedValue({
      results: [
        {
          path: 'gone.txt',
          success: false,
          error: { code: 'C211', message: 'not found: gone.txt' },
        },
        { path: '/w/pic.png', success: true, content: '�', is_utf8: false },
      ],
    })
    const out = await expandFileMentions('/w', ['gone.txt', 'pic.png'], trigger)
    expect(out.blocks[0]).toBe(
      '<attached-file path="gone.txt" error="not found: gone.txt" />',
    )
    expect(out.blocks[1]).toBe(
      '<attached-file path="pic.png" error="binary file" />',
    )
    expect(out.failures).toEqual([
      { path: 'gone.txt', reason: 'not found: gone.txt' },
      { path: 'pic.png', reason: 'binary file' },
    ])
    expect(out.attachments).toEqual([])
  })

  it('skips folder mentions and reads only files', async () => {
    const trigger = vi.fn().mockResolvedValue({
      results: [
        { path: '/w/src/a.rs', success: true, content: 'x', is_utf8: true },
      ],
    })
    const out = await expandFileMentions('/w', ['src/', 'src/a.rs'], trigger)
    expect(trigger).toHaveBeenCalledWith(READ_FILE_FUNCTION_ID, {
      paths: ['src/a.rs'],
      fs_scope: { root: '/w' },
    })
    expect(out.blocks).toHaveLength(1)
    expect(out.blocks[0]).toContain('path="src/a.rs"')
    expect(out.failures).toEqual([])
  })

  it('makes no read call when only folders are mentioned', async () => {
    const trigger = vi.fn()
    const out = await expandFileMentions('/w', ['src/'], trigger)
    expect(trigger).not.toHaveBeenCalled()
    expect(out).toEqual({ blocks: [], attachments: [], failures: [] })
  })

  it('degrades every mention to a failure when the batch call throws', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('shell worker away'))
    const out = await expandFileMentions('/w', ['a.txt', 'b.txt'], trigger)
    expect(out.blocks).toHaveLength(2)
    expect(out.failures.map((f) => f.reason)).toEqual([
      'shell worker away',
      'shell worker away',
    ])
  })
})

describe('attached-file block helpers', () => {
  it('round-trips header attributes, including escaped quotes', () => {
    const block =
      '<attached-file path="a &quot;b&quot;.txt" size="7" total-lines="3" truncated="true">\nx\n</attached-file>'
    expect(isAttachedFileBlock(block)).toBe(true)
    expect(parseAttachedFileHeader(block)).toEqual({
      path: 'a "b".txt',
      size: 7,
      totalLines: 3,
      truncated: true,
    })
  })

  it('parses failure placeholders', () => {
    const block = '<attached-file path="gone.txt" error="not found" />'
    expect(parseAttachedFileHeader(block)).toEqual({
      path: 'gone.txt',
      error: 'not found',
    })
  })

  it('rejects non-attachment text', () => {
    expect(isAttachedFileBlock('hello')).toBe(false)
    expect(parseAttachedFileHeader('hello')).toBeNull()
    expect(parseAttachedFileHeader('<attached-file broken')).toBeNull()
  })
})

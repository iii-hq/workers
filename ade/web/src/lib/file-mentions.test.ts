import { describe, expect, it, vi } from 'vitest'
import {
  attachedFileLabel,
  expandFileMentions,
  isAttachedFileBlock,
  parseAttachedFileHeader,
  parseFileMentions,
  READ_FILE_FUNCTION_ID,
} from './file-mentions'

describe('parseFileMentions', () => {
  it('extracts unique paths in first-appearance order', () => {
    const text =
      'see #file(src/a.rs) and #file(docs/x y.md) then #file(src/a.rs) again'
    expect(parseFileMentions(text)).toEqual([
      { path: 'src/a.rs' },
      { path: 'docs/x y.md' },
    ])
  })

  it('keeps a line window apart from the whole file', () => {
    const text =
      '#file(src/a.rs:12-40) vs #file(src/a.rs) and #file(src/a.rs:7)'
    expect(parseFileMentions(text)).toEqual([
      { path: 'src/a.rs', range: { from: 12, to: 40 } },
      { path: 'src/a.rs' },
      { path: 'src/a.rs', range: { from: 7, to: 7 } },
    ])
  })

  it('returns [] when there are no mentions', () => {
    expect(parseFileMentions('plain text @fn(engine::echo) # heading')).toEqual(
      [],
    )
  })
})

describe('expandFileMentions', () => {
  it('makes no read call for a whole-file mention — the token is the reference', async () => {
    const trigger = vi.fn()
    const out = await expandFileMentions('/w', [{ path: 'src/a.rs' }], trigger)
    expect(trigger).not.toHaveBeenCalled()
    expect(out).toEqual({ blocks: [], attachments: [], failures: [] })
  })

  it('reads a line window and labels the block with it', async () => {
    const trigger = vi.fn().mockResolvedValue({
      results: [
        {
          path: '/w/src/a.rs',
          success: true,
          content: 'fn main() {\n}',
          is_utf8: true,
          total_lines: 80,
          more_lines: true,
          size: 4_000,
        },
      ],
    })
    const out = await expandFileMentions(
      '/w',
      [{ path: 'src/a.rs', range: { from: 12, to: 13 } }],
      trigger,
    )
    expect(trigger).toHaveBeenCalledWith(READ_FILE_FUNCTION_ID, {
      paths: [{ path: 'src/a.rs', line_from: 12, line_to: 13 }],
      fs_scope: { root: '/w', boundary: 'workspace' },
    })
    expect(out.blocks).toEqual([
      '<attached-file path="src/a.rs" lines="12-13" size="4000" total-lines="80">\nfn main() {\n}\n</attached-file>',
    ])
    // The chip names the window and sizes what was actually attached.
    expect(out.attachments).toEqual([{ path: 'src/a.rs:12-13', size: 13 }])
    expect(out.failures).toEqual([])
  })

  it('turns per-window failures and binary files into placeholder blocks', async () => {
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
    const out = await expandFileMentions(
      '/w',
      [
        { path: 'gone.txt', range: { from: 1, to: 3 } },
        { path: 'pic.png', range: { from: 2, to: 2 } },
      ],
      trigger,
    )
    expect(out.blocks[0]).toBe(
      '<attached-file path="gone.txt:1-3" error="not found: gone.txt" />',
    )
    expect(out.blocks[1]).toBe(
      '<attached-file path="pic.png:2" error="binary file" />',
    )
    expect(out.failures).toEqual([
      { path: 'gone.txt:1-3', reason: 'not found: gone.txt' },
      { path: 'pic.png:2', reason: 'binary file' },
    ])
    expect(out.attachments).toEqual([])
  })

  it('reads only line windows — folders and whole files stay references', async () => {
    const trigger = vi.fn().mockResolvedValue({
      results: [
        { path: '/w/src/a.rs', success: true, content: 'x', is_utf8: true },
      ],
    })
    const out = await expandFileMentions(
      '/w',
      [
        { path: 'src/' },
        { path: 'src/b.rs' },
        { path: 'src/a.rs', range: { from: 3, to: 3 } },
      ],
      trigger,
    )
    expect(trigger).toHaveBeenCalledWith(READ_FILE_FUNCTION_ID, {
      paths: [{ path: 'src/a.rs', line_from: 3, line_to: 3 }],
      fs_scope: { root: '/w', boundary: 'workspace' },
    })
    expect(out.blocks).toHaveLength(1)
    expect(out.blocks[0]).toContain('path="src/a.rs" lines="3"')
    expect(out.attachments).toEqual([{ path: 'src/a.rs:3', size: 1 }])
    expect(out.failures).toEqual([])
  })

  it('makes no read call when only folders are mentioned', async () => {
    const trigger = vi.fn()
    const out = await expandFileMentions('/w', [{ path: 'src/' }], trigger)
    expect(trigger).not.toHaveBeenCalled()
    expect(out).toEqual({ blocks: [], attachments: [], failures: [] })
  })

  it('degrades every window to a failure when the batch call throws', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('shell worker away'))
    const out = await expandFileMentions(
      '/w',
      [
        { path: 'a.txt', range: { from: 1, to: 2 } },
        { path: 'b.txt', range: { from: 5, to: 5 } },
      ],
      trigger,
    )
    expect(out.blocks).toHaveLength(2)
    expect(out.failures.map((f) => f.reason)).toEqual([
      'shell worker away',
      'shell worker away',
    ])
  })
})

describe('attached-file block helpers', () => {
  it('reads the line window back out of a header', () => {
    const header = parseAttachedFileHeader(
      '<attached-file path="src/a.rs" lines="12-40" size="9">\nx\n</attached-file>',
    )
    expect(header).toEqual({ path: 'src/a.rs', lines: '12-40', size: 9 })
    expect(attachedFileLabel(header as NonNullable<typeof header>)).toBe(
      'src/a.rs:12-40',
    )
    expect(attachedFileLabel({ path: 'src/a.rs' })).toBe('src/a.rs')
  })

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

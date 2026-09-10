import { describe, expect, it } from 'vitest'
import { readFileRequestSchema, readFileResponseSchema } from '../parsers'
import {
  deriveReadEntries,
  formatNumericMode,
  isBatchRequest,
  nextWindowStart,
  normalizeReadTarget,
  windowLabel,
} from '../ReadFileView'

describe('normalizeReadTarget', () => {
  it('expands a bare path string to whole-file defaults', () => {
    expect(normalizeReadTarget('src/lib.rs')).toEqual({
      path: 'src/lib.rs',
      lineFrom: null,
      lineTo: null,
      numbered: false,
      stat: false,
    })
  })

  it('collapses nullish object fields to one shape', () => {
    expect(
      normalizeReadTarget({
        path: 'src/config.rs',
        line_from: 1,
        line_to: 30,
        numbered: true,
      }),
    ).toEqual({
      path: 'src/config.rs',
      lineFrom: 1,
      lineTo: 30,
      numbered: true,
      stat: false,
    })
  })

  it('keeps explicit null window fields as null', () => {
    const t = normalizeReadTarget({
      path: 'a.txt',
      line_from: null,
      stat: true,
    })
    expect(t.lineFrom).toBeNull()
    expect(t.lineTo).toBeNull()
    expect(t.stat).toBe(true)
  })
})

describe('windowLabel', () => {
  it('returns null for non-windowed reads', () => {
    expect(windowLabel(null, null)).toBeNull()
  })

  it('renders a bounded window', () => {
    expect(windowLabel(10, 50)).toBe('L10–50')
  })

  it('renders an open-ended window to EOF', () => {
    expect(windowLabel(40, null)).toBe('L40–EOF')
  })

  it('defaults line_from to 1 when only line_to is set (wire rule)', () => {
    expect(windowLabel(null, 30)).toBe('L1–30')
  })
})

describe('nextWindowStart', () => {
  it('advances past the returned window', () => {
    expect(nextWindowStart(40, 11)).toBe(51)
  })

  it('treats a missing line_from as 1 (full read cut by budget)', () => {
    expect(nextWindowStart(null, 25)).toBe(26)
  })

  it('tolerates missing lines_returned', () => {
    expect(nextWindowStart(10, null)).toBe(10)
    expect(nextWindowStart(10, undefined)).toBe(10)
  })
})

describe('formatNumericMode', () => {
  it('decodes the lower 9 permission bits', () => {
    expect(formatNumericMode(420)).toBe('rw-r--r--') // 0o644
    expect(formatNumericMode(493)).toBe('rwxr-xr-x') // 0o755
  })

  it('masks a full st_mode down to the permission bits', () => {
    expect(formatNumericMode(0o100644)).toBe('rw-r--r--')
  })

  it('falls back to an em dash on junk', () => {
    expect(formatNumericMode(-1)).toBe('—')
    expect(formatNumericMode(Number.NaN)).toBe('—')
    expect(formatNumericMode(1.5)).toBe('—')
  })
})

describe('deriveReadEntries — single-path mode', () => {
  it('folds top-level scalars into one synthetic success entry', () => {
    const req = readFileRequestSchema.parse({
      path: 'src/main.rs',
      line_from: 10,
      line_to: 50,
    })
    const resp = readFileResponseSchema.parse({
      path: '/work/project/src/main.rs',
      content: 'fn main() {}\n',
      is_utf8: true,
      lines_returned: 1,
      more_lines: true,
      size: 2048,
      mode: 420,
      mtime: 1750000000,
    })
    const entries = deriveReadEntries(req, resp)
    expect(isBatchRequest(req)).toBe(false)
    expect(entries).toHaveLength(1)
    expect(entries[0].requested).toEqual({
      path: 'src/main.rs',
      lineFrom: 10,
      lineTo: 50,
      numbered: false,
      stat: false,
    })
    expect(entries[0].result?.success).toBe(true)
    expect(entries[0].result?.path).toBe('/work/project/src/main.rs')
    expect(entries[0].result?.more_lines).toBe(true)
    // total_lines absent on the wire = not fully traversed, NOT 0.
    expect(entries[0].result?.total_lines).toBeUndefined()
  })

  it('keeps result null while the response is missing (running)', () => {
    const req = readFileRequestSchema.parse({ path: 'a.txt' })
    const entries = deriveReadEntries(req, null)
    expect(entries).toHaveLength(1)
    expect(entries[0].result).toBeNull()
  })

  it('returns no entries when neither path nor paths is set (C210)', () => {
    const req = readFileRequestSchema.parse({})
    expect(deriveReadEntries(req, null)).toEqual([])
  })
})

describe('deriveReadEntries — batch mode', () => {
  it('aligns results to targets by index, mixed success and failure', () => {
    const req = readFileRequestSchema.parse({
      paths: [
        'src/lib.rs',
        { path: 'src/config.rs', line_from: 1, line_to: 30, numbered: true },
        { path: 'huge.bin', stat: true },
      ],
    })
    const resp = readFileResponseSchema.parse({
      results: [
        {
          path: '/work/project/src/lib.rs',
          success: true,
          content: 'pub mod config;\n',
          is_utf8: true,
          lines_returned: 1,
          total_lines: 1,
          more_lines: false,
          size: 16,
        },
        {
          path: '/work/project/src/config.rs',
          success: false,
          error: {
            code: 'C213',
            message:
              'batch_read_budget_bytes (1048576) exhausted after 1048560 bytes — retry this entry in its own call',
          },
        },
      ],
    })
    const entries = deriveReadEntries(req, resp)
    expect(isBatchRequest(req)).toBe(true)
    expect(entries).toHaveLength(3)
    expect(entries[0].requested.path).toBe('src/lib.rs')
    expect(entries[0].result?.success).toBe(true)
    expect(entries[1].requested.numbered).toBe(true)
    expect(entries[1].result?.success).toBe(false)
    expect(entries[1].result?.error?.code).toBe('C213')
    // Fewer results than targets — trailing entries stay pending/null.
    expect(entries[2].requested.stat).toBe(true)
    expect(entries[2].result).toBeNull()
  })

  it('keeps every result null when the response is missing', () => {
    const req = readFileRequestSchema.parse({ paths: ['a.txt', 'b.txt'] })
    const entries = deriveReadEntries(req, null)
    expect(entries).toHaveLength(2)
    expect(entries.every((e) => e.result === null)).toBe(true)
  })
})

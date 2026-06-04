import { describe, expect, it } from 'vitest'
import {
  CODER_MUTATE_FUNCTION_IDS,
  createFileRequestSchema,
  createFileResponseSchema,
  deleteFileRequestSchema,
  deleteFileResponseSchema,
  formatUpdateOp,
  isCoderMutateFunction,
  safeParseRequest,
  safeParseResponse,
  unwrapEnvelope,
  updateFileRequestSchema,
  updateFileResponseSchema,
} from '../parsers'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: true,
  }
}

describe('isCoderMutateFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of CODER_MUTATE_FUNCTION_IDS) {
      expect(isCoderMutateFunction(id)).toBe(true)
    }
  })

  it('rejects unrelated ids', () => {
    expect(isCoderMutateFunction('coder::read-file')).toBe(false)
    expect(isCoderMutateFunction('sandbox::exec')).toBe(false)
  })
})

describe('createFileRequestSchema', () => {
  it('accepts a batched create request', () => {
    const r = safeParseRequest(createFileRequestSchema, {
      files: [{ path: 'src/main.ts', content: 'export {}\n' }],
    })
    expect(r?.files[0]?.path).toBe('src/main.ts')
  })

  it('rejects empty files array', () => {
    expect(safeParseRequest(createFileRequestSchema, { files: [] })).toBeNull()
  })
})

describe('createFileResponseSchema', () => {
  it('parses raw success', () => {
    const r = safeParseResponse(createFileResponseSchema, {
      results: [{ path: 'a.txt', success: true, bytes_written: 5 }],
    })
    expect(r?.results[0]?.bytes_written).toBe(5)
  })

  it('parses wrapped partial failure', () => {
    const r = safeParseResponse(
      createFileResponseSchema,
      wrap({
        results: [
          {
            path: '.env',
            success: false,
            bytes_written: 0,
            error: 'C211: not accessible',
          },
          { path: 'ok.txt', success: true, bytes_written: 1 },
        ],
      }),
    )
    expect(r?.results).toHaveLength(2)
    expect(r?.results[0]?.success).toBe(false)
  })
})

describe('updateFileRequestSchema', () => {
  it('accepts mixed ops', () => {
    const r = safeParseRequest(updateFileRequestSchema, {
      files: [
        {
          path: 'lib.rs',
          ops: [
            { op: 'insert', at_line: 1, content: '// header\n' },
            { op: 'remove', from_line: 10, to_line: 12 },
            {
              op: 'update_lines',
              from_line: 4,
              to_line: 4,
              content: 'fn main() {}\n',
            },
            {
              op: 'replace',
              pattern: 'foo',
              replacement: 'bar',
              ignore_case: true,
            },
          ],
        },
      ],
    })
    expect(r?.files[0]?.ops).toHaveLength(4)
    const ops = r?.files[0]?.ops
    if (!ops?.[0] || !ops[3]) throw new Error('expected ops')
    expect(formatUpdateOp(ops[0])).toBe('insert @ L1')
    expect(formatUpdateOp(ops[3])).toContain('replace')
  })
})

describe('updateFileResponseSchema', () => {
  it('unwraps harness envelope', () => {
    const payload = {
      results: [
        { path: 'a.txt', success: true, applied: 2, new_line_count: 10 },
      ],
    }
    expect(unwrapEnvelope(wrap(payload))).toEqual(payload)
    const r = safeParseResponse(updateFileResponseSchema, wrap(payload))
    expect(r?.results[0]?.applied).toBe(2)
  })

  it('parses before/after snapshots', () => {
    const r = safeParseResponse(updateFileResponseSchema, {
      results: [
        {
          path: 'src/index.ts',
          success: true,
          applied: 1,
          new_line_count: 12,
          before: 'const x = 1\n',
          after: 'const x = 2\n',
        },
      ],
    })
    expect(r?.results[0]?.before).toBe('const x = 1\n')
    expect(r?.results[0]?.after).toBe('const x = 2\n')
  })
})

describe('deleteFileRequestSchema', () => {
  it('accepts recursive batch delete', () => {
    const r = safeParseRequest(deleteFileRequestSchema, {
      paths: ['tmp/', 'old.txt'],
      recursive: true,
    })
    expect(r?.recursive).toBe(true)
    expect(r?.paths).toHaveLength(2)
  })
})

describe('deleteFileResponseSchema', () => {
  it('parses idempotent miss', () => {
    const r = safeParseResponse(deleteFileResponseSchema, {
      results: [{ path: 'gone.txt', success: true, removed: false }],
    })
    expect(r?.results[0]?.removed).toBe(false)
  })
})

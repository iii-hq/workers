import { describe, expect, it } from 'vitest'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import {
  isFpFunction,
  isFpTransformFunction,
  FP_TRANSFORM_OPS,
  pipeRequestSchema,
  pipeResponseSchema,
  safeParseRequest,
  transformOp,
  unwrapEnvelope,
  utilRequestSchema,
  utilResponseSchema,
} from '../parsers'

/** entry-mapper success shape: { content, details }. */
function resultEnvelope(text: string, details: unknown) {
  return { content: [{ type: 'text', text }], details }
}

/** entry-mapper error shape (functionResultOutput, is_error branch). */
function errorEnvelope(code: string, message: string) {
  return {
    error: {
      kind: 'function_error',
      message,
      details: { error: code, message },
      content: [{ type: 'text', text: message }],
    },
  }
}

describe('isFpFunction', () => {
  it('matches the pipe and every transform id', () => {
    expect(isFpFunction('fp::pipe')).toBe(true)
    for (const op of FP_TRANSFORM_OPS) {
      expect(isFpFunction(`fp::${op}`)).toBe(true)
      expect(isFpTransformFunction(`fp::${op}`)).toBe(true)
      expect(transformOp(`fp::${op}`)).toBe(op)
    }
  })

  it('rejects the internal hook function and unrelated ids', () => {
    // explicit id set, NOT a prefix match — the worker's internal plumbing
    // must never fall into the transform view
    expect(isFpFunction('fp::inject-guidance')).toBe(false)
    expect(isFpTransformFunction('fp::pipe')).toBe(false)
    expect(isFpTransformFunction('fp::getter')).toBe(false)
    expect(isFpFunction('harness::pipe')).toBe(false)
    expect(isFpFunction('fp::')).toBe(false)
  })
})

describe('pipe schemas', () => {
  it('parses the canonical through request', () => {
    const r = safeParseRequest(pipeRequestSchema, {
      through: [
        { function: 'scrapling::fetch', payload: { url: 'u' } },
        { function: 'fp::get', payload: { path: '/content' } },
        { function: 'state::set', payload: { scope: 's' }, into: '/value' },
      ],
      preview_chars: 200,
    })
    expect(r?.through?.length).toBe(3)
    expect(r?.through?.[2]?.into).toBe('/value')
  })

  it('tolerates a clipped approval excerpt', () => {
    expect(safeParseRequest(pipeRequestSchema, {})).not.toBeNull()
  })

  it('parses receipts + preview out of success details', () => {
    const details = {
      steps: [
        { function: 'scrapling::fetch', chars: 84213 },
        { function: 'state::set', chars: 46 },
      ],
      value_preview: '## Circuit breakers…',
    }
    const parsed = pipeResponseSchema.safeParse(
      unwrapEnvelope(resultEnvelope(JSON.stringify(details), details)),
    )
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.steps?.[0]?.chars).toBe(84213)
      expect(parsed.data.value_preview).toContain('Circuit')
    }
  })

  it('routes a step failure to the invocation error display', () => {
    const display = parseSandboxErrorDisplay(
      errorEnvelope(
        'handler_error',
        'pipe failed at step 2 (fp::get): path "/body" matched nothing · completed: scrapling::fetch→84213ch',
      ),
    )
    expect(display?.variant).toBe('invocation')
  })
})

describe('util schemas', () => {
  it('reads per-op params from one loose request schema', () => {
    expect(
      safeParseRequest(utilRequestSchema, { value: {}, path: '/a' })?.path,
    ).toBe('/a')
    expect(
      safeParseRequest(utilRequestSchema, { value: {}, paths: ['a', 'b'] })
        ?.paths,
    ).toEqual(['a', 'b'])
    expect(safeParseRequest(utilRequestSchema, { value: 's', n: 20 })?.n).toBe(
      20,
    )
    expect(
      safeParseRequest(utilRequestSchema, {
        value: [],
        matches: { status: 'active' },
      })?.matches,
    ).toEqual({ status: 'active' })
    // fp::nth negative index and fp::getOr's default both parse
    expect(safeParseRequest(utilRequestSchema, { value: [], n: -1 })?.n).toBe(
      -1,
    )
    expect(
      safeParseRequest(utilRequestSchema, {
        value: {},
        path: '/etag',
        default: 'no-etag',
      })?.default,
    ).toBe('no-etag')
  })

  it('unwraps the UtilResponse value wrapper', () => {
    const unwrapped = unwrapEnvelope(resultEnvelope('hé', { value: 'hé' }))
    const parsed = utilResponseSchema.safeParse(unwrapped)
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.value).toBe('hé')
  })

  it('rejects a non-wrapper value so views fall back to the raw output', () => {
    expect(utilResponseSchema.safeParse('bare string').success).toBe(false)
  })
})

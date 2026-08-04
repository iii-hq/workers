import { describe, expect, it } from 'vitest'
import { redactAttributeEntries } from './redactAttributes'

const SECRET = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const MASKED = 'rt-3f9a…'

/** A worker-shaped redactor, matching the code-runner/code-runner shape. */
function mask(value: unknown): unknown {
  if (typeof value === 'string') return value.replaceAll(SECRET, MASKED)
  if (value === null || typeof value !== 'object') return value
  if (Array.isArray(value)) return value.map(mask)
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([k, v]) => [
      mask(k),
      mask(v),
    ]),
  )
}

describe('redactAttributeEntries', () => {
  it('runs every value through redact, keys untouched', () => {
    const out = redactAttributeEntries(
      { 'tool.arguments': { runtime_id: SECRET }, 'span.kind': 'internal' },
      mask,
    )
    expect(out).toEqual([
      ['tool.arguments', { runtime_id: MASKED }],
      ['span.kind', 'internal'],
    ])
  })

  it('passes values through unchanged when redact is absent', () => {
    const attrs = { 'tool.arguments': { runtime_id: SECRET } }
    expect(redactAttributeEntries(attrs)).toEqual([
      ['tool.arguments', { runtime_id: SECRET }],
    ])
  })

  it('treats missing attributes as empty', () => {
    expect(redactAttributeEntries(undefined, mask)).toEqual([])
  })
})

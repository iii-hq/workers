import { describe, expect, it } from 'vitest'
import { parseEmbeddedJson, resultEnvelope } from './FunctionCallCard'

describe('parseEmbeddedJson', () => {
  it('parses double-encoded objects and arrays', () => {
    expect(parseEmbeddedJson('{"trigger_type": "state"}')).toEqual({
      trigger_type: 'state',
    })
    expect(parseEmbeddedJson('  [1, 2]  ')).toEqual([1, 2])
  })

  it('leaves scalars and non-JSON strings alone', () => {
    expect(parseEmbeddedJson('123')).toBeUndefined()
    expect(parseEmbeddedJson('true')).toBeUndefined()
    expect(parseEmbeddedJson('plain text')).toBeUndefined()
    expect(parseEmbeddedJson('{broken')).toBeUndefined()
  })
})

describe('resultEnvelope', () => {
  const details = { configuration_schema: { type: 'object' } }

  it('unwraps the content+details result envelope', () => {
    expect(
      resultEnvelope({
        content: [{ type: 'text', text: JSON.stringify(details) }],
        details,
      }),
    ).toEqual({ texts: [JSON.stringify(details)], details })
  })

  it('accepts content-only envelopes', () => {
    expect(resultEnvelope({ content: [{ type: 'text', text: 'hi' }] })).toEqual(
      { texts: ['hi'], details: undefined },
    )
  })

  it('rejects unknown shapes so raw rendering stays truthful', () => {
    // Extra keys → not the envelope.
    expect(
      resultEnvelope({ content: [{ type: 'text', text: 'x' }], extra: 1 }),
    ).toBeNull()
    // Non-text block → not unwrappable.
    expect(
      resultEnvelope({ content: [{ type: 'image', data: 'x' }] }),
    ).toBeNull()
    // Nothing to show → let the empty branch handle it.
    expect(resultEnvelope({ content: [] })).toBeNull()
    expect(resultEnvelope('string')).toBeNull()
    expect(resultEnvelope(null)).toBeNull()
    // The error envelope keeps its dedicated path.
    expect(resultEnvelope({ error: { kind: 'boom' } })).toBeNull()
  })
})

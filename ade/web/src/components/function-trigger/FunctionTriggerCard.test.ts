import { describe, expect, it } from 'vitest'
import {
  isDeniedOutput,
  parseEmbeddedJson,
  resultEnvelope,
} from './FunctionTriggerCard'

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

describe('isDeniedOutput', () => {
  it('recognizes the approval gate’s denial envelope in the error details', () => {
    // Shape per approval-gate denial.rs → entry-mapper functionResultOutput:
    // the envelope rides in the paired result's `details`.
    expect(
      isDeniedOutput({
        error: {
          kind: 'function_error',
          message: 'Rejected by operator.',
          details: {
            schema_version: 1,
            status: 'denied',
            denied_by: 'user',
            function_id: 'shell::run',
            reason: 'Rejected by operator.',
          },
        },
      }),
    ).toBe(true)
  })

  it('leaves genuine run errors and non-errors alone', () => {
    // A run error without the envelope: the call executed and failed.
    expect(
      isDeniedOutput({
        error: { kind: 'function_error', message: 'boom', details: {} },
      }),
    ).toBe(false)
    expect(
      isDeniedOutput({ error: { kind: 'function_error', message: 'boom' } }),
    ).toBe(false)
    // `status: denied` alone (no denied_by) is not the gate's envelope.
    expect(
      isDeniedOutput({
        error: { kind: 'function_error', details: { status: 'denied' } },
      }),
    ).toBe(false)
    expect(isDeniedOutput({ content: [], details: {} })).toBe(false)
    expect(isDeniedOutput(undefined)).toBe(false)
    expect(isDeniedOutput(null)).toBe(false)
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

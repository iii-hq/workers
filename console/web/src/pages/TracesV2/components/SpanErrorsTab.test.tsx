/**
 * Closes the fourth sibling-tab instance of console-UI review finding #1:
 * SpanErrorsTab reads `exception.message`/`exception.stacktrace` off the
 * SAME `exception` event `functionTriggerFromSpan.ts`'s `exceptionOutput()`
 * reads to build the info tab's redacted error output, renders them
 * verbatim, and has its own ungated stack-trace copy button.
 *
 * `renderToStaticMarkup`, no jsdom — see redact-raw.test.tsx. The
 * click-to-copy value is pinned the same way SpanTagsTab.test.tsx pins it:
 * composing the pure redaction step with what the copy handler is handed,
 * since a DOM click can't be simulated here.
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { redactValue } from '../lib/redactAttributes'
import type { VisualizationSpan } from '../lib/traceTransform'
import { SpanErrorsTab } from './SpanErrorsTab'

const SECRET = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const MASKED = 'rt-3f9a…'

function maskDeep(value: unknown): unknown {
  if (typeof value === 'string') return value.replaceAll(SECRET, MASKED)
  if (value === null || typeof value !== 'object') return value
  if (Array.isArray(value)) return value.map(maskDeep)
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([k, v]) => [
      maskDeep(k),
      maskDeep(v),
    ]),
  )
}

function errorSpan(): VisualizationSpan {
  return {
    name: 'execute code-runner::eval',
    span_id: 's-1',
    trace_id: 't-1',
    duration_ms: 12,
    status: 'error',
    depth: 0,
    start_percent: 0,
    width_percent: 100,
    attributes: {},
    events: [
      {
        name: 'exception',
        timestamp_unix_nano: 1,
        attributes: {
          'exception.type': 'NamespaceDenied',
          'exception.message': `registered id "x" must start with this runtime's namespace "code-runner::${SECRET}::"`,
          'exception.stacktrace': `at eval (code-runner::${SECRET}::hello)`,
        },
      },
    ],
    links: [],
    pending: false,
  }
}

const SPAN = errorSpan()

describe('SpanErrorsTab redaction', () => {
  it('redacts the exception message, type context, and stack trace when redact is given', () => {
    const html = renderToStaticMarkup(
      <SpanErrorsTab span={SPAN} redact={maskDeep} />,
    )
    expect(html).not.toContain(SECRET)
    expect(html).toContain(MASKED)
  })

  it('leaves the exception untouched without a redact prop (positive control)', () => {
    const html = renderToStaticMarkup(<SpanErrorsTab span={SPAN} />)
    expect(html).toContain(SECRET)
  })
})

const RAW_STACK = SPAN.events[0].attributes['exception.stacktrace'] as string

describe('the stack-trace copy value (pinned without simulating a click)', () => {
  it('is redacted: the value copyStackTrace is handed is already redacted', () => {
    const copied = redactValue(RAW_STACK, maskDeep) as string
    expect(copied).not.toContain(SECRET)
    expect(copied).toContain(MASKED)
  })

  it('positive control: the same value leaks without a redactor', () => {
    expect(redactValue(RAW_STACK)).toBe(RAW_STACK)
    expect(RAW_STACK).toContain(SECRET)
  })
})

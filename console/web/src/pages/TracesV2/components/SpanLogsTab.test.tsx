/**
 * Closes console-UI review finding #1 for the EVENTS tab: it pretty-prints
 * every event attribute — `iii.payload.json` (the iii-sdk auto-capture
 * payload) included — verbatim into a `<pre>`. The info tab's card redacts
 * the identical payload; this tab did not.
 *
 * `renderToStaticMarkup`, no jsdom — see redact-raw.test.tsx.
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { VisualizationSpan } from '../lib/traceTransform'
import { SpanLogsTab } from './SpanLogsTab'

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

function spanWithPayloadEvent(payloadJson: string): VisualizationSpan {
  return {
    name: 'execute code-runner::eval',
    span_id: 's-1',
    trace_id: 't-1',
    duration_ms: 12,
    status: 'ok',
    depth: 0,
    start_percent: 0,
    width_percent: 100,
    attributes: {},
    events: [
      {
        name: 'iii.payload',
        timestamp_unix_nano: 1,
        attributes: { 'iii.payload.json': payloadJson },
      },
    ],
    links: [],
    pending: false,
  }
}

const SPAN = spanWithPayloadEvent(
  JSON.stringify({ runtime_id: SECRET, code: '1+1' }),
)

describe('SpanLogsTab redaction', () => {
  it('redacts the iii.payload.json event attribute pretty-printed into the <pre>', () => {
    const html = renderToStaticMarkup(
      <SpanLogsTab span={SPAN} redact={maskDeep} />,
    )
    expect(html).not.toContain(SECRET)
    expect(html).toContain(MASKED)
    // Still pretty-printed JSON, not degraded to a single-line fallback —
    // the regex substitution preserves the surrounding string's syntax.
    // (renderToStaticMarkup HTML-escapes quotes in text content.)
    expect(html).toContain('&quot;code&quot;')
  })

  it('leaves the event untouched without a redact prop (positive control)', () => {
    const html = renderToStaticMarkup(<SpanLogsTab span={SPAN} />)
    expect(html).toContain(SECRET)
  })
})

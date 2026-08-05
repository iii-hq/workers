/**
 * Closes console-UI review finding #1 for the ATTRIBUTES tab: it renders
 * `tool.arguments` (and every other span attribute) and makes each row
 * click-to-copy. The info tab's card redacts the identical payload
 * (functionTriggerFromSpan.ts reads the same sources); this tab did not,
 * so the runtime_id the card hid was one tab-click away.
 *
 * Render assertions go through `renderToStaticMarkup` (no jsdom/testing
 * -library — see redact-raw.test.tsx for the established pattern). The
 * click-to-copy value can't be proven by simulating a click without jsdom,
 * so it is pinned at the same seam that test file uses: the pure function
 * (`attributeCopyText`) fed the exact value `redactAttributeEntries`
 * already redacted — the same `value` the row's `onClick` closes over.
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { redactAttributeEntries } from '../lib/redactAttributes'
import type { VisualizationSpan } from '../lib/traceTransform'
import { attributeCopyText, SpanTagsTab } from './SpanTagsTab'

const SECRET = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const MASKED = 'rt-3f9a…'

/** A worker-shaped redactor, matching sandbox-code-runner/sandbox-code-runner's shape. */
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

function span(attributes: Record<string, unknown>): VisualizationSpan {
  return {
    name: 'execute sandbox-code-runner::run',
    span_id: 's-1',
    trace_id: 't-1',
    duration_ms: 12,
    status: 'ok',
    depth: 0,
    start_percent: 0,
    width_percent: 100,
    attributes,
    events: [],
    links: [],
    pending: false,
  }
}

const SPAN = span({
  'tool.arguments': JSON.stringify({ runtime_id: SECRET, code: '1+1' }),
})

describe('SpanTagsTab redaction', () => {
  it('redacts an attribute value when redact is given', () => {
    const html = renderToStaticMarkup(
      <SpanTagsTab span={SPAN} redact={maskDeep} />,
    )
    expect(html).not.toContain(SECRET)
    expect(html).toContain(MASKED)
  })

  it('leaves attributes untouched without a redact prop (positive control)', () => {
    // Proves the assertions above are not vacuous: the same payload really
    // does reach the HTML verbatim through this exact code path when
    // nothing claims it.
    const html = renderToStaticMarkup(<SpanTagsTab span={SPAN} />)
    expect(html).toContain(SECRET)
  })

  it('leaves an unrelated attribute value untouched even when redact is given', () => {
    const html = renderToStaticMarkup(
      <SpanTagsTab
        span={span({ 'span.kind': 'internal' })}
        redact={maskDeep}
      />,
    )
    expect(html).toContain('internal')
  })
})

describe('the click-to-copy value (pinned without simulating a click)', () => {
  it('is redacted: attributeCopyText fed the value redactAttributeEntries produced', () => {
    const [[key, value]] = redactAttributeEntries(SPAN.attributes, maskDeep)
    const copied = attributeCopyText(key, value)
    expect(copied).not.toContain(SECRET)
    expect(copied).toContain(MASKED)
  })

  it('positive control: the same composition leaks without a redactor', () => {
    const [[key, value]] = redactAttributeEntries(SPAN.attributes)
    expect(attributeCopyText(key, value)).toContain(SECRET)
  })
})

// Bounds the WaterfallChart minimap to a fixed number of markers. The
// minimap previously mounted one absolutely-positioned DOM node per span
// for ALL spans (2000+ on large traces); this downsamples into a capped
// number of rows while keeping the vertical density overview and surfacing
// error spans.

import { describe, expect, it } from 'vitest'
import { type MinimapSpan, sampleMinimapMarkers } from './minimapMarkers'

const span = (overrides: Partial<MinimapSpan> = {}): MinimapSpan => ({
  span_id: 's',
  status: 'ok',
  start_percent: 0,
  width_percent: 10,
  ...overrides,
})

describe('sampleMinimapMarkers', () => {
  it('returns no markers for no spans', () => {
    expect(sampleMinimapMarkers([], 200)).toEqual([])
  })

  it('keeps every span when under the cap, with index-proportional top', () => {
    const spans = [
      span({ span_id: 'a' }),
      span({ span_id: 'b' }),
      span({ span_id: 'c', status: 'error' }),
      span({ span_id: 'd' }),
    ]
    const markers = sampleMinimapMarkers(spans, 200)
    expect(markers).toHaveLength(4)
    expect(markers[0].topPercent).toBe(0)
    expect(markers[2].topPercent).toBe(50)
    expect(markers[2].isError).toBe(true)
  })

  it('caps the marker count when there are more spans than the budget', () => {
    const spans = Array.from({ length: 5000 }, (_, i) =>
      span({ span_id: `s${i}` }),
    )
    const markers = sampleMinimapMarkers(spans, 200)
    expect(markers.length).toBeLessThanOrEqual(200)
    expect(markers.length).toBeGreaterThan(0)
  })

  it('emits unique keys so React does not collapse markers', () => {
    const spans = Array.from({ length: 1000 }, () => span({ span_id: 'dup' }))
    const markers = sampleMinimapMarkers(spans, 200)
    const keys = new Set(markers.map((m) => m.key))
    expect(keys.size).toBe(markers.length)
  })

  it('surfaces an error span as its bucket representative', () => {
    // One error needle buried in a haystack larger than the budget must
    // still appear (errors are the whole point of the minimap).
    const spans = Array.from({ length: 1000 }, (_, i) =>
      span({ span_id: `s${i}`, status: i === 500 ? 'error' : 'ok' }),
    )
    const markers = sampleMinimapMarkers(spans, 200)
    expect(markers.some((m) => m.isError)).toBe(true)
  })

  it('keeps top values within 0..100 and ascending', () => {
    const spans = Array.from({ length: 1000 }, (_, i) =>
      span({ span_id: `s${i}` }),
    )
    const markers = sampleMinimapMarkers(spans, 200)
    for (const m of markers) {
      expect(m.topPercent).toBeGreaterThanOrEqual(0)
      expect(m.topPercent).toBeLessThan(100)
    }
    for (let i = 1; i < markers.length; i++) {
      expect(markers[i].topPercent).toBeGreaterThan(markers[i - 1].topPercent)
    }
  })
})

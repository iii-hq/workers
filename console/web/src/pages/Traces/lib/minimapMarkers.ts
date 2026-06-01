/**
 * Downsample a span list into a bounded set of minimap markers.
 *
 * The WaterfallChart minimap used to mount one absolutely-positioned DOM
 * node per span for the entire trace (2000+ nodes on large traces, plus a
 * full reconcile whenever the data changed). This buckets the spans into at
 * most `maxMarkers` rows, preserving the vertical density overview while
 * bounding the DOM node count regardless of trace size. Errors win their
 * bucket so a single failed span in a huge trace still shows up.
 */

export interface MinimapSpan {
  span_id: string
  status: string
  start_percent: number
  width_percent: number
}

export interface MinimapMarker {
  /** Stable, collision-free React key. */
  key: string
  /** Vertical position 0..100. */
  topPercent: number
  leftPercent: number
  widthPercent: number
  isError: boolean
}

function toMarker(
  span: MinimapSpan,
  key: string,
  topPercent: number,
): MinimapMarker {
  return {
    key,
    topPercent,
    leftPercent: span.start_percent,
    widthPercent: span.width_percent,
    isError: span.status === 'error',
  }
}

export function sampleMinimapMarkers(
  spans: readonly MinimapSpan[],
  maxMarkers = 200,
): MinimapMarker[] {
  const n = spans.length
  if (n === 0) return []

  // Small traces: render every span (unchanged behaviour).
  if (n <= maxMarkers) {
    return spans.map((s, i) => toMarker(s, `m${i}`, (i / n) * 100))
  }

  // Large traces: bucket by index. Each bucket yields one representative,
  // preferring an error span so failures are never sampled away.
  const markers: MinimapMarker[] = []
  for (let b = 0; b < maxMarkers; b++) {
    const start = Math.floor((b / maxMarkers) * n)
    const end = Math.floor(((b + 1) / maxMarkers) * n)
    if (end <= start) continue

    let rep = spans[start]
    for (let i = start; i < end; i++) {
      if (spans[i].status === 'error') {
        rep = spans[i]
        break
      }
    }
    markers.push(toMarker(rep, `b${b}`, (b / maxMarkers) * 100))
  }
  return markers
}

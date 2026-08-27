// Paged loading of one trace's full span set, sized by RESPONSE BYTES rather
// than span count. A span count is the wrong unit: measured against a live
// engine, a trace of ~75KB spans (session::update-message payloads) served a
// 200-span page as a 15MB response in ~1s, while a 230-span page — past the
// transport's ~16MiB message cap — never arrived at all. No error, no
// rejection: the RPC simply hangs forever, which is also why the fetch here
// carries its own timeout (the client wrapper exposes none).
//
// The first page is a small probe; its serialized size prices the trace's
// spans and sizes every following page to a budget well under the cap. The
// timeout is the backstop for a mispriced page (one giant late span): shrink
// and retry the same window, and only a page that cannot be delivered even
// at the floor fails the load.

export const TRACE_DETAIL_MAX_PAGE_SIZE = 250
export const TRACE_DETAIL_PROBE_PAGE_SIZE = 50
export const TRACE_DETAIL_MIN_PAGE_SIZE = 25
/** Response budget per page — half the observed ~16MiB delivery cliff. */
export const TRACE_DETAIL_SAFE_RESPONSE_BYTES = 8 * 1024 * 1024
export const TRACE_DETAIL_PAGE_TIMEOUT_MS = 12_000

export interface TraceDetailPage<S> {
  spans: S[]
  /** Count of the FILTERED span set (verified: `include_internal` applies
   *  before pagination and `total`, so full pages mean more to fetch). */
  total: number
}

const TIMED_OUT = Symbol('trace-detail-page-timeout')

function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
): Promise<T | typeof TIMED_OUT> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => resolve(TIMED_OUT), ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (err) => {
        clearTimeout(timer)
        reject(err)
      },
    )
  })
}

export interface CollectTraceDetailOptions<S> {
  timeoutMs?: number
  /**
   * Called after each non-empty page merges, with the accumulated map (the
   * same live map the promise resolves with — it keeps growing) and the
   * filtered span total the page reported. Lets the caller render
   * progressively — and show how far along the sweep is — instead of
   * holding a skeleton behind the full multi-second load.
   */
  onPage?: (spans: Map<string, S>, total: number) => void
}

/**
 * Fetch every span of one trace through `fetchPage(offset, limit)`,
 * deduplicated by `span_id`. Stops on a short page (the end, per the
 * verified filtered `total` semantics — also the guard that keeps a stale
 * `total` from looping while a trace is completing).
 */
/** Windows fired together after the probe. The scoped/search list scan
 *  costs seconds per call server-side REGARDLESS of limit or offset
 *  (measured: ~3.4s flat), so sequential pages would multiply it. */
export const SEED_WINDOW_PARALLELISM = 3

/**
 * Fetch the most-recent window of a `search_all_spans` query — up to
 * `maxSpans` spans — as byte-priced pages: a probe first, then the
 * remaining windows in parallel batches. A window whose response never
 * arrives (oversized — the transport drops it silently) splits in half and
 * retries; only a window undeliverable at the floor fails the read.
 * Returns the deduped spans plus the server's filtered total.
 */
export async function collectRecentSpanWindow<S extends { span_id: string }>(
  fetchPage: (offset: number, limit: number) => Promise<TraceDetailPage<S>>,
  maxSpans: number,
  timeoutMs: number = TRACE_DETAIL_PAGE_TIMEOUT_MS,
): Promise<{ spans: S[]; total: number }> {
  let probeSize = Math.min(TRACE_DETAIL_PROBE_PAGE_SIZE, maxSpans)
  let probe: TraceDetailPage<S> | typeof TIMED_OUT
  for (;;) {
    probe = await withTimeout(fetchPage(0, probeSize), timeoutMs)
    if (probe !== TIMED_OUT) break
    if (probeSize <= TRACE_DETAIL_MIN_PAGE_SIZE) {
      throw new Error(
        `trace page of ${probeSize} spans never arrived — spans too large to deliver`,
      )
    }
    probeSize = Math.max(TRACE_DETAIL_MIN_PAGE_SIZE, Math.floor(probeSize / 2))
  }

  const spans = new Map<string, S>()
  for (const span of probe.spans) spans.set(span.span_id, span)
  const target = Math.min(probe.total, maxSpans)
  if (probe.spans.length < probeSize || spans.size >= target) {
    return { spans: [...spans.values()], total: probe.total }
  }

  // A recency window mixes spans from different calls, so the probe samples
  // only its thin end (measured: 28KB/span up front, ~83KB deeper in). Half
  // the detail budget absorbs that heterogeneity — an under-priced window
  // costs a full timeout+split round, far more than a smaller page does.
  const bytesPerSpan =
    JSON.stringify(probe.spans).length / Math.max(1, probe.spans.length)
  const pageSize = Math.min(
    TRACE_DETAIL_MAX_PAGE_SIZE,
    Math.max(
      TRACE_DETAIL_MIN_PAGE_SIZE,
      Math.floor(TRACE_DETAIL_SAFE_RESPONSE_BYTES / 2 / bytesPerSpan),
    ),
  )

  const fetchWindow = async (offset: number, limit: number): Promise<S[]> => {
    const page = await withTimeout(fetchPage(offset, limit), timeoutMs)
    if (page !== TIMED_OUT) return page.spans
    if (limit <= TRACE_DETAIL_MIN_PAGE_SIZE) {
      throw new Error(
        `trace page of ${limit} spans never arrived — spans too large to deliver`,
      )
    }
    const half = Math.ceil(limit / 2)
    const halves = await Promise.all([
      fetchWindow(offset, half),
      fetchWindow(offset + half, limit - half),
    ])
    return halves.flat()
  }

  const windows: Array<[number, number]> = []
  for (let offset = probe.spans.length; offset < target; offset += pageSize) {
    windows.push([offset, Math.min(pageSize, target - offset)])
  }
  for (let i = 0; i < windows.length; i += SEED_WINDOW_PARALLELISM) {
    const batch = windows.slice(i, i + SEED_WINDOW_PARALLELISM)
    const results = await Promise.all(
      batch.map(([offset, limit]) => fetchWindow(offset, limit)),
    )
    for (const win of results) {
      for (const span of win) spans.set(span.span_id, span)
    }
  }
  return { spans: [...spans.values()], total: probe.total }
}

export async function collectTraceDetailSpans<S extends { span_id: string }>(
  fetchPage: (offset: number, limit: number) => Promise<TraceDetailPage<S>>,
  opts?: CollectTraceDetailOptions<S>,
): Promise<Map<string, S>> {
  const timeoutMs = opts?.timeoutMs ?? TRACE_DETAIL_PAGE_TIMEOUT_MS
  const spans = new Map<string, S>()
  let pageSize = TRACE_DETAIL_PROBE_PAGE_SIZE
  let priced = false
  let offset = 0
  let total = Number.POSITIVE_INFINITY

  while (offset < total) {
    const limit = pageSize
    const page = await withTimeout(fetchPage(offset, limit), timeoutMs)
    if (page === TIMED_OUT) {
      if (pageSize <= TRACE_DETAIL_MIN_PAGE_SIZE) {
        throw new Error(
          `trace page of ${pageSize} spans never arrived — spans too large to deliver`,
        )
      }
      pageSize = Math.max(TRACE_DETAIL_MIN_PAGE_SIZE, Math.floor(pageSize / 2))
      continue
    }

    for (const span of page.spans) spans.set(span.span_id, span)
    total = page.total
    if (page.spans.length > 0) opts?.onPage?.(spans, total)

    if (!priced && page.spans.length > 0) {
      priced = true
      const bytesPerSpan = JSON.stringify(page.spans).length / page.spans.length
      pageSize = Math.min(
        TRACE_DETAIL_MAX_PAGE_SIZE,
        Math.max(
          TRACE_DETAIL_MIN_PAGE_SIZE,
          Math.floor(TRACE_DETAIL_SAFE_RESPONSE_BYTES / bytesPerSpan),
        ),
      )
    }

    if (page.spans.length < limit) break
    offset += page.spans.length
  }

  return spans
}

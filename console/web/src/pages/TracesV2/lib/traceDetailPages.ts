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

/**
 * Fetch every span of one trace through `fetchPage(offset, limit)`,
 * deduplicated by `span_id`. Stops on a short page (the end, per the
 * verified filtered `total` semantics — also the guard that keeps a stale
 * `total` from looping while a trace is completing).
 */
export async function collectTraceDetailSpans<S extends { span_id: string }>(
  fetchPage: (offset: number, limit: number) => Promise<TraceDetailPage<S>>,
  timeoutMs: number = TRACE_DETAIL_PAGE_TIMEOUT_MS,
): Promise<Map<string, S>> {
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

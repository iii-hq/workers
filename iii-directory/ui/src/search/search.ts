/** Parsed `directory::search_functions` result as the call card renders it. */
export interface DiscoverCandidateView {
  function_id: string
  description: string
}

export interface DiscoverWorkerView {
  namespace: string
  functions: DiscoverCandidateView[]
}

/** A registry worker the search offered as installable: not on the stack,
 * but its functions matched the query. `name` is the registry slug
 * `worker::add` installs. Functions carry names + descriptions only —
 * the worker deliberately withholds schemas so nothing looks callable. */
export interface DiscoverInstallableView {
  name: string
  version: string
  description: string
  functions: DiscoverCandidateView[]
}

export interface DiscoverView {
  guidance: string
  workers: DiscoverWorkerView[]
  installable: DiscoverInstallableView[]
  latency_ms: number
}

/** `{ content: [...], details }` harness result envelope → details.
 * Idempotent: an already-flat payload passes through unchanged. Port of the
 * console's helper — injected bundles copy, not import. */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

/** Error-shaped output (engine `{ error: ... }` wrapper). The renderer
 * returns null for these so the console's default error cards apply. */
export function isErrorOutput(value: unknown): boolean {
  return (
    !!value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    'error' in (value as Record<string, unknown>)
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

/** The request's `query`, tolerating agents that double-encode the payload
 * as a JSON string. Null when no readable query exists. */
export function discoverQuery(input: unknown): string | null {
  let value = input
  if (typeof value === 'string') {
    try {
      value = JSON.parse(value)
    } catch {
      return null
    }
  }
  if (!isRecord(value) || typeof value.query !== 'string') return null
  const query = value.query.trim()
  return query.length > 0 ? query : null
}

/** Strict parse of compact function candidates. Extra legacy contract fields
 * are ignored so settled transcript rows from older workers still render. */
function parseCandidates(value: unknown): DiscoverCandidateView[] | null {
  if (!Array.isArray(value)) return null
  const functions: DiscoverCandidateView[] = []
  for (const fn of value) {
    if (!isRecord(fn)) return null
    if (typeof fn.function_id !== 'string' || fn.function_id.length === 0) return null
    if (typeof fn.description !== 'string') return null
    functions.push({ function_id: fn.function_id, description: fn.description })
  }
  return functions
}

/** Strict parse of a settled discover output (envelope tolerated). Null on
 * any shape drift so the card falls back to the generic JSON panes. The
 * `installable` section is optional — workers predating the registry
 * fallback never send it. */
export function parseDiscoverResponse(output: unknown): DiscoverView | null {
  const value = unwrapEnvelope(output)
  if (!isRecord(value)) return null
  if (typeof value.guidance !== 'string') return null
  if (typeof value.latency_ms !== 'number' || !Number.isFinite(value.latency_ms)) return null
  if (!Array.isArray(value.workers)) return null
  const workers: DiscoverWorkerView[] = []
  for (const worker of value.workers) {
    if (!isRecord(worker)) return null
    if (typeof worker.namespace !== 'string' || worker.namespace.length === 0) return null
    const functions = parseCandidates(worker.functions)
    if (!functions) return null
    workers.push({ namespace: worker.namespace, functions })
  }
  const installable: DiscoverInstallableView[] = []
  if ('installable' in value && value.installable !== undefined) {
    if (!Array.isArray(value.installable)) return null
    for (const candidate of value.installable) {
      if (!isRecord(candidate)) return null
      if (typeof candidate.name !== 'string' || candidate.name.length === 0) return null
      if (typeof candidate.version !== 'string') return null
      if (typeof candidate.description !== 'string') return null
      const functions = parseCandidates(candidate.functions)
      if (!functions) return null
      installable.push({
        name: candidate.name,
        version: candidate.version,
        description: candidate.description,
        functions,
      })
    }
  }
  return { guidance: value.guidance, workers, installable, latency_ms: value.latency_ms }
}

export function functionCount(view: DiscoverView): number {
  return view.workers.reduce((total, worker) => total + worker.functions.length, 0)
}

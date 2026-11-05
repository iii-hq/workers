/**
 * One engine observer row (`engine::observers::list` / `get`), parsed
 * without React or host imports so it can be unit-tested on its own.
 */

/** An outcome of the observed call that fires the target. */
export type Outcome = 'success' | 'error'

const OUTCOMES: readonly Outcome[] = ['success', 'error']

/**
 * The outcomes that fire an observer's target: the engine's `fires` list
 * when it reports one (an array; a number is the legacy activation count),
 * else the legacy `on`.
 */
export function observerOutcomes(fires: unknown, on: unknown): Outcome[] {
  if (Array.isArray(fires)) {
    const listed = OUTCOMES.filter((outcome) => fires.includes(outcome))
    if (listed.length) return listed
  }
  if (on === 'error') return ['error']
  if (on === 'complete') return ['success', 'error']
  return ['success']
}

/** One row of the observer's delivery facts. */
export interface DeliveryFact {
  label: string
  value: string
}

/**
 * What the Observers screen lists under Delivery: the outcomes that fire
 * the target, the legacy raw delivery when an older engine reports one,
 * what is captured, and the policy.
 */
export function observerDeliveryFacts(observer: ObserverInfo): DeliveryFact[] {
  const facts: DeliveryFact[] = [{ label: 'fires on', value: observer.outcomes.join(', ') }]
  if (observer.deliver !== 'envelope') facts.push({ label: 'deliver', value: observer.deliver })
  facts.push(
    {
      label: 'capture',
      value: observer.capture.length ? observer.capture.join(', ') : 'nothing',
    },
    {
      label: 'max fires',
      value: observer.max_fires !== undefined ? String(observer.max_fires) : 'unlimited',
    },
    {
      label: 'expires in',
      value: observer.expires_in_ms !== undefined ? `${observer.expires_in_ms} ms` : 'never',
    },
    {
      label: 'max in flight',
      value:
        observer.max_in_flight !== undefined ? String(observer.max_in_flight) : 'engine default',
    },
  )
  return facts
}

/** One observer, as `engine::observers::list` / `get` report it. */
export interface ObserverInfo {
  id: string
  /** The observed function. */
  function_id: string
  namespace: string
  /** Read by the observed function's worker (a route, a schedule, a topic). */
  args: Record<string, unknown>
  /** Which calls reach the target, normalized. */
  filters: Record<string, unknown>
  target: { function_id: string; namespace: string }
  capture: string[]
  /**
   * Which outcomes call the target. Read from the engine's `fires` list
   * (`observe.capture` engines); older engines report the legacy `on`
   * instead (absent = success, `error`, `complete` = both). Never derived
   * from `capture`.
   */
  outcomes: Outcome[]
  /**
   * Legacy raw delivery an older engine reports (`input` / `output`);
   * `envelope` otherwise, the only delivery an `observe.capture` engine has.
   */
  deliver: string
  /** The worker that created the observer. */
  worker: { worker_id: string; worker_name?: string }
  state: string
  reason?: { code: string; message: string }
  /** Activations so far (the listing's numeric `fires`; 0 when absent). */
  fires: number
  /** Activations dropped, by reason. */
  dropped: Record<string, number>
  activation_failures: Record<string, number>
  max_fires?: number
  expires_in_ms?: number
  max_in_flight?: number
  created_at_ms?: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function num(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function counts(value: unknown): Record<string, number> {
  if (!isRecord(value)) return {}
  const out: Record<string, number> = {}
  for (const [key, n] of Object.entries(value)) {
    if (typeof n === 'number') out[key] = n
  }
  return out
}

export function observerInfo(row: unknown): ObserverInfo | null {
  if (!isRecord(row)) return null
  const id = str(row.id)
  const function_id = str(row.function_id)
  if (!id || !function_id) return null
  const target = isRecord(row.target) ? row.target : {}
  const worker = isRecord(row.worker) ? row.worker : {}
  const reason = isRecord(row.reason) ? row.reason : undefined
  return {
    id,
    function_id,
    namespace: str(row.namespace) ?? 'default',
    args: isRecord(row.args) ? row.args : {},
    filters: isRecord(row.filters) ? row.filters : {},
    target: {
      function_id: str(target.function_id) ?? '',
      namespace: str(target.namespace) ?? 'default',
    },
    capture: Array.isArray(row.capture)
      ? row.capture.filter((c): c is string => typeof c === 'string')
      : [],
    outcomes: observerOutcomes(row.fires, row.on),
    deliver: str(row.deliver) ?? 'envelope',
    worker: {
      worker_id: str(worker.worker_id) ?? '',
      worker_name: str(worker.worker_name),
    },
    state: str(row.state) ?? 'pending',
    reason: reason
      ? { code: str(reason.code) ?? '', message: str(reason.message) ?? '' }
      : undefined,
    fires: num(row.fires) ?? 0,
    dropped: counts(row.dropped),
    activation_failures: counts(row.activation_failures),
    max_fires: num(row.max_fires),
    expires_in_ms: num(row.expires_in_ms),
    max_in_flight: num(row.max_in_flight),
    created_at_ms: num(row.created_at_ms),
  }
}

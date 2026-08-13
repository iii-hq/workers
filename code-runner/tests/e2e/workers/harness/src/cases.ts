import type { IIIClient, InvocationError } from 'iii-sdk'

/**
 * What a case gets.
 *
 * `code-runner::run` with `keep: true` and `code-runner::register_function`
 * both leave state behind, so a case that creates either MUST tear it down —
 * `max_runtimes` is shared across both engines and a leak here fails a later
 * case with a capacity error that says nothing about the real cause.
 */
export interface CaseContext {
  /** Trigger any engine function. Returns parsed JSON; throws on engine error. */
  call: (functionId: string, payload?: unknown) => Promise<any>
  /** Direct SDK access, for anything `call` cannot express. */
  iii: IIIClient
  /**
   * Runs `fn`, asserts it rejects with an error whose MESSAGE mentions
   * `kind`, and returns that error so the caller can go on to check it
   * further. Message, not `code`: the SDK's `code` is its own transport-level
   * one (`invocation_failed` for every handler error) and this worker's
   * `code-runner::…` code travels inside the message.
   */
  expectError: (fn: () => Promise<unknown>, kind: string) => Promise<InvocationError>
}

export interface TestCase {
  name: string
  run(ctx: CaseContext): Promise<void>
}

export interface CaseGroup {
  name: string
  cases: TestCase[]
}

export function expect(cond: boolean, msg: string): asserts cond {
  if (!cond) throw new Error(msg)
}

/**
 * Stable JSON: object keys sorted, array order preserved.
 *
 * A value that round-trips through the engine comes back with its object keys
 * in whatever order the JSON encoders on the way chose, which is not the order
 * it was written in. Comparing raw `JSON.stringify` output therefore fails on
 * equal objects — a false failure that says nothing about the worker. Array
 * order is meaningful and is left alone.
 */
function canonical(v: unknown): string {
  const walk = (x: unknown): unknown => {
    if (Array.isArray(x)) return x.map(walk)
    if (x && typeof x === 'object') {
      return Object.fromEntries(
        Object.keys(x as Record<string, unknown>)
          .sort()
          .map((k) => [k, walk((x as Record<string, unknown>)[k])]),
      )
    }
    return x
  }
  return JSON.stringify(walk(v))
}

export function expectEqual(actual: unknown, expected: unknown, msg: string): void {
  if (canonical(actual) !== canonical(expected)) {
    throw new Error(`${msg}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

export function expectContains(haystack: string, needle: string, msg: string): void {
  if (!haystack.includes(needle)) {
    throw new Error(`${msg}: ${JSON.stringify(needle)} not found in ${JSON.stringify(haystack)}`)
  }
}

/**
 * Poll until `fn()` returns a value, or give up.
 *
 * Registrations reach the bus asynchronously, and even a fixed catalog id
 * like `code-runner::run` is only visible once the worker's own connection
 * has finished registering it — worth polling for rather than assuming. A
 * function published by `register_function` needs the same patience.
 */
export async function until<T>(
  fn: () => Promise<T | undefined>,
  what: string,
  timeoutMs = 8000,
  intervalMs = 100,
): Promise<T> {
  const deadline = Date.now() + timeoutMs
  let last: unknown
  for (;;) {
    try {
      const v = await fn()
      if (v !== undefined) return v
    } catch (e) {
      last = e
    }
    if (Date.now() > deadline) {
      const tail = last ? ` (last error: ${(last as any)?.message ?? last})` : ''
      throw new Error(`timed out after ${timeoutMs}ms waiting for ${what}${tail}`)
    }
    await new Promise((r) => setTimeout(r, intervalMs))
  }
}

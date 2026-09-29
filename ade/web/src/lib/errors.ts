/**
 * One readable string out of whatever a rejected `iii.trigger` throws.
 *
 * The browser SDK rejects with the wire error *object*, not an `Error`, so a
 * naive `String(err)` renders `[object Object]`. The shared
 * `@iii-dev/console-ui/format` implementation unwraps the transport envelope
 * (`handler error: {json}`), keeps the handler's own code and drops the
 * transport-only ones — the same code workers use.
 */
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'

export { errorMessage as errText }

/**
 * Whether a rejected `iii.trigger` says the function is not registered at
 * all — the worker that owns it is not installed (or not up yet). Callers
 * use it to stop a retry ladder: a missing worker does not come back on a
 * timer, it comes back through the `worker` lifecycle trigger.
 */
export function isFunctionNotFound(err: unknown): boolean {
  return errorCode(err)?.toLowerCase() === 'function_not_found'
}

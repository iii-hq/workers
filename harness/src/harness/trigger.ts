/**
 * `harness::trigger` — browser → bus bridge.
 *
 * Accepts `{ function_id, session_id?, message_id?, payload }` (or the same
 * fields at the top level over WS), calls `iii.trigger` for the inner
 * function, and returns `{ status_code, headers, body }`.
 *
 * console/web routes chat turns through this function instead of calling
 * `run::start` directly so `instrumentHandler` can read `session_id` and
 * `message_id` from the outer request and stamp OTel baggage before the
 * nested trigger runs. That keeps "Group by session" / "Group by message"
 * working in the traces UI (`engine::traces::group_by`).
 */

import type { ISdk } from '../runtime/iii.js';
import { unwrapBody } from '../runtime/handler.js';

/** Upper bound for a single harness::trigger. Mirrors the Rust constant. */
const BRIDGE_TIMEOUT_MS = 600_000;

export function register(iii: ISdk): void {
  iii.registerFunction(
    'harness::trigger',
    async (input: unknown) => {
      const body = unwrapBody(input);
      const functionId = body.function_id;
      if (typeof functionId !== 'string' || functionId.length === 0) {
        throw new Error('harness::trigger: missing function_id');
      }
      const inner =
        body.payload && typeof body.payload === 'object'
          ? (body.payload as Record<string, unknown>)
          : {};
      const result = await iii.trigger<unknown, unknown>({
        function_id: functionId,
        payload: inner,
        timeoutMs: BRIDGE_TIMEOUT_MS,
      });
      return {
        status_code: 200,
        headers: { 'content-type': 'application/json' },
        body: result,
      };
    },
    {
      description:
        'Forward {function_id, payload} to iii.trigger and return the result. Used by console/web to reach the bus over the iii-browser-sdk.',
    },
  );
}

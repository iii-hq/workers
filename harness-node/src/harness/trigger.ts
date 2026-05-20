/**
 * Bridge function `harness::trigger`.
 *
 * Port of `workers/harness/src/lib.rs:103-159`. Forwards `{ function_id,
 * payload }` to `iii.trigger` and returns the result wrapped in an
 * HTTP-style envelope (`{ status_code, headers, body }`).
 *
 * Why this exists in the WS path (not just HTTP):
 *
 * The Rust harness exposes the same bridge as an HTTP route and as a
 * bus function (legacy id `harness::call`). The browser-facing pattern
 * (see `workers/harness/web/src/App.tsx:317-340`) routes every chat turn
 * through it so the harness's `instrumentHandler` wrap can seed
 * `iii.session.id` / `iii.message.id` baggage from the OUTER body and
 * downstream worker spans inherit those IDs. Without that, the engine's
 * `engine::traces::group_by` returns empty groups for "Group by
 * session" / "Group by message" in the traces UI.
 *
 * The Node port already wraps every registered function via the
 * `instrumentSdk` Proxy in `runtime/worker.ts`, so the inner trigger's
 * `harness.run::start` span ALREADY gets the IDs from its own payload.
 * Registering `harness::trigger` matches the harness/web pattern explicitly
 * so console/web can use the same wrapper without depending on whether the
 * inner function happens to be auto-wrapped. It also gives us one well-
 * defined ingestion point for browser-originated triggers that keeps the
 * span tree symmetric across Rust and Node deployments.
 *
 * Input shape (HTTP body) — matches the Rust handler:
 *   { function_id, session_id?, message_id?, payload }
 * Falls back to top-level when there is no `body` wrapper (direct WS).
 *
 * The wrapping `instrumentHandler` reads `body.session_id` /
 * `body.message_id` and seeds baggage; the handler here only forwards
 * the trigger.
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

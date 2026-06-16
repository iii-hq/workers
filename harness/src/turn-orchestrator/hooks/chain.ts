/**
 * The pre_trigger hook chain. Consults every bound hook (in priority
 * order) for one agent function call:
 *
 *   - `continue` → next hook (all continue ⇒ allow)
 *   - `deny`     → short-circuit deny with the hook's reason
 *   - `hold`     → short-circuit hold (the call parks awaiting
 *                  `harness::function::resolve`)
 *
 * Transport failures (throw, timeout, unparseable reply) follow the
 * binding's `on_error`: `fail_closed` (default) denies — a crashed gate
 * must not wave calls through; `fail_open` logs and skips the hook.
 *
 * Zero matching bindings ⇒ allow: hooks narrow the trigger policy,
 * never widen it. A deployment without a gate worker is ungated.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { type DenialEnvelope, gateUnavailableEnvelope, hookDenyEnvelope } from './denial.js';
import { snapshotBindings } from './registry.js';
import { HookOutputSchema, type HookInput } from './types.js';

export type HookOutcome =
  | { kind: 'allow' }
  | { kind: 'deny'; denial: DenialEnvelope }
  | { kind: 'hold'; held_by: string; pending_timeout_ms: number };

export async function consultPreTrigger(iii: ISdk, input: HookInput): Promise<HookOutcome> {
  const chain = snapshotBindings(input.call.function_id);

  for (const binding of chain) {
    let output: unknown;
    try {
      output = await iii.trigger<HookInput, unknown>({
        function_id: binding.function_id,
        payload: input,
        timeoutMs: binding.config.timeout_ms,
      });
    } catch (err) {
      const failure = `hook ${binding.function_id} unavailable: ${String(err)}`;
      if (binding.config.on_error === 'fail_open') {
        logger.warn('pre_trigger hook failed; fail_open skips it', {
          hook: binding.function_id,
          function_id: input.call.function_id,
          err: String(err),
        });
        continue;
      }
      logger.warn('pre_trigger hook failed; failing closed', {
        hook: binding.function_id,
        function_id: input.call.function_id,
        err: String(err),
      });
      return {
        kind: 'deny',
        denial: gateUnavailableEnvelope(input.call.function_id, failure),
      };
    }

    const parsed = HookOutputSchema.safeParse(output);
    if (!parsed.success) {
      const failure = `hook ${binding.function_id} returned an unparseable decision`;
      if (binding.config.on_error === 'fail_open') {
        logger.warn('pre_trigger hook reply unparseable; fail_open skips it', {
          hook: binding.function_id,
          function_id: input.call.function_id,
        });
        continue;
      }
      return {
        kind: 'deny',
        denial: gateUnavailableEnvelope(input.call.function_id, failure),
      };
    }

    switch (parsed.data.decision) {
      case 'continue':
        continue;
      case 'deny':
        return {
          kind: 'deny',
          denial: hookDenyEnvelope(input.call.function_id, parsed.data.reason),
        };
      case 'hold':
        return {
          kind: 'hold',
          held_by: binding.function_id,
          pending_timeout_ms: parsed.data.pending_timeout_ms,
        };
    }
  }

  return { kind: 'allow' };
}

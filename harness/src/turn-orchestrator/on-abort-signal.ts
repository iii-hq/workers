/**
 * Reactive abort wake. A `state` trigger on `scope: 'agent'` filtered by
 * the abort_signal key shape (`session/<id>/abort_signal`) and a
 * `new_value === true` write fires this adapter, which publishes
 * `turn::{state}` on the durable FIFO queue so the orchestrator's FSM advances to
 * `steering_check` and observes the abort flag promptly.
 *
 * Without this wake, a session mid-streaming would only check
 * `abort_signal` after the current step completes naturally. The reactive
 * trigger doesn't preempt the running step (durable subscriber publishes
 * queue), but it guarantees the orchestrator runs another FSM step as
 * soon as the current one finishes — which is the earliest moment we
 * can safely react.
 *
 * **Incoming**: agent-scope `state:created` / `state:updated` on
 * `session/<id>/abort_signal` with `new_value === true` (from `state::set` via
 * `performAbortSideEffects` / `router::abort`). Same envelope the engine passes
 * to state trigger adapters.
 *
 * **Outgoing**: `wakeFromRecord` enqueues `{ session_id }` on the `turn-step` queue.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { AbortSignalWriteEventSchema, type ParsedAbortSignalWrite } from './schemas.js';
import { wakeFromRecord } from './wake.js';

export function parseAbortSignalWrite(event: unknown): ParsedAbortSignalWrite | null {
  const result = AbortSignalWriteEventSchema.safeParse(event);
  return result.success ? result.data : null;
}

export function isAbortSignalWrite(event: unknown): boolean {
  return parseAbortSignalWrite(event) !== null;
}

export async function execute(iii: ISdk, write: ParsedAbortSignalWrite): Promise<void> {
  try {
    await wakeFromRecord(iii, write.session_id);
  } catch (err) {
    logger.warn('turn::on_abort_signal: wake failed', {
      session_id: write.session_id,
      err: String(err),
    });
  }
}

export async function handleAbortSignalWrite(iii: ISdk, event: unknown): Promise<void> {
  const write = parseAbortSignalWrite(event);
  if (!write) return;
  await execute(iii, write);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::is_abort_signal_set',
    async (event: unknown) => isAbortSignalWrite(event),
    {
      description:
        'Condition: state event sets session/<id>/abort_signal = true (state:created or state:updated).',
    },
  );

  iii.registerFunction(
    'turn::on_abort_signal',
    async (event: unknown) => handleAbortSignalWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for abort_signal writes; enqueues turn::{state} so the orchestrator picks up the abort promptly.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: 'turn::on_abort_signal',
    config: {
      scope: 'agent',
      condition_function_id: 'turn::is_abort_signal_set',
    },
  });
}

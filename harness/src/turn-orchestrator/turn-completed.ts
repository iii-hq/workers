/**
 * The `harness::turn-completed` trigger type — fired when a turn goes
 * terminal (completed / cancelled / failed). Sibling workers bind to it
 * for cleanup; the approval-gate purges a terminal turn's pending
 * records here instead of polling.
 *
 * Delivery is fire-and-forget, at-least-once, unordered: consumers must
 * treat a duplicate as a no-op (the gate's purge is idempotent), and a
 * lost event is collected by the consumer's own sweep.
 */

import { z } from 'zod';
import { TriggerAction, type ISdk, type TriggerConfig } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnStateRecord } from './state.js';

export const TURN_COMPLETED_TRIGGER_TYPE = 'harness::turn-completed';

export type TurnCompletedStatus = 'completed' | 'cancelled' | 'failed';

export type TurnCompletedEvent = {
  session_id: string;
  turn_id: string;
  status: TurnCompletedStatus;
  reason?: string;
  timestamp: number;
};

const TurnCompletedBindingConfigSchema = z
  .object({
    /** Only deliver events for this session. */
    session_id: z.string().min(1).optional(),
  })
  .strict();
type TurnCompletedBindingConfig = z.infer<typeof TurnCompletedBindingConfigSchema>;

type Binding = {
  id: string;
  function_id: string;
  config: TurnCompletedBindingConfig;
};

const bindings = new Map<string, Binding>();

export function resetTurnCompletedBindingsForTests(): void {
  bindings.clear();
}

export function registerTurnCompletedTriggerType(iii: ISdk): void {
  iii.registerTriggerType<unknown>(
    {
      id: TURN_COMPLETED_TRIGGER_TYPE,
      description:
        'A turn went terminal (status: completed | cancelled | failed). Payload: ' +
        '{session_id, turn_id, status, reason?, timestamp}. Config: {session_id?: string} ' +
        '(equality filter). Delivery is at-least-once and unordered.',
    },
    {
      async registerTrigger(config: TriggerConfig<unknown>) {
        const parsed = TurnCompletedBindingConfigSchema.parse(config.config ?? {});
        bindings.set(config.id, { id: config.id, function_id: config.function_id, config: parsed });
        logger.info('turn_completed trigger bound', {
          id: config.id,
          function_id: config.function_id,
        });
      },
      async unregisterTrigger(config: TriggerConfig<unknown>) {
        bindings.delete(config.id);
        logger.info('turn_completed trigger unbound', { id: config.id });
      },
    },
  );
}

/**
 * Derive the terminal status from the record. `stopped` covers both the
 * natural end and the user abort — the abort path surfaces through the
 * synthetic assistant's stop_reason.
 */
export function terminalStatus(rec: TurnStateRecord): TurnCompletedStatus {
  if (rec.state === 'failed') return 'failed';
  const stopReason = rec.last_assistant?.stop_reason;
  if (stopReason === 'aborted') return 'cancelled';
  if (stopReason === 'error') return 'failed';
  return 'completed';
}

/** Fire-and-forget fan-out; failures are logged and swallowed. */
export async function emitTurnCompleted(iii: ISdk, event: TurnCompletedEvent): Promise<void> {
  for (const binding of bindings.values()) {
    if (binding.config.session_id && binding.config.session_id !== event.session_id) continue;
    try {
      await iii.trigger({
        function_id: binding.function_id,
        payload: event,
        action: TriggerAction.Void(),
      });
    } catch (err) {
      logger.warn('turn_completed fan-out failed', {
        function_id: binding.function_id,
        session_id: event.session_id,
        err: String(err),
      });
    }
  }
}

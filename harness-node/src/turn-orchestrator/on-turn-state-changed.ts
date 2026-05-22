/**
 * State-trigger adapter on `scope: 'agent'` for writes to
 * `session/<id>/turn_state`. Emits `turn_state_changed` on `agent::events`
 * so the UI can derive pending approvals from live state.
 *
 * **Incoming**: agent state write event from the iii engine (`event_type`,
 * `scope`, `key`, `old_value`, `new_value`, `message_type`; key must match
 * `session/<sid>/turn_state`)
 * **Outgoing**: void — side effect via `emit()`; swallow emit failures (log only)
 */

import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emit } from './events.js';

const TurnStateRecordValueSchema = z.object({ state: z.string() }).passthrough();

const AgentTurnStateWriteEventSchema = z.object({
  type: z.literal('state').optional(),
  scope: z.literal('agent').optional(),
  event_type: z.enum(['state:created', 'state:updated']),
  key: z.string().regex(/^session\/[^/]+\/turn_state$/),
  new_value: TurnStateRecordValueSchema,
  old_value: TurnStateRecordValueSchema.nullish(),
});

export const TurnStateWriteEventSchema = AgentTurnStateWriteEventSchema.transform((data) => {
  const session_id = data.key.slice('session/'.length, -'/turn_state'.length);
  return {
    session_id,
    event_type: data.event_type,
    new_value: data.new_value as Record<string, unknown>,
    ...(data.old_value != null && { old_value: data.old_value as Record<string, unknown> }),
  };
});

type ParsedTurnStateWrite = z.infer<typeof TurnStateWriteEventSchema>;

export function parseTurnStateWrite(event: unknown): ParsedTurnStateWrite | null {
  const result = TurnStateWriteEventSchema.safeParse(event);
  return result.success ? result.data : null;
}

export async function handleTurnStateWrite(iii: ISdk, event: unknown): Promise<void> {
  const parsed = parseTurnStateWrite(event);
  if (!parsed) return;

  try {
    await emit(iii, parsed.session_id, {
      type: 'turn_state_changed',
      event_type: parsed.event_type,
      new_value: parsed.new_value,
      ...(parsed.old_value !== undefined && { old_value: parsed.old_value }),
    });
  } catch (err) {
    logger.warn('turn::on_turn_state_changed: emit failed', {
      session_id: parsed.session_id,
      err: String(err),
    });
  }
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::is_turn_state_write',
    async (event: unknown) => parseTurnStateWrite(event) !== null,
    {
      description: 'Condition: state event is a write to session/<sid>/turn_state.',
    },
  );

  iii.registerFunction(
    'turn::on_turn_state_changed',
    async (event: unknown) => handleTurnStateWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for turn_state writes; emits turn_state_changed on agent::events for the subscribed UI.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: 'turn::on_turn_state_changed',
    config: {
      scope: 'agent',
      condition_function_id: 'turn::is_turn_state_write',
    },
  });
}

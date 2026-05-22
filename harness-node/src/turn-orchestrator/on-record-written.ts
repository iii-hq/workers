/**
 * Self-loop wake: a state trigger on `scope: 'agent'` filtered by the
 * turn_state key shape and a stepable state TRANSITION (new state differs
 * from old, non-terminal, non-awaiting) publishes `turn::step_requested`.
 * Saving the record on a real transition is the wake.
 *
 * **Incoming**: agent state write event (`TurnStateWriteEventSchema`) where
 * `new_value.state` is stepable (not `stopped` / `function_awaiting_approval`)
 * and `state:updated` changes `old_value.state`.
 * **Outgoing**: `iii::durable::publish` with
 * `{ topic: 'turn::step_requested', data: { session_id } }` — the durable
 * subscriber receives flat `{ session_id }` only.
 *
 * Same-state writes (e.g. `handlePrepare` calling `saveRecord` while still
 * in `function_prepare` to persist normalized calls) MUST NOT wake step,
 * otherwise the orchestrator races itself: a duplicate `turn::step` runs
 * the same handler again, re-emitting events and re-persisting prepared
 * calls. We filter those out by requiring `new_value.state !== old_value.state`.
 */

import type { ISdk } from '../runtime/iii.js';
import { TurnStateWriteEventSchema } from './on-turn-state-changed.js';
import type { TurnState } from './state.js';

const NON_STEPABLE_STATES = new Set<TurnState>(['stopped', 'function_awaiting_approval']);

export const StepableTurnStateWriteSchema = TurnStateWriteEventSchema.refine(
  (data) => !NON_STEPABLE_STATES.has(data.new_value.state as TurnState),
).refine(
  (data) => data.event_type !== 'state:updated' || data.old_value?.state !== data.new_value.state,
);

export type StepableWrite = { session_id: string };

export function parseStepableWrite(event: unknown): StepableWrite | null {
  const result = StepableTurnStateWriteSchema.safeParse(event);
  if (!result.success) return null;
  return { session_id: result.data.session_id };
}

export async function handleStepableRecordWrite(iii: ISdk, event: unknown): Promise<void> {
  const write = parseStepableWrite(event);
  if (!write) return;
  await iii.trigger<unknown, unknown>({
    function_id: 'iii::durable::publish',
    payload: { topic: 'turn::step_requested', data: { session_id: write.session_id } },
  });
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::is_stepable_record_write',
    async (event: unknown) => parseStepableWrite(event) !== null,
    {
      description:
        'Condition: state event sets session/<id>/turn_state to a stepable state (excludes stopped + function_awaiting_approval).',
    },
  );

  iii.registerFunction(
    'turn::on_record_written',
    async (event: unknown) => handleStepableRecordWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for stepable turn_state writes; publishes turn::step_requested.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: 'turn::on_record_written',
    config: {
      scope: 'agent',
      condition_function_id: 'turn::is_stepable_record_write',
    },
  });
}

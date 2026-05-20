/**
 * Self-loop wake: a state trigger on `scope: 'agent'` filtered by the
 * turn_state key shape and a stepable state TRANSITION (new state differs
 * from old, non-terminal, non-awaiting) invokes `turn::step`. Saving the
 * record on a real transition is the wake — replaces the durable
 * `turn::step_requested` self-publish that used to live in `subscriber.ts`.
 *
 * Same-state writes (e.g. `handlePrepare` calling `saveRecord` while still
 * in `function_prepare` to persist normalized calls) MUST NOT wake step,
 * otherwise the orchestrator races itself: a duplicate `turn::step` runs
 * the same handler again, re-emitting events and re-persisting prepared
 * calls. We filter those out by requiring `new_value.state !== old_value.state`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { TurnStateWriteEventSchema, type ParsedTurnStateWrite } from './on-turn-state-changed.js';
import type { TurnState } from './state.js';

const NON_STEPABLE_STATES = new Set<TurnState>(['stopped', 'function_awaiting_approval']);

const StepableTurnStateWriteSchema = TurnStateWriteEventSchema.refine(
  (data) => !NON_STEPABLE_STATES.has(data.new_value.state as TurnState),
).refine(
  (data) => data.event_type !== 'state:updated' || data.old_value?.state !== data.new_value.state,
);

export type StepableWrite = Pick<ParsedTurnStateWrite, 'session_id'> & { state: TurnState };

export function parseStepableWrite(event: unknown): StepableWrite | null {
  const result = StepableTurnStateWriteSchema.safeParse(event);
  if (!result.success) return null;
  return { session_id: result.data.session_id, state: result.data.new_value.state as TurnState };
}

export async function stepOnStepableWrite(iii: ISdk, write: StepableWrite): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'turn::step',
      payload: { session_id: write.session_id },
    });
    return;
  } catch (err) {
    logger.warn(
      'turn::on_record_written: direct turn::step failed; falling back to durable publish',
      { session_id: write.session_id, err: String(err) },
    );
    try {
      await iii.trigger<unknown, unknown>({
        function_id: 'iii::durable::publish',
        payload: { topic: 'turn::step_requested', data: { session_id: write.session_id } },
      });
    } catch (publishErr) {
      logger.error(
        'turn::on_record_written: durable publish fallback also failed; session may be stuck',
        { session_id: write.session_id, err: String(publishErr) },
      );
    }
  }
}

export async function handleStepableRecordWrite(iii: ISdk, event: unknown): Promise<void> {
  const write = parseStepableWrite(event);
  if (write) {
    await stepOnStepableWrite(iii, write);
  }
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
        'State trigger adapter on scope=agent for stepable turn_state writes; invokes turn::step. Replaces the imperative publishStep self-publish.',
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

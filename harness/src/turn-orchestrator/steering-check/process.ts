/**
 * Drain inboxes, route, apply steering_check outcomes, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { runTransition } from '../run-transition.js';
import { TurnStepPayloadSchema, parseSteeringCheckRecord, type TurnStepPayload } from '../schemas.js';
import type { TurnStateRecord } from '../state.js';
import { createSteeringCheckPorts } from './ports.js';
import { runSteeringCheck } from './run.js';

export async function handleSteering(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const steering = parseSteeringCheckRecord(rec);
  const ports = createSteeringCheckPorts(iii);
  await runSteeringCheck(ports, steering);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::steering_check',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'steering_check', handleSteering, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state steering_check: drain inboxes and route onward.',
    },
  );
}

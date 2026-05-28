/**
 * Register the assistant_streaming FSM step and run one durable transition.
 */

import type { ISdk } from '../../runtime/iii.js';
import { runTransition } from '../run-transition.js';
import {
  TurnStepPayloadSchema,
  parseAssistantStreamingRecord,
  type TurnStepPayload,
} from '../schemas.js';
import type { TurnStateRecord } from '../state.js';
import { createStreamingPorts } from './ports.js';
import { runAssistantStreaming } from './run.js';

export async function handleStreaming(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const streaming = parseAssistantStreamingRecord(rec);
  const ports = createStreamingPorts(iii);
  await runAssistantStreaming(ports, streaming);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::assistant_streaming',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'assistant_streaming', handleStreaming, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state assistant_streaming: start turn, stream provider response, finalize, and route onward.',
    },
  );
}

/**
 * `run::start`. Mirrors `turn-orchestrator/src/run_start.rs`.
 *
 * **Incoming**: flat run request from `harness::trigger` (`body.payload` after
 * `HarnessTriggerInputSchema` parse); console/web sends
 * `{ session_id, message_id?, provider, model, mode?, messages }` and omits
 * `system_prompt`, `image`, `idle_timeout_secs`, `max_turns` (schema defaults).
 * **Outgoing**: `{ session_id }` — persists run config, messages, and seeds
 * `turn_state` to provisioning via `saveRecord`.
 */

import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import type { AgentMessage } from '../types/agent-message.js';
import * as persistence from './persistence.js';
import { newRecord } from './state.js';
import type { Mode } from './system-prompt.js';

export const RunStartPayloadSchema = z.object({
  session_id: z.string().min(1),
  message_id: z.string().optional(),
  provider: z.string(),
  model: z.string(),
  mode: z.enum(['plan', 'ask', 'agent'] satisfies [Mode, Mode, Mode]).optional(),
  messages: z.custom<AgentMessage[]>((v) => Array.isArray(v)).default([]),
  max_turns: z.number().optional(),
  system_prompt: z.string().default(''),
  image: z.string().default('python'),
  idle_timeout_secs: z.number().default(300),
});

export type RunStartPayload = z.infer<typeof RunStartPayloadSchema>;

export type RunStartResult = { session_id: string };

export async function execute(iii: ISdk, payload: RunStartPayload): Promise<RunStartResult> {
  const { session_id, messages, max_turns, message_id: _message_id, ...run } = payload;

  await persistence.saveRunRequest(iii, session_id, {
    ...run,
    mode: run.mode ?? null,
  });
  await persistence.saveMessages(iii, session_id, messages);

  const record = newRecord(session_id, max_turns);
  await persistence.saveRecord(iii, record);
  return { session_id };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'run::start',
    async (payload: unknown) => execute(iii, RunStartPayloadSchema.parse(payload)),
    {
      description: 'Start a durable agent session and return immediately.',
    },
  );
}

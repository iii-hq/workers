/**
 * `run::start` and `run::start_and_wait`. Mirrors
 * `turn-orchestrator/src/run_start.rs`.
 */

import { requireString } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentEvent } from '../types/agent-event.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { TurnOrchestratorConfig } from './config.js';
import { emit } from './events.js';
import * as persistence from './persistence.js';
import { isTerminal, newRecord } from './state.js';

export const FUNCTION_ID = 'run::start';
export const SYNC_FUNCTION_ID = 'run::start_and_wait';
export const STEP_TOPIC = 'turn::step_requested';

export async function publishStep(iii: ISdk, session_id: string): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'iii::durable::publish',
      payload: { topic: STEP_TOPIC, data: { session_id } },
    });
  } catch (err) {
    logger.warn('turn::step_requested publish failed', {
      session_id,
      err: String(err),
    });
  }
}

function buildRunRequest(payload: Record<string, unknown>): Record<string, unknown> {
  return {
    provider: payload.provider ?? '',
    model: payload.model ?? '',
    system_prompt: payload.system_prompt ?? '',
    mode: payload.mode ?? null,
    approval_required: Array.isArray(payload.approval_required) ? payload.approval_required : [],
    image: payload.image ?? 'python',
    idle_timeout_secs: payload.idle_timeout_secs ?? 300,
    cwd: payload.cwd ?? null,
    cwd_hash: payload.cwd_hash ?? null,
  };
}

function buildInitialEventPlan(messages: AgentMessage[]): AgentEvent[] {
  const plan: AgentEvent[] = [{ type: 'agent_start' }];
  for (const m of messages) {
    plan.push({ type: 'message_start', message: m });
    plan.push({ type: 'message_end', message: m });
  }
  return plan;
}

export async function execute(iii: ISdk, payload: unknown): Promise<{ session_id: string }> {
  const obj = (payload ?? {}) as Record<string, unknown>;
  const session_id = requireString(obj, 'session_id');
  const max_turns = typeof obj.max_turns === 'number' ? obj.max_turns : undefined;
  const request = buildRunRequest(obj);
  const initial_messages = Array.isArray(obj.messages) ? (obj.messages as AgentMessage[]) : [];

  await persistence.saveRunRequest(iii, session_id, request);
  await persistence.saveMessages(iii, session_id, initial_messages);
  const record = newRecord(session_id, max_turns);
  await persistence.saveRecord(iii, record);

  if (typeof request.cwd === 'string') {
    await persistence.saveCwd(iii, session_id, request.cwd as string);
    if (typeof request.cwd_hash === 'string') {
      await persistence.saveCwdIndex(iii, request.cwd_hash as string, session_id);
    }
  }

  for (const evt of buildInitialEventPlan(initial_messages)) {
    await emit(iii, session_id, evt);
  }
  await publishStep(iii, session_id);
  return { session_id };
}

export async function executeSync(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  payload: unknown,
): Promise<{ session_id: string; messages: AgentMessage[]; turn_count: number }> {
  const obj = (payload ?? {}) as Record<string, unknown>;
  const timeout_ms =
    typeof obj.timeout_ms === 'number' ? obj.timeout_ms : cfg.sync_default_timeout_ms;
  const { session_id } = await execute(iii, payload);
  const deadline = Date.now() + timeout_ms;
  for (;;) {
    const rec = await persistence.loadRecord(iii, session_id);
    if (rec && isTerminal(rec)) {
      const messages = await persistence.loadMessages(iii, session_id);
      return { session_id, messages, turn_count: rec.turn_count };
    }
    if (Date.now() >= deadline) {
      throw new Error(`run::start_and_wait timed out after ${timeout_ms} ms`);
    }
    await sleep(cfg.sync_poll_interval_ms);
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export function register(iii: ISdk, cfg: TurnOrchestratorConfig): void {
  iii.registerFunction(FUNCTION_ID, async (payload: unknown) => execute(iii, payload), {
    description: 'Start a durable agent session and return immediately.',
  });
  iii.registerFunction(
    SYNC_FUNCTION_ID,
    async (payload: unknown) => executeSync(iii, cfg, payload),
    {
      description: 'Start a durable agent session and block until terminal (test/dev convenience).',
    },
  );
}

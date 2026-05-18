/**
 * `turn::step` durable subscriber. Mirrors
 * `turn-orchestrator/src/subscriber.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnOrchestratorConfig } from './config.js';
import * as persistence from './persistence.js';
import { publishStep } from './run-start.js';
import { isTerminal } from './state.js';
import { step } from './transitions.js';

export const STEP_FN_ID = 'turn::step';
const STEP_TOPIC = 'turn::step_requested';

function extractSessionId(payload: unknown): string | null {
  if (!payload || typeof payload !== 'object') return null;
  const obj = payload as Record<string, unknown>;
  const inner =
    obj.payload && typeof obj.payload === 'object'
      ? (obj.payload as Record<string, unknown>)
      : obj.data && typeof obj.data === 'object'
        ? (obj.data as Record<string, unknown>)
        : obj;
  return typeof inner.session_id === 'string' ? inner.session_id : null;
}

export async function execute(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  payload: unknown,
): Promise<unknown> {
  const session_id = extractSessionId(payload);
  if (!session_id) {
    throw new Error('turn::step_requested payload missing session_id');
  }
  const rec = await persistence.loadRecord(iii, session_id);
  if (!rec) {
    logger.warn('turn::step_requested for unknown session', { session_id });
    return { ok: false, reason: 'unknown_session' };
  }
  if (isTerminal(rec)) {
    return { ok: true, terminal: true };
  }
  const from_state = rec.state;
  try {
    await step(iii, cfg, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  if (!isTerminal(rec) && rec.state !== 'function_awaiting_approval') {
    await publishStep(iii, session_id);
  }
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk, cfg: TurnOrchestratorConfig): void {
  iii.registerFunction(STEP_FN_ID, async (payload: unknown) => execute(iii, cfg, payload), {
    description: 'Run one durable state machine transition for a session.',
  });
  iii.registerTrigger({
    type: 'durable:subscriber',
    function_id: STEP_FN_ID,
    config: { topic: STEP_TOPIC },
  });
}

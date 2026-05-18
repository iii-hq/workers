/**
 * `turn::step` durable subscriber. Mirrors
 * `turn-orchestrator/src/subscriber.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnOrchestratorConfig } from './config.js';
import * as persistence from './persistence.js';
import { buildResumePlan, publishStep } from './run-start.js';
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
  const loaded = await persistence.loadRecord(iii, session_id);
  if (!loaded) {
    logger.warn('turn::step_requested for unknown session', { session_id });
    return { ok: false, reason: 'unknown_session' };
  }
  // PR #150: a step request for a terminal record means something
  // (typically approval-gate's approval::resolve via iii::durable::publish)
  // wants this paused session to resume. Rebuild a fresh provisioning
  // record in place (preserving turn_count + max_turns) and walk the FSM
  // from there. This replaces the old polling resume_session loop.
  let rec = loaded;
  let resumed_from_terminal = false;
  if (isTerminal(rec)) {
    const plan = buildResumePlan(rec);
    if (!plan) return { ok: true, terminal: true };
    rec = plan;
    resumed_from_terminal = true;
    await persistence.saveRecord(iii, rec);
  }
  const from_state = rec.state;
  try {
    await step(iii, cfg, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  if (!isTerminal(rec)) await publishStep(iii, session_id);
  return { ok: true, from_state, to_state: rec.state, resumed_from_terminal };
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

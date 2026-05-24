/**
 * `turn::provisioning`. First FSM step after `run::start`: materialize tool schemas,
 * assemble the system prompt, persist the enriched run request, then advance.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { agentTriggerTool } from '../agent-trigger.js';
import type { TurnOrchestratorConfig } from '../config.js';
import * as persistence from '../persistence.js';
import { type RunRequest } from '../run-request.js';
import { runTransition } from '../run-transition.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { type DefaultSkillBody, buildSystemPrompt, defaultSkillBody } from '../system-prompt.js';

const FETCH_TIMEOUT_MS = 10_000;

export function parseDirectoryBody(resp: unknown): string | null {
  if (typeof resp === 'string') return resp;
  if (resp && typeof resp === 'object') {
    const body = (resp as { body?: unknown }).body;
    if (typeof body === 'string') return body;
  }
  return null;
}

async function fetchSkill(iii: ISdk, id: string): Promise<string | null> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: 'directory::skills::get',
      payload: { id },
      timeoutMs: FETCH_TIMEOUT_MS,
    });
    return parseDirectoryBody(resp);
  } catch (err) {
    logger.warn('directory::skills::get failed', { id, err: String(err) });
    return null;
  }
}

async function fetchDefaultSkills(iii: ISdk, uris: readonly string[]): Promise<DefaultSkillBody[]> {
  const bodies: DefaultSkillBody[] = [];
  for (const uri of uris) {
    const id = uri.startsWith('iii://') ? uri.slice('iii://'.length) : uri;
    const body = await fetchSkill(iii, id);
    bodies.push(defaultSkillBody(uri, body));
  }
  return bodies;
}

async function fetchSkillsIndex(iii: ISdk): Promise<string | null> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: 'directory::skills::index',
      payload: {},
      timeoutMs: FETCH_TIMEOUT_MS,
    });
    const body = parseDirectoryBody(resp);
    return body && body.length > 0 ? body : null;
  } catch (err) {
    logger.warn('directory::skills::index failed', { err: String(err) });
    return null;
  }
}

export async function handleProvisioning(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  rec: TurnStateRecord,
): Promise<void> {
  const request = await persistence.loadRunRequest(iii, rec.session_id);

  const override = request.system_prompt.length > 0 ? request.system_prompt : null;

  const [skillsIndex, bodies] = await Promise.all([
    fetchSkillsIndex(iii),
    fetchDefaultSkills(iii, cfg.system_default_skills),
  ]);
  const prompt = buildSystemPrompt(bodies, null, override, request.mode, skillsIndex);

  const updated: RunRequest = { ...request, system_prompt: prompt, function_schemas: [agentTriggerTool()] };
  await persistence.saveRunRequest(iii, rec.session_id, updated);

  transitionTo(rec, 'assistant_streaming');
}

export function register(iii: ISdk, cfg: TurnOrchestratorConfig): void {
  iii.registerFunction(
    'turn::provisioning',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(
        iii,
        'provisioning',
        (i, rec) => handleProvisioning(i, cfg, rec),
        parsed,
      );
    },
    {
      description:
        'Run one durable FSM transition for session in state provisioning: materialize tool schemas, build system prompt, advance to assistant_streaming.',
    },
  );
}
